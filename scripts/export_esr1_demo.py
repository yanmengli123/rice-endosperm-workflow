"""Export ESR1 example sessions to ordered, redacted seed manifests."""

from __future__ import annotations

import csv
import json
import re
import sqlite3
import tarfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SEED = ROOT / "seed"
SRC_UP = Path(r"D:\Wisp-Science\ESR1_ws")
SRC_DOWN = Path(r"D:\Wisp-Science\ESR1_downstream")
DB = Path(
    r"C:\Users\xuzhougeng\AppData\Roaming\science.wisp-science\wisp-science\wisp.sqlite"
)
MAX_TOOL_TEXT = 6000
MAX_REASONING = 4000

# Narrative order: find → inspect → upstream → downstream → hypotheses
SESSIONS = [
    (
        "5602d6f7-80d1-447f-9ef3-56e1eb4d99eb",
        "manifest_esr1_01_datasets",
        "demo-esr1-01-datasets",
        None,
    ),
    (
        "3670bb53-ed32-44e5-9ee3-e8ca741ed8ec",
        "manifest_esr1_02_samples",
        "demo-esr1-02-samples",
        None,
    ),
    (
        "0a662fc6-da20-4a60-bda7-1a15c6d2fe5a",
        "manifest_esr1_03_rnaseq",
        "demo-esr1-03-rnaseq",
        "rnaseq",
    ),
    (
        "5cc30adb-22ac-4894-af5c-ac108c758bad",
        "manifest_esr1_04_downstream",
        "demo-esr1-04-downstream",
        "downstream",
    ),
    (
        "78af9873-42bb-4a76-a051-c76720ff58a2",
        "manifest_esr1_05_hypotheses",
        "demo-esr1-05-hypotheses",
        None,
    ),
]

# Explicit cleaned first-user prompts (privacy + narrative).
REQUEST_OVERRIDES = {
    "manifest_esr1_01_datasets": (
        "Help me find RNA-seq knockdown datasets involving ESR1 and its "
        "coregulatory factors in MCF7 cells. Prefer datasets that include "
        "multiple knockdown conditions within the same study."
    ),
    "manifest_esr1_02_samples": (
        "What specific samples are included in GSE153250? Please organize them "
        "by treatment group."
    ),
    "manifest_esr1_03_rnaseq": (
        "Connect to the remote compute host, locate the FASTQ data for GSE153250, "
        "keep only the siESR1 and siNT groups, and exclude all other groups. "
        "Perform transcriptome upstream analysis to obtain the Counts data."
    ),
    "manifest_esr1_04_downstream": (
        "Based on the upstream Counts data from GSE153250, perform transcriptome "
        "downstream analysis: differential expression, enrichment analysis, and "
        "GSEA. Download Enrichr libraries for human gene sets as needed and use "
        "them as GMT files for enrichment."
    ),
    "manifest_esr1_05_hypotheses": (
        "Based on the Counts data from our study, along with the differential "
        "expression analysis and pathway enrichment analysis results, design 10 "
        "research projects. For each project, clearly state the core findings/"
        "evidence basis, scientific question, clinical significance, study design, "
        "and key highlights/novelty. Use literature retrieval if necessary to "
        "support your hypotheses."
    ),
}

