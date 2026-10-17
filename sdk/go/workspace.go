package geoledger

import (
	"context"
	"crypto/rand"
	"encoding/json"
	"fmt"
	"sync"

	pb "github.com/mapseekai/geoledger/sdk/go/internal/geoledgerv1"
)

// Workspace owns the optimistic version of a personal draft. Do not copy it.
// Operations are serialized; their contexts also cancel waiting for another
// operation. Cancellation before acquiring the workspace does not send an RPC.
// Obtain a handle from Client.CreateWorkspace, Client.Workspace or Client.Restore;
// the zero value cannot perform service operations. The Client owns its connection.
// Context-free accessors wait for an active operation and return local snapshots.
type Workspace struct {
	mu      workspaceMutex
	client  *Client
	project string
	info    WorkspaceInfo
	pending *Publication
}

// ID returns the immutable workspace identifier.
func (w *Workspace) ID() string { w.mu.Lock(); defer w.mu.Unlock(); return w.info.ID }

// Info returns the last locally observed version and status, without a server call.
func (w *Workspace) Info() WorkspaceInfo { w.mu.Lock(); defer w.mu.Unlock(); return w.info }

// PendingPublication returns an independent copy of the retained publication
// intent, or nil before publishing or after a definitive rejection clears it.
// An intent is retained after success as well as uncertain failure to allow an
// identical retry. Persist the copy unchanged for recovery via Client.Publish.
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

