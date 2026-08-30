import { createHash } from "node:crypto";
import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { gzipSync } from "node:zlib";
import { dirname, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";

const buildRoot = dirname(fileURLToPath(import.meta.url));
const vendorSource = resolve(buildRoot, "../vendor-src");
const staging = resolve(buildRoot, ".office-output");
const entryPoints = {
  "docx-preview": resolve(buildRoot, "src/docx-entry.mjs"),
  "pptx-preview": resolve(buildRoot, "src/pptx-entry.mjs"),
};
const licenseSources = {
  "docx-preview.LICENSE": resolve(buildRoot, "node_modules/docx-preview/LICENSE"),
  "echarts.LICENSE": resolve(buildRoot, "node_modules/echarts/LICENSE"),
  "echarts.LICENSE-d3": resolve(buildRoot, "node_modules/echarts/licenses/LICENSE-d3"),
  "echarts.NOTICE": resolve(buildRoot, "node_modules/echarts/NOTICE"),
  "jszip.LICENSE": resolve(buildRoot, "node_modules/jszip/LICENSE.markdown"),
  "pptx-renderer.LICENSE": resolve(buildRoot, "node_modules/@aiden0z/pptx-renderer/LICENSE"),
  "zrender.LICENSE": resolve(buildRoot, "node_modules/zrender/LICENSE"),
};

await rm(staging, { recursive: true, force: true });
await mkdir(staging, { recursive: true });

const result = await build({
  entryPoints,
  outdir: staging,
  entryNames: "[name]",
  chunkNames: "office-chunks/[name]-[hash]",
  outExtension: { ".js": ".mjs" },
  bundle: true,
  format: "esm",
  splitting: true,
  minify: true,
  sourcemap: false,
  legalComments: "eof",
  target: ["es2020"],
  metafile: true,
  write: true,
});

const normalized = (path) => path.split(sep).join("/");
const outputs = Object.entries(result.metafile.outputs).map(([path, metadata]) => ({
  path: resolve(buildRoot, path),
  relative: normalized(relative(staging, resolve(buildRoot, path))),
  metadata,
}));
const jszipOutputs = outputs.filter(({ metadata }) => (
  Object.keys(metadata.inputs).some((input) => normalized(input).includes("node_modules/jszip/"))
));
if (jszipOutputs.length !== 1 || !jszipOutputs[0].relative.startsWith("office-chunks/")) {
  throw new Error(`Expected one shared JSZip chunk, found: ${jszipOutputs.map((item) => item.relative).join(", ")}`);
}
const jszipChunk = jszipOutputs[0].relative;
for (const entry of ["docx-preview.mjs", "pptx-preview.mjs"]) {
  const metadata = outputs.find((output) => output.relative === entry)?.metadata;
  if (!metadata || !metadata.imports.some((item) => normalized(item.path).endsWith(jszipChunk))) {
    throw new Error(`${entry} does not import the shared JSZip chunk ${jszipChunk}`);
  }
}

const files = {};
for (const output of outputs.sort((left, right) => left.relative.localeCompare(right.relative))) {
  const bytes = await readFile(output.path);
  files[output.relative] = {
    bytes: bytes.length,
    gzipBytes: gzipSync(bytes, { level: 9 }).length,
    sha256: createHash("sha256").update(bytes).digest("hex"),
  };
}
const manifest = {
  schemaVersion: 1,
  toolchain: {
    esbuild: "0.28.1",
    docxPreview: "0.4.0",
    pptxRenderer: "1.2.4",
    jszip: "3.10.1",
    echarts: "6.1.0",
  },
  jszipChunk,
  files,
};

for (const name of ["docx-preview.mjs", "pptx-preview.mjs", "office-chunks"]) {
  await rm(resolve(vendorSource, name), { recursive: true, force: true });
}
for (const output of outputs) {
  const destination = resolve(vendorSource, output.relative);
  await mkdir(dirname(destination), { recursive: true });
  await writeFile(destination, await readFile(output.path));
}
await writeFile(
  resolve(vendorSource, "office-build.json"),
  `${JSON.stringify(manifest, null, 2)}\n`,
);
for (const [name, path] of Object.entries(licenseSources)) {
  await writeFile(resolve(vendorSource, name), await readFile(path));
}
await rm(staging, { recursive: true, force: true });

const gzipBytes = Object.values(files).reduce((total, file) => total + file.gzipBytes, 0);
console.log(`office bundles generated (${jszipChunk}, ${gzipBytes} gzip bytes)`);
