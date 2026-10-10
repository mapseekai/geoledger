"""GeoLedger business client. Network protocol and generated types are private."""
from __future__ import annotations

from dataclasses import dataclass, asdict
from math import isfinite
from threading import RLock
from typing import Any
from urllib.parse import urlparse
from uuid import uuid4
import ipaddress
import json
import os
import grpc as _grpc
from ._internal.v1 import geoledger_pb2 as _pb
from ._internal.v1.geoledger_pb2_grpc import GeoLedgerStub as _Stub

__all__ = ["Client", "Workspace", "Project", "Member", "Dataset", "WorkspaceInfo", "Publication", "Edit", "GeoLedgerError"]

@dataclass(frozen=True)
class Project:
    id: str
    name: str
    head: int
    role: str
    state: str  # active, archived (read-only) or deleted

@dataclass(frozen=True)
class Member:
    subject: str
    role: str

@dataclass(frozen=True)
class Dataset:
    id: str
    name: str
    geometry_type: str
    coordinate_dimension: int = 2
    postgis_table: dict | None = None

@dataclass(frozen=True)
class WorkspaceInfo:
    id: str
    base_revision: int
    version: int
    status: str

@dataclass(frozen=True)
class Publication:
    """Persist asdict(intent) to recover an uncertain publication across processes."""
    project: str
    workspace: str
    expected_workspace_version: int
    request_id: str
    message: str

@dataclass(frozen=True)
class Edit:
    dataset: str
    feature_id: str
    feature: dict[str, Any] | str | None

class GeoLedgerError(Exception):
    def __init__(self, code: str, message: str, *, request_id: str | None = None,
                 conflicts: dict[str, Any] | None = None, uncertain: bool = False):
        super().__init__(f"{code}: {message}")
        self.code, self.message = code, message
        self.request_id, self.conflicts, self.uncertain = request_id, conflicts, uncertain

def _invalid(message):
    return GeoLedgerError("invalid_argument", message)

def _decode(message):
    """Decode only declared message fields; never inspect keys inside user GeoJSON."""
    if message.DESCRIPTOR.name == "Feature":
        return json.loads(message.geojson)
    result = {}
    for field in message.DESCRIPTOR.fields:
        if field.has_presence and not message.HasField(field.name):
            value = None
        else:
            value = getattr(message, field.name)
            if field.is_repeated:
                value = [_decode(item) if field.message_type else item for item in value]
            elif field.message_type:
                value = _decode(value)
        result[field.name] = value
    if message.DESCRIPTOR.name in ("ProjectReply", "DatasetReply", "WorkspaceReply"):
        key = {"ProjectReply": "project", "DatasetReply": "dataset", "WorkspaceReply": "workspace"}[message.DESCRIPTOR.name]
        result["id"] = result.pop(key)
    if message.DESCRIPTOR.name == "AuditEvent":
        result["detail"] = json.loads(result.pop("detail_json"))
    return result

def _error(error):
    codes = {_grpc.StatusCode.UNAUTHENTICATED: "unauthenticated", _grpc.StatusCode.PERMISSION_DENIED: "permission_denied",
             _grpc.StatusCode.NOT_FOUND: "not_found", _grpc.StatusCode.ABORTED: "conflict", _grpc.StatusCode.ALREADY_EXISTS: "conflict",
             _grpc.StatusCode.FAILED_PRECONDITION: "conflict", _grpc.StatusCode.INVALID_ARGUMENT: "invalid_argument",
             _grpc.StatusCode.OUT_OF_RANGE: "invalid_argument", _grpc.StatusCode.DEADLINE_EXCEEDED: "timeout",
             _grpc.StatusCode.CANCELLED: "cancelled", _grpc.StatusCode.RESOURCE_EXHAUSTED: "resource_exhausted"}
    uncertain = error.code() in (_grpc.StatusCode.DEADLINE_EXCEEDED, _grpc.StatusCode.CANCELLED, _grpc.StatusCode.UNAVAILABLE,
                                _grpc.StatusCode.UNKNOWN, _grpc.StatusCode.INTERNAL, _grpc.StatusCode.DATA_LOSS, _grpc.StatusCode.RESOURCE_EXHAUSTED)
    try:
        for key, value in error.trailing_metadata() or []:
            if key == "grpc-status-details-bin":
                for item in _pb.RpcStatus.FromString(value).details:
                    if item.type_url == "type.googleapis.com/geoledger.v1.ErrorDetail":
                        detail = _decode(_pb.ErrorDetail.FromString(item.value))
                        return GeoLedgerError(detail["code"], detail["message"], request_id=detail["request_id"], conflicts=detail["conflicts"], uncertain=uncertain)
    except Exception:
        pass  # Malformed optional metadata must not hide the original failure.
    return GeoLedgerError(codes.get(error.code(), "unavailable"), error.details() or "request failed", uncertain=uncertain)