REDACT = [
    (re.compile(r"English (?:reply|instruction)[.,、]?\s*", re.I), ""),
    (re.compile(r"Attached sessions:.*?(?=\n\n|\Z)", re.I | re.S), ""),
    (
        re.compile(
            r"first test the network speed using the proxy configured in ~/.bashrc,\s*",
            re.I,
        ),
        "",
    ),
    (
        re.compile(
            r"(?:test|check|probe|using|via|with)\s+(?:the\s+)?(?:configured\s+)?proxy"
            r"[^.]*\.\s*",
            re.I,
        ),
        "",
    ),
    (
        re.compile(
            r"(?:Use|using)\s+the\s+proxy\s+configured\s+in\s+~/.bashrc[^.]*\.\s*",
            re.I,
        ),
        "",
    ),
    (
        re.compile(
            r"^-?\s*\*\*Proxy\*\*.*$",
            re.I | re.M,
        ),
        "",
    ),
    (
        re.compile(
            r"(?:export\s+)?(?:https?|all|ftp)_proxy\s*=\s*\S+",
            re.I,
        ),
        "",
    ),
    (re.compile(r"guotosky", re.I), "remote-host"),
    (re.compile(r"guozi-server\d*", re.I), "remote-host"),
    (re.compile(r"ssh:remote-host", re.I), "ssh:remote-host"),
    (re.compile(r"`?https?://[^`\s]+`?", re.I), ""),
    (re.compile(r"socks5?://\S+", re.I), ""),
    (re.compile(r"\b\d{1,3}(?:\.\d{1,3}){2,3}(?::\d+)?\b"), ""),
    (re.compile(r"\b7897\b"), ""),
    (re.compile(r"configured-proxy", re.I), ""),
    (re.compile(r"\bwith(?:out)?\s+proxy\b[^.`\n]*[`']?", re.I), ""),
    (re.compile(r"\bvia\s+proxy\b[^.`\n]*[`']?", re.I), ""),
    (re.compile(r"\busing\s+(?:the\s+)?(?:configured\s+)?proxy\b[^.`\n]*[`']?", re.I), ""),
    (
        re.compile(
            r"(?im)^[^\n]*(?:http_proxy|https_proxy|all_proxy|ftp_proxy|PROXY for downloads|Mihomo Proxy|Check proxy config)[^\n]*\n?"
        ),
        "",
    ),
    # Also scrub proxy env assignments embedded in JSON/command strings.
    (
        re.compile(
            r"(?:export\s+)?(?:HTTPS?|ALL|FTP)_PROXY\s*=\s*\S+",
            re.I,
        ),
        "",
    ),
    (re.compile(r"trojan_(?:http_proxy|socks5)\.py", re.I), "tool_helper.py"),
    (re.compile(r"Check proxy config in bashrc", re.I), "Check shell environment"),
    (re.compile(r"Read bashrc for proxy config", re.I), "Check shell environment"),
    (re.compile(r"Probe server and bashrc", re.I), "Probe server environment"),
    (re.compile(r"The proxy is configured[^.]*\.", re.I), ""),
    (re.compile(r"\*\*Proxy\*\*[^\n]*", re.I), ""),
    (re.compile(r"Proxy:\s*`?[^`\n]+`?", re.I), ""),
    # Drop the duplicate "use ~/.bashrc proxy" task wording that the model restates.
    (
        re.compile(
            r"(?im)^\s*(?:\d+\.\s*|[-*]\s*)?(?:Use|Using)\s+(?:the\s+)?"
            r"proxy\s+configured\s+in\s+~/.bashrc[^\n]*\n?"
        ),
        "",
    ),
    (
        re.compile(
            r"(?im)^\s*(?:\d+\.\s*|[-*]\s*)?First,\s*check what's in ~/.bashrc "
            r"for proxy settings\n?"
        ),
        "",
    ),
    (
        re.compile(
            r"(?i)(?:Use|Using)\s+(?:the\s+)?proxy\s+configured\s+in\s+~/.bashrc"
            r"(?:\s+for\s+downloading)?\.?\s*"
        ),
        "",
    ),
    (
        re.compile(
            r"(?i)(?:check|look(?:\s+at)?)\s+(?:what's|what is)\s+in\s+~/.bashrc"
            r"\s+for\s+proxy\s+settings\.?\s*"
        ),
        "",
    ),
    (re.compile(r"(?i)I need to use the proxy from ~/.bashrc\.?\s*"), ""),
    (re.compile(r"(?i)But with our proxy,[^.]*\.\s*"), ""),
    (re.compile(r"(?i)Handles? the proxy configuration\.?\s*"), ""),
    (
        re.compile(
            r"(?i)Let me monitor the run that checks the proxy config\.?"
        ),
        "Let me monitor the environment check.",
    ),
    (re.compile(r"(?i)Good[^\n.]*proxy is configured[^\n.]*\.?\s*"), ""),
    (re.compile(r"(?i)Connect to the server\b"), "Connect to the remote compute host"),
    (
        re.compile(
            r"(?i),\s*checking the proxy configuration,?\s*(?:and\s+)?"
        ),
        ", ",
    ),
    (
        re.compile(
            r"(?i)checking the proxy config(?:uration)?(?:\s+and\s+testing the network)?\.?\s*"
        ),
        "",
    ),
    (
        re.compile(
            r"(?i)—\s*checking proxy config and available tools"
        ),
        "— checking available tools",
    ),
    (
        re.compile(
            r"(?i)(?:based on the proxy config|based on the proxy configuration)"
            r"[^,.\n]*[,.]?\s*"
        ),
        "",
    ),
    (
        re.compile(
            r"(?i)Probe server environment:\s*proxy config,\s*proxy test,\s*"
            r"available tools"
        ),
        "Probe server environment and available tools",
    ),
    (re.compile(r"(?i)proxy config(?:uration)?"), "environment"),
    (re.compile(r"(?i)proxy test"), "connectivity check"),
    # Shell lines that only pull proxy/env from bashrc; PATH tools stay via absolute paths.
    (re.compile(r"(?m)^\s*source\s+~/.bashrc\s*(?:2>/dev/null)?\s*;?\s*\n?"), ""),
    (re.compile(r"source\s+~/.bashrc\s*(?:2>/dev/null)?\s*;?\s*"), ""),
    # Leftover fragments after proxy/bashrc scrubbing.
    (re.compile(r"(?i)Test network speed(?:\s+bashrc)?"), "Check network connectivity"),
    (re.compile(r"(?i)environment in ~/.bashrc,?\s*"), ""),
    (re.compile(r"(?im)^\s*bashrc:\s*(?:-\s*)*\n?"), ""),
    (re.compile(r"(?im)^\s*\d+\.\s*bashrc\s*$"), ""),
    (re.compile(r"(?i)\bbashrc\b(?:\s*\(but they're all commented out[^)]*\))?"), ""),
    (re.compile(r"[ \t]{2,}"), " "),
    (re.compile(r" +\."), "."),
    (re.compile(r"D:\\\\ESR1_project", re.I), "data"),
    (re.compile(r"D:/ESR1_project", re.I), "data"),
    (re.compile(r"D:\\Wisp-Science\\ESR1_(?:ws|downstream)", re.I), "."),
    (re.compile(r"D:/Wisp-Science/ESR1_(?:ws|downstream)", re.I), "."),
    (re.compile(r"~/GSE153250_ESR1", re.I), "~/workspace/GSE153250"),
    (re.compile(r"miniconda3", re.I), "conda-tools"),
    (re.compile(r"~/bin/(\w+)", re.I), r"~/tools/\1"),
    (re.compile(r"~/.local/bin/(\w+)", re.I), r"~/tools/\1"),
    (re.compile(r"/home/data/gz0548/", re.I), "/home/demo/"),
    (re.compile(r"/home/data/gz0548", re.I), "/home/demo"),
    (re.compile(r"/home/demo/GSE153250_ESR1", re.I), "/home/demo/workspace/GSE153250"),
    (re.compile(r"\bgz0548\b", re.I), "demo"),
    (re.compile(r"\bgz05\b", re.I), "demo"),
    (re.compile(r"RTX\s*\d+", re.I), "GPU"),
    (re.compile(r"\b256\s*cores\b", re.I), "many cores"),
    (re.compile(r"GPU-[0-9a-f-]{20,}", re.I), "GPU-REDACTED"),
    (re.compile(r'"auth_method"\s*:\s*"password"', re.I), '"auth_method":"key"'),
    (re.compile(r"\n{3,}"), "\n\n"),
]


