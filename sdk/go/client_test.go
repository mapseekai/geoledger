package geoledger

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"testing"

	pb "github.com/mapseekai/geoledger/sdk/go/internal/geoledgerv1"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/proto"
)

// These tests substitute the private transport; users never import its types.
type contractRPC struct {
	pb.GeoLedgerClient
	saves        []*pb.SaveRequest
	publications []*pb.PublishRequest
	publishError error
}

func (s *contractRPC) Save(_ context.Context, request *pb.SaveRequest, _ ...grpc.CallOption) (*pb.SaveReply, error) {
	s.saves = append(s.saves, proto.Clone(request).(*pb.SaveRequest))
	return &pb.SaveReply{Version: request.ExpectedWorkspaceVersion + 1, Changes: 1}, nil
}
func (s *contractRPC) Publish(_ context.Context, request *pb.PublishRequest, _ ...grpc.CallOption) (*pb.PublishReply, error) {
	s.publications = append(s.publications, proto.Clone(request).(*pb.PublishRequest))
	if s.publishError != nil {
		err := s.publishError
		s.publishError = nil
		return nil, err
	}
	return &pb.PublishReply{Workspace: request.Workspace, Revision: 2, Version: request.ExpectedWorkspaceVersion + 1, Status: "published", Changes: 1}, nil
}

func draftWith(rpc *contractRPC) *Workspace {
	return &Workspace{client: &Client{rpc: rpc}, project: "project", info: WorkspaceInfo{ID: "draft", BaseRevision: 1, Version: 4, Status: "open"}}
}

func requireBusinessError(t *testing.T, err error, code string, uncertain bool) {
	t.Helper()
	var own *Error
	if !errors.As(err, &own) || own.Code != code || own.Uncertain != uncertain {
		t.Fatalf("expected %s uncertain=%v, received %v", code, uncertain, err)
	}
}

func TestMissingFeatureCannotBecomeDeletion(t *testing.T) {
	rpc := &contractRPC{}
	w := draftWith(rpc)
	ctx := context.Background()
	missing := []Edit{{Dataset: "dataset", FeatureID: "one"}}
	_, err := w.SaveBatch(ctx, missing)
	requireBusinessError(t, err, "invalid_argument", false)
	_, err = w.Resolve(ctx, 1, missing)
	requireBusinessError(t, err, "invalid_argument", false)
	_, err = w.Rebase(ctx, 1, missing)
	requireBusinessError(t, err, "invalid_argument", false)
	if len(rpc.saves) != 0 {
		t.Fatal("missing feature reached the server")
	}
	if _, err := w.Delete(ctx, "dataset", "one"); err != nil {
		t.Fatal(err)
	}
	if rpc.saves[0].Edits[0].Feature != nil || w.Info().Version != 5 {
		t.Fatal("explicit deletion or version update failed")
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

func TestUnknownPublicationOutcomeReusesAnImmutableIntent(t *testing.T) {
	rpc := &contractRPC{publishError: status.Error(codes.Unavailable, "response lost")}
	w := draftWith(rpc)
	ctx := context.Background()
	_, err := w.Publish(ctx, "roads updated")
	requireBusinessError(t, err, "unavailable", true)
	pending := w.PendingPublication()
	if pending == nil {
		t.Fatal("uncertain publication lost its intent")
	}
	pending.Message = "caller mutation must not affect the retry"
	_, err = w.Publish(ctx, "different message")
	requireBusinessError(t, err, "invalid_argument", false)
	_, err = w.Delete(ctx, "dataset", "one")
	requireBusinessError(t, err, "invalid_argument", false)
	if len(rpc.publications) != 1 {
		t.Fatal("changed publication was sent")
	}
	if _, err = w.Publish(ctx, "roads updated"); err != nil {
		t.Fatal(err)
	}
	if !proto.Equal(rpc.publications[0], rpc.publications[1]) {
		t.Fatal("publication retry changed the request")
	}
	if w.Info().Version != 5 || w.Info().Status != "published" {
		t.Fatal("successful publication did not update the handle")
	}
}

func TestBusinessConflictDetailsAndDefiniteFailure(t *testing.T) {
	raw := `{"type":"Feature","id":"one","geometry":null,"properties":{"exact":9007199254740993}}`
	s, err := status.New(codes.Aborted, "merge conflicts").WithDetails(&pb.ErrorDetail{
		Code: "conflict", Message: "merge conflicts", RequestId: "request",
		Conflicts: &pb.ConflictsReply{Head: 2, Version: 4, Total: 1, Conflicts: []*pb.Conflict{{FeatureId: "one", Draft: &pb.Feature{Geojson: raw}}}},
	})
	if err != nil {
		t.Fatal(err)
	}
	rpc := &contractRPC{publishError: s.Err()}
	w := draftWith(rpc)
	_, err = w.Publish(context.Background(), "roads updated")
	requireBusinessError(t, err, "conflict", false)
	var own *Error
	if !errors.As(err, &own) || own.Conflicts == nil || own.RequestID != "request" {
		t.Fatal("business conflict metadata was lost")
	}
	if !json.Valid(own.Conflicts.Conflicts[0].Draft) || string(own.Conflicts.Conflicts[0].Draft) != raw {
		t.Fatal("conflict feature changed")
	}
	if w.PendingPublication() != nil || w.Info().Version != 4 {
		t.Fatal("definitive rejection must release intent without advancing version")
	}
}
