// Business models contain no generated network types.
package geoledger

import "encoding/json"

type ServerInfo struct {
	PlatformAdmin   bool   `json:"platform_admin"`
	Version         string `json:"version"`
	Backend         string `json:"backend"`
	FormatVersion   uint32 `json:"format_version"`
	MaxRequestBytes uint32 `json:"max_request_bytes"`
	MaxFeatureBytes uint32 `json:"max_feature_bytes"`
}

type Project struct {
	ID   string `json:"project"`
	Name string `json:"name"`
	Head int64  `json:"head"`
	Role string `json:"role"`
	// State is active, archived (read-only) or deleted.
	State string `json:"state"`
}

type Member struct {
	Subject string `json:"subject"`
	Role    string `json:"role"`
}

type PostgisTable struct {
	Schema         string `json:"schema"`
	Table          string `json:"table"`
	IDColumn       string `json:"id_column"`
	GeometryColumn string `json:"geometry_column"`
}
type Dataset struct {
	PostgisTable        *PostgisTable `json:"postgis_table,omitempty"`
	GeometryType        string        `json:"geometry_type"`
	CoordinateDimension uint32        `json:"coordinate_dimension"`
	CRS                 string        `json:"crs"`
	ID                  string        `json:"dataset"`
	Name                string        `json:"name"`
}

type WorkspaceInfo struct {
	ID           string `json:"workspace"`
	BaseRevision int64  `json:"base_revision"`
	Version      int64  `json:"version"`
	Status       string `json:"status"`
}

type SaveResult struct {
	Warnings []string `json:"warnings"`
	Version  int64    `json:"version"`
	Changes  int64    `json:"changes"`
}

type DiscardResult struct {
	Version int64  `json:"version"`
	Status  string `json:"status"`
}

type FeaturePage struct {
	CRS              string            `json:"crs"`
	Features         []json.RawMessage `json:"features"`
	Revision         int64             `json:"revision"`
	WorkspaceVersion *int64            `json:"workspace_version"`
	NextAfter        *string           `json:"next_after"`
}

type SchemaField struct {
	Name     string `json:"name"`
	Type     string `json:"type"`
	Nullable bool   `json:"nullable"`
	Editable bool   `json:"editable"`
}
type DatasetSchema struct {
	Revision         int64         `json:"revision"`
	WorkspaceVersion *int64        `json:"workspace_version"`
	Fields           []SchemaField `json:"fields"`
}
type SchemaQuery struct {
	Workspace *string `json:"workspace,omitempty"`
	Revision  *int64  `json:"revision,omitempty"`
}
type AddField struct {
	Name string `json:"name"`
	Type string `json:"type"`
}
type SchemaEdit struct {
	Dataset string     `json:"dataset"`
	Add     []AddField `json:"add"`
	Drop    []string   `json:"drop"`
}
type SchemaChange struct {
	Dataset string        `json:"dataset"`
	Before  []SchemaField `json:"before"`
	After   []SchemaField `json:"after"`
}

type Change struct {
	Cursor    string          `json:"cursor"`
	Dataset   string          `json:"dataset"`
	FeatureID string          `json:"feature_id"`
	Base      json.RawMessage `json:"base"`
	Draft     json.RawMessage `json:"draft"`
	Before    json.RawMessage `json:"before"`
	After     json.RawMessage `json:"after"`
}

type Diff struct {
	SchemaChanges []SchemaChange `json:"schema_changes"`
	BaseRevision  int64          `json:"base_revision"`
	Version       int64          `json:"version"`
	Changes       []Change       `json:"changes"`
	NextAfter     *string        `json:"next_after"`
}

type Conflict struct {
	Cursor                  string          `json:"cursor"`
	Dataset                 string          `json:"dataset"`
	FeatureID               string          `json:"feature_id"`
	Fields                  []string        `json:"fields"`
	Base                    json.RawMessage `json:"base"`
	Current                 json.RawMessage `json:"current"`
	Draft                   json.RawMessage `json:"draft"`
	Reason                  string          `json:"reason"`
	ResolvedAgainstRevision *int64          `json:"resolved_against_revision"`
}

type Conflicts struct {
	Head      int64      `json:"head"`
	Version   int64      `json:"version"`
	Total     uint64     `json:"total"`
	NextAfter *string    `json:"next_after"`
	Truncated bool       `json:"truncated"`
	Conflicts []Conflict `json:"conflicts"`
}

