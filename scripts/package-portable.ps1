param(
    [string]$TargetDir = "target_portable",
    [string]$PackageName = "EasyGPT",
    [string]$ExeName = "$PackageName.exe"
)

$ErrorActionPreference = "Stop"

$ProjectRoot = Split-Path -Parent $PSScriptRoot
$ReleaseDir = Join-Path $ProjectRoot "$TargetDir\release"
$PackageDir = Join-Path $ProjectRoot "$TargetDir\portable\$PackageName"
$PreservedDataDir = Join-Path $ProjectRoot "$TargetDir\.preserved-$PackageName-data"

Push-Location $ProjectRoot
try {
    $env:CARGO_TARGET_DIR = $TargetDir
    cargo build --release

    & (Join-Path $PSScriptRoot "ensure-mihomo.ps1")

    if (Test-Path -LiteralPath $PreservedDataDir) {
        Remove-Item -LiteralPath $PreservedDataDir -Recurse -Force
    }

    $ExistingDataDir = Join-Path $PackageDir "data"
    if (Test-Path -LiteralPath $ExistingDataDir) {
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $PreservedDataDir) | Out-Null
        Move-Item -LiteralPath $ExistingDataDir -Destination $PreservedDataDir -Force
    }

    if (Test-Path -LiteralPath $PackageDir) {
        Remove-Item -LiteralPath $PackageDir -Recurse -Force
    }

    New-Item -ItemType Directory -Force -Path (Join-Path $PackageDir "resources\clash") | Out-Null

    Copy-Item -LiteralPath (Join-Path $ReleaseDir "chatgpt_webview_client.exe") `
        -Destination (Join-Path $PackageDir $ExeName) -Force
    Copy-Item -LiteralPath (Join-Path $ProjectRoot "resources\clash\mihomo.exe") `
        -Destination (Join-Path $PackageDir "resources\clash\mihomo.exe") -Force
    Copy-Item -LiteralPath (Join-Path $ProjectRoot "THIRD_PARTY_NOTICES.txt") `
        -Destination (Join-Path $PackageDir "THIRD_PARTY_NOTICES.txt") -Force
    Copy-Item -LiteralPath (Join-Path $ProjectRoot "README.md") `
        -Destination (Join-Path $PackageDir "README.md") -Force

    if (Test-Path -LiteralPath $PreservedDataDir) {
        Move-Item -LiteralPath $PreservedDataDir -Destination (Join-Path $PackageDir "data") -Force
    }

    Write-Host "Portable package created:"
    Write-Host $PackageDir
}
finally {
    Pop-Location
}
