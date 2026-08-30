// Conditional wait engine for document, URL, selector, and text stability.

function createWaitEngine(chromeLike) {
  var isHttpUrl = function (url) {
    return /^https?:/i.test(url || "");
  };

  var getTab = async function (tabId) {
    try {
      return await chromeLike.tabs.get(tabId);
    } catch (_) {
      return null;
    }
  };

  var snapshot = function (tab, wait) {
    return {
      id: tab && tab.id != null ? tab.id : null,
      url: (tab && tab.url) || "",
      title: (tab && tab.title) || "",
      status: (tab && tab.status) || "",
      ready: !!(tab && tab.status === "complete" && isHttpUrl(tab.url)),
      wait: wait
    };
  };

  var pageMatches = async function (tabId, spec) {
    if (!spec) return { ok: true };
    var details = {};
    if (spec.selector || spec.text_includes || spec.text_not_includes) {
      try {
        var injected = await chromeLike.scripting.executeScript({
          target: { tabId: tabId },
          world: "MAIN",
          func: function (sel, includes, excludes) {
            var text = (document.body && document.body.innerText) || "";
            var hasSel = !sel || !!document.querySelector(sel);
            var hasInc = !includes || text.toLowerCase().indexOf(String(includes).toLowerCase()) >= 0;
            var hasExc = !excludes || text.toLowerCase().indexOf(String(excludes).toLowerCase()) < 0;
            return {
              selector: hasSel,
              text_includes: hasInc,
              text_not_includes: hasExc,
              ready_state: document.readyState,
              url: location.href
            };
          },
          args: [spec.selector || "", spec.text_includes || "", spec.text_not_includes || ""]
        });
        details = (injected[0] && injected[0].result) || {};
        if (spec.selector && !details.selector) return { ok: false, details: details };
        if (spec.text_includes && !details.text_includes) return { ok: false, details: details };
        if (spec.text_not_includes && !details.text_not_includes) return { ok: false, details: details };
      } catch (error) {
        return { ok: false, error: error.message || String(error) };
      }
    }
    return { ok: true, details: details };
  };

  var waitFor = function (tabId, spec, deadlineMs) {
    spec = spec || { until: "complete" };
    var started = Date.now();
    var settleMs = Number(spec.settle_ms);
    if (!Number.isFinite(settleMs) || settleMs < 0) settleMs = 0;
    var urlPattern = spec.url_matches ? String(spec.url_matches) : "";

    return new Promise(function (resolve) {
      var done = false;
      var timer = null;
      var settleTimer = null;
      var lastOkAt = 0;

      var finish = async function (timedOut, eventTab) {
        if (done) return;
        var tab = eventTab || (await getTab(tabId));
        var url = (tab && tab.url) || "";
        var status = (tab && tab.status) || "";
        var urlOk = !urlPattern || url.indexOf(urlPattern) >= 0 || (function () {
          try { return new RegExp(urlPattern).test(url); } catch (_) { return false; }
        })();
        var docOk = status === "complete" && isHttpUrl(url);
        var page = await pageMatches(tabId, spec);
        var ready = docOk && urlOk && page.ok;
        if (ready && settleMs > 0 && !timedOut) {
          if (lastOkAt === 0) {
            lastOkAt = Date.now();
            if (settleTimer == null) {
              settleTimer = setTimeout(function () { finish(false); }, settleMs);
            }
            return;
          }
          if (Date.now() - lastOkAt < settleMs) return;
        }
        if (!ready && !timedOut) {
          lastOkAt = 0;
          if (settleTimer != null) {
            clearTimeout(settleTimer);
            settleTimer = null;
          }
          return;
        }
        done = true;
        if (timer != null) clearTimeout(timer);
        if (settleTimer != null) clearTimeout(settleTimer);
        chromeLike.tabs.onUpdated.removeListener(onUpdated);
        var wait = {
          until: spec.until || "complete",
          waited_ms: Math.max(0, Date.now() - started)
        };
        if (tab && tab.status) wait.status = tab.status;
        if (urlPattern) wait.url_matches = urlOk;
        if (timedOut && !ready) wait.timed_out = true;
        if (page.details) wait.page = page.details;
        resolve(snapshot(tab, wait));
      };

      var onUpdated = function (id, change, tab) {
        if (id !== tabId) return;
        finish(false, tab);
      };

      chromeLike.tabs.onUpdated.addListener(onUpdated);
      timer = setTimeout(function () { finish(true); }, Math.max(0, deadlineMs - Date.now()));
      getTab(tabId).then(function (tab) {
        if (done) return;
        if (!tab) {
          finish(true, null);
          return;
        }
        finish(false, tab);
      });
    });
  };

  return { waitFor: waitFor, isHttpUrl: isHttpUrl, pageMatches: pageMatches };
}

(function exportWaitEngine(root) {
  root.createWaitEngine = createWaitEngine;
})(typeof self !== "undefined" ? self : globalThis);