type Commit struct {
	Revision           int64  `json:"revision"`
	Subject            string `json:"subject"`
	Message            string `json:"message"`
	CreatedAt          string `json:"created_at"`
	SourceWorkspace    string `json:"source_workspace"`
	SourceBaseRevision int64  `json:"source_base_revision"`
}

// HistoryPage contains commits in ascending revision order. A nil NextAfter
// marks the current read snapshot end; later publications can still be appended.
type HistoryPage struct {
	Commits   []Commit `json:"commits"`
	NextAfter *int64   `json:"next_after"`
}

type CommitChanges struct {
	SchemaChanges []SchemaChange `json:"schema_changes"`
	Revision      int64          `json:"revision"`
	Changes       []Change       `json:"changes"`
	NextAfter     *string        `json:"next_after"`
}

type AuditEvent struct {
	ID        int64           `json:"id"`
	Subject   string          `json:"subject"`
	Action    string          `json:"action"`
	Detail    json.RawMessage `json:"detail_json"`
	CreatedAt string          `json:"created_at"`
}

type AuditPage struct {
	Events    []AuditEvent `json:"events"`
	NextAfter *int64       `json:"next_after"`
}

type PublicationResult struct {
	Revision  int64  `json:"revision"`
	Workspace string `json:"workspace"`
	Version   int64  `json:"version"`
	Status    string `json:"status"`
	Changes   uint64 `json:"changes"`
}

type ResolutionResult struct {
	Head               int64  `json:"head"`
	Version            int64  `json:"version"`
	RemainingConflicts uint64 `json:"remaining_conflicts"`
}

type RebaseResult struct {
	BaseRevision int64  `json:"base_revision"`
	Version      int64  `json:"version"`
	Changes      uint64 `json:"changes"`
}

// Feature is ordinary GeoJSON; integer properties may use int64, uint64 or json.Number.
type Feature struct {
	Type       string         `json:"type"`
	ID         string         `json:"id"`
	Properties map[string]any `json:"properties"`
	Geometry   any            `json:"geometry"`
}

// Edit changes one feature. Feature must be explicit JSON; use JSON null
// (or Workspace.Delete) for deletion.
type Edit struct {
	Dataset   string          `json:"dataset"`
	FeatureID string          `json:"feature_id"`
	Feature   json.RawMessage `json:"feature"`
}

// Page selects one page, not automatic iteration. Its zero value selects the
// first page with the server's default size; a nonzero Limit is sent unchanged
// for server validation. After is the method's continuation key: retain returned
// cursors unchanged, and use the last ID for ID-ordered list methods.
type Page struct {
	After string `json:"after,omitempty"`
	Limit int64  `json:"limit,omitempty"`
}

// FeatureQuery selects features from a published revision or a draft. With
// neither Revision nor Workspace set, the server selects the published head.
// Limit zero uses the server default. Continue using FeaturePage.NextAfter
// unchanged with the same filters. Pin the first returned Revision for published
// pages; for drafts, check WorkspaceVersion has not changed between pages.
// A nil NextAfter ends iteration.
// Workspace.Features supplies Workspace itself and rejects either override.
type FeatureQuery struct {
	Workspace *string   `json:"workspace,omitempty"`
	Revision  *int64    `json:"revision,omitempty"`
	FeatureID *string   `json:"feature_id,omitempty"`
	Bbox      []float64 `json:"bbox,omitempty"`
	After     string    `json:"after,omitempty"`
	Limit     int64     `json:"limit,omitempty"`
}

// Publication is safe to serialize for recovery; retry it unchanged, including
// RequestID, expected version and message. Obtain it from PendingPublication
// rather than constructing a new identity after an uncertain result.
type Publication struct {
	Project                  string `json:"project"`
	Workspace                string `json:"workspace"`
	ExpectedWorkspaceVersion int64  `json:"expected_workspace_version"`
	RequestID                string `json:"request_id"`
	Message                  string `json:"message"`
}

type ChangeCounts struct {
	Added    int64 `json:"added"`
	Deleted  int64 `json:"deleted"`
	Modified int64 `json:"modified"`
}
type DatasetChangeCounts struct {
	ID       string `json:"id"`
	Name     string `json:"name"`
	Added    int64  `json:"added"`
	Deleted  int64  `json:"deleted"`
	Modified int64  `json:"modified"`
}
type ChangeSummary struct {
	SchemaChanges []SchemaChange        `json:"schema_changes"`
	Total         ChangeCounts          `json:"total"`
	Datasets      []DatasetChangeCounts `json:"datasets"`
	Version       *int64                `json:"version,omitempty"`
	Revision      *int64                `json:"revision,omitempty"`
}