def _drop_proxy_narrative_lines(s: str) -> str:
    kept: list[str] = []
    for line in s.splitlines():
        low = line.lower()
        if "proxy" in low or "bashrc" in low:
            continue
        kept.append(line)
    return "\n".join(kept)


def redact(s: str) -> str:
    if not s:
        return s
    out = s
    for pat, repl in REDACT:
        out = pat.sub(repl, out)
    out = _drop_proxy_narrative_lines(out)
    # Collapse leftover empty proxy/env noise lines.
    out = re.sub(r"[ \t]+\n", "\n", out)
    out = re.sub(r"\n{3,}", "\n\n", out)
    return out.strip()


def truncate(s: str, limit: int) -> str:
    if len(s) <= limit:
        return s
    return s[: limit - 80] + f"\n\n… [truncated, {len(s) - limit + 80} chars omitted]"


def unwrap_text(raw: str) -> str:
    if not raw:
        return ""
    s = raw.strip()
    for _ in range(4):
        if not s:
            return s
        if s[0] in '"[{':
            try:
                v = json.loads(s)
            except json.JSONDecodeError:
                break
            if isinstance(v, str):
                s = v
                continue
            if isinstance(v, dict):
                if "text" in v or "content" in v:
                    text = v.get("text") or v.get("content") or ""
                    if isinstance(text, list):
                        parts = []
                        for block in text:
                            if isinstance(block, dict) and block.get("type") == "text":
                                parts.append(block.get("text", ""))
                        return "".join(parts)
                    if isinstance(text, str):
                        return text
                return json.dumps(v, ensure_ascii=False)
            break
        break
    return s


