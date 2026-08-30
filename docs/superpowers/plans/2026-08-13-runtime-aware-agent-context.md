# Runtime-Aware Agent Context — Design Proposal

**Date:** 2026-08-13

**Status:** Proposal (open for discussion)

**Scope:** Enhance the agent's awareness of runtime state, available scientific
library APIs, and long-running task progress, so the LLM can make better
tool-selection and code-generation decisions within each agent turn.

## 1. Motivation

wisp-science's `RuntimeManager` already maintains persistent Python/R sessions
that retain loaded data across cells. The `inspect` protocol command returns a
`RuntimeObjectList` with every object's name, type, summary, and size. The
`system_prompt.rs` assembly layer produces a well-structured prompt with
sections for base intro, safety, rules, tool guidance, skills, memory, and
environment.

However, the agent loop currently operates **without runtime-state awareness**
in three critical dimensions:

### 1.1 The agent does not know what is already loaded

The system prompt is assembled once (or refreshed only for skills changes) and
does not include a snapshot of what objects currently exist in the persistent
REPL. When a user asks "continue with UMAP clustering," the agent has no way to
know whether `adata` is already loaded and preprocessed, or whether the kernel
is empty. This leads to:

- redundant `import` and data-loading cells that waste minutes on large datasets;
- the agent re-running preprocessing steps that were already completed;
- confusion when the agent assumes a variable exists but it was defined in a
  previous conversation that shares the same runtime.

The `inspect` command exists and works, but it is only invoked on-demand by a
`RuntimeManager::inspect()` call — never proactively before assembling the
system prompt for a new turn.

### 1.2 The agent cannot discover available library APIs

When the agent needs to use `scanpy`, `omicverse`, `seaborn`, or any other
scientific library, it must rely on its training-time knowledge of the API.
This causes:

- calling deprecated or wrong signatures (e.g. `sc.pp.neighbors` parameter
  names changed across versions);
- missing newer or domain-specific functions entirely;
- wasted tool-call iterations on `import X; help(X.func)` exploration.

The project already has a `method_search.rs` contract for autonomous code
optimization with evaluator-driven selection. But there is no lightweight,
read-only equivalent for **API discovery** — letting the agent ask "what
functions does the currently installed `scanpy` expose for batch correction?"

### 1.3 Long-running computations provide no progress feedback

When the agent executes `sc.tl.umap(adata)` or `ov.pp.preprocess(adata)` in the
persistent REPL, the `RuntimeEvent::Stdout` stream captures text output, but
libraries like `tqdm`, `scanpy`, and `omicverse` write progress bars to stderr
or use carriage-return overwrites that are not captured as structured progress.
The user (and the agent itself) see nothing until the cell completes, which can
take minutes for large datasets.

---

## 2. Proposed enhancements

Five enhancements are proposed, ordered by impact-to-effort ratio. Each is
designed to build on the existing `RuntimeManager`, `system_prompt.rs`, and
`ContextManager` architecture without introducing new crate dependencies.

### 2.1 P0 — Proactive Runtime State Injection

**Goal:** Inject a compact snapshot of the current REPL object list into the
system prompt at the start of each agent turn.

**Current state:**

- `RuntimeManager::inspect()` returns `RuntimeObjectList { objects, total_count }`.
- `RuntimeObject` has `name`, `type_name`, `summary`, `size_bytes`.
- `kernel.rs` implements the `inspect` protocol command.
- `system_prompt.rs::assemble()` does not call `inspect` or include any runtime
  state section.

**Proposed approach:**

Add an optional `RuntimeSnapshot` section to `SystemPrompt`, inserted between
`environment_guidance` and `skills_guidance`. The section is assembled
lazily — only when a runtime session exists for the current project/context —
and is bounded to prevent context-window pollution.

Suggested format (compact, token-efficient):

```text
## Runtime State (Python, local)

adata          AnnData       [n_obs=2700, n_vars=32738, obs_key='louvain']  2.1 MB
adata_raw      AnnData       [n_obs=2700, n_vars=32738]                     2.0 MB
sc             module        scanpy 1.10.2
ov             module        omicverse 2.2.0
results_df     DataFrame     shape=(8, 4)                                  1.2 KB
```

