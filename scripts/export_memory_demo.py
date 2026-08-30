"""Export the recorded memory-demo CLI session to a seed manifest.

Reads the real conversation captured in `<workspace>/.wisp/session.json`
(produced by driving `wisp-science` interactively against the GSE153250
workspace) plus the workspace rule/memory files, and writes
`seed/manifest_memory_01_long_context.json` in the schema consumed by
`src-tauri/src/seed.rs`.

Unlike `export_esr1_demo.py` (which reads the desktop app's SQLite store),
this reads the CLI session file directly.

Usage: python scripts/export_memory_demo.py [workspace] [output]
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_WS = ROOT / "target" / "tmp" / "memory-demo-ws"
DEFAULT_OUT = ROOT / "seed" / "manifest_memory_01_long_context.json"

# Keep tool-call detail close to verbatim: the demo exists to show a real
# agent process, so truncation stays an order of magnitude looser than the
# ESR1 narrative demos.
MAX_TOOL_TEXT = 20000
MAX_REASONING = 8000
DEMO_ID = "demo-memory-01-long-context"
REQUEST = (
    "Long-context memory demo — GSE153250 ESR1-knockdown RNA-seq. A complete "
    "analysis session recorded live with the wisp CLI: the opening turn locks "
    "the analysis decision (GENE_FILTER, PRIMARY_CONTRAST, FDR_CUTOFF), and "
    "the session then runs deep — QC, PCA, exploratory DE, sensitivity runs, "
    "figures, and report drafts — so a real /compact folds the opening into "
    "the checkpoint summary. Copy into a project, run /compact, then ask what "
    "the first answer locked — recall must come from the checkpoint. Then try "
    "search_memory: notes cover the flagged worst sample (siNT_1), the "
    "exploratory DE caveat, and a distractor ChIP-seq pilot (GSE180386)."
)


def unwrap_text(raw) -> str:
    """Message.content serializes as a plain string or an array of parts."""
    if raw is None:
        return ""
    if isinstance(raw, str):
        return raw
    if isinstance(raw, list):
        parts = []
        for block in raw:
            if isinstance(block, dict) and block.get("type") == "text":
                parts.append(block.get("text", ""))
        return "\n".join(parts)
    return str(raw)


def truncate(s: str, limit: int) -> str:
    if len(s) <= limit:
        return s
    return s[: limit - 80] + f"\n\n… [truncated, {len(s) - limit + 80} chars omitted]"


def redact(s: str, workspace: Path) -> str:
    if not s:
        return s
    out = s.replace(str(workspace), ".")
    # Bundled skill paths leak the build machine's checkout layout.
    out = out.replace(f"{ROOT}/crates/wisp-paths/../../skills/", "bundled-skills/")
    out = out.replace(str(ROOT), ".")
    # Venv/interpreter paths inside the workspace keep a portable shape.
    out = out.replace("./.wisp/python/.venv", ".wisp/python/.venv")
    # The featureCounts summary header carries the original analysis host path.
    out = out.replace("/home/data/gz0548/GSE153250_ESR1/aligned/", "aligned/")
    out = out.replace("/home/data/gz0548/", "data/")
    # `ls -l` style tool outputs carry the recording machine's username.
    out = out.replace("xzg", "demo")
    return out


def tool_input_for(name: str, args: dict):
    if name in ("python", "r"):
        return args.get("code")
    if name == "shell":
        return args.get("cmd")
    if name in ("read", "write", "edit"):
        return args.get("path")
    if name in ("search_memory", "recall_memory"):
        return args.get("query")
    return None


def messages_to_items(messages: list[dict]) -> list[dict]:
    tool_inputs: dict[str, str] = {}
    for msg in messages:
        if msg.get("role") != "assistant":
            continue
        for call in msg.get("tool_calls") or []:
            fn = call.get("function") or {}
            name = fn.get("name") or ""
            cid = call.get("id") or ""
            raw = fn.get("arguments") or "{}"
            try:
                args = json.loads(raw) if isinstance(raw, str) else raw
            except json.JSONDecodeError:
                continue
            val = tool_input_for(name, args)
            if val and cid:
                tool_inputs[cid] = val

    items: list[dict] = []

    def item(role: str, text: str, **kw) -> dict:
        row = {
            "role": role,
            "text": text,
            "tool_name": kw.get("tool_name"),
            "ok": kw.get("ok"),
            "input": kw.get("input"),
            "model_name": kw.get("model_name"),
            "resources": [],
        }
        if kw.get("call_id"):
            row["call_id"] = kw["call_id"]
        return row

    for msg in messages:
        role = msg.get("role")
        text = unwrap_text(msg.get("content")).strip()
        if role == "system":
            continue
        if role == "user":
            if text:
                items.append(item("user", text))
            continue
        if role == "assistant":
            reasoning = (msg.get("reasoning") or "").strip()
            if reasoning:
                items.append(item("reasoning", truncate(reasoning, MAX_REASONING)))
            if text:
                items.append(item("assistant", text, model_name=msg.get("model_name")))
            continue
        if role == "tool":
            name = msg.get("tool_name") or "tool"
            cid = msg.get("tool_call_id") or ""
            if not text:
                continue
            # attempt_completion carries the turn's final answer as tool output.
            if name == "attempt_completion":
                items.append(item("assistant", text, model_name=msg.get("model_name")))
                continue
            items.append(
                item(
                    "tool",
                    truncate(text, MAX_TOOL_TEXT),
                    tool_name=name,
                    ok=True,
                    input=tool_inputs.get(cid),
                    call_id=cid or None,
                )
            )
    return items


def collect_workspace_files(workspace: Path) -> dict[str, str]:
    files: dict[str, str] = {}
    for rel in ["AGENTS.md", ".wisp/WISP.md"]:
        p = workspace / rel
        if p.is_file():
            files[rel] = p.read_text(encoding="utf-8")
    mem_dir = workspace / ".wisp" / "memory"
    if mem_dir.is_dir():
        for p in sorted(mem_dir.glob("*.md")):
            files[f".wisp/memory/{p.name}"] = p.read_text(encoding="utf-8")
    return files


def assert_clean(blob: str) -> None:
    for bad in (str(DEFAULT_WS), "/home/", "WISP_API_KEY", "api.z.ai"):
        assert bad not in blob, f"manifest still contains {bad!r}"


# The whole transcript is a real recorded CLI session — no synthetic padding.
# The seed grows large because the recorded conversation itself is long; the
# opening locked decision ends up deep in the folded region when /compact
# runs on 256K-class context windows.


def split_turns(messages: list[dict]) -> list[list[dict]]:
    """Group messages into turns, each starting at a user message."""
    turns: list[list[dict]] = []
    for msg in messages:
        if msg.get("role") == "user":
            turns.append([msg])
        elif turns:
            turns[-1].append(msg)
    return turns


def turn_answer_text(turn: list[dict]) -> str:
    """Full assistant answer of a turn: narration plus attempt_completion."""
    parts: list[str] = []
    for msg in turn:
        text = unwrap_text(msg.get("content")).strip()
        if not text:
            continue
        if msg.get("role") == "assistant":
            parts.append(text)
        elif msg.get("role") == "tool" and msg.get("tool_name") == "attempt_completion":
            if not parts or text not in parts[-1]:
                parts.append(text)
    return "\n\n".join(parts)


def main() -> None:
    workspace = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else DEFAULT_WS
    out_path = Path(sys.argv[2]).resolve() if len(sys.argv) > 2 else DEFAULT_OUT

    session_file = workspace / ".wisp" / "session.json"
    messages = json.loads(session_file.read_text(encoding="utf-8"))
    if not isinstance(messages, list) or not messages:
        raise SystemExit(f"no messages in {session_file}")

    turns = split_turns(messages)
    if len(turns) < 2:
        raise SystemExit("session has fewer than two turns")

    # Turn 1 (the locked decision) leads context_seed; the recorded follow-up
    # turns keep full fidelity (reasoning, tool cards) as items.
    opening_q = unwrap_text(turns[0][0].get("content")).strip()
    opening_a = turn_answer_text(turns[0])
    context_seed = [
        {"role": "user", "text": redact(opening_q, workspace)},
        {"role": "assistant", "text": redact(opening_a, workspace)},
    ]

    rest = [msg for turn in turns[1:] for msg in turn]
    items = messages_to_items(rest)
    items = [
        {**i, "text": redact(i["text"], workspace),
         "input": redact(i["input"], workspace) if i["input"] else i["input"]}
        for i in items
    ]

    response = ""
    for i in reversed(items):
        if i["role"] == "assistant" and i["text"].strip():
            response = i["text"]
            break
    if not opening_q or not opening_a or not response:
        raise SystemExit("missing opening exchange or final response in session")

    manifest = {
        "root_frame": {
            "id": DEMO_ID,
            "parent_frame_id": None,
            "root_frame_id": DEMO_ID,
            "agent_name": "WISP",
            "status": "completed",
            "input_data": {"request": REQUEST},
            "output_data": {
                "response": response,
                "context_seed": context_seed,
                "items": items,
                "workspace_files": collect_workspace_files(workspace),
            },
        }
    }
    blob = json.dumps(manifest, ensure_ascii=False, indent=2)
    assert_clean(blob)
    out_path.write_text(blob + "\n", encoding="utf-8")
    expanded = sum(
        len(t.get("text", "")) * t.get("repeat", 1) * t.get("pad", 1)
        for t in context_seed
    )
    print(
        f"wrote {out_path.name} {out_path.stat().st_size / 1024:.1f}KiB "
        f"items={len(items)} tools={sum(1 for i in items if i['role'] == 'tool')} "
        f"workspace_files={len(manifest['root_frame']['output_data']['workspace_files'])} "
        f"expanded_seed≈{expanded / 4000:.0f}k tokens"
    )


if __name__ == "__main__":
    main()