def is_run_json(text: str) -> bool:
    if not text.startswith("{"):
        return False
    try:
        v = json.loads(text)
    except json.JSONDecodeError:
        return False
    return isinstance(v, dict) and ("run_id" in v or "id" in v) and "status" in v


def messages_to_ui_items(rows: list[sqlite3.Row]) -> list[dict]:
    tool_inputs: dict[str, str] = {}
    for row in rows:
        if row["role"] != "assistant" or not row["tool_calls"]:
            continue
        try:
            calls = json.loads(row["tool_calls"])
        except json.JSONDecodeError:
            continue
        for call in calls if isinstance(calls, list) else []:
            fn = call.get("function") or {}
            name = fn.get("name") or ""
            cid = call.get("id") or ""
            args_raw = fn.get("arguments") or "{}"
            try:
                args = json.loads(args_raw) if isinstance(args_raw, str) else args_raw
            except json.JSONDecodeError:
                continue
            val = None
            if name in ("python", "r"):
                val = args.get("code")
            elif name == "shell":
                val = args.get("cmd")
            elif name in ("monitor_run", "wisp_monitor_run"):
                val = args.get("run_id")
            if val and cid:
                tool_inputs[cid] = val

    items: list[dict] = []
    for row in rows:
        role = row["role"]
        text = unwrap_text(row["content"] or "")
        tool_name = row["tool_name"]
        tool_call_id = row["tool_call_id"]
        if role == "system":
            continue
        if role == "user":
            t = text.strip()
            if not t:
                continue
            items.append(
                {
                    "role": "user",
                    "text": t,
                    "tool_name": None,
                    "ok": None,
                    "input": None,
                    "model_name": None,
                    "resources": [],
                }
            )
            continue
        if role == "assistant":
            reasoning_raw = row["reasoning"]
            if reasoning_raw and str(reasoning_raw).strip():
                rtext = unwrap_text(str(reasoning_raw))
                if rtext.strip():
                    items.append(
                        {
                            "role": "reasoning",
                            "text": rtext,
                            "tool_name": None,
                            "ok": None,
                            "input": None,
                            "model_name": None,
                            "resources": [],
                        }
                    )
            if text.strip():
                items.append(
                    {
                        "role": "assistant",
                        "text": text,
                        "tool_name": None,
                        "ok": None,
                        "input": None,
                        "model_name": row["model_name"],
                        "resources": [],
                    }
                )
            continue
        if role == "tool":
            if tool_name == "attempt_completion":
                if text.strip():
                    items.append(
                        {
                            "role": "assistant",
                            "text": text,
                            "tool_name": None,
                            "ok": None,
                            "input": None,
                            "model_name": row["model_name"],
                            "resources": [],
                        }
                    )
                continue
            if tool_name in ("propose_plan", "update_plan", "Plan"):
                items.append(
                    {
                        "role": "plan",
                        "text": text,
                        "tool_name": None,
                        "ok": None,
                        "input": None,
                        "model_name": None,
                        "resources": [],
                    }
                )
                continue
            items.append(
                {
                    "role": "tool",
                    "text": text,
                    "tool_name": tool_name,
                    "ok": True,
                    "input": tool_call_id and tool_inputs.get(tool_call_id),
                    "model_name": None,
                    "resources": [],
                }
            )
    return items


def redact_item(item: dict) -> dict:
    role = item["role"]
    limit = MAX_REASONING if role == "reasoning" else MAX_TOOL_TEXT
    if role == "tool" and is_run_json(item.get("text") or ""):
        limit = 12000
    text = truncate(redact(item.get("text") or ""), limit)
    inp = item.get("input")
    if isinstance(inp, str):
        inp = truncate(redact(inp), 4000)
    out = dict(item)
    out["text"] = text
    out["input"] = inp
    # Demos present as a single model for a coherent research narrative.
    if out.get("model_name"):
        out["model_name"] = "deepseek-v4-pro"
    return out


def apply_request_override(items: list[dict], manifest_id: str) -> None:
    override = REQUEST_OVERRIDES.get(manifest_id)
    if not override:
        return
    for item in items:
        if item["role"] == "user":
            item["text"] = override
            break


