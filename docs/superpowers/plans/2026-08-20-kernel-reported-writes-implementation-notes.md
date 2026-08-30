# Kernel-reported writes — implementation notes

Tracking notes while implementing `docs/superpowers/plans/2026-08-20-kernel-reported-writes.md` (#937).

## What landed

- `python/kernel_worker.py` registers one `sys.addaudithook` at startup. Per-cell `begin()`/`finish()` collects write-intent `open` paths and `os.rename`/`os.replace` destinations, reports only paths that exist as files, and omits `files_written` entirely once 512 distinct paths are exceeded.
- `python/test_kernel_worker.py` drives a real spawned worker for computed names, C-level saves (numpy/matplotlib present here), append, read-only, failed open, write-then-raise, sqlite3 blind spot, and the cap.
- `KernelResp`/`RawResp`/`read_response` round-trip `files_written` as `Option`. Host `project_relative_writes` canonicalizes, drops outside-root and the root itself, normalizes `\`, sorts, and dedups. Only `LOCAL_CONTEXT_ID` kernels call `ToolEnv::report_written_paths`.
- `ToolEnv::report_written_paths` default no-op. `ToolEnvAdapter` accumulates into an interior mutex and `take_reported_writes` drains it. `agent_loop_inner` drains after every `tools.run` and unions reported paths after unmodified `retain_unambiguous_writes`.
- `union_paths_by_identity` keeps the first spelling for one identity. Engine tests cover the #937 repro, the unreported-path control, spelling, and empty-report no-op.
- CI: Python worker tests on the 3-OS `headless-agent-eval` job after MCP regression. `scripts/probe_write_audit_hook.py` is a standalone probe.

## Post-review fixes (2026-08-20)

- **Bytes path killed the worker:** `open(b'x','wb')` left `bytes` in the
  report and `json.dumps` raised, crashing the process and losing the
  session's kernel. `_note` now `os.fsdecode`s bytes and requires strict
  UTF-8 round-trip; unrepresentable paths are skipped (the host's snapshot
  inference still covers them — reports only ever add).
- **Non-UTF-8 filename broke the protocol:** a surrogate-escaped name
  serialized as a lone `\udcff` escape, which serde_json rejects
  ("lone leading surrogate in hex escape"), losing the whole result frame.
  Covered by the same strict-UTF-8 gate. Both cases have worker tests.
- **Traversal guard:** `project_relative_writes` drops any remainder with a
  non-`Normal` component — when canonicalize fails (path deleted between
  report and host processing), a raw `/root/../etc/x` could otherwise strip
  to `../etc/x` and reach undo.
- **Unix backslash filenames:** separator normalization is now
  Windows-only. On Unix `\` is a legal filename character; replacing it
  could canonicalize to a *different* existing file (`we\ird.txt` vs
  `we/ird.txt`) and credit a file the cell never wrote.
- `union_paths_by_identity` only re-sorts when it inserted something, so an
  all-duplicate report leaves the record byte-identical.
- CI worker-test steps renamed `(Unix)` / `(Windows)`.

**Documented limitation for the PR body** (deliberately not "fixed"): a
reported path that the diff never saw (e.g. `open(p, 'r+')` with nothing
written) is still credited. Same-conversation only, and undo marks it
non-reversible. Intersecting reports with the diff would forfeit real
credit for rewrites the mtime-granularity snapshot cannot see.

## Post-merge fixes (2026-08-21)

- **Reports bypassed the snapshot's skipped directories.** A cell that only
  imported a project-local module reported the `__pycache__` bytecode the
  import wrote, and `project_relative_writes` confines to the project root
  but knows nothing about `SKIP_DIRS`. The path reached the Generated card,
  `execution_log`, session exports, and publication capsules — places the
  snapshot diff could never put it. `provenance::union_reported_writes` now
  applies the snapshot's own rule (skip by directory name at any depth, only
  parent components gate inclusion) before the identity union, and
  `agent_loop_inner` calls it instead of `union_paths_by_identity`.
- **Neither wiring point was covered.** Deleting the `agent.rs` union left all
  160 `wisp-core` tests green, and deleting `env.report_written_paths(&paths)`
  left all 40 `wisp-runtime` tests green: the whole feature could stop working
  silently. The `#937` engine test called `retain_unambiguous_writes` and the
  union directly rather than driving the loop. Added two `agent_loop` tests
  (report folded into the record, snapshot-skipped paths excluded, no leak into
  the next tool call) and lifted the runtime's forwarding branch into
  `report_local_writes` so the local-only gate and the absent-vs-empty
  distinction are testable against a recording `ToolEnv`. All five mutants —
  removed union, removed skip filter, removed report call, ungated context,
  non-draining buffer — now fail a test.

## Report precision (2026-08-21, #947 gaps 1 and 2)

- **Bytecode no longer evicts real outputs.** `MAX_REPORTED_WRITES` is counted
  in the worker, before the host filters, so `__pycache__` paths the host was
  always going to discard consumed the cap. Measured: one cell importing 300
  project-local modules and writing 250 CSVs reported *nothing* — 550 > 512, so
  the field was omitted and the cell fell back to snapshot inference, i.e. #911
  case 1 reproduced again. `_is_bytecode_cache` now drops those paths before the
  cap is consulted. The other `SKIP_DIRS` entries stay a host concern: the
  worker has first-hand knowledge of its own bytecode, not of project layout,
  and duplicating that list would give it two sources of truth.
- **`files_written` means "changed", not "opened with write intent".** The
  `open` audit event carries intent, and it fires *before* the OS call
  completes — so `_note` samples `(size, st_mtime_ns)` at that moment and
  `finish` compares it again. A cell that opens a file `'r+'` or `'a'` and
  writes nothing is no longer credited with it; `h5py.File(p, "a")` and
  `zarr.open(p, mode="a")` are the idiomatic way to open a store you then only
  read. This is not the intersect-with-the-diff approach #942 rejected: the
  comparison happens inside the worker across the open itself, so a
  same-length in-place rewrite — invisible to size, and to the host's
  coarser-grained snapshot — keeps its credit through `st_mtime_ns`.
- Overhead is two extra `os.stat` calls per distinct candidate path. Measured
  on 500 writes with every path under the cap: 0.024 s median baseline vs
  0.028 s with the hook, about 8 µs per file.

## WSL project-root runtimes (2026-08-27, #947 gap 3)

- WSL Python and R REPLs now translate the registered Windows project root with
  `wslpath -a -u` and enter it before starting the worker. Translation or `cd`
  failure aborts startup with an actionable error instead of silently running
  in the context home. This aligns WSL REPLs with local REPLs, WSL terminals,
  and WSL Runs. SSH retains its configured/probed remote workdir.
- WSL Python workers receive the same host-owned write scope as local workers,
  using `.` after the launcher has entered the translated project. Their
  project-relative reports are joined and canonicalized against the Windows
  project root before reaching provenance. Legacy absolute WSL reports and all
  SSH reports remain rejected.
- Pure tests cover launch command quoting, WSL-vs-SSH working-directory
  behavior, write-scope selection, WSL report forwarding, and the SSH gate; CI
  does not require a real WSL distro.

## Host-configured report scope (2026-08-27, #947 gap 1)

- The host now sends a write-scope configuration frame to each local Python
  worker before its first cell. The scope contains the project root and the
  exact `SNAPSHOT_SKIP_DIRS` policy owned by `wisp-core`; the Python worker no
  longer maintains a second project-layout list.
- Configured workers resolve symlinks, discard paths outside the project and
  under every snapshot-skipped parent directory before storing a candidate,
  and return normalized project-relative paths marked with
  `files_written_base: "project"`. The Rust host joins and canonicalizes those
  paths again before they reach `ToolEnv`.
- Candidate memory safety and report completeness are separate limits. A
  larger count/byte guard bounds raw write-intent candidates, while the 512
  semantic cap is applied only after candidates are proven changed. Exceeding
  either guard still omits the field so snapshot inference remains the safe
  fallback.
- Real-worker regressions cover all snapshot-skipped directories, outside-root
  writes, unchanged intent-only opens, the true 513-output cap, and a leaf file
  whose own name matches a skipped directory.

## Deviations

- Windows CI invokes `python` instead of `python3` (sibling step on the same matrix job). `python3` is not on PATH on `windows-latest`; Unix steps keep the planned `python3` command so the worker tests still run on all three OSes.
- Did not `cargo fmt --all` the workspace: check fails on pre-existing drift in `src-tauri/src/lib.rs` and `crates/wisp-mcp/src/client.rs`. Formatting those would violate the empty `src-tauri` diff. Touched crates were formatted with `cargo fmt -p wisp-core -p wisp-runtime -p wisp-tools`.
- `cargo test -p wisp-tauri` failed here: `libsoup-3.0` / WebKit pkg-config missing. Crate-level tests in steps 2–5 are the accepted bar.
