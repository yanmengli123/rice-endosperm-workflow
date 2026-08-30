# Office vendor maintenance build

This directory is used only when updating the committed DOCX/PPTX browser
bundles. It is not part of the application runtime.

```bash
npm ci
npm run build
```

The two ESM entries are bundled together with esbuild splitting enabled. The
build fails unless DOCX and PPTX both import exactly one shared JSZip chunk. It
writes the minified entry modules, shared chunk, and their hashes to
`../vendor-src/office-build.json`. Normal desktop and UI builds only copy those
committed outputs; they do not install these Node dependencies.
