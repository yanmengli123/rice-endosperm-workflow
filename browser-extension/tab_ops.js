// Tab operations with retry for transient Chrome errors (tab drag / moving).

function isTransientTabError(error) {
  var message = (error && error.message) || String(error || "");
  return /Tabs cannot be edited right now|user may be dragging|being dragged|No tab with id/i.test(message);
}

async function withTabRetry(operation, attempts) {
  var max = attempts || 4;
  var lastError = null;
  for (var i = 0; i < max; i++) {
    try {
      return await operation();
    } catch (error) {
      lastError = error;
      if (!isTransientTabError(error) || i === max - 1) {
        var wrapped = new Error(error.message || String(error));
        wrapped.code = isTransientTabError(error) ? "TAB_BUSY" : "TAB_ERROR";
        wrapped.retryable = isTransientTabError(error);
        wrapped.operation = "tab";
        wrapped.attempts = i + 1;
        throw wrapped;
      }
      await new Promise(function (resolve) { setTimeout(resolve, 80 * Math.pow(2, i)); });
    }
  }
  throw lastError;
}

function createTabOps(chromeLike, managedIds) {
  var managed = managedIds || new Set();

  var publicTab = function (tab) {
    if (!tab) return null;
    return {
      id: tab.id,
      url: tab.url || "",
      title: tab.title || "",
      status: tab.status || "",
      active: !!tab.active,
      windowId: tab.windowId,
      managed: managed.has(tab.id)
    };
  };

  return {
    markManaged: function (id) {
      if (Number.isInteger(id)) managed.add(id);
    },
    unmark: function (id) {
      managed.delete(id);
    },
    isManaged: function (id) {
      return managed.has(id);
    },
    releaseAll: function () {
      managed.clear();
      return true;
    },
    create: async function (url, active) {
      var tab = await withTabRetry(function () {
        return chromeLike.tabs.create({ url: url, active: active ?? false });
      });
      managed.add(tab.id);
      return publicTab(tab);
    },
    close: async function (ids) {
      var closed = [];
      for (var i = 0; i < ids.length; i++) {
        var id = ids[i];
        try {
          await withTabRetry(function () { return chromeLike.tabs.remove(id); });
          closed.push(id);
          managed.delete(id);
        } catch (_) {}
      }
      return closed;
    },
    switchTo: async function (tabId) {
      var tab = await withTabRetry(function () {
        return chromeLike.tabs.update(tabId, { active: true });
      });
      await withTabRetry(function () {
        return chromeLike.windows.update(tab.windowId, { focused: true });
      });
      return { ok: true, tab: publicTab(tab) };
    },
    get: async function (tabId) {
      try {
        return publicTab(await chromeLike.tabs.get(tabId));
      } catch (_) {
        return null;
      }
    },
    publicTab: publicTab
  };
}

if (typeof self !== "undefined") {
  self.createTabOps = createTabOps;
  self.isTransientTabError = isTransientTabError;
  self.withTabRetry = withTabRetry;
}