// Refresh reloads version and status without clearing a retained publication.
// It does not resolve conflicts or establish a new publication request identity.
func (w *Workspace) Refresh(ctx context.Context) (WorkspaceInfo, error) {
	if err := w.mu.lock(ctx); err != nil {
		return WorkspaceInfo{}, err
	}
	defer w.mu.Unlock()
	var info WorkspaceInfo
	if err := call(w.client, ctx, w.client.rpc.GetWorkspace, &pb.WorkspaceRequest{Project: w.project, Workspace: w.info.ID}, &info); err != nil {
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

// SaveBatch applies edits atomically against the handle's current version.
// Each edit must contain explicit JSON; null deletes a feature, absent JSON is
// rejected. A retained publication intent prevents further edits.
func (w *Workspace) SaveBatch(ctx context.Context, edits []Edit) (SaveResult, error) {
	converted, err := wireEdits(edits)
	if err != nil {
		return SaveResult{}, err
	}
	if err := w.mu.lock(ctx); err != nil {
		return SaveResult{}, err
	}
	defer w.mu.Unlock()
	var r SaveResult
	if err := w.editable(); err != nil {
		return r, err
	}
	err = call(w.client, ctx, w.client.rpc.Save, &pb.SaveRequest{Project: w.project, Workspace: w.info.ID, ExpectedWorkspaceVersion: w.info.Version, Edits: converted}, &r)
	if err == nil {
		w.info.Version = r.Version
	}
	return r, err
}
func (w *Workspace) Features(ctx context.Context, dataset string, q FeatureQuery) (FeaturePage, error) {
	if err := w.mu.lock(ctx); err != nil {
		return FeaturePage{}, err
	}
	defer w.mu.Unlock()
	if q.Revision != nil || q.Workspace != nil {
		return FeaturePage{}, invalid("draft queries cannot override workspace or revision")
	}
	id := w.info.ID
	q.Workspace = &id
	return w.client.Features(ctx, w.project, dataset, q)
}

// Diff returns one page of draft changes; continue with the last Change.Cursor
// as Page.After and verify the returned Version is unchanged between pages.
func (w *Workspace) Diff(ctx context.Context, page Page) (Diff, error) {
	if err := w.mu.lock(ctx); err != nil {
		return Diff{}, err
	}
	defer w.mu.Unlock()
	var r Diff
	err := call(w.client, ctx, w.client.rpc.Diff, &pb.DiffRequest{Project: w.project, Workspace: w.info.ID, After: page.After, Limit: limit(page.Limit)}, &r)
	return r, err
}

// Conflicts returns one page; pass Conflicts.NextAfter unchanged as Page.After.
// Check Head and Version across pages because the draft or published head may change.
func (w *Workspace) Conflicts(ctx context.Context, page Page) (Conflicts, error) {
	if err := w.mu.lock(ctx); err != nil {
		return Conflicts{}, err
	}
	defer w.mu.Unlock()
	var r Conflicts
	err := call(w.client, ctx, w.client.rpc.Conflicts, &pb.DiffRequest{Project: w.project, Workspace: w.info.ID, After: page.After, Limit: limit(page.Limit)}, &r)
	return r, err
}

// Publish creates an intent on the first attempt and reuses it on later calls.
// Retry with the original message after an uncertain failure, including across
// access failures during recovery. Success retains the same intent; a definitive
// rejection may clear it. Waiting cancellation neither creates nor clears it.
func (w *Workspace) Publish(ctx context.Context, message string) (PublicationResult, error) {
	if err := w.mu.lock(ctx); err != nil {
		return PublicationResult{}, err
	}
	defer w.mu.Unlock()
	var r PublicationResult
	wasPending := w.pending != nil
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
	} else if e, ok := err.(*Error); ok && !e.Uncertain && (!wasPending || (e.Code != "unauthenticated" && e.Code != "permission_denied" && e.Code != "not_found")) {
		w.pending = nil
	}
	return r, err
}
func (w *Workspace) Resolve(ctx context.Context, head int64, edits []Edit) (ResolutionResult, error) {
	converted, err := wireEdits(edits)
	if err != nil {
		return ResolutionResult{}, err
	}
	if err := w.mu.lock(ctx); err != nil {
		return ResolutionResult{}, err
	}
	defer w.mu.Unlock()
	var r ResolutionResult
	if err := w.editable(); err != nil {
		return r, err
	}
	err = call(w.client, ctx, w.client.rpc.Resolve, &pb.ResolveRequest{Project: w.project, Workspace: w.info.ID, ExpectedWorkspaceVersion: w.info.Version, ExpectedHead: head, Resolutions: converted}, &r)
	if err == nil {
		w.info.Version = r.Version
	}
	return r, err
}
func (w *Workspace) Rebase(ctx context.Context, head int64, edits []Edit) (RebaseResult, error) {
	converted, err := wireEdits(edits)
	if err != nil {
		return RebaseResult{}, err
	}
	if err := w.mu.lock(ctx); err != nil {
		return RebaseResult{}, err
	}
	defer w.mu.Unlock()
	var r RebaseResult
	if err := w.editable(); err != nil {
		return r, err
	}
	err = call(w.client, ctx, w.client.rpc.Rebase, &pb.ResolveRequest{Project: w.project, Workspace: w.info.ID, ExpectedWorkspaceVersion: w.info.Version, ExpectedHead: head, Resolutions: converted}, &r)
	if err == nil {
		w.info.Version = r.Version
		w.info.BaseRevision = r.BaseRevision
	}
	return r, err
}
func (w *Workspace) Discard(ctx context.Context) (DiscardResult, error) {
	if err := w.mu.lock(ctx); err != nil {
		return DiscardResult{}, err
	}
	defer w.mu.Unlock()
	var r DiscardResult
	if err := w.editable(); err != nil {
		return r, err
	}
	err := call(w.client, ctx, w.client.rpc.Discard, &pb.VersionRequest{Project: w.project, Workspace: w.info.ID, ExpectedWorkspaceVersion: w.info.Version}, &r)
	if err == nil {
		w.info.Version = r.Version
		w.info.Status = r.Status
	}
	return r, err
}

// workspaceMutex serializes version changes and publication retries across RPCs.
// Its zero value is ready for use, including handles constructed by tests.
type workspaceMutex struct {
	once  sync.Once
	token chan struct{}
}

func (m *workspaceMutex) Lock()   { _ = m.lock(context.Background()) }
func (m *workspaceMutex) Unlock() { <-m.token }

func (m *workspaceMutex) lock(ctx context.Context) error {
	if ctx == nil {
		return invalid("context is required")
	}
	if err := ctx.Err(); err != nil {
		return waitingError(err)
	}
	m.once.Do(func() { m.token = make(chan struct{}, 1) })
	select {
	case m.token <- struct{}{}:
		// Cancellation and acquisition may become ready together. Do not start a
		// request or change pending intent when cancellation already won.
		if err := ctx.Err(); err != nil {
			m.Unlock()
			return waitingError(err)
		}
		return nil
	case <-ctx.Done():
		return waitingError(ctx.Err())
	}
}

func waitingError(err error) error {
	code := "cancelled"
	if err == context.DeadlineExceeded {
		code = "timeout"
	}
	// No RPC has been sent, so this operation's outcome is certain.
	return &Error{Code: code, Message: err.Error()}
}
