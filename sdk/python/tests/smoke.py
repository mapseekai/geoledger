"""Business API roundtrip against a disposable service; no generated or RPC imports."""
import json
import os
from dataclasses import replace
from uuid import uuid4
from geoledger import Client, Edit, GeoLedgerError

with open(os.environ["GL_TOKEN_FILE"]) as source:
    token = json.load(source)[0]["token"]
with Client(os.environ.get("GL_ENDPOINT", "http://127.0.0.1:7882"), token) as client:
    info = client.info()
    project = client.create_project("python-" + str(uuid4()))
    dataset = client.create_dataset(project.id, "places")
    draft = client.create_workspace(project.id)
    feature = {"type": "Feature", "id": "one", "properties": {"exact": 18446744073709551615, "nested": {"geojson": "hello"}, "detail_json": "plain"}, "geometry": {"type": "Point", "coordinates": [1, 2, 3]}}
    assert draft.save(dataset.id, feature)["version"] == 1
    assert draft.save(dataset.id, feature)["version"] == 2
    assert draft.features(dataset.id)["features"][0] == feature
    try:
        draft.save_batch([{"dataset": dataset.id, "feature_id": "one"}])
        raise AssertionError("missing feature became deletion")
    except GeoLedgerError as e:
        assert e.code == "invalid_argument"
    draft.delete(dataset.id, "one")
    assert draft.features(dataset.id)["features"] == []
    draft.save(dataset.id, feature)
    assert draft.info.version == 4
    assert draft.diff()["changes"]
    published = draft.publish("Python SDK")
    assert draft.publish("Python SDK") == published
    assert client.features(project.id, dataset.id, revision=published["revision"])["features"][0] == feature
    assert len(client.history(project.id)) == 1
    assert len(client.commit(project.id, published["revision"])["changes"]) == 1
    assert client.audit(project.id)["events"]
    assert client.project(project.id).head == 1
    assert any(p.id == project.id for p in client.projects(limit=1000))
    assert client.datasets(project.id)[0].id == dataset.id
    assert client.workspaces(project.id)[0].status == "published"
    try:
        client.publish(replace(draft.pending_publication, message="different"))
        raise AssertionError("idempotency mismatch accepted")
    except GeoLedgerError as e:
        assert e.code == "conflict" and e.request_id and not e.uncertain
    try:
        client.create_dataset(project.id, "places")
        raise AssertionError("duplicate accepted")
    except GeoLedgerError as e:
        assert e.code == "conflict"
    left, right = client.create_workspace(project.id), client.create_workspace(project.id)
    left.save(dataset.id, {**feature, "properties": {"side": "left"}})
    right.save(dataset.id, {**feature, "properties": {"side": "right"}})
    head = left.publish("left")["revision"]
    try:
        right.publish("right")
        raise AssertionError("conflict accepted")
    except GeoLedgerError as e:
        assert e.code == "conflict" and e.conflicts["total"] > 0
    assert right.conflicts()["total"] > 0
    right.resolve(head, [Edit(dataset.id, "one", feature)])
    right.rebase(head)
    merged = right.publish("resolved")
    restored = client.restore(project.id, merged["revision"])
    restored.discard()
    assert client.workspace(project.id, restored.id).info.status == "discarded"
    client.set_member(project.id, "sdk-viewer", "viewer")
    print(f"Python SDK: {info['backend']} business API, exact numbers, versions, retry, conflict resolution, restore OK")
