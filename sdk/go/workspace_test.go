package geoledger

import (
	"context"
	"encoding/json"
	"errors"
	"log"
	"math"
	"sync"
	"testing"
	"time"

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

func TestRetryAccessFailuresPreserveOriginalIntent(t *testing.T) {
	for _, code := range []codes.Code{codes.Unauthenticated, codes.PermissionDenied, codes.NotFound} {
		t.Run(code.String(), func(t *testing.T) {
			rpc := &contractRPC{publishError: status.Error(codes.Unavailable, "lost")}
			w := draftWith(rpc)
			ctx := context.Background()
			w.Publish(ctx, "original")
			original := *w.PendingPublication()
			rpc.publishError = status.Error(code, "access denied")
			if _, err := w.Publish(ctx, "original"); err == nil {
				t.Fatal("expected rejection")
			}
			if pending := w.PendingPublication(); pending == nil || *pending != original {
				t.Fatal("lost original intent")
			}
			if _, err := w.Publish(ctx, "changed"); err == nil {
				t.Fatal("changed message accepted")
			}
			if _, err := w.Publish(ctx, "original"); err != nil {
				t.Fatal(err)
			}
			for _, request := range rpc.publications {
				if !proto.Equal(request, rpc.publications[0]) {
					t.Fatal("retry changed request")
				}
			}
		})
	}
}

func TestTypedRequestsPreserveExactGeoJSONAndRejectInvalidInput(t *testing.T) {
	rpc := &contractRPC{}
	w := draftWith(rpc)
	raw := json.RawMessage(` {"type":"Feature", "id":"one","properties":{"exact":9007199254740993}} `)
	if _, err := w.SaveBatch(context.Background(), []Edit{{Dataset: "dataset", FeatureID: "one", Feature: raw}}); err != nil {
		t.Fatal(err)
	}
	if got := rpc.saves[0].Edits[0].Feature.Geojson; got != string(raw) {
		t.Fatalf("GeoJSON changed: %q", got)
	}
	if _, err := w.SaveBatch(context.Background(), []Edit{{Dataset: "dataset", FeatureID: "one", Feature: json.RawMessage("invalid")}}); err == nil {
		t.Fatal("invalid JSON accepted")
	}
	if _, err := w.client.Publish(nil, Publication{}); err == nil {
		t.Fatal("nil context accepted")
	}
	if _, err := w.client.Features(context.Background(), "project", "dataset", FeatureQuery{Bbox: []float64{math.NaN()}}); err == nil {
		t.Fatal("nonfinite bbox accepted")
	}
	w.client.closed.Store(true)
	if _, err := w.client.Publish(context.Background(), Publication{}); err == nil {
		t.Fatal("closed client accepted")
	}
}

func TestFirstAccessFailureAndRetryConflictReleaseIntent(t *testing.T) {
	for _, code := range []codes.Code{codes.Unauthenticated, codes.PermissionDenied, codes.NotFound, codes.Aborted} {
		rpc := &contractRPC{publishError: status.Error(code, "rejected")}
		w := draftWith(rpc)
		if code == codes.Aborted {
			rpc.publishError = status.Error(codes.Unavailable, "lost")
			w.Publish(context.Background(), "original")
			rpc.publishError = status.Error(code, "rejected")
		}
		if _, err := w.Publish(context.Background(), "original"); err == nil {
			t.Fatal("expected failure")
		}
		if w.PendingPublication() != nil {
			t.Fatalf("%s did not release intent", code)
		}
	}
}

// observedContext reports when acquisition starts selecting on cancellation.
type observedContext struct {
	context.Context
	observed chan struct{}
	once     sync.Once
}

func (c *observedContext) Done() <-chan struct{} {
	c.once.Do(func() { close(c.observed) })
	return c.Context.Done()
}

func TestWorkspaceCanceledAcquisitionDoesNotChangeState(t *testing.T) {
	operations := []struct {
		name string
		run  func(*Workspace, context.Context) error
	}{
		{"refresh", func(w *Workspace, ctx context.Context) error { _, err := w.Refresh(ctx); return err }},
		{"save", func(w *Workspace, ctx context.Context) error {
			_, err := w.Save(ctx, "dataset", map[string]any{"id": "one"})
			return err
		}},
		{"delete", func(w *Workspace, ctx context.Context) error { _, err := w.Delete(ctx, "dataset", "one"); return err }},
		{"save batch", func(w *Workspace, ctx context.Context) error { _, err := w.SaveBatch(ctx, nil); return err }},
		{"features", func(w *Workspace, ctx context.Context) error {
			_, err := w.Features(ctx, "dataset", FeatureQuery{})
			return err
		}},
		{"diff", func(w *Workspace, ctx context.Context) error { _, err := w.Diff(ctx, Page{}); return err }},
		{"conflicts", func(w *Workspace, ctx context.Context) error { _, err := w.Conflicts(ctx, Page{}); return err }},
		{"publish", func(w *Workspace, ctx context.Context) error { _, err := w.Publish(ctx, "original"); return err }},
		{"resolve", func(w *Workspace, ctx context.Context) error { _, err := w.Resolve(ctx, 1, nil); return err }},
		{"rebase", func(w *Workspace, ctx context.Context) error { _, err := w.Rebase(ctx, 1, nil); return err }},
		{"discard", func(w *Workspace, ctx context.Context) error { _, err := w.Discard(ctx); return err }},
	}
	for _, op := range operations {
		for _, mode := range []string{"already canceled", "waiting", "expired deadline", "nil"} {
			t.Run(op.name+"/"+mode, func(t *testing.T) {
				rpc := &contractRPC{}
				w := draftWith(rpc)
				original := w.info
				base, cancel := context.WithCancel(context.Background())
				defer cancel()
				var ctx context.Context = base
				code := "cancelled"
				switch mode {
				case "already canceled":
					cancel()
				case "expired deadline":
					var stop context.CancelFunc
					ctx, stop = context.WithDeadline(context.Background(), time.Now().Add(-time.Second))
					defer stop()
					code = "timeout"
				case "nil":
					ctx = nil
					code = "invalid_argument"
				}
				if mode == "waiting" {
					w.mu.Lock()
					defer w.mu.Unlock()
					watched := &observedContext{Context: base, observed: make(chan struct{})}
					done := make(chan error, 1)
					go func() { done <- op.run(w, watched) }()
					select {
					case <-watched.observed:
					case <-time.After(time.Second):
						t.Fatal("operation never began waiting")
					}
					cancel()
					select {
					case err := <-done:
						requireBusinessError(t, err, code, false)
					case <-time.After(time.Second):
						t.Fatal("canceled waiter did not return while lock remained held")
					}
				} else {
					requireBusinessError(t, op.run(w, ctx), code, false)
					// Acquiring again verifies canceled calls never consume the permit.
					w.mu.Lock()
					w.mu.Unlock()
				}
				if w.info != original || w.pending != nil || len(rpc.saves) != 0 || len(rpc.publications) != 0 {
					t.Fatal("canceled operation mutated state or sent an RPC")
				}
			})
		}
	}
}

type gatedSaveRPC struct {
	contractRPC
	requests chan *pb.SaveRequest
	release  chan struct{}
}

func (s *gatedSaveRPC) Save(ctx context.Context, request *pb.SaveRequest, opts ...grpc.CallOption) (*pb.SaveReply, error) {
	s.requests <- request
	<-s.release
	return s.contractRPC.Save(ctx, request, opts...)
}

func TestWorkspaceSerializesVersionsAndPublicationRetries(t *testing.T) {
	rpc := &gatedSaveRPC{requests: make(chan *pb.SaveRequest, 2), release: make(chan struct{})}
	defer close(rpc.release)
	w := &Workspace{client: &Client{rpc: rpc}, project: "project", info: WorkspaceInfo{ID: "draft", Version: 4, Status: "open"}}
	done := make(chan error, 2)
	go func() { _, err := w.Delete(context.Background(), "dataset", "one"); done <- err }()
	var first *pb.SaveRequest
	select {
	case first = <-rpc.requests:
	case <-time.After(time.Second):
		t.Fatal("first save did not enter RPC")
	}
	ctx := &observedContext{Context: context.Background(), observed: make(chan struct{})}
	go func() { _, err := w.Delete(ctx, "dataset", "two"); done <- err }()
	select {
	case <-ctx.observed:
	case <-time.After(time.Second):
		t.Fatal("second save never attempted acquisition")
	}
	select {
	case <-rpc.requests:
		t.Fatal("second save overlapped first RPC")
	default:
	}
	rpc.release <- struct{}{}
	var second *pb.SaveRequest
	select {
	case second = <-rpc.requests:
	case <-time.After(time.Second):
		t.Fatal("second save never entered RPC")
	}
	rpc.release <- struct{}{}
	for range 2 {
		select {
		case err := <-done:
			if err != nil {
				t.Fatal(err)
			}
		case <-time.After(time.Second):
			t.Fatal("save did not finish")
		}
	}
	if first.ExpectedWorkspaceVersion != 4 || second.ExpectedWorkspaceVersion != 5 || w.Info().Version != 6 {
		t.Fatalf("versions: first=%d second=%d current=%d", first.ExpectedWorkspaceVersion, second.ExpectedWorkspaceVersion, w.Info().Version)
	}
	rpc.publishError = status.Error(codes.Unavailable, "response lost")
	_, err := w.Publish(context.Background(), "original")
	requireBusinessError(t, err, "unavailable", true)
	original := *w.PendingPublication()
	canceled, cancel := context.WithCancel(context.Background())
	cancel()
	_, err = w.Publish(canceled, "original")
	requireBusinessError(t, err, "cancelled", false)
	if *w.PendingPublication() != original || len(rpc.publications) != 1 {
		t.Fatal("canceled retry changed or sent pending intent")
	}
	if _, err := w.Publish(context.Background(), "original"); err != nil {
		t.Fatal(err)
	}
	if len(rpc.publications) != 2 || !proto.Equal(rpc.publications[0], rpc.publications[1]) || rpc.publications[1].ExpectedWorkspaceVersion != 6 {
		t.Fatal("publication retry lost serialized version or immutable intent")
	}
}

// This example requires a service and existing project/workspace. Without an
// Output directive, go test compiles it but does not perform these remote calls.
func ExampleWorkspace_PendingPublication() {
	client, err := Dial("http://127.0.0.1:7882", "glk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
	if err != nil {
		log.Fatal(err)
	}
	defer client.Close()
	ctx := context.Background() // Supply a deadline/cancellation for real requests.
	workspace, err := client.Workspace(ctx, "project-id", "workspace-id")
	if err != nil {
		log.Fatal(err)
	}
	if _, err := workspace.Publish(ctx, "roads updated"); err != nil {
		if intent := workspace.PendingPublication(); intent != nil {
			// Persist this exact intent before recovery. Retry later under a suitable
			// context; do not generate a new RequestID or change any other field.
			_, retryErr := client.Publish(ctx, *intent)
			if retryErr != nil {
				log.Print(retryErr)
			}
		}
	}
}
