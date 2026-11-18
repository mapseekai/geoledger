"""SDK contract regressions, without a running service or public transport API."""
from dataclasses import FrozenInstanceError, asdict
from types import SimpleNamespace
import json
import unittest
import grpc

import geoledger
from geoledger import Client, Edit, GeoLedgerError, Workspace, WorkspaceInfo
from geoledger._internal.v1 import geoledger_pb2 as wire


class ContractTests(unittest.TestCase):
    def client(self, stub):
        client = Client("http://127.0.0.1:7882", "glk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        self.addCleanup(client.close)
        client._stub = stub
        return client

    def test_schema_query_and_save_intent_presence(self):
        sent = []
        def save(request, **kwargs):
            sent.append(request)
            return wire.SaveReply(version=request.expected_workspace_version+1, changes=1)
        def schema(request, **kwargs):
            self.assertEqual(request.dataset, "d")
            return wire.SchemaReply(revision=request.revision if request.HasField("revision") else 1,
                                    **({"workspace_version": 4} if request.HasField("workspace") else {}),
                                    fields=[wire.SchemaField(name="id", type="int8", nullable=False, editable=False)])
        client = self.client(SimpleNamespace(Save=save, Schema=schema))
        workspace = Workspace(client, "p", WorkspaceInfo("w", 1, 4, "open"))
        workspace.save_changes([], [geoledger.SchemaEdit("d", [geoledger.AddField("note", "string")])])
        workspace.save_batch([])
        workspace.save_changes([], [])
        self.assertEqual(sent[0].schema_edits.edits[0].add[0].type, "string")
        self.assertFalse(sent[1].HasField("schema_edits"))
        self.assertTrue(sent[2].HasField("schema_edits"))
        self.assertEqual(list(sent[2].schema_edits.edits), [])
        self.assertEqual([r.expected_workspace_version for r in sent], [4, 5, 6])
        self.assertEqual(workspace.schema("d")["workspace_version"], 4)
        result = client.schema("p", "d", revision=7)
        self.assertEqual(result["revision"], 7)
        self.assertIsNone(result["workspace_version"])
        self.assertFalse(result["fields"][0]["editable"])

    def test_register_table_preserves_create_dataset_wire_contract(self):
        source = dict(schema="business", table="roads", id_column="road_id", geometry_column="shape")
        sent = []
        def create(request, **kwargs):
            sent.append(request)
            return wire.DatasetReply(dataset="dataset", name="roads", geometry_type="line",
                                     coordinate_dimension=3, crs="EPSG:3857", postgis_table=request.postgis_table)
        client = self.client(SimpleNamespace(CreateDataset=create))
        dataset = client.register_table("project", "roads", **source)
        self.assertEqual(sent, [wire.DatasetRequest(project="project", name="roads", postgis_table=wire.PostgisTable(**source))])
        self.assertEqual((dataset.id, dataset.geometry_type, dataset.coordinate_dimension, dataset.crs),
                         ("dataset", "line", 3, "EPSG:3857"))
        self.assertEqual(dataset.postgis_table, source)

    def test_plaintext_to_remote_host_requires_opt_in(self):
        with self.assertRaises(geoledger.GeoLedgerError) as raised:
            geoledger.Client("http://geoledger.example:7882", "glk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        self.assertEqual(raised.exception.code, "invalid_argument")
        geoledger.Client("http://geoledger.example:7882", "glk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", allow_insecure=True).close()
        geoledger.Client("http://127.0.0.1:7882", "glk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").close()

    def test_public_api_has_no_generated_transport(self):
        self.assertNotIn("pb", geoledger.__all__)
        client = self.client(SimpleNamespace())
        self.assertFalse(hasattr(client, "rpc"))

    def test_features_decode_once_and_preserve_opaque_properties_and_integers(self):
        properties = {"nested": {"geojson": "ordinary text"}, "detail_json": "plain",
                      "exact": 9007199254740993, "max": 18446744073709551615}
        feature = {"type": "Feature", "id": "one", "geometry": None, "properties": properties}
        raw = json.dumps(feature)
        stub = SimpleNamespace(Features=lambda *a, **k: wire.FeaturesReply(
            features=[wire.Feature(geojson=raw)], revision=1))
        page = self.client(stub).features("project", "dataset")
        self.assertEqual(page["features"][0], feature)
        self.assertIsNone(page["workspace_version"])

    def test_audit_decodes_only_its_envelope(self):
        value = {"nested": {"geojson": "ordinary text", "detail_json": "plain"}}
        stub = SimpleNamespace(Audit=lambda *a, **k: wire.AuditReply(events=[wire.AuditEvent(
            id=1, detail_json=json.dumps(value))]))
        self.assertEqual(self.client(stub).audit("project")["events"][0]["detail"], value)

    def test_query_pages_preserve_continuations_and_terminal_none(self):
        for cursor in [None, "dataset/key/with/slashes"]:
            stub = SimpleNamespace(
                Diff=lambda *a, **k: wire.DiffReply(**({"next_after": cursor} if cursor else {})),
                Commit=lambda *a, **k: wire.CommitReply(**({"next_after": cursor} if cursor else {})))
            client = self.client(stub)
            workspace = Workspace(client, "project", WorkspaceInfo("draft", 0, 0, "open"))
            self.assertEqual(workspace.diff()["next_after"], cursor)
            self.assertEqual(client.commit("project", 1)["next_after"], cursor)
        for cursor in [None, 9007199254740993]:
            stub = SimpleNamespace(
                History=lambda *a, **k: wire.HistoryReply(**({"next_after": cursor} if cursor else {})),
                Audit=lambda *a, **k: wire.AuditReply(**({"next_after": cursor} if cursor else {})))
            client = self.client(stub)
            self.assertEqual(client.history("project"), {"commits": [], "next_after": cursor})
            self.assertEqual(client.audit("project")["next_after"], cursor)

    def test_missing_feature_is_rejected_but_explicit_delete_is_sent(self):
        sent = []
        def save(request, **kwargs):
            sent.append(request)
            return wire.SaveReply(version=1, changes=1)
        client = self.client(SimpleNamespace(Save=save))
        workspace = Workspace(client, "project", WorkspaceInfo("draft", 0, 0, "open"))
        with self.assertRaises(GeoLedgerError) as failure:
            workspace.save_batch([{"dataset": "dataset", "feature_id": "one"}])
        self.assertEqual(failure.exception.code, "invalid_argument")
        self.assertEqual(sent, [])
        workspace.delete("dataset", "one")
        self.assertFalse(sent[0].edits[0].HasField("feature"))
        self.assertEqual(workspace.info.version, 1)

    def test_unknown_publication_outcome_freezes_the_original_intent(self):
        sent = []
        def publish(intent):
            sent.append(asdict(intent))
            if len(sent) == 1:
                raise GeoLedgerError("unavailable", "response lost", uncertain=True)
            return {"revision": 2, "version": 5, "status": "published"}
        workspace = Workspace(SimpleNamespace(publish=publish), "project",
                              WorkspaceInfo("draft", 1, 4, "open"))
        with self.assertRaises(GeoLedgerError):
            workspace.publish("roads updated")
        intent = workspace.pending_publication
        self.assertIsNotNone(intent)
        with self.assertRaises(FrozenInstanceError):
            intent.message = "changed"
        with self.assertRaises(GeoLedgerError):
            workspace.publish("different message")
        with self.assertRaises(GeoLedgerError):
            workspace.delete("dataset", "one")
        self.assertEqual(len(sent), 1)
        workspace.publish("roads updated")
        self.assertEqual(sent[0], sent[1])
        self.assertEqual(workspace.info.version, 5)
        self.assertEqual(workspace.info.status, "published")

    def test_retry_access_and_input_failures_preserve_original_intent(self):
        for code in ("unauthenticated", "permission_denied", "not_found", "invalid_argument", "conflict", "unknown_business_error"):
            with self.subTest(code=code):
                sent = []
                def publish(intent):
                    sent.append(asdict(intent))
                    if len(sent) == 1:
                        raise GeoLedgerError("unavailable", "lost", uncertain=True)
                    if len(sent) == 2:
                        raise GeoLedgerError(code, "access denied")
                    return {"revision": 2, "version": 5, "status": "published"}
                workspace = Workspace(SimpleNamespace(publish=publish), "project",
                                      WorkspaceInfo("draft", 1, 4, "open"))
                for _ in range(2):
                    with self.assertRaises(GeoLedgerError):
                        workspace.publish("original")
                self.assertEqual(asdict(workspace.pending_publication), sent[0])
                with self.assertRaises(GeoLedgerError):
                    workspace.publish("changed")
                workspace.publish("original")
                self.assertEqual(sent, [sent[0]] * 3)

    def test_first_access_failure_releases_intent(self):
        for code in ("unauthenticated", "permission_denied", "not_found", "invalid_argument"):
            def publish(intent):
                raise GeoLedgerError(code, "access denied")
            workspace = Workspace(SimpleNamespace(publish=publish), "project",
                                  WorkspaceInfo("draft", 1, 4, "open"))
            with self.assertRaises(GeoLedgerError):
                workspace.publish("original")
            self.assertIsNone(workspace.pending_publication)

    def test_first_source_rejection_is_definite_at_both_client_levels(self):
        detail = wire.ErrorDetail(code="publication_attempt_rejected", message="source rejected", request_id="request")
        envelope = wire.RpcStatus(code=3, message="source rejected")
        envelope.details.add(type_url="type.googleapis.com/geoledger.v1.ErrorDetail", value=detail.SerializeToString())
        class Rejected(grpc.RpcError):
            def code(self): return grpc.StatusCode.INVALID_ARGUMENT
            def details(self): return "source rejected"
            def trailing_metadata(self): return [("grpc-status-details-bin", envelope.SerializeToString())]
        sent, saved = [], []
        def publish(request, **kwargs):
            sent.append(request)
            if len(sent) <= 2:
                raise Rejected()
            return wire.PublishReply(revision=2, workspace="draft", version=request.expected_workspace_version+1,
                                     status="published", changes=1)
        def save(request, **kwargs):
            saved.append(request)
            return wire.SaveReply(version=request.expected_workspace_version+1, changes=1)
        client = self.client(SimpleNamespace(Publish=publish, Save=save))
        intent = geoledger.Publication("project", "draft", 4, "request", "original")
        with self.assertRaises(GeoLedgerError) as low:
            client.publish(intent)
        self.assertEqual(low.exception.code, "publication_attempt_rejected")
        self.assertFalse(low.exception.uncertain)
        workspace = Workspace(client, "project", WorkspaceInfo("draft", 1, 4, "open"))
        with self.assertRaises(GeoLedgerError) as first:
            workspace.publish("original")
        self.assertFalse(first.exception.uncertain)
        self.assertEqual(first.exception.request_id, "request")
        self.assertIsNone(workspace.pending_publication)
        self.assertEqual(workspace.info.version, 4)
        workspace.delete("dataset", "one")
        workspace.publish("corrected")
        self.assertEqual(len(saved), 1)
        self.assertEqual(sent[2].expected_workspace_version, 5)
        self.assertNotEqual(sent[1].request_id, sent[2].request_id)

    def test_source_rejection_cannot_settle_an_earlier_unknown_attempt(self):
        for uncertain in (False, True):
            with self.subTest(uncertain=uncertain):
                sent = []
                def publish(intent):
                    sent.append(asdict(intent))
                    if len(sent) == 1:
                        raise GeoLedgerError("unavailable", "old attempt still queued", uncertain=True)
                    if len(sent) == 2:
                        raise GeoLedgerError("publication_attempt_rejected", "retry rejected", uncertain=uncertain)
                    return {"revision": 2, "version": 5, "status": "published"}
                workspace = Workspace(SimpleNamespace(publish=publish), "project", WorkspaceInfo("draft", 1, 4, "open"))
                for _ in range(2):
                    with self.assertRaises(GeoLedgerError):
                        workspace.publish("original")
                self.assertEqual(asdict(workspace.pending_publication), sent[0])
                with self.assertRaises(GeoLedgerError):
                    workspace.delete("dataset", "one")
                workspace.publish("original")
                self.assertEqual(sent, [sent[0]] * 3)

    def test_only_workspace_version_conflict_settles_earlier_unknown_intent(self):
        for code in ("workspace_version_conflict", "conflict", "publication_attempt_rejected", "unknown_business_error"):
            sent = []
            def publish(intent):
                sent.append(asdict(intent))
                if len(sent) == 1:
                    raise GeoLedgerError("unavailable", "original still queued", uncertain=True)
                if len(sent) == 2:
                    raise GeoLedgerError(code, "business rejection")
                return {"revision": 2, "version": 5, "status": "published"}
            workspace = Workspace(SimpleNamespace(publish=publish), "project", WorkspaceInfo("draft", 1, 4, "open"))
            for _ in range(2):
                with self.assertRaises(GeoLedgerError):
                    workspace.publish("original")
            if code == "workspace_version_conflict":
                self.assertIsNone(workspace.pending_publication)
            else:
                self.assertEqual(asdict(workspace.pending_publication), sent[0])
                with self.assertRaises(GeoLedgerError):
                    workspace.delete("dataset", "one")
                workspace.publish("original")
                self.assertEqual(sent, [sent[0]] * 3)


    def test_deadline_and_range_errors_keep_canonical_codes_and_publication_intent(self):
        for status, fallback in ((grpc.StatusCode.DEADLINE_EXCEEDED, "deadline_exceeded"),
                                 (grpc.StatusCode.OUT_OF_RANGE, "resource_exhausted")):
            for business_code in (None, "business_range"):
                with self.subTest(status=status, business_code=business_code):
                    metadata = []
                    if business_code:
                        detail = wire.ErrorDetail(code=business_code, message="business rejection", request_id="request")
                        envelope = wire.RpcStatus(code=status.value[0], message="transport failure")
                        envelope.details.add(type_url="type.googleapis.com/geoledger.v1.ErrorDetail", value=detail.SerializeToString())
                        metadata = [("grpc-status-details-bin", envelope.SerializeToString())]
                    class Rejected(grpc.RpcError):
                        def code(self): return status
                        def details(self): return "transport failure"
                        def trailing_metadata(self): return metadata
                    sent = []
                    def publish(request, **kwargs):
                        sent.append(request)
                        raise Rejected()
                    workspace = Workspace(self.client(SimpleNamespace(Publish=publish)), "project", WorkspaceInfo("draft", 1, 4, "open"))
                    for _ in range(2):
                        with self.assertRaises(GeoLedgerError) as rejected:
                            workspace.publish("original")
                        self.assertEqual(rejected.exception.code, business_code or fallback)
                        self.assertTrue(rejected.exception.uncertain)
                    self.assertIsNotNone(workspace.pending_publication)
                    self.assertEqual(sent[0], sent[1])
                    with self.assertRaisesRegex(GeoLedgerError, "pending publication"):
                        workspace.delete("dataset", "one")

    def test_conflict_family_requires_a_matching_current_business_detail(self):
        for status in (grpc.StatusCode.ABORTED, grpc.StatusCode.ALREADY_EXISTS, grpc.StatusCode.FAILED_PRECONDITION):
            for kind in ("absent", "malformed", "mismatched", "empty code", "wrong type"):
                with self.subTest(status=status, kind=kind):
                    metadata = []
                    if kind != "absent":
                        detail = wire.ErrorDetail(code="" if kind == "empty code" else "workspace_version_conflict", message="not trusted", request_id="request")
                        envelope = wire.RpcStatus(code=14 if kind == "mismatched" else status.value[0], message="proxy rejection")
                        envelope.details.add(type_url="type.invalid/geoledger.v1.ErrorDetail" if kind == "wrong type" else "type.googleapis.com/geoledger.v1.ErrorDetail", value=detail.SerializeToString())
                        metadata = [("grpc-status-details-bin", b"\xff" if kind == "malformed" else envelope.SerializeToString())]
                    class Rejected(grpc.RpcError):
                        def code(self): return status
                        def details(self): return "proxy rejection"
                        def trailing_metadata(self): return metadata
                    sent = []
                    def publish(request, **kwargs):
                        sent.append(request)
                        raise Rejected()
                    client = self.client(SimpleNamespace(Publish=publish))
                    workspace = Workspace(client, "project", WorkspaceInfo("draft", 1, 4, "open"))
                    for _ in range(2):
                        with self.assertRaises(GeoLedgerError) as rejected:
                            workspace.publish("original")
                        self.assertEqual(rejected.exception.code, "conflict")
                        self.assertTrue(rejected.exception.uncertain)
                    self.assertIsNotNone(workspace.pending_publication)
                    self.assertEqual(sent[0], sent[1])
                    with self.assertRaises(GeoLedgerError): workspace.delete("dataset", "one")
                    intent = asdict(workspace.pending_publication)
                    def recover(original):
                        self.assertEqual(asdict(original), intent)
                        return {"revision": 2, "version": 5, "status": "published"}
                    client.publish = recover
                    workspace.publish("original")
                    self.assertEqual(workspace.info.status, "published")

    def test_workspace_version_proof_requires_aborted_and_complete_nonmerge_details(self):
        cases = [(grpc.StatusCode.ABORTED, {}, True)]
        cases += [(status, {}, False) for status in (grpc.StatusCode.PERMISSION_DENIED, grpc.StatusCode.INVALID_ARGUMENT, grpc.StatusCode.ALREADY_EXISTS, grpc.StatusCode.FAILED_PRECONDITION, grpc.StatusCode.UNAVAILABLE)]
        cases += [(grpc.StatusCode.ABORTED, changes, False) for changes in ({"message": ""}, {"message": " \t"}, {"request_id": ""}, {"request_id": " \t"}, {"conflicts": wire.ConflictsReply()})]
        for target_status, changes, valid in cases:
            with self.subTest(status=target_status, changes=changes):
                status, metadata = grpc.StatusCode.UNAVAILABLE, []
                class Rejected(grpc.RpcError):
                    def code(self): return status
                    def details(self): return "public failure"
                    def trailing_metadata(self): return metadata
                def publish(request, **kwargs): raise Rejected()
                workspace = Workspace(self.client(SimpleNamespace(Publish=publish)), "project", WorkspaceInfo("draft", 1, 4, "open"))
                with self.assertRaises(GeoLedgerError): workspace.publish("original")
                intent = asdict(workspace.pending_publication)
                fields = dict(code="workspace_version_conflict", message="workspace version changed", request_id="proof")
                fields.update(changes)
                detail = wire.ErrorDetail(**fields)
                status = target_status
                envelope = wire.RpcStatus(code=status.value[0], message="public failure")
                envelope.details.add(type_url="type.googleapis.com/geoledger.v1.ErrorDetail", value=detail.SerializeToString())
                metadata = [("grpc-status-details-bin", envelope.SerializeToString())]
                with self.assertRaises(GeoLedgerError) as rejected: workspace.publish("original")
                self.assertEqual(rejected.exception.code, "workspace_version_conflict")
                self.assertEqual(rejected.exception.uncertain, not valid)
                if valid: self.assertIsNone(workspace.pending_publication)
                else: self.assertEqual(asdict(workspace.pending_publication), intent)

    def test_invalid_publication_receipt_retains_intent_until_exact_receipt(self):
        valid = dict(workspace="draft", status="published", revision=2, version=5, changes=0)
        for bad in ({}, {**valid, "workspace": "other"}, {**valid, "version": 4}, {**valid, "status": "open"}, {**valid, "revision": 0}):
            with self.subTest(receipt=bad):
                receipt, sent = wire.PublishReply(**bad), []
                def publish(request, **kwargs):
                    sent.append(request)
                    return receipt
                workspace = Workspace(self.client(SimpleNamespace(Publish=publish)), "project", WorkspaceInfo("draft", 1, 4, "open"))
                with self.assertRaises(GeoLedgerError) as rejected: workspace.publish("original")
                self.assertEqual(rejected.exception.code, "invalid_response")
                self.assertTrue(rejected.exception.uncertain)
                intent = asdict(workspace.pending_publication)
                self.assertEqual(workspace.info.version, 4)
                self.assertEqual(workspace.info.status, "open")
                with self.assertRaises(GeoLedgerError): workspace.delete("dataset", "one")
                receipt = wire.PublishReply(**valid)
                result = workspace.publish("original")
                self.assertEqual(result["changes"], 0)
                self.assertEqual(workspace.info.status, "published")
                self.assertEqual(sent[0], sent[1])
                self.assertEqual(asdict(workspace.pending_publication), intent)


if __name__ == "__main__":
    unittest.main()
