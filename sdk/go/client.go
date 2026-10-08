// Package geoledger provides a transport-independent spatial version client.
package geoledger

import (
	"context"
	"crypto/tls"
	"encoding/json"
	"errors"
	"fmt"
	pb "github.com/mapseekai/geoledger/sdk/go/internal/geoledgerv1"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/credentials"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/proto"
	"math"
	"net"
	"net/url"
	"os"
	"strings"
	"sync/atomic"
	"time"
)

// Error is the stable business error contract; no transport imports are needed.
type Error struct {
	Code      string     `json:"code"`
	Message   string     `json:"message"`
	RequestID string     `json:"request_id,omitempty"`
	Conflicts *Conflicts `json:"conflicts,omitempty"`
	Uncertain bool       `json:"uncertain"`
}

func (e *Error) Error() string      { return e.Code + ": " + e.Message }
func invalid(message string) *Error { return &Error{Code: "invalid_argument", Message: message} }
func failure(err error) error {
	var own *Error
	if errors.As(err, &own) {
		return own
	}
	s := status.Convert(err)
	uncertain := false
	switch s.Code() {
	case codes.Canceled, codes.Unknown, codes.DeadlineExceeded, codes.ResourceExhausted, codes.Internal, codes.Unavailable, codes.DataLoss:
		uncertain = true
	}
	code := "unavailable"
	switch s.Code() {
	case codes.Unauthenticated:
		code = "unauthenticated"
	case codes.PermissionDenied:
		code = "permission_denied"
	case codes.NotFound:
		code = "not_found"
	case codes.Aborted, codes.AlreadyExists, codes.FailedPrecondition:
		code = "conflict"
	case codes.InvalidArgument, codes.OutOfRange:
		code = "invalid_argument"
	case codes.DeadlineExceeded:
		code = "timeout"
	case codes.Canceled:
		code = "cancelled"
	case codes.ResourceExhausted:
		code = "resource_exhausted"
	}
	result := &Error{Code: code, Message: s.Message(), Uncertain: uncertain}
	for _, item := range s.Details() {
		if detail, ok := item.(*pb.ErrorDetail); ok {
			result.Code = detail.Code
			result.Message = detail.Message
			result.RequestID = detail.RequestId
			if detail.Conflicts != nil {
				var conflicts Conflicts
				if decode(detail.Conflicts, &conflicts) == nil {
					result.Conflicts = &conflicts
				}
			}
		}
	}
	return result
}

type Client struct {
	rpc    pb.GeoLedgerClient
	conn   *grpc.ClientConn
	closed atomic.Bool
}

// DialOptions configures a client. Plaintext http:// is accepted only for loopback hosts
// unless AllowInsecure is set (or GL_ALLOW_INSECURE_TRANSPORT=true): the bearer token would
// otherwise cross the network unencrypted.
type DialOptions struct {
	Timeout       time.Duration
	AllowInsecure bool
}

// IsLoopbackHost reports whether host is localhost or a loopback IP literal.
func IsLoopbackHost(host string) bool {
	host = strings.Trim(host, "[]")
	if strings.EqualFold(host, "localhost") {
		return true
	}
	ip := net.ParseIP(host)
	return ip != nil && ip.IsLoopback()
}

func insecureOptIn() bool {
	switch os.Getenv("GL_ALLOW_INSECURE_TRANSPORT") {
	case "1", "true", "yes":
		return true
	}
	return false
}

