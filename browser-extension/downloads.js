// Host-permission downloads. Never return large base64 payloads.

async function downloadAsset(spec) {
  var url = spec && spec.url;
  if (!url || !/^https?:/i.test(url)) {
    throw Object.assign(new Error("download url must be http(s)"), { code: "ASSET_BLOCKED" });
  }
  var filename = (spec.filename || url.split("?")[0].split("/").pop() || "download.bin").replace(/[\\/:*?"<>|]/g, "_");
  if (filename.length > 180) filename = filename.slice(-180);
  var target = "WispBrowserStaging/" + filename;
  var referrer = spec.referrer || "";

  var waitDownload = function (downloadId, timeoutMs) {
    return new Promise(function (resolve) {
      var done = false;
      var timer = setTimeout(function () {
        if (done) return;
        done = true;
        chrome.downloads.onChanged.removeListener(onChanged);
        resolve({ timed_out: true, download_id: downloadId });
      }, timeoutMs || 20000);
      var onChanged = function (delta) {
        if (delta.id !== downloadId || done) return;
        if (delta.state && delta.state.current === "complete") {
          done = true;
          clearTimeout(timer);
          chrome.downloads.onChanged.removeListener(onChanged);
          resolve({ complete: true, download_id: downloadId });
        } else if (delta.state && delta.state.current === "interrupted") {
          done = true;
          clearTimeout(timer);
          chrome.downloads.onChanged.removeListener(onChanged);
          resolve({ interrupted: true, download_id: downloadId });
        }
      };
      chrome.downloads.onChanged.addListener(onChanged);
    });
  };
  var describeDownload = async function (downloadId, extra) {
    var items = [];
    try { items = await chrome.downloads.search({ id: downloadId }); } catch (_) {}
    var item = items[0] || {};
    return Object.assign({
      ok: !item.error,
      download_id: downloadId,
      filename: item.filename || target,
      absolute_path: item.filename || "",
      mime: item.mime || extra.mime || null,
      bytes: item.fileSize || extra.bytes || null,
      final_url: item.finalUrl || extra.final_url || url,
      source_url: url
    }, extra);
  };

  try {
    var headers = {};
    if (referrer) headers.Referer = referrer;
    var response = await fetch(url, { headers: headers, credentials: "include", redirect: "follow" });
    if (!response.ok) {
      throw new Error("HTTP " + response.status);
    }
    var buffer = await response.arrayBuffer();
    var mime = response.headers.get("content-type") || spec.mime || "application/octet-stream";
    var blob = new Blob([buffer], { type: mime.split(";")[0] });
    var objectUrl = URL.createObjectURL(blob);
    try {
      var downloadId = await chrome.downloads.download({
        url: objectUrl,
        filename: target,
        conflictAction: "uniquify",
        saveAs: false
      });
      await waitDownload(downloadId, 20000);
      return await describeDownload(downloadId, {
        method: "extension_fetch",
        bytes: buffer.byteLength,
        mime: mime,
        final_url: response.url || url
      });
    } finally {
      setTimeout(function () { URL.revokeObjectURL(objectUrl); }, 15000);
    }
  } catch (error) {
    var downloadId = await chrome.downloads.download({
      url: url,
      filename: target,
      conflictAction: "uniquify",
      saveAs: false
    });
    await waitDownload(downloadId, 20000);
    return await describeDownload(downloadId, {
      method: "chrome_downloads",
      mime: spec.mime || null,
      note: "extension fetch failed (" + (error.message || error) + "); used browser download"
    });
  }
}

if (typeof self !== "undefined") {
  self.downloadAsset = downloadAsset;
}
