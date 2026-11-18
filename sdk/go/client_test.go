package geoledger

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"strings"
	"testing"
	"time"

	pb "github.com/mapseekai/geoledger/sdk/go/internal/geoledgerv1"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/proto"
	"google.golang.org/protobuf/types/known/anypb"
)

func TestRegisterTablePreservesCreateDatasetWireContract(t *testing.T) {
	source := PostgisTable{Schema: "business", Table: "roads", IDColumn: "road_id", GeometryColumn: "shape"}
	rpc := &datasetRPC{}
	client := &Client{rpc: rpc}
	dataset, err := client.RegisterTable(context.Background(), "project", "roads", source)
	if err != nil {
		t.Fatal(err)
	}
	want := &pb.DatasetRequest{Project: "project", Name: "roads", PostgisTable: &pb.PostgisTable{Schema: source.Schema, Table: source.Table, IdColumn: source.IDColumn, GeometryColumn: source.GeometryColumn}}
	if !proto.Equal(rpc.request, want) {
		t.Fatalf("registration changed the wire request: %v", rpc.request)
	}
	if dataset.ID != "dataset" || dataset.GeometryType != "line" || dataset.CoordinateDimension != 3 || dataset.CRS != "EPSG:3857" || dataset.PostgisTable == nil || *dataset.PostgisTable != source {
		t.Fatalf("registration lost inferred source metadata: %+v", dataset)
	}
	rpc.err = status.Error(codes.PermissionDenied, "registration denied")
	_, err = client.RegisterTable(context.Background(), "project", "roads", source)
	var failure *Error
	if !errors.As(err, &failure) || failure.Code != "permission_denied" || failure.Uncertain {
		t.Fatalf("registration changed permission errors: %v", err)
	}
}

func TestOpaquePropertiesAndExactNumbersSurviveReplyConversion(t *testing.T) {
	raw := `{"type":"Feature","id":"one","geometry":null,"properties":{"nested":{"geojson":"ordinary text"},"detail_json":"plain","exact":9007199254740993,"max":18446744073709551615}}`
	var page FeaturePage
	if err := decode(&pb.FeaturesReply{Features: []*pb.Feature{{Geojson: raw}}, Revision: 1}, &page); err != nil {
		t.Fatal(err)
	}
	if len(page.Features) != 1 || string(page.Features[0]) != raw {
		t.Fatalf("feature was modified: %s", page.Features)
	}
	var event AuditPage
	if err := decode(&pb.AuditReply{Events: []*pb.AuditEvent{{Id: 1, DetailJson: `{"nested":{"geojson":"ordinary text"},"detail_json":"plain"}`}}}, &event); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(event.Events[0].Detail), `"detail_json":"plain"`) {
		t.Fatal("nested business data was mistaken for a wire envelope")
	}
}

func TestPlaintextRemoteRequiresOptIn(t *testing.T) {
	t.Setenv("GL_ALLOW_INSECURE_TRANSPORT", "")
	if _, err := Dial("http://geoledger.example:7882", "glk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"); err == nil || err.(*Error).Code != "invalid_argument" {
		t.Fatalf("expected refusal, got %v", err)
	}
	c, err := DialWithOptions("http://geoledger.example:7882", "glk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", DialOptions{Timeout: time.Second, AllowInsecure: true})
	if err != nil {
		t.Fatal(err)
	}
	_ = c.Close()
	for host, want := range map[string]bool{"127.0.0.1": true, "::1": true, "localhost": true, "10.1.2.3": false} {
		if IsLoopbackHost(host) != want {
			t.Fatalf("%s", host)
		}
	}
}

