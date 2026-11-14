# Build a Windows release of Manim Studio.
#
#   script\bundle-windows.ps1                 # portable zip + update manifest in dist\
#   script\bundle-windows.ps1 -Installer      # also the Inno Setup installer (needs ISCC)
#
# Updates: the build verifies updates with the public key in packaging\update-key.pub
# (override with STUDIO_UPDATE_PUBKEY). To sign the manifest set MINISIGN_KEY (path to the
# secret key) or MINISIGN_SECRET (its contents, as CI does). The zip goes on the vX.Y.Z
# release and manifest-<channel>.json(.minisig) on the "updates" release.
param(
    [switch]$Installer,
    [string]$Channel = "stable",
    [string]$Notes = "",
    [string]$BaseUrl = "https://github.com/ItsTatsuya/manim-studio/releases/download"
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
Write-Host "Manim Studio $version ($Channel)"
if (-not $env:STUDIO_UPDATE_PUBKEY -and (Test-Path packaging\update-key.pub)) {
    $env:STUDIO_UPDATE_PUBKEY = (Get-Content packaging\update-key.pub)[1].Trim()
}
if (-not $env:STUDIO_UPDATE_PUBKEY) { Write-Warning "STUDIO_UPDATE_PUBKEY is not set: this build won't update itself." }

cargo build --release -p studio-app --bins
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

$dist = Join-Path $root "dist"
New-Item -ItemType Directory -Force $dist | Out-Null
$stage = Join-Path $dist "stage"
Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item target\release\manim-studio.exe, target\release\studio-engine.exe $stage

$zipName = "ManimStudio-$version-windows-x86_64.zip"
$zip = Join-Path $dist $zipName
Remove-Item $zip -ErrorAction SilentlyContinue
Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $zip
$sha = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
$size = (Get-Item $zip).Length

$manifest = [ordered]@{
    version = $version
    notes   = $Notes
    assets  = [ordered]@{
        "windows-x86_64" = [ordered]@{ url = "$BaseUrl/v$version/$zipName"; sha256 = $sha; size = $size }
    }
}
$manifestPath = Join-Path $dist "manifest-$Channel.json"
$manifest | ConvertTo-Json -Depth 5 | Set-Content -Encoding utf8NoBOM $manifestPath
Write-Host "zip:      $zip ($size bytes, sha256 $sha)"
Write-Host "manifest: $manifestPath"

if ($env:MINISIGN_KEY -or $env:MINISIGN_SECRET) {
    cargo run -q --release -p release-sign -- sign $manifestPath
    if ($LASTEXITCODE -ne 0) { throw "signing failed" }
} else {
    Write-Warning "MINISIGN_KEY / MINISIGN_SECRET is not set: the manifest is unsigned (apps will refuse it)."
}

if ($Installer) {
    $iscc = $env:ISCC
    if (-not $iscc) { $iscc = (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source }
    if (-not $iscc) {
        # winget and choco record the install folder in the uninstall keys.
        $keys = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*",
                "HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*",
                "HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*"
        $inno = Get-ItemProperty $keys -ErrorAction SilentlyContinue |
            Where-Object { $_.DisplayName -like "Inno Setup version 6*" } | Select-Object -First 1
        if ($inno) { $iscc = Join-Path $inno.InstallLocation "ISCC.exe" }
    }
    if (-not $iscc) { $iscc = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe" }
    if (-not (Test-Path $iscc)) { throw "Inno Setup not found (winget install JRSoftware.InnoSetup)" }
    & $iscc "/DAppVersion=$version" packaging\windows\manim-studio.iss
    if ($LASTEXITCODE -ne 0) { throw "ISCC failed" }
}
Remove-Item -Recurse -Force $stage
