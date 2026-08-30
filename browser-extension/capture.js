// Viewport / full-page / selector screenshots via CDP. JPEG viewport stays default.

async function captureWithCdp(tabId, params) {
  await chrome.debugger.attach({ tabId: tabId }, "1.3");
  try {
    return await chrome.debugger.sendCommand({ tabId: tabId }, "Page.captureScreenshot", params || {});
  } finally {
    await chrome.debugger.detach({ tabId: tabId }).catch(function () {});
  }
}

async function elementClip(tabId, selector) {
  var injected = await chrome.scripting.executeScript({
    target: { tabId: tabId },
    world: "MAIN",
    func: function (sel) {
      var el = document.querySelector(sel);
      if (!el) return null;
      var r = el.getBoundingClientRect();
      return { x: r.x, y: r.y, width: r.width, height: r.height, scale: window.devicePixelRatio || 1 };
    },
    args: [selector]
  });
  return injected[0] && injected[0].result;
}

async function capturePage(tabId, spec) {
  spec = spec || {};
  if (spec.selector) {
    var clip = await elementClip(tabId, spec.selector);
    if (!clip || clip.width <= 0 || clip.height <= 0) {
      throw new Error("selector did not match a visible element");
    }
    var shot = await captureWithCdp(tabId, {
      format: spec.format || "png",
      clip: { x: Math.max(0, clip.x), y: Math.max(0, clip.y), width: clip.width, height: clip.height, scale: 1 }
    });
    return { format: spec.format || "png", data: shot.data, clip: clip };
  }
  if (spec.clip) {
    var shot2 = await captureWithCdp(tabId, { format: spec.format || "png", clip: spec.clip });
    return { format: spec.format || "png", data: shot2.data, clip: spec.clip };
  }
  if (spec.full_page) {
    var metrics = await (async function () {
      await chrome.debugger.attach({ tabId: tabId }, "1.3");
      try {
        var layout = await chrome.debugger.sendCommand({ tabId: tabId }, "Page.getLayoutMetrics", {});
        var css = (layout && (layout.cssContentSize || layout.contentSize)) || { width: 800, height: 600 };
        var shot3 = await chrome.debugger.sendCommand({ tabId: tabId }, "Page.captureScreenshot", {
          format: spec.format || "png",
          captureBeyondViewport: true,
          clip: { x: 0, y: 0, width: css.width, height: Math.min(css.height, 16000), scale: 1 }
        });
        return { format: spec.format || "png", data: shot3.data, width: css.width, height: css.height };
      } finally {
        await chrome.debugger.detach({ tabId: tabId }).catch(function () {});
      }
    })();
    return metrics;
  }
  var viewport = await captureWithCdp(tabId, { format: spec.format || "jpeg", quality: spec.quality || 80 });
  return { format: spec.format || "jpeg", data: viewport.data };
}

if (typeof self !== "undefined") {
  self.capturePage = capturePage;
  self.captureWithCdp = captureWithCdp;
}