func TestQueryPageContinuationsSurviveReplyConversion(t *testing.T) {
	for _, cursor := range []*string{nil, new("dataset/key/with/slashes")} {
		var diff Diff
		if err := decode(&pb.DiffReply{NextAfter: cursor}, &diff); err != nil {
			t.Fatal(err)
		}
		var commit CommitChanges
		if err := decode(&pb.CommitReply{NextAfter: cursor}, &commit); err != nil {
			t.Fatal(err)
		}
		for _, got := range []*string{diff.NextAfter, commit.NextAfter} {
			if (got == nil) != (cursor == nil) || got != nil && *got != *cursor {
				t.Fatalf("cursor = %v, want %v", got, cursor)
			}
		}
	}
	for _, cursor := range []*int64{nil, new(int64(9007199254740993))} {
		var history HistoryPage
		if err := decode(&pb.HistoryReply{NextAfter: cursor}, &history); err != nil {
			t.Fatal(err)
		}
		var audit AuditPage
		if err := decode(&pb.AuditReply{NextAfter: cursor}, &audit); err != nil {
			t.Fatal(err)
		}
		for _, got := range []*int64{history.NextAfter, audit.NextAfter} {
			if (got == nil) != (cursor == nil) || got != nil && *got != *cursor {
				t.Fatalf("cursor = %v, want %v", got, cursor)
			}
		}
	}
}

