<#
.SYNOPSIS
  Installs the Swift toolchain pinned in swift-windows-version.txt on a GitHub-hosted
  Windows runner (windows-latest = x64, windows-11-arm = arm64) and exports it to the
  following steps through $GITHUB_PATH and $GITHUB_ENV.

.DESCRIPTION
  No third-party action and no winget: windows-11-arm images have no winget, so both
  architectures use the same route. The installer is downloaded from download.swift.org,
  its SHA-256 is compared with the value pinned in swift-windows-version.txt, its
  Authenticode signature must be valid and Apple's, and it is run silently (per-user
  install under %LOCALAPPDATA%\Programs\Swift). The runner images already provide
  Visual Studio 2026 with the MSVC x64 and ARM64 tools and the Windows SDK, which the
  toolchain links against, so nothing else is installed.

  To move to a new version: put the version and the installer's SHA-256 for both
  architectures in swift-windows-version.txt. swift.org publishes no checksum file for
  the Windows installers; cross-check the hashes against the winget-pkgs manifest
  (manifests/s/Swift/Toolchain/<version>/Swift.Toolchain.installer.yaml).

  Exits 1 on any failure.
#>
[CmdletBinding()]
param(
    [string] $VersionFile = (Join-Path $PSScriptRoot 'swift-windows-version.txt')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

function Write-Step([string] $Message) { Write-Host "==> $Message" }

# Runs a native command with its output shown, returns its exit code. Stderr text must not
# become a terminating error, so the preference is relaxed around the call only.
function Invoke-Native([string] $Exe, [string[]] $Arguments) {
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & $Exe @Arguments 2>&1 | ForEach-Object { Write-Host "$_" }
        return $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previous
    }
}

# Runs $Action up to three times, pausing 15 s and 30 s between attempts.
function Invoke-WithRetry([string] $What, [scriptblock] $Action) {
    $pauses = @(15, 30)
    for ($attempt = 1; $attempt -le 3; $attempt++) {
        try {
            & $Action
            return
        } catch {
            Write-Host "$What failed (attempt $attempt of 3): $($_.Exception.Message)"
            if ($attempt -eq 3) { throw "$What failed after 3 attempts: $($_.Exception.Message)" }
            Write-Host "Retrying in $($pauses[$attempt - 1]) seconds."
            Start-Sleep -Seconds $pauses[$attempt - 1]
        }
    }
}

