#!/usr/bin/env bash
# 构建本地工作流 Worker（WISP fork）并产出交付清单（发布 SBOM 输入）。
#
# 用法：scripts/build_workflow_worker.sh
# 产物：
#   target/release/wisp-science(.exe)                    —— Supervisor 的 dev 回退路径
#   target/release/worker-build.json                     —— 版本/校验和清单
set -euo pipefail

# WSL/Git Bash can expose both a Linux rustup proxy and the Windows toolchain.
# Prefer cargo.exe whenever it is available so a Windows release cannot
# silently build/download the wrong target toolchain.
WINDOWS_CARGO="$(command -v cargo.exe 2>/dev/null || true)"
if [[ -z "$WINDOWS_CARGO" ]]; then
  # Non-login WSL does not always inherit Windows PATH. Resolve the current
  # Windows profile without assuming a username.
  for _cmd in /mnt/c/Windows/System32/cmd.exe /c/Windows/System32/cmd.exe; do
    [[ -x "$_cmd" ]] || continue
    _win_home="$("$_cmd" /d /c 'echo %USERPROFILE%' 2>/dev/null | tr -d '\r')"
    if command -v wslpath >/dev/null 2>&1; then
      _unix_home="$(wslpath -u "$_win_home")"
    elif command -v cygpath >/dev/null 2>&1; then
      _unix_home="$(cygpath -u "$_win_home")"
    else
      _unix_home=""
    fi
    [[ -x "$_unix_home/.cargo/bin/cargo.exe" ]] && WINDOWS_CARGO="$_unix_home/.cargo/bin/cargo.exe"
    [[ -n "$WINDOWS_CARGO" ]] && break
  done
fi

if [[ -n "$WINDOWS_CARGO" ]]; then
  CARGO_BIN="$WINDOWS_CARGO"
  # A generic `channel = "stable"` can resolve to the GNU host when the
  # repository is invoked from WSL/Git Bash.  Tauri's Windows loader and the
  # release CI use MSVC, so make the release worker target explicit.
  export RUSTUP_TOOLCHAIN="stable-x86_64-pc-windows-msvc"
  # WSL passes only variables listed in WSLENV to Windows executables.
  case ":${WSLENV:-}:" in
    *:RUSTUP_TOOLCHAIN:*) ;;
    *) export WSLENV="${WSLENV:+$WSLENV:}RUSTUP_TOOLCHAIN" ;;
  esac
elif command -v cargo >/dev/null 2>&1; then
  CARGO_BIN="cargo"
else
  echo "cargo not found on PATH" >&2
  exit 1
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
TOOLCHAIN_HOST="$("$CARGO_BIN" -vV | awk -F': ' '/^host:/ { gsub(/\r/, "", $2); print $2; exit }')"
if [[ "$CARGO_BIN" == *.exe && "$TOOLCHAIN_HOST" != "x86_64-pc-windows-msvc" ]]; then
  echo "refusing Windows release with non-MSVC Rust host: $TOOLCHAIN_HOST" >&2
  exit 1
fi
"$CARGO_BIN" build --release -p wisp-cli

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
  "toolchain_host": "$TOOLCHAIN_HOST",
  "built_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
EOF2

echo "worker built: $OUT"
echo "sha256: $SHA256"
cat "$ROOT/target/release/worker-build.json"