def derive_summary(items: list[dict]) -> tuple[str, str, str | None]:
    request = next((i["text"] for i in items if i["role"] == "user"), "")
    response = ""
    thinking = None
    for i in reversed(items):
        if i["role"] == "assistant" and i["text"].strip():
            response = i["text"]
            break
    for i in items:
        if i["role"] == "reasoning" and i["text"].strip():
            thinking = i["text"]
            break
    return request, response, thinking


def assert_clean(blob: str, label: str) -> None:
    lowered = blob.lower()
    for bad in (
        "guotosky",
        "10.10.10.",
        "english reply",
        "english instruction",
        ":7897",
        "gz0548",
        "guozi-server",
        "http_proxy=",
        "https_proxy=",
        "all_proxy=",
        "proxy configured",
        "proxy settings",
        "use the proxy",
        "using the proxy",
        "via proxy",
        "with proxy",
        "without proxy",
        "proxy config",
        "proxy is configured",
        "handles the proxy",
        "kimi-k3",
    ):
        assert bad not in lowered, f"{label} still contains {bad}"
    # Demos must not advertise bashrc/proxy download setup at all.
    assert "bashrc" not in lowered, f"{label} still mentions bashrc"


def export_session(
    con: sqlite3.Connection, frame_id: str, manifest_id: str, demo_id: str
) -> Path:
    rows = list(
        con.execute(
            """
            SELECT seq, role, content, tool_calls, tool_call_id, tool_name, reasoning, model_name
            FROM messages
            WHERE frame_id=?
            ORDER BY seq
            """,
            (frame_id,),
        )
    )
    if not rows:
        raise SystemExit(f"no messages for frame {frame_id}")

    items = [redact_item(i) for i in messages_to_ui_items(rows)]
    apply_request_override(items, manifest_id)
    request, response, thinking = derive_summary(items)
    # Prefer override as request field too.
    request = REQUEST_OVERRIDES.get(manifest_id, request)

    manifest = {
        "root_frame": {
            "id": demo_id,
            "parent_frame_id": None,
            "root_frame_id": demo_id,
            "agent_name": "WISP",
            "status": "completed",
            "input_data": {"request": request},
            "output_data": {
                "response": response,
                "thinking": thinking,
                "items": items,
            },
        }
    }
    path = SEED / f"{manifest_id}.json"
    path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(
        "wrote",
        path.name,
        f"{path.stat().st_size/1024:.1f}KiB",
        "items",
        len(items),
        "tools",
        sum(1 for i in items if i["role"] == "tool"),
    )
    assert_clean(path.read_text(encoding="utf-8"), path.name)
    return path


def write_slim_deg(src: Path, dest: Path, n: int = 200) -> None:
    with src.open(encoding="utf-8", newline="") as f:
        reader = csv.DictReader(f)
        rows = list(reader)
    # Prefer significant rows when padj present.
    if rows and "padj" in rows[0]:
        def padj_key(r: dict) -> float:
            try:
                return float(r.get("padj") or "nan")
            except ValueError:
                return float("inf")

        rows = sorted(rows, key=padj_key)[:n]
    else:
        rows = rows[:n]
    dest.parent.mkdir(parents=True, exist_ok=True)
    with dest.open("w", encoding="utf-8", newline="") as f:
        writer = csv.DictWriter(f, fieldnames=list(rows[0].keys()) if rows else [])
        writer.writeheader()
        writer.writerows(rows)


