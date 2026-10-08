"""SDK contract regressions, without a running service or public transport API."""
from dataclasses import FrozenInstanceError, asdict
from types import SimpleNamespace
import json
import unittest

import geoledger
from geoledger import Client, Edit, GeoLedgerError, Workspace, WorkspaceInfo
from geoledger._internal.v1 import geoledger_pb2 as wire


class ContractTests(unittest.TestCase):
    def client(self, stub):
        client = Client("http://127.0.0.1:7882", "test-only-token")
        self.addCleanup(client.close)
        client._stub = stub
        return client

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

    def test_definite_conflict_releases_publication_intent(self):
        def publish(intent):
            raise GeoLedgerError("conflict", "merge conflicts")
        workspace = Workspace(SimpleNamespace(publish=publish), "project",
                              WorkspaceInfo("draft", 1, 4, "open"))
        with self.assertRaises(GeoLedgerError):
            workspace.publish("roads updated")
        self.assertIsNone(workspace.pending_publication)
        self.assertEqual(workspace.info.version, 4)


if __name__ == "__main__":
    unittest.main()
