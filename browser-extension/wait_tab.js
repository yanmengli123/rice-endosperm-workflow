// Tab document-complete waiter used by the Wisp Real Browser Bridge.
// Loaded via importScripts from background.js; also unit-tested in Node.

function createTabWaiter(chromeLike) {
  const isHttpUrl = (url) => /^https?:/i.test(url || "");

  const isReady = (tab, change) => {
    const url = (tab && tab.url) || (change && change.url) || "";
    const status = (tab && tab.status) || (change && change.status) || "";
    return status === "complete" && isHttpUrl(url);
  };

  const getTab = async (tabId) => {
    try {
      return await chromeLike.tabs.get(tabId);
    } catch (_) {
      return null;
    }
  };

  const snapshot = (tab, wait) => ({
    id: tab && tab.id != null ? tab.id : null,
    url: (tab && tab.url) || "",
    title: (tab && tab.title) || "",
    status: (tab && tab.status) || "",
    ready: !!(tab && isReady(tab)),
    wait,
  });

  const waitTabComplete = (tabId, deadlineMs) => {
    const started = Date.now();
    return new Promise((resolve) => {
      let done = false;
      let timer = null;

      const finish = async (timedOut, eventTab) => {
        if (done) return;
        const tab = eventTab || (await getTab(tabId));
        const ready = isReady(tab);
        if (!ready && !timedOut) return;
        done = true;
        if (timer != null) clearTimeout(timer);
        chromeLike.tabs.onUpdated.removeListener(onUpdated);
        const wait = {
          until: "complete",
          waited_ms: Math.max(0, Date.now() - started),
        };
        if (tab && tab.status) wait.status = tab.status;
        if (timedOut && !ready) wait.timed_out = true;
        resolve(snapshot(tab, wait));
      };

      const onUpdated = (id, change, tab) => {
        if (id !== tabId) return;
        if (isReady(tab, change)) finish(false, tab);
      };

      chromeLike.tabs.onUpdated.addListener(onUpdated);

      const remaining = deadlineMs - Date.now();
      timer = setTimeout(() => finish(true), Math.max(0, remaining));

      getTab(tabId).then((tab) => {
        if (done) return;
        if (isReady(tab)) {
          finish(false, tab);
          return;
        }
        if (!tab) finish(true, null);
      });
    });
  };

  return { waitTabComplete, isHttpUrl, isReady };
}

(function exportWaiter(root) {
  root.createTabWaiter = createTabWaiter;
})(typeof self !== "undefined" ? self : globalThis);