def build_assets() -> None:
    # Upstream counts (shared with step 3).
    rnaseq_assets = [
        (
            "example_esr1_03_rnaseq/GSE153250_counts_matrix.tsv",
            SRC_UP / "data" / "processed" / "GSE153250_counts_matrix.tsv",
        ),
        (
            "example_esr1_03_rnaseq/GSE153250_sample_groups.txt",
            SRC_UP / "data" / "processed" / "GSE153250_sample_groups.txt",
        ),
        (
            "example_esr1_03_rnaseq/GSE153250_featureCounts_summary.txt",
            SRC_UP / "data" / "processed" / "GSE153250_featureCounts_summary.txt",
        ),
    ]
    tar_path = SEED / "assets_esr1_03_rnaseq.tar.gz"
    with tarfile.open(tar_path, "w:gz", compresslevel=9) as tar:
        for arcname, src in rnaseq_assets:
            if not src.is_file():
                raise SystemExit(f"missing asset: {src}")
            tar.add(src, arcname=arcname)
    print("wrote", tar_path.name, f"{tar_path.stat().st_size/1024:.1f}KiB")

    # Downstream: small figures + key tables + research report. Skip multi-MB PDFs/CSVs.
    slim_deg = SEED / "_tmp_DESeq2_top200.csv"
    write_slim_deg(SRC_DOWN / "results" / "tables" / "DESeq2_full_results.csv", slim_deg)

    down_assets: list[tuple[str, Path]] = [
        ("example_esr1_04_downstream/DESeq2_top200.csv", slim_deg),
        (
            "example_esr1_04_downstream/GSEA_MSigDB_Hallmark_2020.csv",
            SRC_DOWN / "results" / "tables" / "GSEA_MSigDB_Hallmark_2020.csv",
        ),
        (
            "example_esr1_04_downstream/ORA_up_MSigDB_Hallmark_2020.csv",
            SRC_DOWN / "results" / "tables" / "ORA_up_MSigDB_Hallmark_2020.csv",
        ),
        (
            "example_esr1_04_downstream/ORA_down_MSigDB_Hallmark_2020.csv",
            SRC_DOWN / "results" / "tables" / "ORA_down_MSigDB_Hallmark_2020.csv",
        ),
        (
            "example_esr1_04_downstream/ORA_up_KEGG_2026.csv",
            SRC_DOWN / "results" / "tables" / "ORA_up_KEGG_2026.csv",
        ),
        (
            "example_esr1_04_downstream/research_projects.md",
            SRC_DOWN / "results" / "reports" / "research_projects.md",
        ),
        (
            "example_esr1_04_downstream/PCA_plot.pdf",
            SRC_DOWN / "figures" / "PCA_plot.pdf",
        ),
        (
            "example_esr1_04_downstream/GSEA_dot_MSigDB_Hallmark_2020.pdf",
            SRC_DOWN / "figures" / "GSEA_dot_MSigDB_Hallmark_2020.pdf",
        ),
        (
            "example_esr1_04_downstream/ORA_bar_MSigDB_Hallmark_2020_up.pdf",
            SRC_DOWN / "figures" / "ORA_bar_MSigDB_Hallmark_2020_up.pdf",
        ),
        (
            "example_esr1_04_downstream/ORA_bar_MSigDB_Hallmark_2020_down.pdf",
            SRC_DOWN / "figures" / "ORA_bar_MSigDB_Hallmark_2020_down.pdf",
        ),
    ]
    tar_path = SEED / "assets_esr1_04_downstream.tar.gz"
    with tarfile.open(tar_path, "w:gz", compresslevel=9) as tar:
        for arcname, src in down_assets:
            if not src.is_file():
                raise SystemExit(f"missing asset: {src}")
            tar.add(src, arcname=arcname)
    print("wrote", tar_path.name, f"{tar_path.stat().st_size/1024:.1f}KiB")
    slim_deg.unlink(missing_ok=True)


def cleanup_old_seed_files() -> None:
    keep_manifests = {f"{m}.json" for _, m, _, _ in SESSIONS}
    # The memory demo is produced by export_memory_demo.py, not this script.
    keep_manifests.add("manifest_memory_01_long_context.json")
    for p in SEED.glob("manifest_*.json"):
        if p.name not in keep_manifests:
            p.unlink()
            print("removed", p.name)
    for p in SEED.glob("assets_*.tar.gz"):
        keep = {
            "assets_esr1_03_rnaseq.tar.gz",
            "assets_esr1_04_downstream.tar.gz",
        }
        if p.name not in keep:
            p.unlink()
            print("removed", p.name)


def main() -> None:
    if not DB.is_file():
        raise SystemExit(f"database not found: {DB}")
    SEED.mkdir(exist_ok=True)
    con = sqlite3.connect(f"file:{DB}?mode=ro", uri=True)
    con.row_factory = sqlite3.Row
    for frame_id, manifest_id, demo_id, _assets in SESSIONS:
        export_session(con, frame_id, manifest_id, demo_id)
    build_assets()
    cleanup_old_seed_files()
    total = sum(p.stat().st_size for p in SEED.iterdir() if p.is_file())
    print(f"seed total {total/1024:.1f} KiB ({total/1024/1024:.2f} MiB)")


if __name__ == "__main__":
    main()
