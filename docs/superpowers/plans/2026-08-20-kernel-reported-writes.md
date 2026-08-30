# Kernel-Reported Writes — attributing runtime-computed filenames (#937)

> Governing principle, carried over from the #911 work: **wrong credit is worse
> than missing credit.** Attribution records feed exports, publication
> capsules, and "undo this turn", so a path must never be credited to a
> conversation that cannot be proven to have written it. This plan adds the one
> proof source that can end the last "missing credit" case without ever risking
> wrong credit: the interpreter reporting the files it actually wrote.

**Status:** active ·
**Upstream issue:** #937 (open follow-up of #911, case 1) ·
**Base:** `main` at `a27f3284` (contains merged #919 and #935) ·
**Branch:** `feat/937-kernel-reported-writes` ·
**Updated:** 2026-08-20

## 1. Problem and context

Two merged upstream fixes frame what remains:

- **#919** (v1.5.0) gave every conversation and every subagent its own Python
  kernel (`RuntimeKey.session_id`; subagent kernels under `agent-{request_id}`
  frames). Cross-session variable leakage — the destructive incident class of
  #911 — is gone.
- **#935** (`b2b6fe49`) made snapshot-based attribution honest under overlap.
  Every producing tool call registers a window per workspace root
  (`provenance::begin_window(root, scope)`); windows carry a conversation
  scope (`Output::provenance_scope()`, the root frame id), so a conversation
  and its subagents are never foreign to each other. On completion,
  `retain_unambiguous_writes` keeps a changed path only when the call's source
  names it (whole-token matching via `normalize_path_text` /
  `mentions_path_token`) or its mtime falls inside the call's own window and
  outside every foreign one (±2 s slack).

**What remains — #911 case 1, tracked as #937.** A file whose name is computed
at runtime (`f"fig_{i}.png"`) is never named in the source, and under a
genuine foreign overlap its mtime proves nothing. #935's rule therefore drops
it — from *everyone's* Generated list. Reproduction from the issue: session B
writes `fig_1.png` (computed name) and `fig_2.png` (literal) while session A
sleeps in an overlapping call; only `fig_2.png` appears on B's card, and
`fig_1.png` vanishes entirely.

This is not a bug in #935 — it is the designed floor of the snapshot/diff
approach. There is no safe guess left in mtimes or source text. The issue
itself names the way out: interpreter-level write tracking, so the Python tool
reports exact `files_written` instead of the host inferring them. The Python
worker (`python/kernel_worker.py`) is the right place: `sys.addaudithook`
observes file opens made from C extensions (matplotlib, numpy, pypdfium2,
python-pptx) that patching `builtins.open` would miss, and the worker already
instruments itself (it wraps `builtins.__import__` at startup), so this is an
established pattern in that file.

**Why a kernel report is safe to trust.** Since #919, every kernel belongs to
exactly one conversation. A path reported by a kernel can therefore only ever
credit the conversation that ran the cell — the report is a *stronger* proof
than a source mention, and it can never produce wrong credit across
conversations.

## 2. Goal, deliverable, and acceptance criteria

**Goal:** Python cells report the files they actually wrote; those reports
survive overlap arbitration as proven attribution; #911 case 1 stops
reproducing.

**Deliverable:** one PR, fork → upstream, closing #937. Reviewable in the
order of §4; each step compiles and passes tests on its own.

**Acceptance criteria** (mirroring #937):

1. **The repro is fixed:** session B writes `fig_1.png` (computed) and
   `fig_2.png` (literal) during a foreign overlap → both files appear on B's
   Generated card; neither appears on any other conversation's.
2. **No regressions:** every #935 behavior is unchanged — token matching,
   own-window mtime rule, same-scope non-contest — and the full existing test
   suite stays green.
3. **Executable evidence:** automated tests cover the
   computed-name-under-foreign-overlap case at the engine level, the worker's
   observation semantics, and the worker↔host protocol round-trip; CI runs the
   worker tests on all three OSes.

**Top-level verification:** the commands in §6, plus the manual smoke test:
run the #937 two-session prompt against a build of this branch and confirm
both files land on B's card.

## 3. Design overview

The pipeline gains one arrow; everything else is #935, untouched:

```
kernel worker (audit hook) ──files_written──▶ KernelResp        (§4.1, §4.2)
        ──▶ ReplTool relativizes to the project root            (§4.2)
        ──▶ ToolEnv::report_written_paths, accumulated per call (§4.3)
        ──▶ agent loop: diff → retain_unambiguous_writes (#935, unchanged)
                        → union reported paths in (proven, spelling-safe)
                        → ProvenanceRecord                      (§4.4)
```

**The fold-in lives entirely in `wisp-core`.** The agent loop
(`agent_loop_inner` in `crates/wisp-core/src/agent.rs`) already owns the
per-call producing window, the retain step, and a per-loop `ToolEnvAdapter`.
The adapter accumulates paths reported during `tools.run(...)`; the loop
drains them right after the call and unions them into the written list
**after** `retain_unambiguous_writes`. Consequences that make this the
smallest correct design:

- An ambiguity drop can never erase a reported path (the union comes after),
  and a report never widens what retain keeps for *unreported* paths.
- `run_native_agent` (delegated agents) drives this same loop, so mainline
  turns, parallel subagents, and delegated agents all inherit the behavior
  with **zero `src-tauri` changes**.
- No new host-side attribution plumbing: #935's engine windows already answer
  "who else was in flight"; this PR only adds "what my own interpreter saw".

**Global constraints, binding on every step:**

- The `Output` trait does not change. Hosts never see reported paths except
  inside the final `ProvenanceRecord`.
- `files_written` in the protocol is `Option`: **absent means "worker did not
  or could not observe — host infers as before"; it is never conflated with an
  empty list, which means "wrote nothing"**. Old workers and new hosts (and
  vice versa) must interoperate.
- A report that might be incomplete is not sent at all (see the cap in §4.1):
  a truncated list would look authoritative while being wrong.
- Only local-context kernels report (`LOCAL_CONTEXT_ID`): a remote worker's
  absolute paths describe another machine's filesystem.
- No timing-dependent tests — deterministic fakes only (Windows CI rejects
  sleep-based tests as flaky).

## 4. Implementation steps

### 4.1 `python/kernel_worker.py` — the write observer

**Goal:** each executed cell's response carries the project files the
interpreter itself opened for writing, with no false positives and no
partial-looking lists.

**Changes.** Add a module-level observer and register one audit hook at
startup (audit hooks cannot be removed, so per-cell collection is toggled on
the observer, `begin()` before `exec`, `finish()` in the `finally`):

- The hook watches two event families and nothing else:
  - `open` with write intent: a string mode containing any of `w a x +`, or
    integer flags intersecting
    `os.O_WRONLY | os.O_RDWR | os.O_APPEND | os.O_CREAT | os.O_TRUNC`.
  - `os.rename` / `os.replace` **destinations** (a rename creates its target
    without opening it).
  - Deliberately **not** mkdir/remove/`shutil.*`: probing showed matplotlib's
    first import creates `~/.cache/matplotlib` and `~/.config/matplotlib`
    (noise from outside the project), and the host already learns deletions
    and directories from its own before/after snapshot.
- Paths are resolved with `os.path.abspath(os.fspath(path))` and deduplicated
  in insertion order.
- The hook body is wrapped in a broad `try/except: return` and returns
  immediately while collection is off — **an audit hook that raises would
  propagate into arbitrary user code**, and it runs on every audited event.
- **Failed opens:** the `open` audit event fires before the OS call completes,
  so a failed `open()` still notes its path. `finish()` therefore reports only
  paths that exist as files afterwards (`os.path.isfile`, exceptions treated
  as "missing"). Omit, never guess.
- **Cap:** `MAX_REPORTED_WRITES = 512`. Once exceeded, the observer marks
  itself truncated and `finish()` returns `None` — the response then simply
  omits the field and the host falls back to inference.
- Response wiring: after the existing usage block, add
  `if files_written is not None: resp["files_written"] = files_written`.
  Absent ≠ empty, per §3.

**Success criteria / verification** — `python/test_kernel_worker.py`
(unittest, run via
`python3 -m unittest discover -s python -p "test_kernel_worker.py" -v`) must
cover, against a real spawned worker:

| Case | Expected report |
| --- | --- |
| computed name: `name = f"fig_{1}.png"; open(name,'w')` | the file |
| C-level library write (`matplotlib` `savefig` or `np.save`) | the file |
| append mode | the file |
| read-only `open` | not reported |
| failed `open()` to a non-creatable path | not reported |
| write then `raise` | the written path (error still reported) |
| `sqlite3` database write | **not** reported — asserts the documented blind spot |
| > 512 distinct paths | `files_written` **absent** from the response |

Test pitfall, learned the hard way: compare against the worker's *resolved*
paths — `tempfile.mkdtemp()` can sit behind a symlink and the worker reports
`os.path.abspath` output.

**Boundaries:** no other change to the worker; no attempt to observe
subprocess or C-`sqlite3` writes (documented blind spot, §5); the worker never
filters by project root — relativization and confinement are the host's job
(§4.2), because only the host knows the root.

### 4.2 `crates/wisp-runtime` — protocol and relativization

**Goal:** the report crosses the worker↔host protocol tolerantly in both
directions, and only project-internal paths, spelled the way every other
provenance path is spelled, reach the engine.

**Changes.**

- `kernel.rs`: `KernelResp` gains
  `pub files_written: Option<Vec<String>>`; the internal `RawResp` gains the
  same field with `#[serde(default)]` so a frame without it deserializes as
  `None`. `read_response` copies it through.
- `tool.rs`: add

  ```rust
  fn project_relative_writes(root: &Path, reported: &[String]) -> Vec<String>
  ```

  which canonicalizes the root (`dunce::canonicalize`, falling back to the
  raw root — add `dunce` to this crate's `Cargo.toml`; the workspace already
  uses it elsewhere), keeps only paths that `strip_prefix(root)` to a
  non-empty remainder, converts `\` to `/`, sorts, and dedups. Everything
  outside the root — `/tmp`, home-directory caches — is silently dropped:
  not part of the project's record.
- In `run_runtime`, on `Finished(Ok(response))`: if
  `response.files_written` is `Some` **and** `key.context_id ==
  LOCAL_CONTEXT_ID`, relativize and, when non-empty, call
  `env.report_written_paths(&paths)` (the `ToolEnv` method added in §4.3).

**Success criteria / verification** — unit tests in the two files:

- Protocol round-trips (in-memory duplex stream against `read_response`):
  a result frame carrying `files_written` yields exactly those paths; a frame
  without the field yields `None` ("absent must not be read as 'wrote
  nothing'"); a frame with `[]` yields `Some([])` ("an explicit empty list
  means 'wrote nothing', not 'ask the host'").
- `project_relative_writes`: outside-root paths dropped, root itself dropped,
  backslashes normalized, duplicates collapsed, output sorted.
- `cargo test -p wisp-runtime` green.

**Boundaries:** no changes to `RuntimeKey`, session identity, kernel
lifecycle, tool descriptions, or the R tool (R has no comparable hook and
stays on the conservative rule). Remote contexts never report.

### 4.3 `crates/wisp-tools` + `ToolEnvAdapter` — the per-call channel

**Goal:** a tool can hand reported paths to whoever is running it, without
the host learning a new interface.

**Changes.**

- `crates/wisp-tools/src/env.rs`, on the `ToolEnv` trait — the entire
  `wisp-tools` change:

  ```rust
  /// Paths an interpreter reported writing during the current tool call.
  /// Default: dropped — hosts that do not track attribution lose nothing.
  fn report_written_paths(&self, _paths: &[String]) {}
  ```

- `crates/wisp-core/src/output.rs`: `ToolEnvAdapter` implements it by pushing
  into an interior `Mutex<Vec<String>>` (the adapter is shared as `&self`),
  and gains a crate-visible `take_reported_writes(&self) -> Vec<String>` that
  drains the buffer.

**Success criteria / verification:** compiles with no `Output`-trait change
and no `src-tauri` diff; a `wisp-core` unit test drives
`report_written_paths` → `take_reported_writes` → empty-after-drain.

**Boundaries / safety argument to record in code comments:** tool calls
within one agent loop run strictly sequentially, so drain-per-call is
race-free; parallel subagent loops each construct their own adapter, so
reports cannot cross loops. The buffer must be drained **unconditionally
after every tool call** (§4.4) so a stale report can never leak into the next
call's record.

### 4.4 `crates/wisp-core` — the fold-in

**Goal:** reported paths enter the provenance record as proven attribution:
they survive the ambiguity drop, dedup against diff-derived spellings, and
change nothing when no report arrived.

**Changes.**

- `provenance.rs`: add the spelling-safe union (placed beside
  `path_identity`, which already keys the window registry):

  ```rust
  fn relative_identity(path: &str) -> String {
      path_identity(Path::new(path))
  }

  /// Union `extra` into `paths`, keeping the spelling already present when
  /// two spellings resolve to one file (`out\b.txt` vs `out/b.txt`; case
  /// variants on Windows). Every merge of written paths goes through here so
  /// the equality rule cannot drift between call sites.
  pub fn union_paths_by_identity(paths: &mut Vec<String>, extra: &[String])
  ```

  (insert-if-identity-unseen, then sort).
- `agent.rs`, in the producing-call block of `agent_loop_inner`: immediately
  after `tools.run(...)` returns, drain
  `let reported = env.take_reported_writes();` — unconditionally, even for
  non-producing calls. Then inside the `if let Some(root) = &root` block,
  after `retain_unambiguous_writes` and `augment_written_paths`:

  ```rust
  provenance::union_paths_by_identity(&mut written, &reported);
  ```

  Ordering is the correctness argument: union after retain means a reported
  path survives the drop; `read.retain(|p| !written.contains(p))` and
  `undo_file_changes` already run after this point and need no change.

**Success criteria / verification** — deterministic tests in
`provenance.rs`/`agent.rs` test modules:

- **The #937 reproduction, engine-level:** two foreign-scope windows overlap;
  the diff yields a computed-name path the source never mentions;
  `retain_unambiguous_writes` drops it; the reported union restores it. The
  record ends with both the literal-named and the computed-name file.
- **The control:** an *unreported* ambiguous path under the same overlap stays
  dropped — reports must not weaken #935's rule for anything they don't cover.
- **Spelling:** `out\b.txt` reported vs `out/b.txt` in the diff produces one
  entry, keeping the diff's spelling.
- **No-report no-op:** with an empty buffer the record is byte-identical to
  #935 behavior.
- `cargo test -p wisp-core` green, including every pre-existing #935 test
  untouched.

**Boundaries:** `retain_unambiguous_writes`, `begin_window`,
`FinishedWindow`, and the matcher functions are not modified. No arbitration
logic is added — the engine's retain step *is* the arbitration.

### 4.5 CI and evidence

**Goal:** the worker tests run where they can actually differ — per OS — and
the PR body's claims are reproducible.

**Changes.**

- `.github/workflows/test.yml`, in the existing test job (runs on the 3-OS
  matrix), after the MCP regression step:

  ```yaml
  # The kernel worker is Python, so its tests are too. Run them on all
  # three OSes: the worker's write reporting and its process handling are
  # exactly the parts that differ per platform.
  - name: Run Python kernel worker tests
    run: python3 -m unittest discover -s python -p "test_kernel_worker.py" -v
  ```

- `scripts/probe_write_audit_hook.py`: a standalone probe that registers the
  same hook, runs a table of representative cells (write, read, append,
  library save, subprocess, write-then-raise), prints what was observed, and
  times a compute-bound cell and a 2000-file write loop with and without the
  hook. It exists so the maintainer can reproduce the §7 numbers on any
  machine.

**Success criteria / verification:** CI green on all three OSes; the probe
runs standalone with only the standard library plus optional
numpy/matplotlib.

**Boundaries:** no other workflow changes.

## 5. Deliberately out of scope

Stated here so the PR cannot be held up by adjacent ideas, and recorded in
the PR body so reviewers don't re-propose the rejected ones:

- **Subprocess and C-level `sqlite3` writes** escape the audit hook. They
  fall back to #935's conservative rule — under overlap, missing rather than
  wrong. Stated as a known limitation, with the test from §4.1 as proof it is
  understood.
- **R and shell self-reporting:** no comparable hook exists; both stay on the
  conservative rule.
- **Remote execution contexts** never report (§3 constraint).
- **Deletions, directory creation:** the host's snapshot diff already sees
  them; hooking them adds only user-cache noise (measured, §4.1).
- **Rejected: patching `builtins.open`** — misses every C-level writer, which
  is the dominant case (figures).
- **Rejected: model-declared outputs** — moves truth to the layer the #911
  forensics proved was not at fault.
- **Rejected: any host-side attribution plumbing** (new `Output` methods,
  host window registries) — #935's engine windows already exist; duplicating
  them would create two sources of truth.
- **Kernel identity and lifecycle** (per-helper kernels, idle reaping, panel
  labeling, deletion records): orthogonal quality-of-life topics; none is
  needed for case 1.

## 6. Verification protocol

Per-step verification is listed in §4; this is the whole-branch gate before
the PR is opened:

1. `cargo fmt --all -- --check`
2. `cargo test -p wisp-core -p wisp-runtime -p wisp-tools -p wisp-dto`
3. `python3 -m unittest discover -s python -p "test_kernel_worker.py" -v`
4. `cargo test -p wisp-tauri` — **required even though this plan touches no
   `src-tauri` file**: any accidental drift must be caught by a compile, not
   a review. On a Linux box without root and without the dbus/GTK/WebKit dev
   packages, build a local sysroot (`apt-get download` the `-dev` packages
   and their pkg-config dependency closure, `dpkg -x` into one root), then
   set `PKG_CONFIG_PATH` to the sysroot's pkgconfig dirs,
   `PKG_CONFIG_SYSROOT_DIR` to the sysroot root,
   `PKG_CONFIG_ALLOW_SYSTEM_CFLAGS=1`, `PKG_CONFIG_ALLOW_SYSTEM_LIBS=1`, and
   — for the *test binary*, which links `libwebkit2gtk-4.1.so.0` at load
   time — `LD_LIBRARY_PATH` to the sysroot's `lib/x86_64-linux-gnu` dirs.
   Watch for a pipeline masking the exit code (`cargo test | tail` reports
   tail's status, not cargo's). Known pre-existing flake, unrelated to this
   work: `delegation_tool::tests::background_batch_returns_handle_then_delivers_one_internal_result`
   can fail under full-suite load and passes alone.
5. `cargo test --workspace` on a desktop machine (Windows) before opening
   the PR, plus the manual smoke test: the #937 two-session prompt against a
   build of this branch — both files on B's card, on nobody else's.
6. CI (three OSes, stable + MSRV, Playwright) green on the PR.

## 7. Measured evidence for the PR body

Measurements taken with `scripts/probe_write_audit_hook.py` (Linux,
Python 3.13.11); reproduce anywhere with the script.

What the hook observes:

| Cell | Reported |
| --- | --- |
| `open('a.txt','w')` | `a.txt` |
| `open('a.txt')` (read) | nothing |
| `open('a.txt','a')` (append) | `a.txt` |
| `print('hello')` | nothing |
| `plt.savefig('plot.png')` (C-level write) | `plot.png` |
| `np.save('arr.npy', ...)` | `arr.npy` |
| write then `raise` | the written path; error still reported |
| `subprocess.run([...open(...,'w')...])` | **nothing** — the documented blind spot |

Overhead (two runs each, alternating):

| | Compute cell (2M-iteration loop) | 2000 file writes |
| --- | --- | --- |
| baseline | 0.230 s / 0.198 s | 0.154 s / 0.127 s |
| with hook | 0.195 s / 0.194 s | 0.141 s / 0.155 s |

Within run-to-run noise in both directions: the hook returns immediately for
unaudited events and for every event while collection is off.

## 8. PR body checklist (upstream house format)

- User-facing problem: the #937 reproduction verbatim; **Fixes #937**,
  references #911 and builds on #919/#935.
- Changed files by layer (worker → protocol → env → engine → CI), tests, and
  the manual smoke steps.
- The §7 tables.
- Known limitations from §5, stated plainly.
- Rejected designs from §5, one line each.

## 9. Sequencing

1. §4.1 + §4.2 (worker, protocol, relativization) — independently testable
   half; land their tests green first.
2. §4.3 + §4.4 (env method, adapter, loop fold-in, union) with the engine
   tests — this is the commit whose tests flip the #937 reproduction.
3. §4.5 (CI step, probe script), then the full §6 gate, then the PR.
