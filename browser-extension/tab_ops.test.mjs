import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const dir = path.dirname(fileURLToPath(import.meta.url));
const source = fs.readFileSync(path.join(dir, "tab_ops.js"), "utf8");
const root = {};
vm.runInNewContext(source, { self: root, globalThis: root, setTimeout, clearTimeout, Promise });

test("tab drag errors are retryable TAB_BUSY", () => {
  assert.equal(root.isTransientTabError(new Error("Tabs cannot be edited right now (user may be dragging a tab).")), true);
  assert.equal(root.isTransientTabError(new Error("permission denied")), false);
});

test("withTabRetry eventually succeeds", async () => {
  let n = 0;
  const value = await root.withTabRetry(async () => {
    n += 1;
    if (n < 3) throw new Error("Tabs cannot be edited right now");
    return "ok";
  }, 4);
  assert.equal(value, "ok");
  assert.equal(n, 3);
});