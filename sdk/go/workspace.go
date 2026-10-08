package geoledger

import (
	"context"
	"crypto/rand"
	"encoding/json"
	"fmt"
	"sync"
)

// Workspace owns the optimistic version of a personal draft. Do not copy it.
type Workspace struct {
	mu      sync.Mutex
	client  *Client
	project string
	info    WorkspaceInfo
	pending *Publication
}

func (w *Workspace) ID() string          { w.mu.Lock(); defer w.mu.Unlock(); return w.info.ID }
func (w *Workspace) Info() WorkspaceInfo { w.mu.Lock(); defer w.mu.Unlock(); return w.info }
func (w *Workspace) PendingPublication() *Publication {
	w.mu.Lock()
	defer w.mu.Unlock()
	if w.pending == nil {
		return nil
	}
	p := *w.pending
	return &p
}
func (w *Workspace) editable() error {
	if w.pending != nil {
		return invalid("retry the pending publication before changing this workspace")
	}
	if w.info.Status != "open" {
		return invalid("workspace is closed")
	}
	return nil
}
func (w *Workspace) Refresh(ctx context.Context) (WorkspaceInfo, error) {
	w.mu.Lock()
	defer w.mu.Unlock()
	var info WorkspaceInfo
	if err := w.client.call(ctx, "GetWorkspace", map[string]any{"project": w.project, "workspace": w.info.ID}, &info); err != nil {
		return w.info, err
	}
	w.info = info
	return w.info, nil
}
func (w *Workspace) Save(ctx context.Context, dataset string, feature any) (SaveResult, error) {
	raw, err := json.Marshal(feature)
	if err != nil {
		return SaveResult{}, invalid("invalid GeoJSON")
	}
	var f struct {
		ID string `json:"id"`
	}
	if json.Unmarshal(raw, &f) != nil || f.ID == "" {
		return SaveResult{}, invalid("GeoJSON Feature requires a string id")
	}
	return w.SaveBatch(ctx, []Edit{{Dataset: dataset, FeatureID: f.ID, Feature: raw}})
}
func (w *Workspace) Delete(ctx context.Context, dataset, id string) (SaveResult, error) {
	return w.SaveBatch(ctx, []Edit{{Dataset: dataset, FeatureID: id, Feature: json.RawMessage("null")}})
}
func (w *Workspace) SaveBatch(ctx context.Context, edits []Edit) (SaveResult, error) {
	for _, edit := range edits {
		if len(edit.Feature) == 0 {
			return SaveResult{}, invalid("edit requires an explicit feature; use null for deletion")
		}
	}
	w.mu.Lock()
	defer w.mu.Unlock()
	var r SaveResult
	if err := w.editable(); err != nil {
		return r, err
	}
	err := w.client.call(ctx, "Save", map[string]any{"project": w.project, "workspace": w.info.ID, "expected_workspace_version": w.info.Version, "edits": edits}, &r)
	if err == nil {
		w.info.Version = r.Version
	}
	return r, err
}
func (w *Workspace) Features(ctx context.Context, dataset string, q FeatureQuery) (FeaturePage, error) {
	w.mu.Lock()
	defer w.mu.Unlock()
	if q.Revision != nil || q.Workspace != nil {
		return FeaturePage{}, invalid("draft queries cannot override workspace or revision")
	}
	id := w.info.ID
	q.Workspace = &id
	return w.client.Features(ctx, w.project, dataset, q)
}
func (w *Workspace) Diff(ctx context.Context, page Page) (Diff, error) {
	w.mu.Lock()
	defer w.mu.Unlock()
	var r Diff
	err := w.client.call(ctx, "Diff", map[string]any{"project": w.project, "workspace": w.info.ID, "after": page.After, "limit": limit(page.Limit)}, &r)
	return r, err
}
func (w *Workspace) Conflicts(ctx context.Context, page Page) (Conflicts, error) {
	w.mu.Lock()
	defer w.mu.Unlock()
	var r Conflicts
	err := w.client.call(ctx, "Conflicts", map[string]any{"project": w.project, "workspace": w.info.ID, "after": page.After, "limit": limit(page.Limit)}, &r)
	return r, err
}
func (w *Workspace) Publish(ctx context.Context, message string) (PublicationResult, error) {
	w.mu.Lock()
	defer w.mu.Unlock()
	var r PublicationResult
	if w.pending == nil {
		if err := w.editable(); err != nil {
			return r, err
		}
		id := make([]byte, 16)
		if _, err := rand.Read(id); err != nil {
			return r, &Error{Code: "unavailable", Message: "could not generate publication identity"}
		}
		id[6] = (id[6] & 15) | 64
		id[8] = (id[8] & 63) | 128
		w.pending = &Publication{Project: w.project, Workspace: w.info.ID, ExpectedWorkspaceVersion: w.info.Version, RequestID: fmt.Sprintf("%x-%x-%x-%x-%x", id[:4], id[4:6], id[6:8], id[8:10], id[10:]), Message: message}
	} else if w.pending.Message != message {
		return r, invalid("retry the pending publication with the original message")
	}
	r, err := w.client.Publish(ctx, *w.pending)
	if err == nil {
		w.info.Version = r.Version
		w.info.Status = r.Status
	} else if e, ok := err.(*Error); ok && !e.Uncertain {
		w.pending = nil
	}
	return r, err
}
func (w *Workspace) Resolve(ctx context.Context, head int64, edits []Edit) (ResolutionResult, error) {
	for _, edit := range edits {
		if len(edit.Feature) == 0 {
			return ResolutionResult{}, invalid("edit requires an explicit feature; use null for deletion")
		}
	}
	w.mu.Lock()
	defer w.mu.Unlock()
	var r ResolutionResult
	if err := w.editable(); err != nil {
		return r, err
	}
	err := w.client.call(ctx, "Resolve", map[string]any{"project": w.project, "workspace": w.info.ID, "expected_workspace_version": w.info.Version, "expected_head": head, "resolutions": edits}, &r)
	if err == nil {
		w.info.Version = r.Version
	}
	return r, err
}
func (w *Workspace) Rebase(ctx context.Context, head int64, edits []Edit) (RebaseResult, error) {
	for _, edit := range edits {
		if len(edit.Feature) == 0 {
			return RebaseResult{}, invalid("edit requires an explicit feature; use null for deletion")
		}
	}
	w.mu.Lock()
	defer w.mu.Unlock()
	var r RebaseResult
	if err := w.editable(); err != nil {
		return r, err
	}
	err := w.client.call(ctx, "Rebase", map[string]any{"project": w.project, "workspace": w.info.ID, "expected_workspace_version": w.info.Version, "expected_head": head, "resolutions": edits}, &r)
	if err == nil {
		w.info.Version = r.Version
		w.info.BaseRevision = r.BaseRevision
	}
	return r, err
}
func (w *Workspace) Discard(ctx context.Context) (DiscardResult, error) {
	w.mu.Lock()
	defer w.mu.Unlock()
	var r DiscardResult
	if err := w.editable(); err != nil {
		return r, err
	}
	err := w.client.call(ctx, "Discard", map[string]any{"project": w.project, "workspace": w.info.ID, "expected_workspace_version": w.info.Version}, &r)
	if err == nil {
		w.info.Version = r.Version
		w.info.Status = r.Status
	}
	return r, err
}
