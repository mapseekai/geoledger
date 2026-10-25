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
