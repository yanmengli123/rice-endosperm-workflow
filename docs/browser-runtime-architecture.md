# Browser Runtime architecture

Wisp 0.3.0 treats the Chrome extension as a **controlled access adapter**.
The desktop process owns sessions, waits, staging, and approval.

```
Agent tools
    -> Browser Runtime (src-tauri/src/browser_bridge)
        -> ws://127.0.0.1:18765  shared / daily Chrome
        -> ws://127.0.0.1:18766  workspace Chrome (dedicated profile)
            -> Manifest V3 extension (browser-extension/)
```

## Sessions

| Session | Browser | Login state | Port |
|---|---|---|---|
| `shared` | User's existing Chrome/Edge profile | Daily cookies and extensions | 18765 |
| `workspace` | Chrome-family build launched with `%APPDATA%/science.wisp-science/browser-workspace` | Clean until the user signs in there | 18766 |

Both can be connected at once. If they are, tools must pass `session`.

### Workspace mode needs a build that still loads unpacked extensions

The workspace window is launched with `--load-extension` pointed at a copy
of `browser-extension/` whose `session_config.js` targets port 18766.
**Official Google Chrome removed that flag in version 137** and now only
logs `--load-extension is not allowed in Google Chrome, ignoring`, so the
window opens with no Wisp extension. Chromium and Chrome for Testing keep
the flag.

So `resolve_browser()` prefers a build that honors the flag (Chromium,
Chrome for Testing, or whatever `WISP_WORKSPACE_BROWSER` points at) and
falls back to branded Chrome only because releases older than 137 still
work. `start_workspace` then **waits up to 20 s for the workspace
extension to connect**; if it never does it closes the window and fails
with `WORKSPACE_EXTENSION_BLOCKED` explaining the flag removal and the
shared-session alternative. It never reports a spawned process as ready,
which used to leave the agent driving a blank `about:blank` window (#952).

Shared mode is unaffected: the user loads the extension once from
`chrome://extensions` in the browser they already use. The chat banner's
**Set up browser** button opens that page and copies the bundled extension
path.

### When a connected extension is not a usable session

The extension popup only reports whether its own socket is open, so it can
read *Connected to Wisp* while Wisp reports `connected=false`.
`browser_setup` therefore names the cause instead of only the symptom:

| Field | Meaning |
|---|---|
| `refused_connection` | A client reached a bridge port and was refused — a foreign extension id, or another loopback bridge on the port. Carries the observed `origin`, the `expected_origin`, and the handshake error. |
| `reload_required` | A connected extension is below `protocol_version` 2 or reported no capabilities. Chrome never auto-updates an unpacked extension, so the user must Reload it from `extension_path`. |

## What the extension does

- Handshake: `extension_version`, `protocol_version=2`, `capabilities[]`
- Conditional wait (URL / selector / text / settle)
- Article scan (`images[]`, `figures[]`, `code_blocks[]`)
- Host-permission asset download into `Downloads/WispBrowserStaging`
- Viewport / full-page / selector capture
- Pause control from the popup (`USER_CONTROLLING`)

The extension never writes project directories and never returns large base64 files as the archive path.

## What the Runtime does

- Multiplexes two WebSocket listeners
- Browser Task Lease (`last_session` + explicit `session`)
- Copies staged files into the project and hashes SHA-256
- Starts/stops the workspace browser window and verifies it connected
- Per-turn ledger of tabs Wisp created (`web_open_tab` / tab-create), closed at turn end or confirmed in the UI
- Records the last refused connection so an unclaimed extension has a reason
- In-browser chat one-shot (`web_agent_send` / `wait` / `read`) on an
  already-logged-in ChatGPT, Gemini, or Google AI Mode tab

Playwright is not used. The user's daily Chrome User Data directory is never passed as `--user-data-dir`.

## Safety checks

- `web_agent_*` accepts only already-open HTTPS tabs at `chatgpt.com` /
  `chat.openai.com`, `gemini.google.com`, or `google.com` with `udm=50`
  (Google AI Mode). Lookalike hosts such as `chatgpt.com.evil.com` or
  `gemini.google.com.evil.com` are rejected before any prompt is filled.
  Ordinary `google.com/search` tabs without `udm=50` are not treated as chat.
- `web_save_assets` `dest_dir` and `web_screenshot` `save_path` must be
  project-relative: absolute paths and `..` segments are rejected, so tool
  arguments cannot write outside the project root.
- The extension's pause gate parses each incoming command and lets only
  `cmd:"control"` through while paused; it never sniffs the raw request
  string.