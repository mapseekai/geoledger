import { ApiError } from "./browser-api";
import assert from "node:assert/strict";
import test from "node:test";
import { resolveProject } from "./project-context";

test("project resolution preserves explicit failures and only recovers invalid cached projects", async () => {
  const project = {
    id: "a",
    name: "A",
    head: "0",
    role: "owner",
    state: "active",
  };
  let explicitListed = false;
  await assert.rejects(
    resolveProject({
      explicit: "bad",
      remembered: "a",
      getProject: async (id) => {
        if (id === "bad") throw new ApiError("不存在", 404);
        return project;
      },
      listProjectIds: async () => {
        explicitListed = true;
        return ["a"];
      },
    }),
  );
  assert.equal(explicitListed, false);
  assert.deepEqual(
    await resolveProject({
      explicit: undefined,
      remembered: "bad",
      getProject: async (id) => {
        if (id === "bad") throw new ApiError("不存在", 404);
        return project;
      },
      listProjectIds: async () => ["a"],
    }),
    { project, source: "default", forgotRemembered: true },
  );
  for (const status of [401, 500]) {
    await assert.rejects(
      resolveProject({
        explicit: undefined,
        remembered: "bad",
        getProject: async () => {
          throw new ApiError("服务错误", status);
        },
        listProjectIds: async () => ["a"],
      }),
    );
  }
  assert.deepEqual(
    await resolveProject({
      explicit: undefined,
      remembered: "a",
      getProject: async () => project,
      listProjectIds: async () => ["b"],
    }),
    { project, source: "remembered", forgotRemembered: false },
  );
  assert.deepEqual(
    await resolveProject({
      explicit: undefined,
      remembered: null,
      getProject: async () => project,
      listProjectIds: async () => ["a"],
    }),
    { project, source: "default", forgotRemembered: false },
  );
});
