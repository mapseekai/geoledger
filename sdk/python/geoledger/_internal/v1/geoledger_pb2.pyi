from google.protobuf.internal import containers as _containers
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Iterable as _Iterable, Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class NameRequest(_message.Message):
    __slots__ = ("name",)
    NAME_FIELD_NUMBER: _ClassVar[int]
    name: str
    def __init__(self, name: _Optional[str] = ...) -> None: ...

class ProjectRequest(_message.Message):
    __slots__ = ("project",)
    PROJECT_FIELD_NUMBER: _ClassVar[int]
    project: str
    def __init__(self, project: _Optional[str] = ...) -> None: ...

class PageRequest(_message.Message):
    __slots__ = ("after", "limit")
    AFTER_FIELD_NUMBER: _ClassVar[int]
    LIMIT_FIELD_NUMBER: _ClassVar[int]
    after: str
    limit: int
    def __init__(self, after: _Optional[str] = ..., limit: _Optional[int] = ...) -> None: ...

class ProjectPageRequest(_message.Message):
    __slots__ = ("project", "after", "limit")
    PROJECT_FIELD_NUMBER: _ClassVar[int]
    AFTER_FIELD_NUMBER: _ClassVar[int]
    LIMIT_FIELD_NUMBER: _ClassVar[int]
    project: str
    after: str
    limit: int
    def __init__(self, project: _Optional[str] = ..., after: _Optional[str] = ..., limit: _Optional[int] = ...) -> None: ...

class MemberRequest(_message.Message):
    __slots__ = ("project", "subject", "role")
    PROJECT_FIELD_NUMBER: _ClassVar[int]
    SUBJECT_FIELD_NUMBER: _ClassVar[int]
    ROLE_FIELD_NUMBER: _ClassVar[int]
    project: str
    subject: str
    role: str
    def __init__(self, project: _Optional[str] = ..., subject: _Optional[str] = ..., role: _Optional[str] = ...) -> None: ...

class DatasetRequest(_message.Message):
    __slots__ = ("project", "name")
    PROJECT_FIELD_NUMBER: _ClassVar[int]
    NAME_FIELD_NUMBER: _ClassVar[int]
    project: str
    name: str
    def __init__(self, project: _Optional[str] = ..., name: _Optional[str] = ...) -> None: ...

class WorkspaceRequest(_message.Message):
    __slots__ = ("project", "workspace")
    PROJECT_FIELD_NUMBER: _ClassVar[int]
    WORKSPACE_FIELD_NUMBER: _ClassVar[int]
    project: str
    workspace: str
    def __init__(self, project: _Optional[str] = ..., workspace: _Optional[str] = ...) -> None: ...

class VersionRequest(_message.Message):
    __slots__ = ("project", "workspace", "expected_workspace_version")
    PROJECT_FIELD_NUMBER: _ClassVar[int]
    WORKSPACE_FIELD_NUMBER: _ClassVar[int]
    EXPECTED_WORKSPACE_VERSION_FIELD_NUMBER: _ClassVar[int]
    project: str
    workspace: str
    expected_workspace_version: int
    def __init__(self, project: _Optional[str] = ..., workspace: _Optional[str] = ..., expected_workspace_version: _Optional[int] = ...) -> None: ...

class Feature(_message.Message):
    __slots__ = ("geojson",)
    GEOJSON_FIELD_NUMBER: _ClassVar[int]
    geojson: str
    def __init__(self, geojson: _Optional[str] = ...) -> None: ...

class Edit(_message.Message):
    __slots__ = ("dataset", "feature_id", "feature")
    DATASET_FIELD_NUMBER: _ClassVar[int]
    FEATURE_ID_FIELD_NUMBER: _ClassVar[int]
    FEATURE_FIELD_NUMBER: _ClassVar[int]
    dataset: str
    feature_id: str
    feature: Feature
    def __init__(self, dataset: _Optional[str] = ..., feature_id: _Optional[str] = ..., feature: _Optional[_Union[Feature, _Mapping]] = ...) -> None: ...

