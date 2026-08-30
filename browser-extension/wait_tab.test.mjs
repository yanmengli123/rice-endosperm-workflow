import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const dir = path.dirname(fileURLToPath(import.meta.url));
const source = fs.readFileSync(path.join(dir, "wait_tab.js"), "utf8");
const root = {};
vm.runInNewContext(source, {
  self: root,
  globalThis: root,
  setTimeout,
  clearTimeout,
  Date,
});
const { createTabWaiter } = root;

function fakeChrome(initial) {
  const listeners = new Set();
  const tabs = new Map();
  if (initial) tabs.set(initial.id, { ...initial });
  return {
    tabs: {
      async get(id) {
        const tab = tabs.get(id);
        if (!tab) throw new Error("no tab");
        return { ...tab };
      },
      onUpdated: {
        addListener(fn) {
          listeners.add(fn);
        },
        removeListener(fn) {
          listeners.delete(fn);
        },
      },
      set(tab) {
        tabs.set(tab.id, { ...tab });
      },
      emit(id, change) {
        const current = tabs.get(id) || { id };
        const next = { ...current, ...change };
        tabs.set(id, next);
        for (const fn of listeners) fn(id, change, { ...next });
      },
      listenerCount() {
        return listeners.size;
      },
    },
  };
}

test("already-complete http(s) tab returns immediately", async () => {
  const chrome = fakeChrome({
    id: 1,
    url: "https://pubmed.ncbi.nlm.nih.gov/",
    title: "PubMed",
    status: "complete",
  });
  const { waitTabComplete } = createTabWaiter(chrome);
  const result = await waitTabComplete(1, Date.now() + 1_000);
  assert.equal(result.ready, true);
  assert.equal(result.url, "https://pubmed.ncbi.nlm.nih.gov/");
  assert.equal(result.wait.until, "complete");
  assert.ok(result.wait.waited_ms < 200);
  assert.equal(result.wait.timed_out, undefined);
  assert.equal(chrome.tabs.listenerCount(), 0);
});

test("about:blank complete is ignored until the http(s) document completes", async () => {
  const chrome = fakeChrome({
    id: 2,
    url: "about:blank",
    title: "",
    status: "complete",
  });
  const { waitTabComplete } = createTabWaiter(chrome);
  const pending = waitTabComplete(2, Date.now() + 1_000);
  await new Promise((resolve) => setTimeout(resolve, 20));
  chrome.tabs.emit(2, {
    status: "loading",
    url: "https://example.com",
  });
  chrome.tabs.emit(2, {
    status: "complete",
    url: "https://example.com",
    title: "Example",
  });
  const result = await pending;
  assert.equal(result.ready, true);
  assert.equal(result.url, "https://example.com");
  assert.equal(result.title, "Example");
  assert.equal(chrome.tabs.listenerCount(), 0);
});

test("loading then complete resolves ready", async () => {
  const chrome = fakeChrome({
    id: 3,
    url: "https://example.com",
    title: "",
    status: "loading",
  });
  const { waitTabComplete } = createTabWaiter(chrome);
  const pending = waitTabComplete(3, Date.now() + 1_000);
  setTimeout(() => {
    chrome.tabs.emit(3, { status: "complete", title: "Example" });
  }, 15);
  const result = await pending;
  assert.equal(result.ready, true);
  assert.equal(result.status, "complete");
  assert.equal(result.wait.timed_out, undefined);
});

test("timeout while still loading returns ready=false", async () => {
  const chrome = fakeChrome({
    id: 4,
    url: "https://example.com",
    title: "",
    status: "loading",
  });
  const { waitTabComplete } = createTabWaiter(chrome);
  const result = await waitTabComplete(4, Date.now() + 30);
  assert.equal(result.ready, false);
  assert.equal(result.wait.timed_out, true);
  assert.equal(result.wait.status, "loading");
  assert.equal(chrome.tabs.listenerCount(), 0);
});

test("missing tab times out immediately", async () => {
  const chrome = fakeChrome(null);
  const { waitTabComplete } = createTabWaiter(chrome);
  const result = await waitTabComplete(99, Date.now() + 1_000);
  assert.equal(result.ready, false);
  assert.equal(result.wait.timed_out, true);
  assert.ok(result.wait.waited_ms < 200);
});
