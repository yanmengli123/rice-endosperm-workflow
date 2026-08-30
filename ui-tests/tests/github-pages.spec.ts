import { expect, test } from "@playwright/test";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { resolve } from "node:path";

const repositoryRoot = resolve(__dirname, "../..");
const readRepositoryFile = (path: string) =>
  readFileSync(resolve(repositoryRoot, path), "utf8");

const skillCount = readdirSync(resolve(repositoryRoot, "skills")).filter((name) =>
  existsSync(resolve(repositoryRoot, "skills", name, "SKILL.md")),
).length;

const loadI18n = () => {
  const source = readRepositoryFile("docs/assets/i18n.js");
  const sandbox: Record<string, unknown> = {
    document: {
      addEventListener() {},
      documentElement: {
        lang: "",
        dataset: {},
        classList: { add() {} },
      },
      querySelector() {
        return null;
      },
      querySelectorAll() {
        return [];
      },
    },
    location: { search: "", href: "https://example.test/" },
    history: { replaceState() {} },
    URL,
    URLSearchParams,
    localStorage: { getItem() { return null; }, setItem() {} },
  };
  sandbox.globalThis = sandbox;
  runInNewContext(source, sandbox);
  return sandbox.WISP_PAGES_I18N as { zh: Record<string, string>; en: Record<string, string> };
};

test("GitHub Pages homepage aligns with v1.5.0 and ships a language switch", () => {
  const index = readRepositoryFile("docs/index.html");
  const i18nJs = readRepositoryFile("docs/assets/i18n.js");

  expect(index).toContain('class="lang-switch"');
  expect(index).toContain("assets/i18n.js");
  expect(index).toContain("34 个内置 SKILL");
  expect(index).toContain("v1.5.0");
  expect(index).toContain("Linux");
  expect(index).toContain("Python / R");
  expect(index).not.toContain("30 个内置");
  expect(index).not.toContain("29 bundled");
  expect(index).not.toContain("暂未签名");
  expect(index).not.toContain("仅支持从源码构建");
  expect(index).not.toContain("v0.2 仍为 beta");
  expect(index).toContain("trusted-logos/pku.svg");
  expect(index).toContain("trusted-logos/cas.svg");
  expect(index).toContain("trusted-logos/zhejiang.svg");
  expect(index).toContain("trusted-logos/washu.png");
  expect(index).toContain("trusted-logos/slu.png");
  expect(index).toContain("trusted-logos/sjtu.svg");
  expect(index).toContain("trusted-logos/meduniwien.svg");
  expect(i18nJs).toContain(`${skillCount} bundled`);
  expect(i18nJs).toContain(`${skillCount} 个内置`);
  expect(i18nJs).toContain(`${skillCount} bundled SKILL`);
});

test("Pages i18n dictionaries cover every data-i18n key and stay in sync", () => {
  const i18n = loadI18n();
  const zhKeys = Object.keys(i18n.zh).sort();
  const enKeys = Object.keys(i18n.en).sort();
  expect(zhKeys).toEqual(enKeys);

  for (const page of ["index.html", "model-configuration.html", "acp-agents.html"]) {
    const html = readRepositoryFile(`docs/${page}`);
    expect(html).toContain('class="lang-switch"');
    expect(html).toContain("assets/i18n.js");
    const used = new Set(
      [...html.matchAll(/data-i18n(?:-html|-aria)?="([^"]+)"/g)].map((match) => match[1]),
    );
    const missing = [...used].filter((key) => !i18n.zh[key]).sort();
    expect(missing, page).toEqual([]);
  }
});
