<#
.SYNOPSIS
Compares bundled WinDivert files against an official WinDivert directory.

.DESCRIPTION
The script compares SHA-256 hashes, Authenticode signature status, and version
metadata for WinDivert.dll and WinDivert64.sys. It can also write a Markdown
report that can be attached to a release checklist.

.EXAMPLE
powershell -ExecutionPolicy Bypass -File .\scripts\verify-windivert.ps1 `
  -OfficialDir C:\Downloads\WinDivert-2.2.2-A `
  -ReportPath docs\security-check.generated.md
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$OfficialDir,
    [string]$RepoRoot,
    [string]$LocalDllPath,
    [string]$LocalSysPath,
    [string]$ReportPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptDir = if ($PSScriptRoot) { $PSScriptRoot } else { Split-Path -Parent $PSCommandPath }
if (-not $RepoRoot) {
    $RepoRoot = (Resolve-Path (Join-Path $scriptDir "..")).Path
}
if (-not $LocalDllPath) {
    $LocalDllPath = Join-Path $RepoRoot "src-tauri\WinDivert.dll"
}
if (-not $LocalSysPath) {
    $LocalSysPath = Join-Path $RepoRoot "src-tauri\WinDivert64.sys"
}

function Get-FileSummary {
    param(
        [string]$Name,
        [string]$LocalPath,
        [string]$OfficialPath
    )

    if (-not (Test-Path $LocalPath)) {
        throw "Local file not found: $LocalPath"
    }
    if (-not (Test-Path $OfficialPath)) {
        throw "Official file not found: $OfficialPath"
    }

    $localHash = (Get-FileHash $LocalPath -Algorithm SHA256).Hash
    $officialHash = (Get-FileHash $OfficialPath -Algorithm SHA256).Hash
    $localSignature = Get-AuthenticodeSignature $LocalPath
    $officialSignature = Get-AuthenticodeSignature $OfficialPath
    $localVersion = [System.Diagnostics.FileVersionInfo]::GetVersionInfo($LocalPath).ProductVersion
    $officialVersion = [System.Diagnostics.FileVersionInfo]::GetVersionInfo($OfficialPath).ProductVersion

    [pscustomobject]@{
        Name = $Name
        LocalPath = (Resolve-Path $LocalPath).Path
        OfficialPath = (Resolve-Path $OfficialPath).Path
        LocalSha256 = $localHash
        OfficialSha256 = $officialHash
        Match = ($localHash -eq $officialHash)
        LocalSignature = $localSignature.Status
        OfficialSignature = $officialSignature.Status
        LocalVersion = $localVersion
        OfficialVersion = $officialVersion
    }
}

$resolvedOfficialDir = (Resolve-Path $OfficialDir).Path
$reportItems = @(
    Get-FileSummary -Name "WinDivert.dll" -LocalPath $LocalDllPath -OfficialPath (Join-Path $resolvedOfficialDir "WinDivert.dll")
    Get-FileSummary -Name "WinDivert64.sys" -LocalPath $LocalSysPath -OfficialPath (Join-Path $resolvedOfficialDir "WinDivert64.sys")
)

$reportItems | Format-Table -AutoSize

$hasMismatch = @($reportItems | Where-Object { -not $_.Match }).Count -gt 0

if ($ReportPath) {
    if ([System.IO.Path]::IsPathRooted($ReportPath)) {
        $resolvedReportPath = $ReportPath
    } else {
        $resolvedReportPath = Join-Path $RepoRoot $ReportPath
    }

    $reportDir = Split-Path -Parent $resolvedReportPath
    if ($reportDir) {
        New-Item -ItemType Directory -Force -Path $reportDir | Out-Null
    }

    $lines = New-Object System.Collections.Generic.List[string]
    $lines.Add("# WinDivert Verification Report")
    $lines.Add("")
    $lines.Add("- Date: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss zzz')")
    $lines.Add("- Official source directory: $resolvedOfficialDir")
    $lines.Add("")

    foreach ($item in $reportItems) {
        $lines.Add("## $($item.Name)")
        $lines.Add("")
        $lines.Add("- Local path: $($item.LocalPath)")
        $lines.Add("- Official path: $($item.OfficialPath)")
        $lines.Add("- Local SHA-256: $($item.LocalSha256)")
        $lines.Add("- Official SHA-256: $($item.OfficialSha256)")
        $lines.Add("- Hash match: $($item.Match)")
        $lines.Add("- Local signature: $($item.LocalSignature)")
        $lines.Add("- Official signature: $($item.OfficialSignature)")
        $lines.Add("- Local version: $($item.LocalVersion)")
        $lines.Add("- Official version: $($item.OfficialVersion)")
        $lines.Add("")
    }

    $lines.Add("## Result")
    $lines.Add("")
    $lines.Add("- Overall status: $(if ($hasMismatch) { "FAIL" } else { "PASS" })")
    $lines.Add("")

    Set-Content -Path $resolvedReportPath -Value $lines -Encoding UTF8
    Write-Host ""
    Write-Host "Report written to: $resolvedReportPath"
}

if ($hasMismatch) {
    Write-Error "Hash mismatch detected between local and official WinDivert files."
    exit 1
}

Write-Host ""
Write-Host "Local WinDivert files match the provided official files."