class SaveRequest(_message.Message):
    __slots__ = ("project", "workspace", "expected_workspace_version", "edits")
    PROJECT_FIELD_NUMBER: _ClassVar[int]
    WORKSPACE_FIELD_NUMBER: _ClassVar[int]
    EXPECTED_WORKSPACE_VERSION_FIELD_NUMBER: _ClassVar[int]
    EDITS_FIELD_NUMBER: _ClassVar[int]
    project: str
    workspace: str
    expected_workspace_version: int
    edits: _containers.RepeatedCompositeFieldContainer[Edit]
    def __init__(self, project: _Optional[str] = ..., workspace: _Optional[str] = ..., expected_workspace_version: _Optional[int] = ..., edits: _Optional[_Iterable[_Union[Edit, _Mapping]]] = ...) -> None: ...

class FeaturesRequest(_message.Message):
    __slots__ = ("project", "dataset", "workspace", "revision", "feature_id", "bbox", "after", "limit")
    PROJECT_FIELD_NUMBER: _ClassVar[int]
    DATASET_FIELD_NUMBER: _ClassVar[int]
    WORKSPACE_FIELD_NUMBER: _ClassVar[int]
    REVISION_FIELD_NUMBER: _ClassVar[int]
    FEATURE_ID_FIELD_NUMBER: _ClassVar[int]
    BBOX_FIELD_NUMBER: _ClassVar[int]
    AFTER_FIELD_NUMBER: _ClassVar[int]
    LIMIT_FIELD_NUMBER: _ClassVar[int]
    project: str
    dataset: str
    workspace: str
    revision: int
    feature_id: str
    bbox: _containers.RepeatedScalarFieldContainer[float]
    after: str
    limit: int
    def __init__(self, project: _Optional[str] = ..., dataset: _Optional[str] = ..., workspace: _Optional[str] = ..., revision: _Optional[int] = ..., feature_id: _Optional[str] = ..., bbox: _Optional[_Iterable[float]] = ..., after: _Optional[str] = ..., limit: _Optional[int] = ...) -> None: ...

class DiffRequest(_message.Message):
    __slots__ = ("project", "workspace", "after", "limit")
    PROJECT_FIELD_NUMBER: _ClassVar[int]
    WORKSPACE_FIELD_NUMBER: _ClassVar[int]
    AFTER_FIELD_NUMBER: _ClassVar[int]
    LIMIT_FIELD_NUMBER: _ClassVar[int]
    project: str
    workspace: str
    after: str
    limit: int
    def __init__(self, project: _Optional[str] = ..., workspace: _Optional[str] = ..., after: _Optional[str] = ..., limit: _Optional[int] = ...) -> None: ...

class HistoryRequest(_message.Message):
    __slots__ = ("project", "after", "limit")
    PROJECT_FIELD_NUMBER: _ClassVar[int]
    AFTER_FIELD_NUMBER: _ClassVar[int]
    LIMIT_FIELD_NUMBER: _ClassVar[int]
    project: str
    after: int
    limit: int
    def __init__(self, project: _Optional[str] = ..., after: _Optional[int] = ..., limit: _Optional[int] = ...) -> None: ...

class CommitRequest(_message.Message):
    __slots__ = ("project", "revision", "after", "limit")
    PROJECT_FIELD_NUMBER: _ClassVar[int]
    REVISION_FIELD_NUMBER: _ClassVar[int]
    AFTER_FIELD_NUMBER: _ClassVar[int]
    LIMIT_FIELD_NUMBER: _ClassVar[int]
    project: str
    revision: int
    after: str
    limit: int
    def __init__(self, project: _Optional[str] = ..., revision: _Optional[int] = ..., after: _Optional[str] = ..., limit: _Optional[int] = ...) -> None: ...

class PublishRequest(_message.Message):
    __slots__ = ("project", "workspace", "expected_workspace_version", "request_id", "message")
    PROJECT_FIELD_NUMBER: _ClassVar[int]
    WORKSPACE_FIELD_NUMBER: _ClassVar[int]
    EXPECTED_WORKSPACE_VERSION_FIELD_NUMBER: _ClassVar[int]
    REQUEST_ID_FIELD_NUMBER: _ClassVar[int]
    MESSAGE_FIELD_NUMBER: _ClassVar[int]
    project: str
    workspace: str
    expected_workspace_version: int
    request_id: str
    message: str
    def __init__(self, project: _Optional[str] = ..., workspace: _Optional[str] = ..., expected_workspace_version: _Optional[int] = ..., request_id: _Optional[str] = ..., message: _Optional[str] = ...) -> None: ...