try {
    # 1. The pinned version and hashes.
    Write-Step "Reading $VersionFile"
    if (-not (Test-Path -LiteralPath $VersionFile)) { throw "Version file not found: $VersionFile" }
    $version = $null
    $hashes = @{}
    foreach ($raw in Get-Content -LiteralPath $VersionFile) {
        $line = $raw.Trim()
        if ($line -eq '' -or $line.StartsWith('#')) { continue }
        if ($line -match '^(x64|arm64)\s+([0-9a-fA-F]{64})$') {
            $hashes[$Matches[1]] = $Matches[2].ToLowerInvariant()
        } elseif ($line -match '^\d+\.\d+(\.\d+)?$' -and $null -eq $version) {
            $version = $line
        } else {
            throw "Unrecognised line in ${VersionFile}: '$line'"
        }
    }
    if ($null -eq $version) { throw "No version in $VersionFile" }

    # 2. The architecture of this runner.
    $osArchitecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
    switch ($osArchitecture) {
        'X64'   { $arch = 'x64';   $platformDir = 'windows10' }
        'Arm64' { $arch = 'arm64'; $platformDir = 'windows10-arm64' }
        default { throw "Unsupported architecture: $osArchitecture (need X64 or Arm64)" }
    }
    if (-not $hashes.ContainsKey($arch)) { throw "No SHA-256 for $arch in $VersionFile" }
    $expectedHash = $hashes[$arch]
    Write-Step "Swift $version for $arch"

    # 3. Download, verify, install.
    $tempRoot = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [System.IO.Path]::GetTempPath() }
    $installer = Join-Path $tempRoot "swift-$version-$platformDir.exe"
    $installLog = Join-Path $tempRoot "swift-$version-$platformDir-install.log"
    $url = "https://download.swift.org/swift-$version-release/$platformDir/swift-$version-RELEASE/swift-$version-RELEASE-$platformDir.exe"

    $clock = [System.Diagnostics.Stopwatch]::StartNew()
    Invoke-WithRetry "Download of $url" {
        if (Test-Path -LiteralPath $installer) { Remove-Item -LiteralPath $installer -Force }
        $code = Invoke-Native 'curl.exe' @('--fail', '--location', '--silent', '--show-error', '--output', $installer, $url)
        if ($code -ne 0) { throw "curl.exe exited with $code" }
        $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $installer).Hash.ToLowerInvariant()
        if ($actual -ne $expectedHash) {
            throw "SHA-256 mismatch for the $arch installer: expected $expectedHash, got $actual"
        }
    }
    Write-Step "SHA-256 verified ($expectedHash); download took $([int]$clock.Elapsed.TotalSeconds) s"

    $signature = Get-AuthenticodeSignature -LiteralPath $installer
    if ($signature.Status -ne 'Valid') { throw "Installer signature is not valid: $($signature.Status) $($signature.StatusMessage)" }
    if ($signature.SignerCertificate.Subject -notmatch 'O=Apple Inc\.') {
        throw "Installer is not signed by Apple Inc.: $($signature.SignerCertificate.Subject)"
    }

    $clock.Restart()
    Invoke-WithRetry 'Swift installer' {
        $process = Start-Process -FilePath $installer -ArgumentList @('/quiet', '/norestart', '/log', $installLog) -Wait -PassThru
        # 3010 means "success, reboot required"; the toolchain is usable without one.
        if ($process.ExitCode -ne 0 -and $process.ExitCode -ne 3010) {
            if (Test-Path -LiteralPath $installLog) { Get-Content -LiteralPath $installLog -Tail 40 | ForEach-Object { Write-Host $_ } }
            throw "Installer exited with $($process.ExitCode)"
        }
    }

    Write-Step "Installer finished in $([int]$clock.Elapsed.TotalSeconds) s"

    # 4. Locate what the installer laid down (per user, under %LOCALAPPDATA%\Programs\Swift).
    $swiftRoot = Join-Path $env:LOCALAPPDATA 'Programs\Swift'
    $toolchainBin = Get-ChildItem -LiteralPath (Join-Path $swiftRoot 'Toolchains') -Directory -Filter "$version*" -ErrorAction SilentlyContinue |
        ForEach-Object { Join-Path $_.FullName 'usr\bin' } | Where-Object { Test-Path -LiteralPath (Join-Path $_ 'swift.exe') } | Select-Object -First 1
    if (-not $toolchainBin) { throw "swift.exe not found under $swiftRoot\Toolchains\$version*\usr\bin" }
    $runtimeBin = Join-Path $swiftRoot "Runtimes\$version\usr\bin"
    if (-not (Test-Path -LiteralPath $runtimeBin)) { throw "Swift runtime directory not found: $runtimeBin" }
    $sdkRoot = Join-Path $swiftRoot "Platforms\$version\Windows.platform\Developer\SDKs\Windows.sdk"
    if (-not (Test-Path -LiteralPath $sdkRoot)) { throw "Windows SDK directory not found: $sdkRoot" }

    # 5. Export to the following steps and to this process.
    $env:PATH = "$toolchainBin;$runtimeBin;$env:PATH"
    $env:SDKROOT = $sdkRoot
    if ($env:GITHUB_PATH) {
        Add-Content -LiteralPath $env:GITHUB_PATH -Value $toolchainBin
        Add-Content -LiteralPath $env:GITHUB_PATH -Value $runtimeBin
    } else {
        Write-Host 'GITHUB_PATH is not set; nothing exported for later steps.'
    }
    if ($env:GITHUB_ENV) {
        Add-Content -LiteralPath $env:GITHUB_ENV -Value "SDKROOT=$sdkRoot"
    } else {
        Write-Host 'GITHUB_ENV is not set; nothing exported for later steps.'
    }

    # 6. Prove it: version, then compile and run a one-line program.
    Write-Step 'swift --version'
    $code = Invoke-Native 'swift' @('--version')
    if ($code -ne 0) { throw "swift --version exited with $code" }

    Write-Step 'Compiling a one-line program'
    $checkDir = Join-Path $tempRoot "swift-check-$([guid]::NewGuid().ToString('N'))"
    New-Item -ItemType Directory -Path $checkDir | Out-Null
    $source = Join-Path $checkDir 'check.swift'
    $program = Join-Path $checkDir 'check.exe'
    Set-Content -LiteralPath $source -Value 'print("swift-toolchain-ok")'
    $code = Invoke-Native 'swiftc' @($source, '-o', $program)
    if ($code -ne 0) { throw "swiftc exited with $code" }
    $output = & $program
    if ($LASTEXITCODE -ne 0) { throw "The compiled check program exited with $LASTEXITCODE" }
    if ("$output".Trim() -ne 'swift-toolchain-ok') { throw "The compiled check program printed '$output'" }
    Remove-Item -LiteralPath $checkDir -Recurse -Force

    # 7. Last: what is installed, and for which architecture.
    Write-Host "Swift toolchain $version installed for $arch ($osArchitecture)"
    Write-Host "Toolchain: $toolchainBin"
    Write-Host "SDKROOT: $sdkRoot"
    exit 0
} catch {
    Write-Host "::error::install-swift-windows.ps1: $($_.Exception.Message)"
    Write-Host $_.ScriptStackTrace
    exit 1
}
