// Heuristic composers / replies for in-browser chat sites.
// Avoid hashed class names; match role, test-id, aria-label, and stable tags.

function httpsHost(href) {
  try {
    var url = new URL(href || "");
    if (url.protocol !== "https:") return "";
    return (url.hostname || "").replace(/^www\./i, "").toLowerCase();
  } catch (_) {
    return "";
  }
}

function queryParam(href, name) {
  try {
    return new URL(href || "").searchParams.get(name);
  } catch (_) {
    return null;
  }
}

function chatSiteKind(href) {
  var host = httpsHost(href);
  if (host === "chatgpt.com" || host === "chat.openai.com") return "chatgpt";
  if (host === "gemini.google.com") return "gemini";
  if (host === "google.com" && queryParam(href, "udm") === "50") return "google_ai";
  return null;
}

function firstMatch(root, selectors) {
  for (var i = 0; i < selectors.length; i++) {
    var el = root.querySelector(selectors[i]);
    if (el) return el;
  }
  return null;
}

function allMatches(root, selectors) {
  var seen = [];
  var out = [];
  for (var i = 0; i < selectors.length; i++) {
    var nodes = root.querySelectorAll(selectors[i]);
    for (var j = 0; j < nodes.length; j++) {
      if (seen.indexOf(nodes[j]) >= 0) continue;
      seen.push(nodes[j]);
      out.push(nodes[j]);
    }
  }
  return out;
}

function citationLinks(root) {
  return Array.prototype.slice.call(root.querySelectorAll("a[href]")).map(function (a) {
    return { text: (a.innerText || "").trim().slice(0, 200), href: a.href };
  }).filter(function (x) { return x.href && !/^javascript:/i.test(x.href); }).slice(0, 30);
}

function fillComposer(el, prompt) {
  el.focus();
  if ("value" in el) {
    var proto = el.tagName === "TEXTAREA" ? window.HTMLTextAreaElement.prototype : window.HTMLInputElement.prototype;
    var setter = Object.getOwnPropertyDescriptor(proto, "value");
    if (setter && setter.set) setter.set.call(el, prompt);
    else el.value = prompt;
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
  } else {
    document.execCommand("selectAll", false, null);
    document.execCommand("insertText", false, prompt);
  }
  return { ok: true, chars: String(prompt).length };
}

function clickSend(sendBtn, composer) {
  if (sendBtn && !sendBtn.disabled) {
    sendBtn.click();
    return { ok: true, method: "button" };
  }
  if (!composer) throw new Error("chat send control not found");
  composer.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", code: "Enter", bubbles: true }));
  return { ok: true, method: "enter" };
}

function pageBlocked(extraNeedles) {
  var text = ((document.body && document.body.innerText) || "").toLowerCase();
  if (text.indexOf("are you a robot") >= 0 && (text.indexOf("confirm you are a human") >= 0 || text.indexOf("captcha") >= 0)) {
    return "captcha";
  }
  if (text.indexOf("unusual traffic") >= 0 || text.indexOf("sorry, we have detected unusual") >= 0) {
    return "captcha";
  }
  var needles = extraNeedles || [];
  for (var i = 0; i < needles.length; i++) {
    if (text.indexOf(needles[i]) >= 0) return needles[i];
  }
  return null;
}

function chatgptDomFns() {
  var composerSels = [
    "#prompt-textarea",
    '[data-testid="prompt-textarea"]',
    "div[contenteditable=\"true\"]#prompt-textarea",
    "form textarea",
    '[contenteditable="true"][role="textbox"]',
    "div[contenteditable=\"true\"]"
  ];
  var sendSels = [
    '[data-testid="send-button"]',
    'button[aria-label*="Send" i]',
    'button[data-testid="composer-send-button"]'
  ];
  var stopSels = ['button[aria-label*="Stop" i]', '[data-testid="stop-button"]'];
  var turnSels = ['[data-message-author-role="assistant"]'];
  var composer = function () { return firstMatch(document, composerSels); };
  var lastAssistant = function () {
    var turns = allMatches(document, turnSels);
    var last = turns[turns.length - 1];
    if (!last) return { text: "", citations: [] };
    return { text: (last.innerText || "").trim(), citations: citationLinks(last) };
  };
  var blocked = function () {
    var reason = pageBlocked();
    if (reason) return reason;
    if (((document.body && document.body.innerText) || "").toLowerCase().indexOf("log in") >= 0 && !composer()) {
      return "login";
    }
    return null;
  };
  return {
    site: "chatgpt",
    ready: function () {
      return {
        url: location.href,
        site: "chatgpt",
        has_composer: !!composer(),
        sending: !!firstMatch(document, stopSels),
        blocked: blocked(),
        last: lastAssistant()
      };
    },
    fill: function (prompt) {
      var el = composer();
      if (!el) throw new Error("ChatGPT composer not found");
      return fillComposer(el, prompt);
    },
    send: function () {
      return clickSend(firstMatch(document, sendSels), composer());
    },
    read: function () {
      var last = lastAssistant();
      return {
        url: location.href,
        title: document.title,
        site: "chatgpt",
        blocked: blocked(),
        sending: !!firstMatch(document, stopSels),
        answer_text: last.text,
        citations: last.citations
      };
    }
  };
}