_REQUESTS = {"RenameProject": "RenameProjectRequest", "RenameDataset": "RenameDatasetRequest", "DeleteDataset": "DeleteDatasetRequest", "Info": "Empty", "CreateProject": "NameRequest", "ListProjects": "PageRequest", "GetProject": "ProjectRequest",
             "SetMember": "MemberRequest", "CreateDataset": "DatasetRequest", "ListDatasets": "ProjectPageRequest",
             "CreateWorkspace": "ProjectRequest", "ListWorkspaces": "ProjectPageRequest", "GetWorkspace": "WorkspaceRequest",
             "Save": "SaveRequest", "Discard": "VersionRequest", "Features": "FeaturesRequest", "Diff": "DiffRequest",
             "WorkspaceSummary": "WorkspaceRequest", "CommitSummary": "SummaryCommitRequest",
             "Conflicts": "DiffRequest", "History": "HistoryRequest", "Commit": "CommitRequest", "Audit": "HistoryRequest",
             "Publish": "PublishRequest", "Resolve": "ResolveRequest", "Rebase": "ResolveRequest", "Restore": "RestoreRequest",
             "ListMembers": "ProjectPageRequest", "RemoveMember": "MemberRefRequest",
             "ArchiveProject": "ArchiveProjectRequest", "DeleteProject": "DeleteProjectRequest"}

def _is_loopback(host: str) -> bool:
    host = host.strip("[]")
    if host.lower() == "localhost":
        return True
    try:
        return ipaddress.ip_address(host).is_loopback
    except ValueError:
        return False

