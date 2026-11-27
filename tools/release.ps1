<#
.SYNOPSIS
    Stage release assets and generate SHA256SUMS in the format the auto-updater
    requires.

.DESCRIPTION
    The auto-updater parses SHA256SUMS as "<64-hex>  <name>" per line. PowerShell's
    Out-File writes CRLF, and an inverted "<name> <hash>" order silently breaks
    every install, so this script writes LF-only, hash-first lines and verifies
    each file's digest before handing anything to `gh release upload`.
#>
param(
    [Parameter(Mandatory = $true)]
    [string]$Version,
    [string]$Repo = 'akagikay/Harness',
    [string]$StageDir = "$env:TEMP\harness-release-$Version"
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$assets = @('harness.exe', 'harness-client.exe', 'harness-server.exe')

New-Item -ItemType Directory -Force -Path $StageDir | Out-Null

foreach ($name in $assets) {
    $src = Join-Path $root "target\release\$name"
    if (-not (Test-Path $src)) {
        throw "Missing release binary: $src (run cargo build --release first)"
    }
    Copy-Item $src (Join-Path $StageDir $name) -Force
}

$lines = foreach ($name in $assets) {
    $hash = (Get-FileHash (Join-Path $StageDir $name) -Algorithm SHA256).Hash.ToLower()
    # LF-only, hash first, two spaces (sha256sum convention).
    "$hash  $name`n"
}
[System.IO.File]::WriteAllText((Join-Path $StageDir 'SHA256SUMS'), ($lines -join ''), (New-Object System.Text.UTF8Encoding($false)))

# Self-check: parse the file back the way the updater does.
$raw = [System.IO.File]::ReadAllBytes((Join-Path $StageDir 'SHA256SUMS'))
if (($raw | Where-Object { $_ -eq 13 }).Count -ne 0) { throw 'SHA256SUMS contains CR bytes' }
$text = [System.Text.Encoding]::UTF8.GetString($raw)
foreach ($line in ($text -split "`n")) {
    if ([string]::IsNullOrWhiteSpace($line)) { continue }
    $parts = $line.Trim() -split '\s+'
    if ($parts.Count -ne 2) { throw "Malformed SHA256SUMS line: $line" }
    if ($parts[0] -notmatch '^[0-9a-f]{64}$') { throw "Not a sha256 digest: $($parts[0])" }
    $actual = (Get-FileHash (Join-Path $StageDir $parts[1]) -Algorithm SHA256).Hash.ToLower()
    if ($actual -ne $parts[0]) { throw "Digest mismatch for $($parts[1])" }
}
Write-Host "SHA256SUMS verified for $Version"
Get-Content (Join-Path $StageDir 'SHA256SUMS')

if (Get-Command gh -ErrorAction SilentlyContinue) {
    gh release create "v$Version" "$StageDir\harness.exe" "$StageDir\harness-client.exe" "$StageDir\harness-server.exe" "$StageDir\SHA256SUMS" --repo $Repo --title "Harness v$Version" --notes-file (Join-Path $StageDir 'RELEASE-NOTES.md')
} else {
    Write-Host "gh not found - upload $StageDir manually"
}