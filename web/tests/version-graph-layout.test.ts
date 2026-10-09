import assert from "node:assert/strict";
import test from "node:test";
import { layoutGraph } from "../src/lib/version-graph-layout";

test("assigns stale-base merges to their real lanes", () => {
  const layout = layoutGraph([
    { id: "r1", base: "0" },
    { id: "r2", base: "1" },
    { id: "r3", base: "1" },
    { id: "r4", base: "3" },
    { id: "r5", base: "4" },
    { id: "r6", base: "5" },
    { id: "r7", base: "4" },
  ]);

  const lane = (id: string) =>
    layout.nodes.find((node) => node.id === id)!.lane;
  assert.equal(lane("r1"), 0);
  assert.equal(lane("r3"), 0);
  assert.equal(lane("r4"), 0);
  assert.equal(lane("r7"), 0);
  assert.ok(lane("r2") > 0);
  assert.ok(lane("r5") > 0);
  assert.ok(lane("r6") > 0);
  assert.deepEqual(layout.edges.map(({ from, to }) => `${from}:${to}`).sort(), [
    "r2:r1",
    "r3:r1",
    "r3:r2",
    "r4:r3",
    "r5:r4",
    "r6:r5",
    "r7:r4",
    "r7:r6",
  ]);
  assert.ok(
    layout.edges.every(
      (edge) =>
        edge.fromLane === lane(edge.from) &&
        (edge.to === undefined || edge.toLane === lane(edge.to)),
    ),
  );
});

test("keeps continuous releases on one lane", () => {
  const layout = layoutGraph([
    { id: "r1", base: "0" },
    { id: "r2", base: "1" },
    { id: "r3", base: "2" },
  ]);

  assert.deepEqual(
    layout.nodes.map((node) => node.lane),
    [0, 0, 0],
  );
  assert.equal(layout.laneCount, 1);
});

test("keeps the sequential merge parent when a later release has base r0", () => {
  const layout = layoutGraph([
    { id: "r1", base: "0" },
    { id: "r2", base: "1" },
    { id: "r3", base: "0" },
  ]);

  assert.deepEqual(layout.edges.map(({ from, to }) => `${from}:${to}`).sort(), [
    "r2:r1",
    "r3:r2",
  ]);
});

test("puts off-page numeric sources at their true boundaries", () => {
  const layout = layoutGraph([
    { id: "r9007199254740993", base: "9007199254740000" },
    { id: "r9007199254740994", base: "9007199254740993" },
    { id: "draft:future", base: "9007199254741999", draft: true },
  ]);

  const older = layout.edges.find((edge) => edge.from === "r9007199254740993")!;
  const newer = layout.edges.find((edge) => edge.from === "draft:future")!;
  assert.equal(older.boundary, "top");
  assert.equal(newer.boundary, "bottom");
  assert.equal(older.to, undefined);
  assert.equal(newer.to, undefined);
});

test("keeps overlapping drafts on separate lanes", () => {
  const layout = layoutGraph([
    { id: "r1", base: "0" },
    { id: "r2", base: "1" },
    { id: "draft:a", base: "2", draft: true },
    { id: "draft:b", base: "1", draft: true },
  ]);

  const lane = (id: string) =>
    layout.nodes.find((node) => node.id === id)!.lane;
  assert.notEqual(lane("draft:a"), lane("draft:b"));
  assert.ok(lane("draft:a") > 0);
  assert.ok(lane("draft:b") > 0);
  assert.ok(layout.edges.every((edge) => edge.fromLane === lane(edge.from)));
});

test("keeps one head draft inline with its base", () => {
  const layout = layoutGraph([
    { id: "r1", base: "0" },
    { id: "r2", base: "1" },
    { id: "draft:head", base: "2", draft: true },
  ]);

  const lane = (id: string) =>
    layout.nodes.find((node) => node.id === id)!.lane;
  assert.equal(lane("draft:head"), lane("r2"));
});

test("lays out a single draft in an empty project without a parent", () => {
  const layout = layoutGraph([{ id: "draft:head", base: "0", draft: true }]);

  assert.equal(layout.nodes[0].lane, 0);
  assert.deepEqual(layout.edges, []);
});

test("does not invent an edge without a source base", () => {
  const layout = layoutGraph([{ id: "r1" }, { id: "r2" }]);

  assert.deepEqual(layout.edges, []);
});