func ExampleDial() {
	// Dial is lazy: this example does not contact a service.
	client, err := Dial("http://127.0.0.1:7882", "glk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
	if err != nil {
		log.Fatal(err)
	}
	defer client.Close()
	fmt.Println(client != nil)
	// Output: true
}

// datasetRPC substitutes only the private RPC used by table registration.
type datasetRPC struct {
	pb.GeoLedgerClient
	request *pb.DatasetRequest
	err     error
}

func (s *datasetRPC) CreateDataset(_ context.Context, request *pb.DatasetRequest, _ ...grpc.CallOption) (*pb.DatasetReply, error) {
	s.request = proto.Clone(request).(*pb.DatasetRequest)
	if s.err != nil {
		return nil, s.err
	}
	return &pb.DatasetReply{Dataset: "dataset", Name: request.Name, GeometryType: "line", CoordinateDimension: 3, Crs: "EPSG:3857", PostgisTable: request.PostgisTable}, nil
}

func TestSchemaChangesSurviveDiffAndCommitConversion(t *testing.T) {
	change := &pb.SchemaChange{Dataset: "d", Before: []*pb.SchemaField{}, After: []*pb.SchemaField{{Name: "note", Type: "text", Nullable: true, Editable: true}}}
	var diff Diff
	if err := decode(&pb.DiffReply{SchemaChanges: []*pb.SchemaChange{change}}, &diff); err != nil {
		t.Fatal(err)
	}
	var commit CommitChanges
	if err := decode(&pb.CommitReply{SchemaChanges: []*pb.SchemaChange{change}}, &commit); err != nil {
		t.Fatal(err)
	}
	if len(diff.SchemaChanges) != 1 || len(commit.SchemaChanges) != 1 || diff.SchemaChanges[0].After[0].Type != "text" {
		t.Fatal("schema-only changes were hidden")
	}
}

func TestSchemaCollectionsMarshalAsEmptyArrays(t *testing.T) {
	var schema DatasetSchema
	if err := decode(&pb.SchemaReply{}, &schema); err != nil {
		t.Fatal(err)
	}
	raw, err := json.Marshal(schema)
	if err != nil || !strings.Contains(string(raw), `"fields":[]`) {
		t.Fatalf("empty schema fields became null: %s %v", raw, err)
	}
	var diff Diff
	if err := decode(&pb.DiffReply{SchemaChanges: []*pb.SchemaChange{{Dataset: "d"}}}, &diff); err != nil {
		t.Fatal(err)
	}
	raw, err = json.Marshal(diff)
	if err != nil || !strings.Contains(string(raw), `"before":[]`) || !strings.Contains(string(raw), `"after":[]`) {
		t.Fatalf("schema change arrays became null: %s %v", raw, err)
	}
}

func TestTransportDeadlineAndRangeErrorsKeepCanonicalCodesAndUncertainty(t *testing.T) {
	for _, test := range []struct {
		status codes.Code
		code   string
	}{
		{codes.DeadlineExceeded, "deadline_exceeded"},
		{codes.OutOfRange, "resource_exhausted"},
	} {
		requireBusinessError(t, failure(status.Error(test.status, "transport failure")), test.code, true)
		detailed, err := status.New(test.status, "transport failure").WithDetails(&pb.ErrorDetail{
			Code: "business_range", Message: "business rejection", RequestId: "request",
		})
		if err != nil {
			t.Fatal(err)
		}
		requireBusinessError(t, failure(detailed.Err()), "business_range", true)
	}
}

func TestUndetailedConflictFamilyCannotClaimWorkspaceVersionProof(t *testing.T) {
	for _, code := range []codes.Code{codes.Aborted, codes.AlreadyExists, codes.FailedPrecondition} {
		for _, kind := range []string{"absent", "malformed", "empty code", "wrong type"} {
			t.Run(code.String()+"/"+kind, func(t *testing.T) {
				envelope := status.New(code, "proxy rejection").Proto()
				if kind != "absent" {
					detail := &pb.ErrorDetail{Code: "workspace_version_conflict", Message: "not a trusted detail", RequestId: "request"}
					if kind == "empty code" {
						detail.Code = ""
					}
					value, err := proto.Marshal(detail)
					if err != nil {
						t.Fatal(err)
					}
					if kind == "malformed" {
						value = []byte{255}
					}
					typeURL := "type.googleapis.com/geoledger.v1.ErrorDetail"
					if kind == "wrong type" {
						typeURL = "type.invalid/geoledger.v1.ErrorDetail"
					}
					envelope.Details = []*anypb.Any{{TypeUrl: typeURL, Value: value}}
				}
				rpc := &contractRPC{publishError: status.Error(codes.Unavailable, "original request is queued")}
				w := draftWith(rpc)
				ctx := context.Background()
				if _, err := w.Publish(ctx, "original"); err == nil {
					t.Fatal("expected unknown")
				}
				original := *w.PendingPublication()
				rpc.publishError = status.FromProto(envelope).Err()
				_, err := w.Publish(ctx, "original")
				requireBusinessError(t, err, "conflict", true)
				if pending := w.PendingPublication(); pending == nil || *pending != original {
					t.Fatal("proxy error lost original intent")
				}
				if _, err = w.Delete(ctx, "dataset", "one"); err == nil {
					t.Fatal("proxy error enabled edits")
				}
				if _, err = w.Publish(ctx, "original"); err != nil {
					t.Fatal(err)
				}
				for _, request := range rpc.publications {
					if !proto.Equal(request, rpc.publications[0]) {
						t.Fatal("recovery changed intent")
					}
				}
			})
		}
	}
}

func TestWorkspaceVersionProofRequiresItsExactTransportAndDetail(t *testing.T) {
	cases := []struct {
		name      string
		status    codes.Code
		message   string
		requestID string
		conflicts *pb.ConflictsReply
		valid     bool
	}{
		{"valid", codes.Aborted, "workspace version changed", "proof", nil, true},
		{"permission denied", codes.PermissionDenied, "workspace version changed", "proof", nil, false},
		{"invalid argument", codes.InvalidArgument, "workspace version changed", "proof", nil, false},
		{"already exists", codes.AlreadyExists, "workspace version changed", "proof", nil, false},
		{"failed precondition", codes.FailedPrecondition, "workspace version changed", "proof", nil, false},
		{"unavailable", codes.Unavailable, "workspace version changed", "proof", nil, false},
		{"empty message", codes.Aborted, "", "proof", nil, false},
		{"blank message", codes.Aborted, " \t", "proof", nil, false},
		{"empty request", codes.Aborted, "workspace version changed", "", nil, false},
		{"blank request", codes.Aborted, "workspace version changed", " \t", nil, false},
		{"merge metadata", codes.Aborted, "workspace version changed", "proof", &pb.ConflictsReply{}, false},
	}
	for _, test := range cases {
		t.Run(test.name, func(t *testing.T) {
			s, err := status.New(test.status, "public failure").WithDetails(&pb.ErrorDetail{Code: "workspace_version_conflict", Message: test.message, RequestId: test.requestID, Conflicts: test.conflicts})
			if err != nil {
				t.Fatal(err)
			}
			requireBusinessError(t, failure(s.Err()), "workspace_version_conflict", !test.valid)
			rpc := &contractRPC{publishError: status.Error(codes.Unavailable, "original request remains queued")}
			w := draftWith(rpc)
			ctx := context.Background()
			if _, err = w.Publish(ctx, "original"); err == nil {
				t.Fatal("expected unknown outcome")
			}
			original := *w.PendingPublication()
			rpc.publishError = s.Err()
			_, err = w.Publish(ctx, "original")
			requireBusinessError(t, err, "workspace_version_conflict", !test.valid)
			if test.valid {
				if w.PendingPublication() != nil {
					t.Fatal("valid version proof retained intent")
				}
			} else if pending := w.PendingPublication(); pending == nil || *pending != original {
				t.Fatal("incomplete or mismatched proof discarded unknown intent")
			}
		})
	}
}

type publicationReplyRPC struct {
	pb.GeoLedgerClient
	reply    *pb.PublishReply
	requests []*pb.PublishRequest
}

func (s *publicationReplyRPC) Publish(_ context.Context, request *pb.PublishRequest, _ ...grpc.CallOption) (*pb.PublishReply, error) {
	s.requests = append(s.requests, proto.Clone(request).(*pb.PublishRequest))
	return s.reply, nil
}

func TestInvalidPublicationReceiptRetainsIntentUntilAnExactReceipt(t *testing.T) {
	valid := &pb.PublishReply{Revision: 2, Version: 5, Workspace: "draft", Status: "published", Changes: 0}
	for _, test := range []struct {
		name  string
		reply *pb.PublishReply
	}{
		{"empty", &pb.PublishReply{}},
		{"wrong workspace", &pb.PublishReply{Revision: 2, Version: 5, Workspace: "other", Status: "published"}},
		{"wrong version", &pb.PublishReply{Revision: 2, Version: 4, Workspace: "draft", Status: "published"}},
		{"wrong status", &pb.PublishReply{Revision: 2, Version: 5, Workspace: "draft", Status: "open"}},
		{"zero revision", &pb.PublishReply{Revision: 0, Version: 5, Workspace: "draft", Status: "published"}},
	} {
		t.Run(test.name, func(t *testing.T) {
			rpc := &publicationReplyRPC{reply: test.reply}
			w := &Workspace{client: &Client{rpc: rpc}, project: "project", info: WorkspaceInfo{ID: "draft", BaseRevision: 1, Version: 4, Status: "open"}}
			ctx := context.Background()
			_, err := w.Publish(ctx, "original")
			requireBusinessError(t, err, "invalid_response", true)
			pending := w.PendingPublication()
			if pending == nil || w.Info().Version != 4 || w.Info().Status != "open" {
				t.Fatal("invalid receipt lost original state")
			}
			original := *pending
			if _, err = w.Delete(ctx, "dataset", "one"); err == nil {
				t.Fatal("invalid receipt permitted edits")
			}
			rpc.reply = valid
			result, err := w.Publish(ctx, "original")
			if err != nil {
				t.Fatal(err)
			}
			if result.Changes != 0 || result.Version != 5 || w.Info().Status != "published" {
				t.Fatal("valid zero-change receipt was rejected")
			}
			if len(rpc.requests) != 2 || !proto.Equal(rpc.requests[0], rpc.requests[1]) {
				t.Fatal("receipt recovery changed original intent")
			}
			if current := w.PendingPublication(); current == nil || *current != original {
				t.Fatal("successful receipt changed retained intent")
			}
		})
	}
}
