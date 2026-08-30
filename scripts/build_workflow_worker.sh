#!/usr/bin/env bash
# 构建本地工作流 Worker（WISP fork）并产出交付清单（发布 SBOM 输入）。
#
# 用法：scripts/build_workflow_worker.sh
# 产物：
#   target/release/wisp-science(.exe)                    —— Supervisor 的 dev 回退路径
#   target/release/worker-build.json                     —— 版本/校验和清单
set -euo pipefail

# Git Bash 默认 PATH 不含 cargo；显式补齐
for _cargo_dir in "$USERPROFILE/.cargo/bin" "$HOME/.cargo/bin"; do
  [[ -d "$_cargo_dir" ]] && export PATH="$_cargo_dir:$PATH"
done

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
cargo build --release -p wisp-cli

EXE="$ROOT/target/release/wisp-science.exe"
EXE_UNIX="$ROOT/target/release/wisp-science"
if [[ -f "$EXE" ]]; then OUT="$EXE"; elif [[ -f "$EXE_UNIX" ]]; then OUT="$EXE_UNIX"; else echo "worker binary not found" >&2; exit 1; fi

SHA256="$(sha256sum "$OUT" | awk '{print $1}')"
VERSION="$("$OUT" --version 2>/dev/null | head -1 || echo "1.8.0")"
COMMIT="$(git rev-parse HEAD)"

cat > "$ROOT/target/release/worker-build.json" <<EOF2
{
  "worker": "rice-workflow-worker",
  "engine": "wisp",
  "engine_version": "$VERSION",
  "fork_commit": "$COMMIT",
  "fork_repo": "https://github.com/yanmengli123/rice-endosperm-workflow",
  "upstream": "https://github.com/xuzhougeng/wisp-science",
  "binary": "$(basename "$OUT")",
  "sha256": "$SHA256",
  "protocol": "wisp.agent-rpc.v1",
  "built_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
EOF2

echo "worker built: $OUT"
echo "sha256: $SHA256"
cat "$ROOT/target/release/worker-build.json"
