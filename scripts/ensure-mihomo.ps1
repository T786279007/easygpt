param(
    [string]$Destination = "",
    [string]$AssetPattern = "mihomo-windows-amd64-v2-*.zip",
    [switch]$Force
)

$ErrorActionPreference = "Stop"

$ProjectRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($Destination)) {
    $Destination = Join-Path $ProjectRoot "resources\clash\mihomo.exe"
}

if ((Test-Path -LiteralPath $Destination) -and (-not $Force)) {
    Write-Host "mihomo already exists:"
    Write-Host $Destination
    return
}

$destinationDir = Split-Path -Parent $Destination
New-Item -ItemType Directory -Force -Path $destinationDir | Out-Null

$workDir = Join-Path $ProjectRoot "target\mihomo-download"
if (Test-Path -LiteralPath $workDir) {
    Remove-Item -LiteralPath $workDir -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $workDir | Out-Null

$releaseApi = "https://api.github.com/repos/MetaCubeX/mihomo/releases/latest"
$release = Invoke-RestMethod -Uri $releaseApi -Headers @{ "User-Agent" = "easygpt-packager" }
$asset = $release.assets | Where-Object { $_.name -like $AssetPattern } | Select-Object -First 1

if (-not $asset) {
    $assetNames = ($release.assets | ForEach-Object { $_.name }) -join ", "
    throw "Could not find mihomo release asset matching '$AssetPattern'. Available assets: $assetNames"
}

$archivePath = Join-Path $workDir $asset.name
Write-Host "Downloading mihomo:"
Write-Host $asset.browser_download_url
Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $archivePath -Headers @{ "User-Agent" = "easygpt-packager" }

$extractDir = Join-Path $workDir "extract"
New-Item -ItemType Directory -Force -Path $extractDir | Out-Null
Expand-Archive -LiteralPath $archivePath -DestinationPath $extractDir -Force

$mihomoExe = Get-ChildItem -LiteralPath $extractDir -Filter "mihomo*.exe" -Recurse | Select-Object -First 1
if (-not $mihomoExe) {
    throw "Downloaded mihomo archive did not contain a mihomo executable."
}

Copy-Item -LiteralPath $mihomoExe.FullName -Destination $Destination -Force
Write-Host "mihomo prepared:"
Write-Host $Destination