class Client:
    """Plaintext ``http://`` is accepted only for loopback hosts unless ``allow_insecure=True``
    (or ``GL_ALLOW_INSECURE_TRANSPORT=true``): the bearer token would cross the network unencrypted."""
    def __init__(self, endpoint: str, token: str, timeout: float = 30, *, allow_insecure: bool = False):
        try:
            url = urlparse(endpoint)
            if url.scheme not in ("http", "https") or not url.hostname or url.path not in ("", "/") or url.query or url.fragment or url.username is not None or url.password is not None or not url.port:
                raise ValueError()
        except ValueError:
            raise _invalid("endpoint must be http(s)://host:port") from None
        if not token or not token.isascii() or any(ord(c) < 32 or ord(c) >= 127 for c in token) or not isfinite(timeout) or timeout <= 0:
            raise _invalid("ASCII token and finite positive timeout required")
        opted_in = allow_insecure or os.environ.get("GL_ALLOW_INSECURE_TRANSPORT", "") in ("1", "true", "yes")
        if url.scheme == "http" and not _is_loopback(url.hostname or "") and not opted_in:
            raise _invalid("refusing to send credentials over plaintext http to a non-loopback host; use https or allow_insecure=True")
        options = [("grpc.max_receive_message_length", -1),
                   ("grpc.max_send_message_length", -1), ("grpc.enable_retries", 0)]
        self._channel = (_grpc.secure_channel(url.netloc, _grpc.ssl_channel_credentials(), options)
                         if url.scheme == "https" else _grpc.insecure_channel(url.netloc, options))
        self._stub = _Stub(self._channel)
        self._token, self._timeout, self._closed = token, timeout, False

    def close(self):
        self._closed = True
        self._channel.close()

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.close()

    def _call(self, operation, **fields):
        if self._closed:
            raise _invalid("client is closed")
        try:
            for key in ("edits", "resolutions"):
                if key in fields:
                    converted = []
                    for edit in fields[key]:
                        e = asdict(edit) if isinstance(edit, Edit) else dict(edit)
                        if "feature" not in e:
                            raise ValueError("edit requires an explicit feature; use None for deletion")
                        feature = e["feature"]
                        if feature is not None:
                            if isinstance(feature, str):
                                json.loads(feature)
                                raw = feature
                            else:
                                raw = json.dumps(feature, ensure_ascii=False, allow_nan=False, separators=(",", ":"))
                            e["feature"] = {"geojson": raw}
                        converted.append(e)
                    fields[key] = converted
            request = getattr(_pb, _REQUESTS[operation])(**{k: v for k, v in fields.items() if v is not None})
        except (TypeError, ValueError, KeyError) as e:
            raise _invalid(str(e)) from None
        try:
            reply = getattr(self._stub, operation)(request, timeout=self._timeout, metadata=(("authorization", "Bearer " + self._token),))
        except _grpc.RpcError as e:
            raise _error(e) from None
        try:
            return _decode(reply)
        except (ValueError, TypeError) as e:
            raise GeoLedgerError("invalid_response", "invalid data received from server", uncertain=True) from None

    def info(self) -> dict[str, Any]:
        return self._call("Info")

    def create_project(self, name: str) -> Project:
        return Project(**self._call("CreateProject", name=name))

    def project(self, project: str) -> Project:
        return Project(**self._call("GetProject", project=project))

    def projects(self, *, after: str = "", limit: int = 100) -> list[Project]:
        return [Project(**r) for r in self._call("ListProjects", after=after, limit=limit)["projects"]]

    def set_member(self, project: str, subject: str, role: str) -> None:
        self._call("SetMember", project=project, subject=subject, role=role)

    def members(self, project: str, *, after: str = "", limit: int = 100) -> list[Member]:
        """Active members ordered by subject."""
        return [Member(**r) for r in self._call("ListMembers", project=project, after=after, limit=limit)["members"]]

    def remove_member(self, project: str, subject: str) -> None:
        """Remove a member (owners/administrators) or leave when subject is the caller."""
        self._call("RemoveMember", project=project, subject=subject)

    def archive_project(self, project: str, archived: bool = True) -> Project:
        """Archive (read-only) or reactivate a project."""
        return Project(**self._call("ArchiveProject", project=project, archived=archived))

    def delete_project(self, project: str, confirm_name: str) -> None:
        """Delete all project data and retain its audit tombstone; confirm_name must equal its name."""
        self._call("DeleteProject", project=project, confirm_name=confirm_name)

    def rename_project(self, project: str, name: str) -> Project:
        return Project(**self._call("RenameProject", project=project, name=name))

    def rename_dataset(self, project: str, dataset: str, name: str) -> Dataset:
        return Dataset(**self._call("RenameDataset", project=project, dataset=dataset, name=name))

    def delete_dataset(self, project: str, dataset: str, confirm_name: str) -> None:
        self._call("DeleteDataset", project=project, dataset=dataset, confirm_name=confirm_name)

    def create_dataset(self, project: str, name: str, geometry_type: str, coordinate_dimension: int = 2) -> Dataset:
        return Dataset(**self._call("CreateDataset", project=project, name=name, geometry_type=geometry_type, coordinate_dimension=coordinate_dimension))

    def attach_postgis_table(self, project: str, name: str, *, schema: str, table: str, id_column: str, geometry_column: str) -> Dataset:
        return Dataset(**self._call("CreateDataset", project=project, name=name, postgis_table=dict(schema=schema, table=table, id_column=id_column, geometry_column=geometry_column)))

    def datasets(self, project: str, *, after: str = "", limit: int = 100) -> list[Dataset]:
        return [Dataset(**r) for r in self._call("ListDatasets", project=project, after=after, limit=limit)["datasets"]]

    def create_workspace(self, project: str) -> Workspace:
        return Workspace(self, project, WorkspaceInfo(**self._call("CreateWorkspace", project=project)))

    def workspace(self, project: str, workspace: str) -> Workspace:
        return Workspace(self, project, WorkspaceInfo(**self._call("GetWorkspace", project=project, workspace=workspace)))

    def workspaces(self, project: str, *, after: str = "", limit: int = 100) -> list[WorkspaceInfo]:
        return [WorkspaceInfo(**r) for r in self._call("ListWorkspaces", project=project, after=after, limit=limit)["workspaces"]]

    def features(self, project: str, dataset: str, *, workspace: str | None = None, revision: int | None = None,
                 feature_id: str | None = None, bbox: list[float] | None = None, after: str = "", limit: int = 100) -> dict[str, Any]:
        return self._call("Features", project=project, dataset=dataset, workspace=workspace, revision=revision, feature_id=feature_id, bbox=bbox, after=after, limit=limit)

    def history(self, project: str, *, after: int = 0, limit: int = 100) -> list[dict[str, Any]]:
        return self._call("History", project=project, after=after, limit=limit)["commits"]

    def workspace_summary(self, project: str, workspace: str) -> dict[str, Any]:
        return self._call("WorkspaceSummary", project=project, workspace=workspace)

    def commit_summary(self, project: str, revision: int) -> dict[str, Any]:
        return self._call("CommitSummary", project=project, revision=revision)

    def commit(self, project: str, revision: int, *, after: str = "", limit: int = 100) -> dict[str, Any]:
        return self._call("Commit", project=project, revision=revision, after=after, limit=limit)

    def audit(self, project: str, *, after: int = 0, limit: int = 100) -> dict[str, Any]:
        return self._call("Audit", project=project, after=after, limit=limit)

    def publish(self, publication: Publication) -> dict[str, Any]:
        """Retry a persisted publication without changing its identity or payload."""
        return self._call("Publish", **asdict(publication))

    def restore(self, project: str, revision: int) -> Workspace:
        return Workspace(self, project, WorkspaceInfo(**self._call("Restore", project=project, revision=revision)))

