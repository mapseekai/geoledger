import assert from "node:assert/strict";
import test from "node:test";
import { recoverImportBatch } from "../src/lib/import-recovery";
import { call, type Workspace } from "../src/lib/browser-api";

const previous: Workspace = {
  id: "w",
  baseRevision: "0",
  version: "9007199254740993",
  status: "open",
};
const raw =
  '{"type":"Feature","id":"one","properties":{"exact":18446744073709551615},"geometry":null}';
const batch = [{ id: "one", raw }];
function mock(
  workspace: Workspace,
  feature = raw,
  pageVersion = workspace.version,
) {
  const calls: Record<string, unknown>[] = [];
  const request = (async (payload: Record<string, unknown>) => {
    calls.push(payload);
    return payload.action === "workspace"
      ? workspace
      : {
          workspaceVersion: pageVersion,
          features: [{ id: "one", geojson: feature }],
        };
  }) as typeof call;
  return { calls, request };
}
test("uncommitted save retries using the original version", async () => {
  const { request, calls } = mock(previous);
  assert.equal(
    await recoverImportBatch("p", "d", previous, batch, request),
    undefined,
  );
  assert.equal(calls.length, 1);
});
test("lost committed response confirms exact batch with pinned version", async () => {
  const next = { ...previous, version: "9007199254740994" };
  const reordered =
    '{"geometry":null,"properties":{"exact":18446744073709551615},"id":"one","type":"Feature"}';
  const { request, calls } = mock(next, reordered);
  assert.deepEqual(
    await recoverImportBatch("p", "d", previous, batch, request),
    next,
  );
  assert.equal(calls[1].workspaceVersion, next.version);
});
test("concurrent edits, rebase, closed workspace and stale feature snapshots stop recovery", async () => {
  const next = { ...previous, version: "9007199254740994" };
  for (const { request } of [
    mock({ ...next, version: "9007199254740995" }),
    mock({ ...next, baseRevision: "1" }),
    mock({ ...next, status: "published" }),
    mock(next, raw.replace("18446744073709551615", "18446744073709551614")),
    mock(next, raw, previous.version),
  ])
    await assert.rejects(
      recoverImportBatch("p", "d", previous, batch, request),
      /其他修改/,
    );
});
test("every pending row must be confirmed", async () => {
  const { request } = mock({ ...previous, version: "9007199254740994" });
  await assert.rejects(
    recoverImportBatch(
      "p",
      "d",
      previous,
      [...batch, { id: "two", raw: raw.replace("one", "two") }],
      request,
    ),
    /其他修改/,
  );
});

test("numeric formatting normalizes without rounding integer values", async () => {
  const variants = raw
    .replace(
      '"geometry":null',
      '"geometry":{"type":"Point","coordinates":[1.0,-0.0]}',
    )
    .replace(
      '"exact":18446744073709551615',
      '"exact":18446744073709551615,"exponent":1e0',
    );
  const normalized = variants.replace("1.0,-0.0", "1,0").replace("1e0", "1");
  const { request } = mock(
    { ...previous, version: "9007199254740994" },
    normalized,
  );
  assert.ok(
    await recoverImportBatch(
      "p",
      "d",
      previous,
      [{ id: "one", raw: variants }],
      request,
    ),
  );
});

test("decimal recovery follows binary64 rounding but rejects integer loss and underflow", async () => {
  const next = { ...previous, version: "9007199254740994" };
  const withNumber = (token: string) =>
    raw.replace("18446744073709551615", token);
  const { request } = mock(next, withNumber("1.2345678901234567"));
  assert.ok(
    await recoverImportBatch(
      "p",
      "d",
      previous,
      [{ id: "one", raw: withNumber("1.23456789012345678") }],
      request,
    ),
  );
  for (const [wanted, actual] of [
    ["9007199254740993", "9007199254740992"],
    ["1e-400", "0"],
  ]) {
    const { request } = mock(next, withNumber(actual));
    await assert.rejects(
      recoverImportBatch(
        "p",
        "d",
        previous,
        [{ id: "one", raw: withNumber(wanted) }],
        request,
      ),
      /其他修改/,
    );
  }
});
