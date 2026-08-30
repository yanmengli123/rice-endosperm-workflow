import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const dir = path.dirname(fileURLToPath(import.meta.url));
const source = fs.readFileSync(path.join(dir, "chat_adapter.js"), "utf8");

function loadAdapter(href) {
  const nodes = [];
  function node(spec) {
    const el = {
      tagName: (spec.tag || "DIV").toUpperCase(),
      id: spec.id || "",
      value: spec.value || "",
      innerText: spec.text || "",
      href: spec.href || "",
      disabled: !!spec.disabled,
      clicked: false,
      focused: false,
      events: [],
      children: spec.children || [],
      selectors: spec.selectors || [],
      focus() {
        this.focused = true;
      },
      click() {
        this.clicked = true;
      },
      matches(sel) {
        return this.selectors.includes(sel);
      },
      querySelector(sel) {
        return this.querySelectorAll(sel)[0] || null;
      },
      querySelectorAll(sel) {
        const out = [];
        for (const child of this.children) {
          if (child.matches(sel)) out.push(child);
          out.push(...child.querySelectorAll(sel));
        }
        return out;
      },
      dispatchEvent(ev) {
        this.events.push(ev && ev.type ? ev.type : "event");
      },
    };
    nodes.push(el);
    return el;
  }
  const body = node({ tag: "BODY", text: specBodyText() });
  function specBodyText() {
    return "";
  }
  const document = {
    body,
    title: "chat",
    execCommand() {},
    querySelector(sel) {
      return document.querySelectorAll(sel)[0] || null;
    },
    querySelectorAll(sel) {
      const out = [];
      if (body.matches(sel)) out.push(body);
      out.push(...body.querySelectorAll(sel));
      return out;
    },
  };
  const location = { href, hostname: new URL(href).hostname };
  const root = {};
  const context = {
    self: root,
    globalThis: root,
    document,
    location,
    window: {
      HTMLTextAreaElement: { prototype: {} },
      HTMLInputElement: { prototype: {} },
    },
    Event: function Event(type) {
      this.type = type;
    },
    KeyboardEvent: function KeyboardEvent(type, init) {
      this.type = type;
      Object.assign(this, init || {});
    },
    Object,
    String,
    Array,
    URL,
    Error,
  };
  vm.runInNewContext(source, context);
  return { api: root, document, body, node };
}

test("chatSiteKind accepts official chat hosts only", () => {
  const { api } = loadAdapter("https://chatgpt.com/");
  assert.equal(api.chatSiteKind("https://chatgpt.com/c/1"), "chatgpt");
  assert.equal(api.chatSiteKind("https://www.chat.openai.com/"), "chatgpt");
  assert.equal(api.chatSiteKind("https://gemini.google.com/app"), "gemini");
  assert.equal(api.chatSiteKind("https://www.google.com/search?udm=50"), "google_ai");
  assert.equal(api.chatSiteKind("https://google.com/search?q=x&udm=50"), "google_ai");
  assert.equal(api.chatSiteKind("https://www.google.com/search?q=papers"), null);
  assert.equal(api.chatSiteKind("https://chatgpt.com.evil.com/"), null);
  assert.equal(api.chatSiteKind("http://gemini.google.com/"), null);
});

test("ChatGPT adapter fills the composer and reads the last assistant turn", () => {
  const href = "https://chatgpt.com/";
  const { api, body, node } = loadAdapter(href);
  const composer = node({
    tag: "TEXTAREA",
    id: "prompt-textarea",
    selectors: ["#prompt-textarea"],
  });
  const send = node({
    tag: "BUTTON",
    selectors: ['[data-testid="send-button"]'],
  });
  const cite = node({
    tag: "A",
    href: "https://pubmed.example/1",
    text: "paper",
    selectors: ["a[href]"],
  });
  const assistant = node({
    tag: "DIV",
    text: "hello from gpt",
    selectors: ['[data-message-author-role="assistant"]'],
    children: [cite],
  });
  body.children.push(composer, send, assistant);
  const chat = api.chatgptDomFns();
  assert.equal(chat.ready().has_composer, true);
  assert.equal(chat.fill("find ESR1 papers").chars, 16);
  assert.equal(composer.value, "find ESR1 papers");
  const sent = chat.send();
  assert.equal(sent.ok, true);
  assert.equal(sent.method, "button");
  assert.equal(send.clicked, true);
  const read = chat.read();
  assert.equal(read.answer_text, "hello from gpt");
  assert.equal(read.citations[0].href, "https://pubmed.example/1");
  assert.equal(read.site, "chatgpt");
});

test("Gemini adapter uses prompt box and model-response", () => {
  const { api, body, node } = loadAdapter("https://gemini.google.com/app");
  const composer = node({
    tag: "DIV",
    selectors: ['[aria-label*="Enter a prompt" i][contenteditable="true"]'],
  });
  const send = node({
    tag: "BUTTON",
    selectors: ['button[aria-label="Send message"]'],
  });
  const assistant = node({
    tag: "DIV",
    text: "gemini answer",
    selectors: ['[data-test-id="model-response"]'],
  });
  body.children.push(composer, send, assistant);
  const chat = api.geminiDomFns();
  assert.equal(chat.site, "gemini");
  chat.fill("summarize this");
  assert.equal(composer.focused, true);
  chat.send();
  assert.equal(send.clicked, true);
  assert.equal(chat.read().answer_text, "gemini answer");
});

test("Google AI Mode adapter reads article citations", () => {
  const { api, body, node } = loadAdapter("https://www.google.com/search?udm=50");
  const composer = node({
    tag: "TEXTAREA",
    selectors: ['textarea[aria-label*="Ask a follow up" i]'],
  });
  const send = node({
    tag: "BUTTON",
    selectors: ['button[aria-label*="Send" i]'],
  });
  const cite = node({
    tag: "A",
    href: "https://nature.example/ai",
    text: "Nature",
    selectors: ["a[href]"],
  });
  const article = node({
    tag: "DIV",
    text: "AI mode answer",
    selectors: ['[role="article"]'],
    children: [cite],
  });
  body.children.push(composer, send, article);
  const chat = api.googleAiModeDomFns();
  assert.equal(chat.site, "google_ai");
  chat.fill("what is RNA-seq");
  assert.equal(composer.value, "what is RNA-seq");
  const read = chat.read();
  assert.equal(read.answer_text, "AI mode answer");
  assert.equal(read.citations[0].href, "https://nature.example/ai");
});

test("dispatcher rejects unsupported hosts", () => {
  const { api } = loadAdapter("https://example.com/");
  assert.throws(() => api.chatSiteDomFns(), /web_agent_\*/);
});
