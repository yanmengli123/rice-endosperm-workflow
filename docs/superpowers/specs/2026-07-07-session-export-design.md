# Session Export Design

**Date:** 2026-07-07
**Status:** Approved
**Scope:** Add a right-click export action for the active chat session. The export is a `.zip` containing the readable transcript, raw messages with tool calls, extracted tool-call records, artifact files including images, and provenance JSON when available.

## Goal

Users can right-click the current chat page and choose export. The app opens a native save dialog and writes a zip for the active session.

## Architecture

The frontend only owns the menu entry and the current artifact path list, because produced files are detected in the UI transcript. The backend owns zip generation, because it can read persisted messages, stored artifacts, provenance, and workspace files safely.

Transcript artifact detection normalizes common assistant shorthand such as `figure.png/.pdf` to the previewable image path (`figure.png`) before display or export path collection.

## Zip Contents

- `manifest.json`: export metadata, included files, missing artifacts.
- `transcript.md`: readable user/assistant/reasoning/tool transcript.
- `messages.json`: raw persisted messages, preserving `tool_calls`.
- `tool-calls.json`: normalized tool calls with matched tool results.
- `terminal-events.json`: persisted turn completion/failure boundaries, including
  provider errors such as gateway timeouts. Older sessions can export an empty
  list because earlier app versions did not retain these events.
- `artifacts/`: copied artifact files, including images.
- `provenance/`: provenance JSON for artifact paths with recorded lineage.

## Error Handling

If no active session exists, the frontend does not call export. If the user cancels the save dialog, the command returns `None`. Missing artifact files do not fail the export; they are listed in `manifest.json`.

## Tests

Add a Playwright test that enters the chat, opens the custom context menu, clicks the export action, and verifies that `export_session` is invoked with the active session id.

## Import (added 2026-08-07)

The Edit menu's "Import session archive" action re-imports an export zip into the current project via the `import_session_archive` command.

- The command opens a native zip picker; cancel returns `None`.
- `manifest.json` and `messages.json` are required; anything else marks the file as not a wisp session export.
- Messages are appended to a new frame created in an `imported` sidebar folder, with frame timestamps taken from the message timeline so the session keeps its original chronology.
- A `session_imports` table (migration `0036_session_imports`) maps the exporting side's `session_id` to the local frame. Re-importing the same source session fast-forwards it (`replace_messages`) when the archive holds more messages, and reports `skipped` otherwise — importing never duplicates a session or merges diverged histories.
- Artifacts are extracted to their recorded `workspace_path` when it is relative, traversal-free, and unoccupied; otherwise they land under `imports/<session_id>/`. Extracted files are registered as artifacts of the new frame. Artifact extraction runs only on first import, not on fast-forward updates.
- Not restored: `provenance/*.json` (execution_log cell indexes are meaningless across databases) and the derived `transcript.md` / `tool-calls.json`.