function geminiDomFns() {
  var composerSels = [
    "div.ql-editor[contenteditable=\"true\"]",
    "rich-textarea .ql-editor",
    '[aria-label*="Enter a prompt" i][contenteditable="true"]',
    '[aria-label*="prompt" i][contenteditable="true"]',
    'div[contenteditable="true"][role="textbox"]'
  ];
  var sendSels = [
    'button[aria-label="Send message"]',
    'button[aria-label*="Send" i]',
    'button[aria-label*="Submit" i]'
  ];
  var stopSels = ['button[aria-label*="Stop" i]'];
  var turnSels = [
    "message-content",
    '[data-test-id="model-response"]',
    "model-response",
    ".response-content"
  ];
  var composer = function () { return firstMatch(document, composerSels); };
  var lastAssistant = function () {
    var turns = allMatches(document, turnSels);
    var last = turns[turns.length - 1];
    if (!last) return { text: "", citations: [] };
    return { text: (last.innerText || "").trim(), citations: citationLinks(last) };
  };
  var blocked = function () {
    var reason = pageBlocked(["couldn't sign you in"]);
    if (reason) return reason;
    var text = ((document.body && document.body.innerText) || "").toLowerCase();
    if ((text.indexOf("sign in") >= 0 || text.indexOf("use your google account") >= 0) && !composer()) {
      return "login";
    }
    return null;
  };
  return {
    site: "gemini",
    ready: function () {
      return {
        url: location.href,
        site: "gemini",
        has_composer: !!composer(),
        sending: !!firstMatch(document, stopSels),
        blocked: blocked(),
        last: lastAssistant()
      };
    },
    fill: function (prompt) {
      var el = composer();
      if (!el) throw new Error("Gemini composer not found");
      return fillComposer(el, prompt);
    },
    send: function () {
      return clickSend(firstMatch(document, sendSels), composer());
    },
    read: function () {
      var last = lastAssistant();
      return {
        url: location.href,
        title: document.title,
        site: "gemini",
        blocked: blocked(),
        sending: !!firstMatch(document, stopSels),
        answer_text: last.text,
        citations: last.citations
      };
    }
  };
}

function googleAiModeDomFns() {
  var composerSels = [
    'textarea[aria-label*="Ask a follow up" i]',
    'textarea[aria-label*="Ask" i]',
    'textarea[placeholder*="Ask" i]',
    '[contenteditable="true"][aria-label*="Ask" i]',
    'textarea[aria-label*="follow" i]'
  ];
  var sendSels = [
    'button[aria-label*="Send" i]',
    'button[type="submit"]'
  ];
  var stopSels = ['button[aria-label*="Stop" i]'];
  var turnSels = [
    '[role="article"]',
    "[data-conversation-id]"
  ];
  var composer = function () { return firstMatch(document, composerSels); };
  var lastAssistant = function () {
    var turns = allMatches(document, turnSels);
    var last = turns[turns.length - 1];
    if (!last) return { text: "", citations: [] };
    return { text: (last.innerText || "").trim(), citations: citationLinks(last) };
  };
  var blocked = function () {
    var reason = pageBlocked();
    if (reason) return reason;
    var text = ((document.body && document.body.innerText) || "").toLowerCase();
    if ((text.indexOf("sign in") >= 0 || text.indexOf("before you continue") >= 0) && !composer()) {
      return "login";
    }
    return null;
  };
  return {
    site: "google_ai",
    ready: function () {
      return {
        url: location.href,
        site: "google_ai",
        has_composer: !!composer(),
        sending: !!firstMatch(document, stopSels),
        blocked: blocked(),
        last: lastAssistant()
      };
    },
    fill: function (prompt) {
      var el = composer();
      if (!el) throw new Error("Google AI Mode composer not found");
      return fillComposer(el, prompt);
    },
    send: function () {
      return clickSend(firstMatch(document, sendSels), composer());
    },
    read: function () {
      var last = lastAssistant();
      return {
        url: location.href,
        title: document.title,
        site: "google_ai",
        blocked: blocked(),
        sending: !!firstMatch(document, stopSels),
        answer_text: last.text,
        citations: last.citations
      };
    }
  };
}

function chatSiteDomFns() {
  var kind = chatSiteKind(location.href);
  if (kind === "gemini") return geminiDomFns();
  if (kind === "google_ai") return googleAiModeDomFns();
  if (kind === "chatgpt") return chatgptDomFns();
  throw new Error(
    "web_agent_* supports chatgpt.com, gemini.google.com, and google.com/search?udm=50 (Google AI Mode). This tab is " +
      (location.hostname || "unknown")
  );
}

if (typeof self !== "undefined") {
  self.chatSiteKind = chatSiteKind;
  self.chatSiteDomFns = chatSiteDomFns;
  self.chatgptDomFns = chatgptDomFns;
  self.geminiDomFns = geminiDomFns;
  self.googleAiModeDomFns = googleAiModeDomFns;
}
