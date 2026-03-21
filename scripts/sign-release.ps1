<#
.SYNOPSIS
Signs the release EXE and installer, then verifies the signatures.

.DESCRIPTION
Use a PFX file or a certificate thumbprint from the Windows certificate store.
The script signs each target with a SHA-256 digest and RFC 3161 timestamp, then
verifies the signature with signtool and Get-AuthenticodeSignature.

.EXAMPLE
powershell -ExecutionPolicy Bypass -File .\scripts\sign-release.ps1 `
  -TimestampUrl https://timestamp.digicert.com `
  -CertificatePath C:\secure\codesign.pfx

.EXAMPLE
powershell -ExecutionPolicy Bypass -File .\scripts\sign-release.ps1 `
  -TimestampUrl https://timestamp.digicert.com `
  -Thumbprint ABCD1234EF567890ABCD1234EF567890ABCD1234
#>
[CmdletBinding(DefaultParameterSetName = "Pfx")]
param(
    [string]$RepoRoot,
    [string]$SignToolPath,
    [Parameter(Mandatory = $true)]
    [string]$TimestampUrl,
    [string[]]$Targets,

    [Parameter(ParameterSetName = "Pfx", Mandatory = $true)]
    [string]$CertificatePath,
    [Parameter(ParameterSetName = "Pfx")]
    [string]$CertificatePassword,

    [Parameter(ParameterSetName = "Store", Mandatory = $true)]
    [string]$Thumbprint,
    [Parameter(ParameterSetName = "Store")]
    [switch]$UseMachineStore
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptDir = if ($PSScriptRoot) { $PSScriptRoot } else { Split-Path -Parent $PSCommandPath }
if (-not $RepoRoot) {
    $RepoRoot = (Resolve-Path (Join-Path $scriptDir "..")).Path
}

function Resolve-SignTool {
    param([string]$Candidate)

    if ($Candidate) {
        if (-not (Test-Path $Candidate)) {
            throw "SignTool not found at: $Candidate"
        }
        return (Resolve-Path $Candidate).Path
    }

    $cmd = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if ($cmd) {
        return $cmd.Source
    }

    $roots = @(
        "C:\Program Files (x86)\Windows Kits\10\bin",
        "C:\Program Files\Windows Kits\10\bin"
    )

    foreach ($root in $roots) {
        if (-not (Test-Path $root)) {
            continue
        }

        $match = Get-ChildItem $root -Directory -ErrorAction SilentlyContinue |
            Sort-Object Name -Descending |
            ForEach-Object {
                $candidatePath = Join-Path $_.FullName "x64\signtool.exe"
                if (Test-Path $candidatePath) {
                    return $candidatePath
                }
            }

        if ($match) {
            return $match
        }
    }

    throw "signtool.exe was not found. Install the Windows SDK or pass -SignToolPath."
}

function Resolve-Targets {
    param(
        [string]$Root,
        [string[]]$UserTargets
    )

    if ($UserTargets -and $UserTargets.Count -gt 0) {
        return $UserTargets | ForEach-Object {
            if (-not (Test-Path $_)) {
                throw "Signing target not found: $_"
            }
            (Resolve-Path $_).Path
        }
    }

    $defaultTargets = New-Object System.Collections.Generic.List[string]

    $exePath = Join-Path $Root "src-tauri\target\release\resonance-chat.exe"
    if (-not (Test-Path $exePath)) {
        throw "Default signing target not found: $exePath"
    }
    $defaultTargets.Add($exePath)

    $nsisDir = Join-Path $Root "src-tauri\target\release\bundle\nsis"
    if (-not (Test-Path $nsisDir)) {
        throw "NSIS bundle directory not found: $nsisDir"
    }

    $installer = Get-ChildItem -Path $nsisDir -Filter "resonance-chat_*_x64-setup.exe" -File -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1

    if (-not $installer) {
        throw "Default signing target not found under: $nsisDir"
    }

    $defaultTargets.Add($installer.FullName)

    return $defaultTargets
}

function Invoke-SignTool {
    param(
        [string]$ToolPath,
        [string[]]$Arguments
    )

    & $ToolPath @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "signtool failed with exit code $LASTEXITCODE"
    }
}

$signTool = Resolve-SignTool -Candidate $SignToolPath
$resolvedTargets = Resolve-Targets -Root $RepoRoot -UserTargets $Targets

$results = @()

foreach ($target in $resolvedTargets) {
    Write-Host "Signing: $target"

    $args = @(
        "sign",
        "/fd", "SHA256",
        "/td", "SHA256",
        "/tr", $TimestampUrl
    )

    if ($PSCmdlet.ParameterSetName -eq "Pfx") {
        $resolvedCertificatePath = (Resolve-Path $CertificatePath).Path
        $args += @("/f", $resolvedCertificatePath)
        if ($CertificatePassword) {
            $args += @("/p", $CertificatePassword)
        }
    } else {
        $normalizedThumbprint = $Thumbprint.Replace(" ", "")
        $args += @("/sha1", $normalizedThumbprint)
        if ($UseMachineStore) {
            $args += "/sm"
        }
    }

    $args += $target
    Invoke-SignTool -ToolPath $signTool -Arguments $args
    Invoke-SignTool -ToolPath $signTool -Arguments @("verify", "/pa", "/v", $target)

    $signature = Get-AuthenticodeSignature $target
    if ($signature.Status -ne "Valid") {
        throw "Signature verification failed for $target. Status: $($signature.Status)"
    }

    $hash = (Get-FileHash $target -Algorithm SHA256).Hash
    $results += [pscustomobject]@{
        Target = $target
        SignatureStatus = $signature.Status
        Signer = if ($signature.SignerCertificate) { $signature.SignerCertificate.Subject } else { "" }
        Sha256 = $hash
    }
}

Write-Host ""
Write-Host "Signing completed successfully."
$results | Format-Table -AutoSize
