param(
    [string]$TargetDir = "target_installer",
    [string]$PackageName = "EasyGPT",
    [string]$AppVersion = "",
    [switch]$SkipBuild,
    [switch]$IncludeCurrentData
)

$ErrorActionPreference = "Stop"

function Find-InnoSetupCompiler {
    $commands = @("iscc.exe", "ISCC.exe", "iscc")
    foreach ($command in $commands) {
        $resolved = Get-Command $command -ErrorAction SilentlyContinue
        if ($resolved) {
            return $resolved.Source
        }
    }

    $candidatePaths = @(
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
    )

    foreach ($path in $candidatePaths) {
        if ($path -and (Test-Path -LiteralPath $path)) {
            return $path
        }
    }

    return $null
}

function Read-CargoVersion {
    param([string]$CargoTomlPath)

    $versionLine = Select-String -LiteralPath $CargoTomlPath -Pattern '^\s*version\s*=\s*"([^"]+)"' | Select-Object -First 1
    if ($versionLine -and $versionLine.Matches.Count -gt 0) {
        return $versionLine.Matches[0].Groups[1].Value
    }

    return "0.1.2"
}

$ProjectRoot = Split-Path -Parent $PSScriptRoot
$InstallerScript = Join-Path $ProjectRoot "installer\ChatGPTWebviewClient.iss"
$PackageTargetDir = Join-Path $TargetDir "installer_staging"
$PackageDir = Join-Path $ProjectRoot "$PackageTargetDir\portable\$PackageName"
$InstallerOutputDir = Join-Path $ProjectRoot "$TargetDir\installer"
$PortableDataDir = Join-Path $PackageDir "data"
$SourceDataDir = Join-Path $ProjectRoot "data"

Push-Location $ProjectRoot
try {
    if ([string]::IsNullOrWhiteSpace($AppVersion)) {
        $AppVersion = Read-CargoVersion -CargoTomlPath (Join-Path $ProjectRoot "Cargo.toml")
    }

    if (-not $SkipBuild) {
        & (Join-Path $PSScriptRoot "package-portable.ps1") -TargetDir $PackageTargetDir -PackageName $PackageName -ExeName "EasyGPT.exe"
    }

    if (-not (Test-Path -LiteralPath $PackageDir)) {
        throw "Portable package directory not found: $PackageDir"
    }

    if ((-not $IncludeCurrentData) -and (Test-Path -LiteralPath $PortableDataDir)) {
        Remove-Item -LiteralPath $PortableDataDir -Recurse -Force
    }

    if ($IncludeCurrentData -and (Test-Path -LiteralPath $SourceDataDir)) {
        if (Test-Path -LiteralPath $PortableDataDir) {
            Remove-Item -LiteralPath $PortableDataDir -Recurse -Force
        }
        Copy-Item -LiteralPath $SourceDataDir -Destination $PortableDataDir -Recurse -Force
    }

    New-Item -ItemType Directory -Force -Path $InstallerOutputDir | Out-Null

    $iscc = Find-InnoSetupCompiler
    if (-not $iscc) {
        Write-Warning "Inno Setup compiler was not found. Install Inno Setup 6, then run this script again to build the .exe installer."
        Write-Host "Installer script prepared:"
        Write-Host $InstallerScript
        Write-Host "Portable source prepared:"
        Write-Host $PackageDir
        return
    }

    $env:APP_VERSION = $AppVersion
    $env:PACKAGE_SOURCE_DIR = $PackageDir
    $env:INSTALLER_OUTPUT_DIR = $InstallerOutputDir

    & $iscc $InstallerScript

    if ($LASTEXITCODE -ne 0) {
        throw "Inno Setup compiler failed with exit code $LASTEXITCODE"
    }

    $installerPath = Join-Path $InstallerOutputDir "EasyGPT-windows-x64-Setup-$AppVersion.exe"
    if (-not (Test-Path -LiteralPath $installerPath)) {
        throw "Installer was not created at expected path: $installerPath"
    }

    Write-Host "Installer package created:"
    Write-Host $installerPath
}
finally {
    Remove-Item Env:\APP_VERSION -ErrorAction SilentlyContinue
    Remove-Item Env:\PACKAGE_SOURCE_DIR -ErrorAction SilentlyContinue
    Remove-Item Env:\INSTALLER_OUTPUT_DIR -ErrorAction SilentlyContinue
    Pop-Location
}
