// Page scanners. Default mode keeps the historical element snapshot.
// mode=article adds images, figures, and code blocks.

function pageScanFunctions() {
  var helpers = function () {
    var visible = function (el) {
      var s = getComputedStyle(el);
      var r = el.getBoundingClientRect();
      return s.display !== "none" && s.visibility !== "hidden" && Number(s.opacity) > 0 && r.width > 0 && r.height > 0;
    };
    var selector = function (el) {
      if (el.id) {
        var id = "#" + CSS.escape(el.id);
        if (document.querySelectorAll(id).length === 1) return id;
      }
      var parts = [];
      for (var node = el; node && node.nodeType === 1 && parts.length < 6; node = node.parentElement) {
        var part = node.tagName.toLowerCase();
        var siblings = node.parentElement
          ? Array.prototype.slice.call(node.parentElement.children).filter(function (x) { return x.tagName === node.tagName; })
          : [];
        if (siblings.length > 1) part += ":nth-of-type(" + (siblings.indexOf(node) + 1) + ")";
        parts.unshift(part);
        var candidate = parts.join(" > ");
        if (document.querySelectorAll(candidate).length === 1) return candidate;
      }
      return parts.join(" > ");
    };
    var rectOf = function (el) {
      var r = el.getBoundingClientRect();
      return [Math.round(r.x), Math.round(r.y), Math.round(r.width), Math.round(r.height)];
    };
    var inViewport = function (el) {
      var r = el.getBoundingClientRect();
      return r.bottom > 0 && r.right > 0 && r.top < innerHeight && r.left < innerWidth;
    };
    return { visible: visible, selector: selector, rectOf: rectOf, inViewport: inViewport };
  };

  var defaultScan = function () {
    var h = helpers();
    var query = "a,button,input,textarea,select,summary,[role],[contenteditable=true],h1,h2,h3,label";
    var elements = Array.prototype.slice.call(document.querySelectorAll(query)).filter(h.visible).slice(0, 400).map(function (el) {
      var type = el.getAttribute("type") || "";
      return {
        selector: h.selector(el),
        tag: el.tagName.toLowerCase(),
        role: el.getAttribute("role") || undefined,
        text: (el.innerText || el.textContent || "").trim().replace(/\s+/g, " ").slice(0, 500) || undefined,
        aria_label: el.getAttribute("aria-label") || undefined,
        href: el.href || undefined,
        type: type || undefined,
        value: type.toLowerCase() === "password" ? undefined : (el.value || undefined),
        disabled: !!el.disabled,
        rect: h.rectOf(el)
      };
    });
    return {
      url: location.href,
      title: document.title,
      viewport: [innerWidth, innerHeight],
      ready_state: document.readyState,
      text: ((document.body && document.body.innerText) || "").slice(0, 30000),
      elements: elements
    };
  };

  var textScan = function () {
    return {
      url: location.href,
      title: document.title,
      ready_state: document.readyState,
      text: ((document.body && document.body.innerText) || "").slice(0, 50000)
    };
  };

  var articleScan = function () {
    var h = helpers();
    var base = defaultScan();
    var images = Array.prototype.slice.call(document.querySelectorAll("img")).slice(0, 300).map(function (el) {
      var src = el.currentSrc || el.src || "";
      var dataSrc = el.getAttribute("data-src") || el.getAttribute("data-original") || "";
      return {
        selector: h.selector(el),
        src: src,
        data_src: dataSrc || undefined,
        current_src: el.currentSrc || undefined,
        alt: el.alt || undefined,
        natural_width: el.naturalWidth || 0,
        natural_height: el.naturalHeight || 0,
        width: el.width || 0,
        height: el.height || 0,
        in_viewport: h.inViewport(el),
        visible: h.visible(el),
        lazy_undecoded: !src || (el.naturalWidth === 0 && el.naturalHeight === 0),
        rect: h.rectOf(el)
      };
    });
    var figures = Array.prototype.slice.call(document.querySelectorAll("figure, [class*='figure' i], [class*='fig-' i]")).slice(0, 80).map(function (el) {
      var img = el.querySelector("img");
      var caption = el.querySelector("figcaption, .caption, [class*='caption' i]");
      return {
        selector: h.selector(el),
        caption: caption ? (caption.innerText || "").trim().slice(0, 500) : undefined,
        image: img ? (img.currentSrc || img.src || "") : undefined,
        in_viewport: h.inViewport(el),
        rect: h.rectOf(el)
      };
    });
    var codeBlocks = Array.prototype.slice.call(document.querySelectorAll("pre, pre code, code[class*='language-'], .highlight pre")).slice(0, 80).map(function (el) {
      var text = (el.innerText || "").replace(/\s+$/g, "");
      return {
        selector: h.selector(el),
        tag: el.tagName.toLowerCase(),
        language: (el.className || "").toString().slice(0, 120) || undefined,
        text: text.slice(0, 8000),
        chars: text.length,
        in_viewport: h.inViewport(el),
        rect: h.rectOf(el)
      };
    });
    base.images = images;
    base.figures = figures;
    base.code_blocks = codeBlocks;
    return base;
  };

  return { defaultScan: defaultScan, textScan: textScan, articleScan: articleScan };
}

if (typeof self !== "undefined") {
  self.pageScanFunctions = pageScanFunctions;
}