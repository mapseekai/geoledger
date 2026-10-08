package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	gl "github.com/mapseekai/geoledger/sdk/go"
	"os"
	"time"
)

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
func run() error {
	raw, err := os.ReadFile(os.Getenv("GL_TOKEN_FILE"))
	if err != nil {
		return err
	}
	var tokens []struct {
		Token string `json:"token"`
	}
	if err = json.Unmarshal(raw, &tokens); err != nil {
		return err
	}
	if len(tokens) == 0 {
		return fmt.Errorf("empty token file")
	}
	endpoint := os.Getenv("GL_ENDPOINT")
	if endpoint == "" {
		endpoint = "http://127.0.0.1:7882"
	}
	client, err := gl.Dial(endpoint, tokens[0].Token)
	if err != nil {
		return err
	}
	defer client.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	project, err := client.CreateProject(ctx, fmt.Sprintf("go-%d", time.Now().UnixNano()))
	if err != nil {
		return err
	}
	dataset, err := client.CreateDataset(ctx, project.ID, "places")
	if err != nil {
		return err
	}
	draft, err := client.CreateWorkspace(ctx, project.ID)
	if err != nil {
		return err
	}
	feature := gl.Feature{Type: "Feature", ID: "one", Properties: map[string]any{"exact": uint64(18446744073709551615), "nested": map[string]any{"geojson": "hello"}, "detail_json": "plain"}, Geometry: map[string]any{"type": "Point", "coordinates": []int{1, 2, 3}}}
	if _, err = draft.Save(ctx, dataset.ID, feature); err != nil {
		return err
	}
	if _, err = draft.Save(ctx, dataset.ID, feature); err != nil {
		return err
	}
	if draft.Info().Version != 2 {
		return fmt.Errorf("version did not advance")
	}
	_, err = draft.SaveBatch(ctx, []gl.Edit{{Dataset: dataset.ID, FeatureID: "one"}})
	var own *gl.Error
	if !errors.As(err, &own) || own.Code != "invalid_argument" {
		return fmt.Errorf("missing feature accepted: %v", err)
	}
	if _, err = draft.Delete(ctx, dataset.ID, "one"); err != nil {
		return err
	}
	empty, err := draft.Features(ctx, dataset.ID, gl.FeatureQuery{})
	if err != nil {
		return err
	}
	if len(empty.Features) != 0 {
		return fmt.Errorf("delete failed")
	}
	if _, err = draft.Save(ctx, dataset.ID, feature); err != nil {
		return err
	}
	receipt, err := draft.Publish(ctx, "Go SDK")
	if err != nil {
		return err
	}
	again, err := draft.Publish(ctx, "Go SDK")
	if err != nil {
		return err
	}
	if receipt != again {
		return fmt.Errorf("retry changed result")
	}
	rows, err := client.Features(ctx, project.ID, dataset.ID, gl.FeatureQuery{Revision: &receipt.Revision})
	if err != nil {
		return err
	}
	if len(rows.Features) != 1 || !bytes.Contains(rows.Features[0], []byte("18446744073709551615")) || !bytes.Contains(rows.Features[0], []byte(`"geojson":"hello"`)) {
		return fmt.Errorf("feature roundtrip failed")
	}
	changed := *draft.PendingPublication()
	changed.Message = "different"
	_, err = client.Publish(ctx, changed)
	if !errors.As(err, &own) || own.Code != "conflict" || own.Uncertain {
		return fmt.Errorf("wrong error: %v", err)
	}
	_, err = client.CreateDataset(ctx, project.ID, "places")
	if !errors.As(err, &own) || own.Code != "conflict" {
		return fmt.Errorf("wrong duplicate error: %v", err)
	}
	if err = client.SetMember(ctx, project.ID, "sdk-viewer", "viewer"); err != nil {
		return err
	}
	members, err := client.Members(ctx, project.ID, gl.Page{})
	if err != nil || len(members) != 2 {
		return fmt.Errorf("members: %v %v", members, err)
	}
	if err = client.RemoveMember(ctx, project.ID, "sdk-viewer"); err != nil {
		return err
	}
	if p, err := client.ArchiveProject(ctx, project.ID, true); err != nil || p.State != "archived" {
		return fmt.Errorf("archive: %v %v", p, err)
	}
	if err = client.DeleteProject(ctx, project.ID, project.Name); err != nil {
		return err
	}
	fmt.Println("Go SDK: business API, exact integers, automatic versions, publish retry, membership lifecycle OK")
	return nil
}
