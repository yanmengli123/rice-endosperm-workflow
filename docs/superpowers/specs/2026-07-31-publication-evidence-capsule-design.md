# Publication Evidence Capsule — Design Spec

**Date:** 2026-07-31  
**Issue:** #576  
**Status:** Execution baseline  
**Target:** v0.30 first usable slice, followed by fine-grained evidence and isolated verification

## 1. Problem and product boundary

Project transfer, Session export, and publication evidence solve different
problems:

- Project transfer preserves a whole project for migration or backup.
- Session export shares one exploratory conversation and its current files.
- A Publication Workspace selects exact evidence, freezes it, explains its
  lineage, and builds an allowlisted capsule for one manuscript revision.

The publication feature must never infer that the latest version of an
Artifact is the version used by a paper. Every binding names an immutable
source ID. A frozen revision is historical state: later Runs, files, reviews,
and manuscript revisions cannot change its meaning.

The first release is called a **Publication Evidence Capsule** or
**Traceability Capsule**. It reports one of four capabilities instead of
claiming that every capsule is fully reproducible:

1. `archived`: selected evidence bytes or immutable references are frozen.
2. `traceable`: producing Runs and exact declared dependencies are recorded.
3. `re_executable`: the recorded workflow has all required accessible inputs,
   code, parameters, and environment metadata.
4. `reproduced`: an isolated verification run passed its declared comparators.

## 2. Required invariants

1. Evidence binds an exact `ArtifactVersion`, `Run`, or stable fine-grained
   anchor. It never binds `artifacts.latest_version_id`.
2. Local files selected for a frozen revision are either immutable SHA-256
   snapshots or explicit references with recorded SHA-256 and size.
3. A historical live file without a creation-time checksum is never
   retroactively presented as the original bytes. Freeze creates a new
   `late_capture` version and reports `historical_content_unverified`.
4. Run inputs and outputs bind exact `ArtifactVersion` IDs. Compatibility
   `run_artifacts` rows remain readable but are not authoritative lineage.
5. Dependency edges record `basis` (`declared`, `observed`, `inferred`,
   `user_asserted`) and `confidence` (`exact`, `likely`, `uncertain`).
   Inferred or uncertain edges cannot by themselves satisfy traceability.
6. Freezing is atomic at the database boundary. Failed work may leave an
   unreferenced content-addressed blob, but never a partially frozen revision.
7. Frozen and Published revisions are immutable in SQLite. Changes require
   cloning a full new revision.
8. Supersession is a relation between bindings in a revision, not a global
   Artifact state.
9. Manual review and automated reproduction are independent states.
10. Public capsule builds never include Restricted or Private bytes.
11. Frozen bindings protect their ArtifactVersions and blobs from ordinary
    deletion, Session deletion, undo cleanup, and garbage collection.
12. Project transfer copies every Publication and lineage table plus referenced
    project-local blobs.
13. Repeated builds of one frozen revision use its stored canonical manifest
    and therefore return the same revision manifest SHA-256.

## 3. Canonical lineage graph

The authoritative graph is bipartite around Runs:

```text
ArtifactVersion / ExternalResource
                │ consumes
                ▼
               Run ─────► CodeSnapshot
                │         EnvironmentSnapshot
                │         Parameters / randomness
                │ produces
                ▼
          ArtifactVersion
```

`artifact_dependencies` remains a useful projection, but it is not the sole
source of truth. Research Graph nodes and edges are also projections for
navigation; Publication tables own revision, order, review, visibility, freeze,
and supersession semantics.

## 4. Content-addressed snapshot service

`src-tauri` owns one reusable snapshot service used by message resources, Run
inputs and outputs, Publication Freeze, and Capsule Builder.

For a regular file it:

1. rejects symlinks and non-files;
2. streams bytes through SHA-256 without loading the whole file;
3. records size;
4. optionally writes the stream to a temporary file under
   `.wisp/artifacts/`, then atomically moves it to
   `.wisp/artifacts/sha256/<prefix>/<sha256>.<ext>`;
5. returns a project-relative storage path for a copied blob, or the original
   reference for reference-only capture.

Default policy:

- files up to 32 MiB: snapshot;
- larger local files: creation-time checksum and reference;
- remote resources: immutable reference metadata where available;
- Restricted or Private data: reference-only unless the user explicitly
  chooses a private build policy.

The 32 MiB threshold is a storage policy, not a hashing limit.

Artifact versions record two independent facts:

```text
materialization: snapshot / reference / external
capture_timing:  at_creation / late / unknown
```

Legacy rows are `reference + unknown`. Freeze never mutates their historical
metadata.

## 5. Lineage schema

