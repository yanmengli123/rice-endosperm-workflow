import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const dir = path.dirname(fileURLToPath(import.meta.url));
const source = fs.readFileSync(path.join(dir, "wait_engine.js"), "utf8");

function loadEngine(tabsApi) {
  const root = { setTimeout, clearTimeout, Promise };
  vm.runInNewContext(source, { self: root, globalThis: root, setTimeout, clearTimeout, Promise });
  return root.createWaitEngine({
    tabs: tabsApi,
    scripting: {
      executeScript: async () => [{ result: { selector: true, text_includes: true, text_not_includes: true, ready_state: "complete" } }],
    },
  });
}

test("wait_engine becomes ready on complete https tab", async () => {
  const tab = { id: 7, url: "https://zenodo.org/records/1", title: "Record", status: "complete" };
  const listeners = [];
  const engine = loadEngine({
    get: async () => tab,
    onUpdated: {
      addListener: (fn) => listeners.push(fn),
      removeListener: (fn) => {
        const idx = listeners.indexOf(fn);
        if (idx >= 0) listeners.splice(idx, 1);
      },
    },
  });
  const result = await engine.waitFor(7, { until: "complete" }, Date.now() + 1000);
  assert.equal(result.ready, true);
  assert.equal(result.url, tab.url);
  assert.equal(result.wait.timed_out, undefined);
});

test("wait_engine times out when the tab never completes", async () => {
  const tab = { id: 8, url: "", title: "", status: "loading" };
  const engine = loadEngine({
    get: async () => tab,
    onUpdated: {
      addListener: () => {},
      removeListener: () => {},
    },
  });
  const result = await engine.waitFor(8, { until: "complete" }, Date.now() + 30);
  assert.equal(result.ready, false);
  assert.equal(result.wait.timed_out, true);
});