class ResolveRequest(_message.Message):
    __slots__ = ("project", "workspace", "expected_workspace_version", "expected_head", "resolutions")
    PROJECT_FIELD_NUMBER: _ClassVar[int]
    WORKSPACE_FIELD_NUMBER: _ClassVar[int]
    EXPECTED_WORKSPACE_VERSION_FIELD_NUMBER: _ClassVar[int]
    EXPECTED_HEAD_FIELD_NUMBER: _ClassVar[int]
    RESOLUTIONS_FIELD_NUMBER: _ClassVar[int]
    project: str
    workspace: str
    expected_workspace_version: int
    expected_head: int
    resolutions: _containers.RepeatedCompositeFieldContainer[Edit]
    def __init__(self, project: _Optional[str] = ..., workspace: _Optional[str] = ..., expected_workspace_version: _Optional[int] = ..., expected_head: _Optional[int] = ..., resolutions: _Optional[_Iterable[_Union[Edit, _Mapping]]] = ...) -> None: ...

class RestoreRequest(_message.Message):
    __slots__ = ("project", "revision")
    PROJECT_FIELD_NUMBER: _ClassVar[int]
    REVISION_FIELD_NUMBER: _ClassVar[int]
    project: str
    revision: int
    def __init__(self, project: _Optional[str] = ..., revision: _Optional[int] = ...) -> None: ...