**Design considerations:**

- **Token budget:** Cap the snapshot at N objects (suggested 30). Objects
  beyond the cap are summarized as a count ("...and 12 more: 3 functions, 9
  small values"). Exclude dunder names, modules without a `__version__`, and
  objects smaller than a threshold to reduce noise.
- **Freshness:** The snapshot is taken once per turn, before the first
  `llm_chunk`. It is not refreshed mid-turn (a cell execution mid-turn changes
  state, but re-snapshotting would be expensive and confusing).
- **Context compaction integration:** The snapshot should live in its own
  `ContextManager` bucket (e.g. `kernel_state`) so it can be apportioned
  independently during context compaction, similar to how `tool_guidance` and
  `skills_guidance` are handled.
- **Failure tolerance:** If `inspect` fails (runtime dead, starting, or SSH
  timeout), omit the section silently rather than blocking the turn.
- **Multi-runtime:** If multiple runtimes exist (Python + R, or local + SSH),
  include a one-line header per runtime, each with its own compact object list.

**What this does NOT include:**

- Variable values, data previews, or DataFrame heads (those stay in the
  `python` tool's stdout).
- Automatic re-execution of previous cells.
- Persistence across application restarts (runtime state is ephemeral by
  design).

---

### 2.2 P0 — Library API Discovery Tool

**Goal:** Give the agent a read-only tool to query function signatures and
docstrings from installed scientific libraries, so it writes correct code on
the first attempt.

**Current state:**

- The `python` tool can run `help()` or `dir()`, but this wastes a full
  tool-call iteration and produces unbounded output.
- `method_search.rs` focuses on autonomous code optimization with frozen
  evaluators — it is heavyweight and not designed for quick API lookups.
- No lightweight, LLM-facing API discovery mechanism exists.

**Proposed approach:**

Add a new tool — suggested name `lookup_api` — backed by the existing
`RuntimeManager`. The tool accepts:

```json
{
  "query": "batch correction harmony",
  "library": "scanpy",  // optional; omit to search across all loaded modules
  "limit": 10
}
```

The implementation sends a new protocol command (e.g. `registry_lookup`) to
`kernel_worker.py`, which uses Python's `inspect` module to:

1. Resolve the target module (or iterate loaded modules if unspecified).
2. Match function/class names and docstrings against the query (simple
   substring + keyword matching; no embedding or network call).
3. Return a bounded list of matches with name, signature, one-line docstring
   summary, and module path.

**Suggested response format:**

```json
{
  "results": [
    {
      "name": "sc.external.pp.harmony_integrate",
      "signature": "(adata, key, basis='X_pca', adjusted_basis='X_pca_harmony')",
      "summary": "Integrate embeddings using Harmony batch correction.",
      "module": "scanpy.external.pp.harmony"
    }
  ],
  "truncated": false
}
```

**Design considerations:**

- **No external dependencies:** Pure Python `inspect` + `difflib` for matching.
  No network calls, no embedding models, no pip installs.
- **Bounded output:** Hard-cap at `limit` results (default 10), each with a
  truncated docstring (first paragraph, max 200 chars).
- **Works in any execution context:** The lookup runs inside the existing
  `RuntimeManager` session, so it naturally respects WSL/SSH/local context.
- **Protocol extension:** Add a `lookup` op to the JSON-lines protocol, handled
  by `kernel_worker.py`. This is additive — existing `execute` and `inspect`
  ops are unchanged.
- **Skill integration:** Skills like `analysis-workflow` can register
  additional function metadata or curated examples that the lookup tool
  surfaces alongside `inspect`-based results.

**What this does NOT include:**

- Semantic search or embeddings (overkill for v1; substring matching is
  sufficient and deterministic).
- Web-based API documentation lookup.
- Execution of discovered functions (the agent still uses the `python` tool for
  that).

---

### 2.3 P1 — Long-Running Task Progress Telemetry

**Goal:** Surface structured progress from long-running Python/R computations
so the user and the agent can see how far a task has progressed.

**Current state:**

- `RuntimeEvent` has `Stdout(String)` and `Finished(Result<KernelResp, String>)`.
- `kernel_worker.py` captures stdout via `stdout_chunk` frames.
- `tqdm` and similar libraries write progress bars to stderr with
  carriage-return (`\r`) overwrites; these are captured in the bounded stderr
  tail but not surfaced as structured events.
- The agent and user see nothing during a multi-minute `umap` or `preprocess`.

**Proposed approach:**

Extend the JSON-lines protocol with a `progress` frame type:

```json
{
  "type": "progress",
  "id": "<request_id>",
  "progress": 1500,
  "total": 5000,
  "description": "Computing UMAP"
}
```

On the Python side, `kernel_worker.py` installs a lightweight `tqdm`-compatible
progress hook that intercepts `tqdm.update()` calls and emits `progress` frames
at a throttled rate (e.g. at most one frame every 500ms, or every 5% increment,
whichever comes first).

On the Rust side:

- Add `RuntimeEvent::Progress { progress, total, description }`.
- The agent loop forwards progress as a tool-output event visible in the UI.
- The UI can render a progress bar or percentage in the tool-execution panel.

**Design considerations:**

- **Non-invasive:** The tqdm hook is installed transparently by
  `kernel_worker.py` at startup. User code does not need to call anything
  special. If `tqdm` is not installed, progress frames are simply not emitted.
- **Throttling:** Aggressive throttling is essential — a 5000-iteration loop
  must not generate 5000 frames. Suggested: minimum 500ms between frames per
  active progress bar, plus a final frame on completion.
- **Fallback:** Libraries that do not use `tqdm` (e.g. `scanpy`'s built-in
  logging) will not emit progress. This is acceptable — the feature is
  best-effort, not guaranteed.
- **Protocol backward compatibility:** A `progress` frame is ignored by older
  Rust hosts that do not handle it (the `read_response` loop skips unknown frame
  types by matching on the `type` field). However, the Rust side should be
  updated to surface it.

**What this does NOT include:**

- Progress for shell commands (those use the existing Run heartbeat mechanism).
- Progress for R computations (R's progress libraries are fragmented; defer to a
  future iteration).
- ETA estimation (the raw `progress/total` ratio is sufficient for v1).

---

### 2.4 P2 — Non-Blocking Background Runtime Scope

**Goal:** Allow the agent to fork a long-running computation into a background
runtime scope, so the main conversation is not blocked while waiting for
results.

**Current state:**

- `RuntimeKey` already has a `scope_key` field, and `RuntimeManager` supports
  `python_in_scope()` / `r_in_scope()` constructors and `stop_scope()`.
- The `MAINLINE_RUNTIME_SCOPE` is used for the main conversation. No code
  currently creates non-mainline scopes for background work.
- The agent's `python` tool blocks until the cell completes. A 10-minute
  computation means 10 minutes of no interaction.
- `RunManager` exists for one-off shell jobs with heartbeat + logs, but it does
  not share the persistent REPL's loaded data.

**Proposed approach:**

Introduce a `background_task` tool that:

1. Creates a new `RuntimeKey` with a unique scope_key (e.g.
   `bg-<uuid>`).
2. Seeds the new scope by re-executing a minimal set of import statements
   (captured from the mainline scope's inspect data) or by serializing
   picklable objects from the mainline scope.
3. Executes the requested code in the background scope.
4. Returns immediately with a task handle (scope_key + request_id).
5. The agent can poll with a `check_background_task` tool, or the completion
   event is delivered as a tool-output event when the cell finishes.

**Design considerations:**

- **State seeding v1:** For simplicity, the background scope starts empty and
  the caller provides the full code including necessary imports and data
  loading. This avoids the complexity of cross-process object serialization. A
  future v2 could pickle-transfer annotated objects from the mainline scope.
- **Resource limits:** Background scopes count against the same
  `RuntimeManager` process pool. The UI should show them in the Runtimes panel
  with a "background" label, and the user can stop them explicitly.
- **Failure isolation:** A crash in a background scope does not affect the
  mainline scope. The background scope's `RuntimeStatus::Dead` is reported to
  the agent as a tool error.
- **Scope cleanup:** Background scopes are stopped when the conversation ends or
  when the user explicitly stops them. They are not persisted across
  application restart (consistent with mainline runtime lifecycle).

**What this does NOT include:**

- `os.fork()` (not portable to Windows; use a new worker process instead).
- Checkpoint/resume of background tasks.
- Multi-node or HPC background execution (use `RunManager` + SLURM for that).

---

### 2.5 P3 — Context-Aware Specialist Auto-Routing

**Goal:** Reduce user cognitive load by having the agent automatically recommend
or invoke the most appropriate Specialist based on the current runtime context
and user message content.

**Current state:**

- Specialists (Reviewer, Reader, Illustrator, etc.) are user-selected via
  `system_prompt.rs` specialist section injection.
- The agent can delegate via the existing Controlled Delegation mechanism with
  integrity hashes and approval chains.
- There is no automatic routing — the user must know which Specialist to
  select, or the agent must guess from the user message alone.

**Proposed approach:**

This is the lightest-weight enhancement and requires no new tools:

1. In `system_prompt.rs`, add an optional `## Available Specialists` section
   that lists the names and one-line specializations of each configured
   Specialist (similar to how `skills_guidance` lists skill counts).
2. Add guidance text: "If the user's request matches a Specialist's
   specialization more closely than the default agent, consider delegating via
   `delegate_tasks`."
3. The runtime state snapshot from §2.1 can further inform routing — e.g., if
   the kernel has `matplotlib` Figure objects loaded, the Illustrator
   Specialist is more likely relevant.

**Design considerations:**

- **No new infrastructure:** This reuses the existing delegation pipeline. The
  only change is adding a summary section to the system prompt and a routing
  hint.
- **User override:** The user's explicit Specialist selection always takes
  precedence. Auto-routing is a suggestion, not a mandate.
- **Delegation safety:** The existing `delegation_policy.rs` integrity checks
  and approval chain apply unchanged.

---

## 3. Priority and effort summary

| Priority | Enhancement | Est. effort (Rust + Python) | Key files touched |
|---|---|---|---|
| **P0** | Proactive Runtime State Injection | ~200 Rust + ~30 Python | `system_prompt.rs`, `manager.rs`, `kernel_worker.py` |
| **P0** | Library API Discovery Tool | ~150 Rust + ~100 Python | `tool.rs` (new tool), `kernel.rs`, `kernel_worker.py` |
| **P1** | Long-Running Task Progress Telemetry | ~80 Rust + ~80 Python | `kernel.rs`, `manager.rs`, `kernel_worker.py` |
| **P2** | Non-Blocking Background Runtime Scope | ~300 Rust + ~100 Python | `tool.rs`, `manager.rs`, `kernel_worker.py` |
| **P3** | Context-Aware Specialist Auto-Routing | ~50 Rust | `system_prompt.rs` |

P0 items are the highest impact-to-effort ratio and can be delivered
independently in PR-sized stages.

---

## 4. Architecture fit

All five enhancements are designed as natural extensions of existing wisp-science
abstractions:

```
                    ┌──────────────────────────────────────────────────────────┐
                    │                   system_prompt.rs                       │
                    │                                                          │
   §2.1 ──────────► │  ... tool_guidance | ★ runtime_state | skills | memory  │
   §2.5 ──────────► │  ... | ★ available_specialists | environment            │
                    └────────────────────────┬─────────────────────────────────┘
                                             │ assembled prompt
                                             ▼
                    ┌──────────────────────────────────────────────────────────┐
                    │                    Agent Loop                            │
                    │                                                          │
   §2.2 ──────────► │  tool: lookup_api ──────► RuntimeManager::execute       │
                    │  tool: python   ────────► RuntimeManager::execute       │
   §2.4 ──────────► │  tool: background_task ─► RuntimeManager (new scope)    │
                    │                          │                               │
   §2.3 ◄──────────│  RuntimeEvent::Progress │ Stdout │ Finished              │
                    └────────────────────────┬─────────────────────────────────┘
                                             │ JSON-lines protocol
                                             ▼
                    ┌──────────────────────────────────────────────────────────┐
                    │              kernel_worker.py                            │
                    │                                                          │
                    │  ops: execute | inspect | ★ lookup | ★ progress_hook    │
                    └──────────────────────────────────────────────────────────┘

  ★ = proposed additions
```

No new crates are needed. No new external dependencies are introduced. The
JSON-lines protocol gains two additive frame types (`progress`, `lookup`
response) that older hosts safely ignore.

---

## 5. Non-goals

- **Variable value injection:** The system prompt should not contain data
  values, DataFrame heads, or matrix previews. Those belong in tool output.
- **Cross-restart runtime state:** Runtime sessions remain ephemeral. This
  proposal does not add persistence, checkpointing, or re-attachment.
- **Semantic API search:** v1 uses substring matching only. Embedding-based
  semantic search is a future enhancement if substring matching proves
  insufficient.
- **Generic plugin registry:** The `lookup_api` tool is specific to Python/R
  library introspection, not a general-purpose plugin system.
- **HPC background execution:** Use the existing `RunManager` + SLURM pipeline
  for cluster jobs. Background runtime scopes are for local/WSL/SSH interactive
  sessions only.
- **Replacing the `python` tool:** The `python` tool remains the primary code
  execution interface. `lookup_api` and `background_task` are complementary.

---

## 6. Suggested delivery sequence

Each item can be delivered as an independent, PR-sized stage:

1. **Stage 1 (P0):** Proactive Runtime State Injection — add `RuntimeSnapshot`
   section to `system_prompt.rs`, wire up `RuntimeManager::inspect()` call
   before prompt assembly, add `kernel_state` bucket to `ContextManager`.

2. **Stage 2 (P0):** Library API Discovery Tool — add `lookup` op to
   `kernel_worker.py`, add `lookup_api` tool adapter in `tool.rs`, extend
   `kernel.rs` protocol reader.

3. **Stage 3 (P1):** Progress Telemetry — add tqdm hook to `kernel_worker.py`,
   add `progress` frame type to protocol, add `RuntimeEvent::Progress` variant.

4. **Stage 4 (P2):** Background Runtime Scope — add `background_task` and
   `check_background_task` tools, leverage existing `scope_key` mechanism.

5. **Stage 5 (P3):** Specialist Auto-Routing — add `available_specialists`
   section to system prompt, add routing guidance text.

Stages 1 and 2 have no interdependencies and can be developed in parallel.
Stages 3–5 build on the protocol and tool infrastructure but are otherwise
independent.

---

## 7. Open questions

1. **Snapshot token budget:** How many tokens should the runtime state snapshot
   consume? Should it be a fixed cap (e.g. 500 tokens) or proportional to the
   context window?

2. **Lookup scope:** Should `lookup_api` search only currently-loaded modules,
   or also modules that are installed but not yet imported? Searching installed
   modules is more useful but slower (requires `importlib.metadata` iteration).

3. **Progress for non-tqdm libraries:** Should we provide a decorator or helper
   that skill authors can use to emit progress from custom code? Or is the tqdm
   hook sufficient for v1?

4. **Background scope limits:** Should there be a maximum number of concurrent
   background scopes per project? What happens if the user starts 10 background
   tasks — do they queue, or do they all run in parallel (potentially exhausting
   memory)?

5. **Specialist routing confidence:** Should the auto-routing suggestion be
   surfaced to the user for confirmation before delegation, or should it be
   fully automatic with post-hoc undo?

---

## 8. Testing strategy

All enhancements should be testable without real models, API keys, network,
SSH, or GPU, consistent with the project's existing testing philosophy:

- **Runtime State Injection:** Use fake `RuntimeKernel` implementations that
  return canned `RuntimeObjectList` data; verify the system prompt contains the
  expected section and respects the token budget.
- **API Discovery:** Mock `kernel_worker.py` responses; verify the tool returns
  bounded, well-formed results and handles missing modules gracefully.
- **Progress Telemetry:** Use a fake worker that emits `progress` frames at
  controlled intervals; verify throttling and forwarding.
- **Background Scope:** Use the existing `RuntimeManager` test harness with
  fake launchers; verify scope isolation, cleanup, and failure reporting.
- **Specialist Routing:** Verify the system prompt contains the specialists
  section and that the routing guidance text is present.
