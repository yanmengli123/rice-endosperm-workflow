"""Simulate `copy_demo_into_project` for the memory demo: expand the seed
manifest into a CLI session file plus workspace files in a fresh directory.

Usage: python scripts/simulate_demo_copy.py <manifest> <dest_workspace>
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

MAX_SEED_REPEAT = 200
MAX_SEED_PAD = 64


def expand(manifest_path: Path) -> tuple[list[dict], dict[str, str]]:
    v = json.loads(manifest_path.read_text(encoding="utf-8"))
    od = v["root_frame"]["output_data"]

    def seed_item(role: str, text: str) -> dict:
        return {"role": role, "text": text}

    items: list[dict] = []
    for turn in od.get("context_seed") or []:
        n = max(1, min(int(turn.get("repeat", 1)), MAX_SEED_REPEAT))
        pad = max(1, min(int(turn.get("pad", 1)), MAX_SEED_PAD))
        text = (turn.get("text") or "") * pad
        for _ in range(n):
            items.append(seed_item(turn.get("role") or "user", text))
    items.extend(od.get("items") or [])

    messages: list[dict] = []
    pending_reasoning = None
    for idx, it in enumerate(items):
        role = it.get("role")
        text = it.get("text") or ""
        if role == "reasoning":
            pending_reasoning = text
            continue
        if role == "user":
            pending_reasoning = None
            messages.append({"role": "user", "content": text, "ts": 0})
        elif role == "assistant":
            msg = {"role": "assistant", "content": text, "ts": 0}
            if pending_reasoning:
                msg["reasoning"] = pending_reasoning
                pending_reasoning = None
            if it.get("model_name"):
                msg["model_name"] = it["model_name"]
            messages.append(msg)
        elif role == "tool":
            messages.append(
                {
                    "role": "tool",
                    "content": text,
                    "tool_call_id": it.get("call_id") or f"demo-tool-{idx}",
                    "tool_name": it.get("tool_name") or "tool",
                    "ts": 0,
                }
            )
    return messages, od.get("workspace_files") or {}


def main() -> None:
    manifest = Path(sys.argv[1])
    dest = Path(sys.argv[2])
    messages, files = expand(manifest)
    (dest / ".wisp").mkdir(parents=True, exist_ok=True)
    (dest / ".wisp" / "session.json").write_text(
        json.dumps(messages, ensure_ascii=False), encoding="utf-8"
    )
    for rel, content in files.items():
        p = dest / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(content, encoding="utf-8")
    total_chars = sum(len(json.dumps(m)) for m in messages)
    print(
        f"wrote {len(messages)} messages (~{total_chars / 4000:.0f}k tokens) "
        f"and {len(files)} workspace files to {dest}"
    )


if __name__ == "__main__":
    main()