class Workspace:
    def __init__(self, client: Client, project: str, info: WorkspaceInfo):
        self._client, self._project, self._info = client, project, info
        self._pending: Publication | None = None
        self._lock = RLock()

    @property
    def id(self) -> str:
        return self._info.id

    @property
    def info(self) -> WorkspaceInfo:
        return self._info

    @property
    def pending_publication(self) -> Publication | None:
        return self._pending

    def _editable(self):
        if self._pending is not None:
            raise _invalid("retry the pending publication before changing this workspace")
        if self.info.status != "open":
            raise _invalid("workspace is closed")

    def _update(self, reply):
        self._info = WorkspaceInfo(self.id, reply.get("base_revision", self.info.base_revision), reply.get("version", self.info.version), reply.get("status", self.info.status))
        return reply

    def refresh(self) -> WorkspaceInfo:
        with self._lock:
            self._info = WorkspaceInfo(**self._client._call("GetWorkspace", project=self._project, workspace=self.id))
            return self.info

    def save(self, dataset: str, feature: dict[str, Any] | str) -> dict[str, Any]:
        try:
            decoded = json.loads(feature) if isinstance(feature, str) else feature
            if not isinstance(decoded, dict) or not isinstance(decoded.get("id"), str):
                raise ValueError()
        except (ValueError, TypeError):
            raise _invalid("GeoJSON Feature requires a string id") from None
        return self.save_batch([Edit(dataset, decoded["id"], feature)])

    def delete(self, dataset: str, feature_id: str) -> dict[str, Any]:
        return self.save_batch([Edit(dataset, feature_id, None)])

    def save_batch(self, edits: list[Edit]) -> dict[str, Any]:
        return self._mutate("Save", edits=edits)

    def _mutate(self, operation, **fields):
        with self._lock:
            self._editable()
            return self._update(self._client._call(operation, project=self._project, workspace=self.id, expected_workspace_version=self.info.version, **fields))

    def features(self, dataset: str, **query) -> dict[str, Any]:
        if "revision" in query or "workspace" in query:
            raise _invalid("draft queries cannot override the workspace or revision")
        return self._client.features(self._project, dataset, workspace=self.id, **query)

    def diff(self, *, after: str = "", limit: int = 100) -> dict[str, Any]:
        return self._client._call("Diff", project=self._project, workspace=self.id, after=after, limit=limit)

    def conflicts(self, *, after: str = "", limit: int = 100) -> dict[str, Any]:
        return self._client._call("Conflicts", project=self._project, workspace=self.id, after=after, limit=limit)

    def publish(self, message: str) -> dict[str, Any]:
        with self._lock:
            was_pending = self._pending is not None
            if self._pending is None:
                self._editable()
                self._pending = Publication(self._project, self.id, self.info.version, str(uuid4()), message)
            elif self._pending.message != message:
                raise _invalid("retry the pending publication with the original message")
            try:
                return self._update(self._client.publish(self._pending))
            except GeoLedgerError as e:
                if not e.uncertain and (not was_pending or e.code not in ("unauthenticated", "permission_denied", "not_found")):
                    self._pending = None
                raise

    def resolve(self, head: int, resolutions: list[Edit]) -> dict[str, Any]:
        return self._mutate("Resolve", expected_head=head, resolutions=resolutions)

    def rebase(self, head: int, resolutions: list[Edit] | None = None) -> dict[str, Any]:
        return self._mutate("Rebase", expected_head=head, resolutions=resolutions or [])

    def discard(self) -> dict[str, Any]:
        return self._mutate("Discard")
