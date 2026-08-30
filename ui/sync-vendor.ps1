$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
& node (Join-Path $root "sync-vendor.mjs")
if ($LASTEXITCODE -ne 0) {
  throw "vendor runtime generation failed with exit code $LASTEXITCODE"
}
