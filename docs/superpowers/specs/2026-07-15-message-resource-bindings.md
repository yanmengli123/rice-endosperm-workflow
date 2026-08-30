# Message Resource Bindings

## Problem

Assistant and ACP output can contain relative paths, Windows drive paths,
`file://` URIs, percent-encoded paths, and Codex `<image path="...">` blocks.
Passing those strings through Markdown, a WebView URL parser, and then back to
the filesystem makes preview behavior dependent on quoting, operating system,
and the UI surface that opened the file.

## Design

For every newly persisted assistant message, the backend parses local Markdown
links and images exactly once. It resolves each reference against the owning
project root, applies the normal path-containment checks, and snapshots supported
files into project-local content-addressed storage. A `message_resource_links`
row binds the original reference to an immutable `ArtifactVersion`.

The original message text is not rewritten. The UI matches rendered Markdown
tags to the structured bindings and opens the bound artifact version. Inline
images are also loaded from that version. Markdown and text resources use the
center file preview, which owns its scrolling; image, PDF, and table resources
use the artifact preview policy.

Unresolvable references are persisted as structured failures. Their raw paths
are not sent back through the WebView navigation path.

## Scope boundary

This design applies only when a new assistant message is persisted after the
feature is installed. Existing messages are not scanned, migrated, backfilled,
or reinterpreted. There is no compatibility parser that manufactures bindings
for historical transcript rows.

Project export/import carries bindings that already exist, together with their
artifacts and immutable versions. That preserves new-format project data without
upgrading old messages.

During export, artifact and artifact-version storage paths are normalized to
project-relative forward-slash paths. Content-addressed snapshots therefore
travel as `.wisp/artifacts/sha256/...` workspace files. Import restores those
paths under the newly selected project directory, including across Windows and
macOS. The Markdown text and `original_reference` are deliberately not rewritten:
they identify the original tag, while preview always reads the imported immutable
version. References that never produced a snapshot remain unresolved after a
move rather than being guessed against the destination machine.

## Supported previews

The resource binder snapshots PNG, JPEG, GIF, WebP, SVG, PDF, DOCX, XLSX, PPTX,
Markdown, BibTeX, CSV, TSV, JSON, HTML, text, and log files up to 32 MiB. Unsupported,
missing, oversized, or out-of-project resources remain explicit unresolved
bindings.