// Dial accepts a service URL and a token. It never retries writes automatically.
func Dial(endpoint, token string) (*Client, error) {
	return DialWithOptions(endpoint, token, DialOptions{Timeout: 30 * time.Second})
}
func DialWithTimeout(endpoint, token string, timeout time.Duration) (*Client, error) {
	return DialWithOptions(endpoint, token, DialOptions{Timeout: timeout})
}
func DialWithOptions(endpoint, token string, options DialOptions) (*Client, error) {
	timeout := options.Timeout
	u, err := url.Parse(endpoint)
	if err != nil || u.Hostname() == "" || u.Port() == "" || (u.Scheme != "http" && u.Scheme != "https") || (u.Path != "" && u.Path != "/") || u.RawQuery != "" || u.Fragment != "" || u.User != nil {
		return nil, invalid("endpoint must be http(s)://host:port")
	}
	if token == "" || timeout <= 0 || strings.IndexFunc(token, func(r rune) bool { return r < 32 || r > 126 }) >= 0 {
		return nil, invalid("ASCII token and positive timeout required")
	}
	if u.Scheme == "http" && !IsLoopbackHost(u.Hostname()) && !options.AllowInsecure && !insecureOptIn() {
		return nil, invalid("refusing to send credentials over plaintext http to a non-loopback host; use https or set AllowInsecure / GL_ALLOW_INSECURE_TRANSPORT=true")
	}
	var transport credentials.TransportCredentials
	if u.Scheme == "https" {
		transport = credentials.NewTLS(&tls.Config{MinVersion: tls.VersionTLS12})
	} else {
		transport = insecure.NewCredentials()
	}
	auth := func(ctx context.Context, method string, req, reply any, cc *grpc.ClientConn, invoker grpc.UnaryInvoker, opts ...grpc.CallOption) error {
		if _, ok := ctx.Deadline(); !ok {
			var cancel context.CancelFunc
			ctx, cancel = context.WithTimeout(ctx, timeout)
			defer cancel()
		}
		md, _ := metadata.FromOutgoingContext(ctx)
		md = md.Copy()
		md.Set("authorization", "Bearer "+token)
		return invoker(metadata.NewOutgoingContext(ctx, md), method, req, reply, cc, opts...)
	}
	conn, err := grpc.NewClient(u.Host, grpc.WithTransportCredentials(transport), grpc.WithUnaryInterceptor(auth), grpc.WithDisableRetry(), grpc.WithDefaultCallOptions(grpc.MaxCallRecvMsgSize(4<<20), grpc.MaxCallSendMsgSize(4<<20)))
	if err != nil {
		return nil, &Error{Code: "unavailable", Message: "could not initialize client"}
	}
	return &Client{rpc: pb.NewGeoLedgerClient(conn), conn: conn}, nil
}
func (c *Client) Close() error { c.closed.Store(true); return c.conn.Close() }
func call[Request any, Reply proto.Message](c *Client, ctx context.Context, rpc func(context.Context, *Request, ...grpc.CallOption) (Reply, error), request *Request, output any) error {
	if ctx == nil {
		return invalid("context is required")
	}
	if c.closed.Load() {
		return invalid("client is closed")
	}
	reply, err := rpc(ctx, request)
	if err != nil {
		return failure(err)
	}
	if err = decode(reply, output); err != nil {
		return &Error{Code: "invalid_response", Message: "invalid data received from server", Uncertain: true}
	}
	return nil
}
func wireEdits(edits []Edit) ([]*pb.Edit, error) {
	result := make([]*pb.Edit, 0, len(edits))
	for _, edit := range edits {
		if len(edit.Feature) == 0 {
			return nil, invalid("edit requires an explicit feature; use null for deletion")
		}
		if !json.Valid(edit.Feature) {
			return nil, invalid("invalid GeoJSON")
		}
		var feature *pb.Feature
		if strings.TrimSpace(string(edit.Feature)) != "null" {
			feature = &pb.Feature{Geojson: string(edit.Feature)}
		}
		result = append(result, &pb.Edit{Dataset: edit.Dataset, FeatureId: edit.FeatureID, Feature: feature})
	}
	return result, nil
}
func decode(reply proto.Message, out any) error {
	raw, err := json.Marshal(reply)
	if err != nil {
		return err
	}
	var obj map[string]json.RawMessage
	if err = json.Unmarshal(raw, &obj); err != nil {
		return err
	}
	feature := func(raw json.RawMessage) (json.RawMessage, error) {
		if len(raw) == 0 || string(raw) == "null" {
			return raw, nil
		}
		var f struct {
			GeoJSON string `json:"geojson"`
		}
		if err := json.Unmarshal(raw, &f); err != nil {
			return nil, err
		}
		if !json.Valid([]byte(f.GeoJSON)) {
			return nil, fmt.Errorf("invalid feature")
		}
		return json.RawMessage(f.GeoJSON), nil
	}
	if raw, ok := obj["features"]; ok {
		var rows []json.RawMessage
		if err = json.Unmarshal(raw, &rows); err != nil {
			return err
		}
		for i := range rows {
			rows[i], err = feature(rows[i])
			if err != nil {
				return err
			}
		}
		obj["features"], err = json.Marshal(rows)
		if err != nil {
			return err
		}
	}
	for _, key := range []string{"changes", "conflicts"} {
		if raw, ok := obj[key]; ok && len(raw) > 0 && raw[0] == '[' {
			var rows []map[string]json.RawMessage
			if err = json.Unmarshal(raw, &rows); err != nil {
				return err
			}
			for _, row := range rows {
				for _, field := range []string{"base", "current", "draft", "before", "after"} {
					if v, ok := row[field]; ok {
						row[field], err = feature(v)
						if err != nil {
							return err
						}
					}
				}
			}
			obj[key], err = json.Marshal(rows)
			if err != nil {
				return err
			}
		}
	}
	if raw, ok := obj["events"]; ok {
		var rows []map[string]json.RawMessage
		if err = json.Unmarshal(raw, &rows); err != nil {
			return err
		}
		for _, row := range rows {
			var text string
			if err = json.Unmarshal(row["detail_json"], &text); err != nil {
				return err
			}
			if !json.Valid([]byte(text)) {
				return fmt.Errorf("invalid audit detail")
			}
			row["detail_json"] = json.RawMessage(text)
		}
		obj["events"], err = json.Marshal(rows)
		if err != nil {
			return err
		}
	}
	raw, err = json.Marshal(obj)
	if err != nil {
		return err
	}
	return json.Unmarshal(raw, out)
}
func (c *Client) Info(ctx context.Context) (ServerInfo, error) {
	var r ServerInfo
	e := call(c, ctx, c.rpc.Info, &pb.Empty{}, &r)
	return r, e
}
func (c *Client) CreateProject(ctx context.Context, name string) (Project, error) {
	var r Project
	e := call(c, ctx, c.rpc.CreateProject, &pb.NameRequest{Name: name}, &r)
	return r, e
}
func (c *Client) Project(ctx context.Context, id string) (Project, error) {
	var r Project
	e := call(c, ctx, c.rpc.GetProject, &pb.ProjectRequest{Project: id}, &r)
	return r, e
}
func (c *Client) Projects(ctx context.Context, page Page) ([]Project, error) {
	var r struct {
		Projects []Project `json:"projects"`
	}
	e := call(c, ctx, c.rpc.ListProjects, &pb.PageRequest{After: page.After, Limit: limit(page.Limit)}, &r)
	return r.Projects, e
}
func (c *Client) SetMember(ctx context.Context, project, subject, role string) error {
	var r struct{ OK bool }
	return call(c, ctx, c.rpc.SetMember, &pb.MemberRequest{Project: project, Subject: subject, Role: role}, &r)
}
func (c *Client) CreateDataset(ctx context.Context, project, name string) (Dataset, error) {
	var r Dataset
	e := call(c, ctx, c.rpc.CreateDataset, &pb.DatasetRequest{Project: project, Name: name}, &r)
	return r, e
}
func (c *Client) Datasets(ctx context.Context, project string, page Page) ([]Dataset, error) {
	var r struct {
		Datasets []Dataset `json:"datasets"`
	}
	e := call(c, ctx, c.rpc.ListDatasets, &pb.ProjectPageRequest{Project: project, After: page.After, Limit: limit(page.Limit)}, &r)
	return r.Datasets, e
}
func limit(n int64) *int64 {
	if n == 0 {
		return nil
	}
	return &n
}
func (c *Client) CreateWorkspace(ctx context.Context, project string) (*Workspace, error) {
	var info WorkspaceInfo
	if err := call(c, ctx, c.rpc.CreateWorkspace, &pb.ProjectRequest{Project: project}, &info); err != nil {
		return nil, err
	}
	return &Workspace{client: c, project: project, info: info}, nil
}
func (c *Client) Workspace(ctx context.Context, project, id string) (*Workspace, error) {
	var info WorkspaceInfo
	if err := call(c, ctx, c.rpc.GetWorkspace, &pb.WorkspaceRequest{Project: project, Workspace: id}, &info); err != nil {
		return nil, err
	}
	return &Workspace{client: c, project: project, info: info}, nil
}
func (c *Client) Workspaces(ctx context.Context, project string, page Page) ([]WorkspaceInfo, error) {
	var r struct {
		Workspaces []WorkspaceInfo `json:"workspaces"`
	}
	e := call(c, ctx, c.rpc.ListWorkspaces, &pb.ProjectPageRequest{Project: project, After: page.After, Limit: limit(page.Limit)}, &r)
	return r.Workspaces, e
}
func (c *Client) Features(ctx context.Context, project, dataset string, query FeatureQuery) (FeaturePage, error) {
	var r FeaturePage
	for _, value := range query.Bbox {
		if math.IsNaN(value) || math.IsInf(value, 0) {
			return r, invalid("invalid JSON request")
		}
	}
	e := call(c, ctx, c.rpc.Features, &pb.FeaturesRequest{Project: project, Dataset: dataset, Workspace: query.Workspace, Revision: query.Revision, FeatureId: query.FeatureID, Bbox: query.Bbox, After: query.After, Limit: limit(query.Limit)}, &r)
	return r, e
}
func (c *Client) History(ctx context.Context, project string, after int64, n int64) ([]Commit, error) {
	var r struct {
		Commits []Commit `json:"commits"`
	}
	e := call(c, ctx, c.rpc.History, &pb.HistoryRequest{Project: project, After: after, Limit: limit(n)}, &r)
	return r.Commits, e
}
func (c *Client) Commit(ctx context.Context, project string, revision int64, page Page) (CommitChanges, error) {
	var r CommitChanges
	e := call(c, ctx, c.rpc.Commit, &pb.CommitRequest{Project: project, Revision: revision, After: page.After, Limit: limit(page.Limit)}, &r)
	return r, e
}
func (c *Client) Audit(ctx context.Context, project string, after, n int64) (AuditPage, error) {
	var r AuditPage
	e := call(c, ctx, c.rpc.Audit, &pb.HistoryRequest{Project: project, After: after, Limit: limit(n)}, &r)
	return r, e
}
func (c *Client) Publish(ctx context.Context, intent Publication) (PublicationResult, error) {
	var r PublicationResult
	e := call(c, ctx, c.rpc.Publish, &pb.PublishRequest{Project: intent.Project, Workspace: intent.Workspace, ExpectedWorkspaceVersion: intent.ExpectedWorkspaceVersion, RequestId: intent.RequestID, Message: intent.Message}, &r)
	return r, e
}
func (c *Client) Restore(ctx context.Context, project string, revision int64) (*Workspace, error) {
	var info WorkspaceInfo
	if err := call(c, ctx, c.rpc.Restore, &pb.RestoreRequest{Project: project, Revision: revision}, &info); err != nil {
		return nil, err
	}
	return &Workspace{client: c, project: project, info: info}, nil
}
