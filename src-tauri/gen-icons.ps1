$ErrorActionPreference = "Stop"
Push-Location $PSScriptRoot
try {
    # Two masters, one mark. macOS Dock/Launchpad mask a full-bleed tile;
    # Windows taskbar and Linux docks draw the bitmap as-is, so they need
    # baked rounded corners. Never pass ui/logo.svg — that in-app mark fills
    # the canvas and would look oversized on every launcher.
    $fullBleed = Resolve-Path "icons/app-icon.svg"
    $rounded = Resolve-Path "icons/app-icon-rounded.svg"
    $ico = Join-Path $PSScriptRoot "icons/icon.ico"
    $icns = Join-Path $PSScriptRoot "icons/icon.icns"
    $need = -not (Test-Path $ico) -or -not (Test-Path $icns)
    if (-not $need) {
        $fullTime = (Get-Item $fullBleed).LastWriteTimeUtc
        $roundTime = (Get-Item $rounded).LastWriteTimeUtc
        $need = $fullTime -gt (Get-Item $icns).LastWriteTimeUtc -or
            $roundTime -gt (Get-Item $ico).LastWriteTimeUtc
    }
    if (-not $need) {
        return
    }

    $fullOut = Join-Path $PSScriptRoot "icons/.gen-full"
    $roundOut = Join-Path $PSScriptRoot "icons/.gen-rounded"
    Remove-Item $fullOut, $roundOut -Recurse -Force -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Path $fullOut | Out-Null
    New-Item -ItemType Directory -Path $roundOut | Out-Null
    cargo tauri icon $fullBleed -o $fullOut
    cargo tauri icon $rounded -o $roundOut

    Copy-Item -Path (Join-Path $fullOut "*") -Destination "icons" -Recurse -Force
    foreach ($name in @(
        "icon.ico",
        "icon.png",
        "32x32.png",
        "64x64.png",
        "128x128.png",
        "128x128@2x.png"
    )) {
        Copy-Item -Force (Join-Path $roundOut $name) (Join-Path "icons" $name)
    }
    Remove-Item $fullOut, $roundOut -Recurse -Force
} finally {
    Pop-Location
}
