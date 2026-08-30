import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const dir = path.dirname(fileURLToPath(import.meta.url));
const source = fs.readFileSync(path.join(dir, "scan_page.js"), "utf8");
assert.match(source, /ready_state: document.readyState/);
assert.match(source, /images/);
assert.match(source, /code_blocks/);
assert.match(source, /lazy_undecoded/);
assert.match(source, /data_src/);

test("scan_page.js exports article fields", () => {
  const root = {};
  vm.runInNewContext(source, { self: root, globalThis: root });
  assert.equal(typeof root.pageScanFunctions, "function");
});