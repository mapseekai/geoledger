"""Business API roundtrip against a disposable service; no generated or RPC imports."""
import json
import os
from dataclasses import replace
from uuid import uuid4
from geoledger import Client, Edit, GeoLedgerError

with open(os.environ["GL_API_KEY_FILE"]) as source:
    api_key = source.read().strip()
with Client(os.environ.get("GL_ENDPOINT", "http://127.0.0.1:7882"), api_key) as client:
    info = client.info()
    project = client.create_project("python-" + str(uuid4()))
    dataset = client.create_dataset(project.id, "places", "point", 3, crs="EPSG:3857")
    assert dataset.crs == "EPSG:3857"
    draft = client.create_workspace(project.id)
    feature = {"type": "Feature", "id": "one", "properties": {"exact": 18446744073709551615, "nested": {"geojson": "hello"}, "detail_json": "plain"}, "geometry": {"type": "Point", "coordinates": [500000, 3500000, 3]}}
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
    assert client.workspace_summary(project.id,published["workspace"])["total"]["added"] == 1
    assert client.commit_summary(project.id,published["revision"])["total"]["added"] == 1
    assert client.features(project.id, dataset.id, revision=published["revision"])["features"][0] == feature
    history = client.history(project.id)
    assert len(history) == 1 and history[0]["source_workspace"] == published["workspace"] and history[0]["source_base_revision"] == 0
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
        client.create_dataset(project.id, "places", "point", 3, crs="EPSG:3857")
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
    changed = {**feature, "geometry": {"type": "Point", "coordinates": [4, 5, 6]}}
    edit = client.create_workspace(project.id)
    edit.save(dataset.id, changed)
    edit.publish("installed SDK update")
    assert client.features(project.id, dataset.id)["features"][0] == changed
    removal = client.create_workspace(project.id)
    removal.delete(dataset.id, "one")
    removal.publish("installed SDK delete")
    assert client.features(project.id, dataset.id)["features"] == []
    assert client.features(project.id, dataset.id, revision=published["revision"])["features"][0] == feature
    client.set_member(project.id, os.environ["GL_TEST_MEMBER_ID"], "viewer")
    assert {m.subject for m in client.members(project.id)} >= {os.environ["GL_TEST_MEMBER_ID"]}
    client.remove_member(project.id, os.environ["GL_TEST_MEMBER_ID"])
    assert os.environ["GL_TEST_MEMBER_ID"] not in {m.subject for m in client.members(project.id)}
    assert client.archive_project(project.id, True).state == "archived"
    assert client.archive_project(project.id, False).state == "active"
    client.delete_project(project.id, project.name)
    print(f"Python SDK: {info['backend']} business API, exact numbers, versions, retry, conflict resolution, restore OK")
