package geoledger

import (
	"fmt"
	"log"
	"strings"
	"testing"
	"time"

	pb "github.com/mapseekai/geoledger/sdk/go/internal/geoledgerv1"
)

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