### 5.1 Artifact identity

`artifacts.logical_key` is nullable and unique within a project. It names a
logical output independently of its filename.

- `OutputSpec.logical_key`, when supplied, is authoritative.
- Otherwise a normalized project-relative path is the default key candidate.
- URI identity uses the normalized URI.
- Users may later explicitly connect a successor to an existing logical
  Artifact.
- A filename alone is never sufficient identity.

Repeated output for one logical key creates a new version under the same
Artifact. The prior latest version becomes `parent_version_id`.

### 5.2 Exact Run relationships

```text
run_inputs
- id
- run_id
- artifact_version_id | external_resource_id
- source_ref
- role
- required
- basis
- confidence
- created_at

run_outputs
- id
- run_id
- artifact_version_id
- role
- logical_output_key
- source_path
- created_at

run_code_snapshots
- id
- run_id
- source_kind
- source_path
- source_text
- checksum
- storage_path
- git_commit
- dirty_patch
- created_at

run_environment_snapshots
- run_id
- env_snapshot_hash
```

Declared local input paths are captured before execution. Unresolvable
declarations remain visible with their `source_ref`, basis, confidence, and a
missing exact source rather than disappearing.

Environment JSON is recursively key-sorted, serialized canonically, and hashed
with SHA-256. The snapshot may include:

- context identity and capabilities;
- OS and architecture;
- interpreter/package or lockfile metadata when available;
- container image digest, CUDA/GPU/driver information when available;
- locale, timezone, random seeds, and allowlisted environment variables.

Missing fields remain explicit readiness findings. Secrets and unrestricted
environment dumps are never stored.

## 6. Publication domain model

```text
publications
publication_revisions
publication_items
publication_item_links
evidence_bindings
evidence_reviews
evidence_supersessions
publication_readiness_reports
publication_waivers
capsule_builds
external_resources
reproduction_runs
reproduction_results
```

### 6.1 Revisions and structure

`PublicationRevision` has `parent_revision_id`, a label such as
`Submission v1.0`, and state `draft`, `frozen`, or `published`.

`PublicationItem` kinds are `section`, `claim`, `figure`, `table`, `methods`,
and `supplement`. `parent_item_id` and `ordinal` define manuscript order.
`PublicationItemLink` expresses semantic relations such as
`figure supports claim`.

Clone copies the complete materialized revision with new IDs. Runtime reads
never merge deltas from parent revisions.

### 6.2 Evidence axes

`EvidenceBinding` stores independent axes:

```text
selection_state:    candidate / selected / rejected
review_state:       unreviewed / reviewed
reproduction_state: not_run / passed / failed / not_applicable
visibility:         public / restricted / private
```

A binding also stores exact `source_kind + source_id`, purpose, supported
claim, and a source snapshot used for stable display. `EvidenceReview` records
reviewer, method, time, environment, comparator, tolerance, result, and report.
`EvidenceSupersession` connects old and replacement bindings inside one
revision.

## 7. Stable source anchors

The generic binding supports these source kinds:

- `artifact_version`
- `run`
- `execution_log`
- `message_span`
- `tool_call`
- `code_cell`
- `external_resource`

Fine-grained anchors are immutable:

```text
MessageSpan:
frame_id + message_seq + UTF-8 byte range + text_snapshot_hash + text_snapshot

ToolCall:
frame_id + message_seq + tool_call_id + arguments_hash + result_hash

CodeCell:
execution_log_id + language + source_hash + source_snapshot
```

Publication evidence never resolves an execution by “most recent row for this
path”.

## 8. Atomic Freeze and readiness

Freeze has a short lock/prepare/commit protocol:

1. atomically move the Draft revision to an internal `freezing` state;
2. resolve every Selected binding;
3. validate or materialize exact local snapshots;
4. create prepared late-capture versions where historical bytes are unknown;
5. resolve Run inputs, outputs, code, environment, and external resources;
6. evaluate visibility, license, sensitivity, path, symlink, and secret policy;
7. generate blocker, warning, and waiver findings;
8. build canonical manifest JSON and SHA-256;
9. in one transaction insert prepared versions, retarget affected bindings,
   store the readiness report and manifest, and switch to `frozen`.

On failure the revision returns to Draft with an error. The final transaction
is all-or-nothing.

Readiness reports distinguish:

- missing facts;
- declared exact lineage;
- observed lineage;
- inferred candidates;
- user assertions;
- waived findings.

Waivers include author, reason, timestamp, and finding code. They do not erase
the original finding.

## 9. Visibility and sensitive-information policy

Before Public freeze/build, the backend checks at minimum:

- Public/Restricted/Private dependency conflicts;
- absolute paths, home directories, usernames, SSH hosts, and internal network
  addresses;
- common API key, token, password, and private-key patterns;
- symlinks and path traversal;
- file type and size;
- missing license or redistribution metadata;
- restricted data without access instructions;
- explicit confirmation that PHI/PII and human-subject data were reviewed.

If Public evidence depends on Restricted or Private data, the public capsule
contains only resource identity, checksum where permitted, license/access
metadata, and access instructions.

## 10. Canonical evidence manifest

The frozen revision stores canonical JSON with recursively sorted object keys
and deterministically sorted arrays:

```json
{
  "schema_version": 1,
  "publication_revision_id": "...",
  "capability_level": "traceable",
  "evidence": [],
  "runs": [],
  "inputs": [],
  "outputs": [],
  "code": [],
  "environments": [],
  "external_resources": [],
  "omissions": [],
  "verification": []
}
```

Every included file records capsule-relative path, SHA-256, size, MIME, exact
source identity, visibility, license, producing Run, and dependency role.

Volatile build timestamps are excluded from the revision manifest. Archive
entry order, timestamps, and permissions are normalized. A Capsule Build
records its own timestamp separately.

## 11. Selective Capsule Builder

Input is one Frozen or Published revision. The builder reads only its stored
manifest and immutable sources; it never scans the current workspace for
additional content.

The first layout is:

```text
paper-capsule/
├── capsule.json
├── README.md
├── REPRODUCE.md
├── CITATION.cff
├── workflow/
├── data/
│   ├── manifest.json
│   ├── checksums.sha256
│   └── access-instructions.md
├── figures/
├── tables/
├── reference-results/
├── evidence/
├── provenance/
└── verification-report.json
```

Only allowlisted Public bytes enter a public build. Restricted and Private
entries become omissions with access instructions. A build fails if an
included blob no longer matches the frozen checksum.

## 12. Isolated rerun and comparison

Verification runs in a fresh temporary workspace populated only from the
capsule allowlist. It never requires a real SSH host, WSL distro, scheduler,
GPU, network, or API key in tests.

The runner:

1. materializes allowed inputs and workflow files;
2. records the actual verification environment;
3. executes the declared entry points using the existing structured Run
   runner abstraction;
4. harvests exact outputs;
5. applies declared comparators (`sha256`, text, JSON, numeric tolerance);
6. stores a reproduction report and per-output results.

A temporary-workspace rerun without a recreated dependency environment is
reported as `re_executable`, not `reproduced`, unless the environment contract
is satisfied and every required comparator passes.

## 13. Retention, deletion, and transfer

- Database triggers reject deletion of ArtifactVersions referenced by Frozen
  or Published bindings.
- Session deletion may remove conversation rows but must preserve protected
  evidence anchors or reject with an actionable error.
- Blob GC treats frozen manifests as roots.
- Project export/import includes all new tables and content-addressed blobs.
- Transfer path rewriting never rewrites immutable blob identities.
- Import validates revision manifest hashes before accepting Frozen state.

## 14. UI behavior

Artifact and Run surfaces expose **Use in publication**. The dialog selects a
Publication, Draft revision, manuscript item, purpose/claim, selection state,
and visibility. The backend resolves an Artifact selection to its exact latest
version at the moment the user confirms.

Publication Workspace shows:

- manuscript tree;
- exact evidence version and source quality;
- producing Run, inputs, code, environment, and external resources;
- review and reproduction states;
- blockers, warnings, and waivers;
- drift and available successor versions;
- revision clone, freeze, capsule build, and verification actions.

Every dialog, menu, and nested overlay participates in the window-level Escape
stack. One immediate Escape closes only the visually topmost surface.

## 15. Acceptance evidence

The implementation is complete only when tests prove:

1. Frozen bindings contain exact IDs, never dynamic latest-version lookups.
2. Editing a workspace file cannot change frozen preview, checksum, or capsule.
3. Rebuilding one revision preserves its manifest hash.
4. Run output lineage names the exact produced ArtifactVersion.
5. Figure/Table readiness shows Run, code, inputs, and environment or an
   explicit missing reason and waiver.
6. Public builds contain no Restricted/Private bytes.
7. New results require a new revision and cannot mutate old revisions.
8. Supersession in a new revision does not alter old revision semantics.
9. Session/Artifact deletion and GC cannot break frozen evidence.
10. Historical live files become visible `late_capture` versions.
11. Project export/import preserves Publication state and validates hashes.
12. Fine-grained anchors remain stable after later messages or executions.
13. Isolated verification records its actual environment and comparator
    results without depending on external infrastructure in automated tests.