class Empty(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class InfoReply(_message.Message):
    __slots__ = ("version", "backend", "format_version", "max_request_bytes", "max_feature_bytes")
    VERSION_FIELD_NUMBER: _ClassVar[int]
    BACKEND_FIELD_NUMBER: _ClassVar[int]
    FORMAT_VERSION_FIELD_NUMBER: _ClassVar[int]
    MAX_REQUEST_BYTES_FIELD_NUMBER: _ClassVar[int]
    MAX_FEATURE_BYTES_FIELD_NUMBER: _ClassVar[int]
    version: str
    backend: str
    format_version: int
    max_request_bytes: int
    max_feature_bytes: int
    def __init__(self, version: _Optional[str] = ..., backend: _Optional[str] = ..., format_version: _Optional[int] = ..., max_request_bytes: _Optional[int] = ..., max_feature_bytes: _Optional[int] = ...) -> None: ...

class ProjectReply(_message.Message):
    __slots__ = ("project", "name", "head", "role")
    PROJECT_FIELD_NUMBER: _ClassVar[int]
    NAME_FIELD_NUMBER: _ClassVar[int]
    HEAD_FIELD_NUMBER: _ClassVar[int]
    ROLE_FIELD_NUMBER: _ClassVar[int]
    project: str
    name: str
    head: int
    role: str
    def __init__(self, project: _Optional[str] = ..., name: _Optional[str] = ..., head: _Optional[int] = ..., role: _Optional[str] = ...) -> None: ...

class ProjectsReply(_message.Message):
    __slots__ = ("projects",)
    PROJECTS_FIELD_NUMBER: _ClassVar[int]
    projects: _containers.RepeatedCompositeFieldContainer[ProjectReply]
    def __init__(self, projects: _Optional[_Iterable[_Union[ProjectReply, _Mapping]]] = ...) -> None: ...

class OkReply(_message.Message):
    __slots__ = ("ok",)
    OK_FIELD_NUMBER: _ClassVar[int]
    ok: bool
    def __init__(self, ok: bool = ...) -> None: ...

class DatasetReply(_message.Message):
    __slots__ = ("dataset", "name")
    DATASET_FIELD_NUMBER: _ClassVar[int]
    NAME_FIELD_NUMBER: _ClassVar[int]
    dataset: str
    name: str
    def __init__(self, dataset: _Optional[str] = ..., name: _Optional[str] = ...) -> None: ...

class DatasetsReply(_message.Message):
    __slots__ = ("datasets",)
    DATASETS_FIELD_NUMBER: _ClassVar[int]
    datasets: _containers.RepeatedCompositeFieldContainer[DatasetReply]
    def __init__(self, datasets: _Optional[_Iterable[_Union[DatasetReply, _Mapping]]] = ...) -> None: ...

class WorkspaceReply(_message.Message):
    __slots__ = ("workspace", "base_revision", "version", "status")
    WORKSPACE_FIELD_NUMBER: _ClassVar[int]
    BASE_REVISION_FIELD_NUMBER: _ClassVar[int]
    VERSION_FIELD_NUMBER: _ClassVar[int]
    STATUS_FIELD_NUMBER: _ClassVar[int]
    workspace: str
    base_revision: int
    version: int
    status: str
    def __init__(self, workspace: _Optional[str] = ..., base_revision: _Optional[int] = ..., version: _Optional[int] = ..., status: _Optional[str] = ...) -> None: ...

class WorkspacesReply(_message.Message):
    __slots__ = ("workspaces",)
    WORKSPACES_FIELD_NUMBER: _ClassVar[int]
    workspaces: _containers.RepeatedCompositeFieldContainer[WorkspaceReply]
    def __init__(self, workspaces: _Optional[_Iterable[_Union[WorkspaceReply, _Mapping]]] = ...) -> None: ...

class SaveReply(_message.Message):
    __slots__ = ("version", "changes")
    VERSION_FIELD_NUMBER: _ClassVar[int]
    CHANGES_FIELD_NUMBER: _ClassVar[int]
    version: int
    changes: int
    def __init__(self, version: _Optional[int] = ..., changes: _Optional[int] = ...) -> None: ...

class DiscardReply(_message.Message):
    __slots__ = ("version", "status")
    VERSION_FIELD_NUMBER: _ClassVar[int]
    STATUS_FIELD_NUMBER: _ClassVar[int]
    version: int
    status: str
    def __init__(self, version: _Optional[int] = ..., status: _Optional[str] = ...) -> None: ...

class FeaturesReply(_message.Message):
    __slots__ = ("features", "revision", "workspace_version", "next_after")
    FEATURES_FIELD_NUMBER: _ClassVar[int]
    REVISION_FIELD_NUMBER: _ClassVar[int]
    WORKSPACE_VERSION_FIELD_NUMBER: _ClassVar[int]
    NEXT_AFTER_FIELD_NUMBER: _ClassVar[int]
    features: _containers.RepeatedCompositeFieldContainer[Feature]
    revision: int
    workspace_version: int
    next_after: str
    def __init__(self, features: _Optional[_Iterable[_Union[Feature, _Mapping]]] = ..., revision: _Optional[int] = ..., workspace_version: _Optional[int] = ..., next_after: _Optional[str] = ...) -> None: ...

class Change(_message.Message):
    __slots__ = ("cursor", "dataset", "feature_id", "base", "draft", "before", "after")
    CURSOR_FIELD_NUMBER: _ClassVar[int]
    DATASET_FIELD_NUMBER: _ClassVar[int]
    FEATURE_ID_FIELD_NUMBER: _ClassVar[int]
    BASE_FIELD_NUMBER: _ClassVar[int]
    DRAFT_FIELD_NUMBER: _ClassVar[int]
    BEFORE_FIELD_NUMBER: _ClassVar[int]
    AFTER_FIELD_NUMBER: _ClassVar[int]
    cursor: str
    dataset: str
    feature_id: str
    base: Feature
    draft: Feature
    before: Feature
    after: Feature
    def __init__(self, cursor: _Optional[str] = ..., dataset: _Optional[str] = ..., feature_id: _Optional[str] = ..., base: _Optional[_Union[Feature, _Mapping]] = ..., draft: _Optional[_Union[Feature, _Mapping]] = ..., before: _Optional[_Union[Feature, _Mapping]] = ..., after: _Optional[_Union[Feature, _Mapping]] = ...) -> None: ...

class DiffReply(_message.Message):
    __slots__ = ("base_revision", "version", "changes")
    BASE_REVISION_FIELD_NUMBER: _ClassVar[int]
    VERSION_FIELD_NUMBER: _ClassVar[int]
    CHANGES_FIELD_NUMBER: _ClassVar[int]
    base_revision: int
    version: int
    changes: _containers.RepeatedCompositeFieldContainer[Change]
    def __init__(self, base_revision: _Optional[int] = ..., version: _Optional[int] = ..., changes: _Optional[_Iterable[_Union[Change, _Mapping]]] = ...) -> None: ...

class Conflict(_message.Message):
    __slots__ = ("cursor", "dataset", "feature_id", "fields", "base", "current", "draft", "reason", "resolved_against_revision")
    CURSOR_FIELD_NUMBER: _ClassVar[int]
    DATASET_FIELD_NUMBER: _ClassVar[int]
    FEATURE_ID_FIELD_NUMBER: _ClassVar[int]
    FIELDS_FIELD_NUMBER: _ClassVar[int]
    BASE_FIELD_NUMBER: _ClassVar[int]
    CURRENT_FIELD_NUMBER: _ClassVar[int]
    DRAFT_FIELD_NUMBER: _ClassVar[int]
    REASON_FIELD_NUMBER: _ClassVar[int]
    RESOLVED_AGAINST_REVISION_FIELD_NUMBER: _ClassVar[int]
    cursor: str
    dataset: str
    feature_id: str
    fields: _containers.RepeatedScalarFieldContainer[str]
    base: Feature
    current: Feature
    draft: Feature
    reason: str
    resolved_against_revision: int
    def __init__(self, cursor: _Optional[str] = ..., dataset: _Optional[str] = ..., feature_id: _Optional[str] = ..., fields: _Optional[_Iterable[str]] = ..., base: _Optional[_Union[Feature, _Mapping]] = ..., current: _Optional[_Union[Feature, _Mapping]] = ..., draft: _Optional[_Union[Feature, _Mapping]] = ..., reason: _Optional[str] = ..., resolved_against_revision: _Optional[int] = ...) -> None: ...

class ConflictsReply(_message.Message):
    __slots__ = ("head", "version", "total", "next_after", "truncated", "conflicts")
    HEAD_FIELD_NUMBER: _ClassVar[int]
    VERSION_FIELD_NUMBER: _ClassVar[int]
    TOTAL_FIELD_NUMBER: _ClassVar[int]
    NEXT_AFTER_FIELD_NUMBER: _ClassVar[int]
    TRUNCATED_FIELD_NUMBER: _ClassVar[int]
    CONFLICTS_FIELD_NUMBER: _ClassVar[int]
    head: int
    version: int
    total: int
    next_after: str
    truncated: bool
    conflicts: _containers.RepeatedCompositeFieldContainer[Conflict]
    def __init__(self, head: _Optional[int] = ..., version: _Optional[int] = ..., total: _Optional[int] = ..., next_after: _Optional[str] = ..., truncated: bool = ..., conflicts: _Optional[_Iterable[_Union[Conflict, _Mapping]]] = ...) -> None: ...

class CommitInfo(_message.Message):
    __slots__ = ("revision", "subject", "message", "created_at")
    REVISION_FIELD_NUMBER: _ClassVar[int]
    SUBJECT_FIELD_NUMBER: _ClassVar[int]
    MESSAGE_FIELD_NUMBER: _ClassVar[int]
    CREATED_AT_FIELD_NUMBER: _ClassVar[int]
    revision: int
    subject: str
    message: str
    created_at: str
    def __init__(self, revision: _Optional[int] = ..., subject: _Optional[str] = ..., message: _Optional[str] = ..., created_at: _Optional[str] = ...) -> None: ...

class HistoryReply(_message.Message):
    __slots__ = ("commits",)
    COMMITS_FIELD_NUMBER: _ClassVar[int]
    commits: _containers.RepeatedCompositeFieldContainer[CommitInfo]
    def __init__(self, commits: _Optional[_Iterable[_Union[CommitInfo, _Mapping]]] = ...) -> None: ...

class CommitReply(_message.Message):
    __slots__ = ("revision", "changes")
    REVISION_FIELD_NUMBER: _ClassVar[int]
    CHANGES_FIELD_NUMBER: _ClassVar[int]
    revision: int
    changes: _containers.RepeatedCompositeFieldContainer[Change]
    def __init__(self, revision: _Optional[int] = ..., changes: _Optional[_Iterable[_Union[Change, _Mapping]]] = ...) -> None: ...

class AuditEvent(_message.Message):
    __slots__ = ("id", "subject", "action", "detail_json", "created_at")
    ID_FIELD_NUMBER: _ClassVar[int]
    SUBJECT_FIELD_NUMBER: _ClassVar[int]
    ACTION_FIELD_NUMBER: _ClassVar[int]
    DETAIL_JSON_FIELD_NUMBER: _ClassVar[int]
    CREATED_AT_FIELD_NUMBER: _ClassVar[int]
    id: int
    subject: str
    action: str
    detail_json: str
    created_at: str
    def __init__(self, id: _Optional[int] = ..., subject: _Optional[str] = ..., action: _Optional[str] = ..., detail_json: _Optional[str] = ..., created_at: _Optional[str] = ...) -> None: ...

class AuditReply(_message.Message):
    __slots__ = ("events", "next_after")
    EVENTS_FIELD_NUMBER: _ClassVar[int]
    NEXT_AFTER_FIELD_NUMBER: _ClassVar[int]
    events: _containers.RepeatedCompositeFieldContainer[AuditEvent]
    next_after: int
    def __init__(self, events: _Optional[_Iterable[_Union[AuditEvent, _Mapping]]] = ..., next_after: _Optional[int] = ...) -> None: ...

class PublishReply(_message.Message):
    __slots__ = ("revision", "workspace", "version", "status", "changes")
    REVISION_FIELD_NUMBER: _ClassVar[int]
    WORKSPACE_FIELD_NUMBER: _ClassVar[int]
    VERSION_FIELD_NUMBER: _ClassVar[int]
    STATUS_FIELD_NUMBER: _ClassVar[int]
    CHANGES_FIELD_NUMBER: _ClassVar[int]
    revision: int
    workspace: str
    version: int
    status: str
    changes: int
    def __init__(self, revision: _Optional[int] = ..., workspace: _Optional[str] = ..., version: _Optional[int] = ..., status: _Optional[str] = ..., changes: _Optional[int] = ...) -> None: ...

class ResolveReply(_message.Message):
    __slots__ = ("head", "version", "remaining_conflicts")
    HEAD_FIELD_NUMBER: _ClassVar[int]
    VERSION_FIELD_NUMBER: _ClassVar[int]
    REMAINING_CONFLICTS_FIELD_NUMBER: _ClassVar[int]
    head: int
    version: int
    remaining_conflicts: int
    def __init__(self, head: _Optional[int] = ..., version: _Optional[int] = ..., remaining_conflicts: _Optional[int] = ...) -> None: ...

class RebaseReply(_message.Message):
    __slots__ = ("base_revision", "version", "changes")
    BASE_REVISION_FIELD_NUMBER: _ClassVar[int]
    VERSION_FIELD_NUMBER: _ClassVar[int]
    CHANGES_FIELD_NUMBER: _ClassVar[int]
    base_revision: int
    version: int
    changes: int
    def __init__(self, base_revision: _Optional[int] = ..., version: _Optional[int] = ..., changes: _Optional[int] = ...) -> None: ...

class ErrorDetail(_message.Message):
    __slots__ = ("code", "message", "request_id", "conflicts")
    CODE_FIELD_NUMBER: _ClassVar[int]
    MESSAGE_FIELD_NUMBER: _ClassVar[int]
    REQUEST_ID_FIELD_NUMBER: _ClassVar[int]
    CONFLICTS_FIELD_NUMBER: _ClassVar[int]
    code: str
    message: str
    request_id: str
    conflicts: ConflictsReply
    def __init__(self, code: _Optional[str] = ..., message: _Optional[str] = ..., request_id: _Optional[str] = ..., conflicts: _Optional[_Union[ConflictsReply, _Mapping]] = ...) -> None: ...

class RpcStatus(_message.Message):
    __slots__ = ("code", "message", "details")
    CODE_FIELD_NUMBER: _ClassVar[int]
    MESSAGE_FIELD_NUMBER: _ClassVar[int]
    DETAILS_FIELD_NUMBER: _ClassVar[int]
    code: int
    message: str
    details: _containers.RepeatedCompositeFieldContainer[ErrorAny]
    def __init__(self, code: _Optional[int] = ..., message: _Optional[str] = ..., details: _Optional[_Iterable[_Union[ErrorAny, _Mapping]]] = ...) -> None: ...

class ErrorAny(_message.Message):
    __slots__ = ("type_url", "value")
    TYPE_URL_FIELD_NUMBER: _ClassVar[int]
    VALUE_FIELD_NUMBER: _ClassVar[int]
    type_url: str
    value: bytes
    def __init__(self, type_url: _Optional[str] = ..., value: _Optional[bytes] = ...) -> None: ...
