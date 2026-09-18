<#
.SYNOPSIS
    Convenience wrapper around `cargo xtask` for Windows.

.DESCRIPTION
    `cargo xtask` is the real entry point. This script exists because on a
    Windows machine without Visual Studio, even building the build driver needs
    a toolchain that can link, and `rust-toolchain.toml` pins the default one.
    It picks a working toolchain, then hands over to xtask.

    On a machine with the Visual Studio Build Tools, or on Linux and macOS,
    `cargo xtask <command>` works directly and this script is unnecessary.

.PARAMETER Command
    The xtask command: build, kernel, bootloader, image, size, test, clippy,
    fmt, check, help.

.EXAMPLE
    ./tools/build.ps1 test

.EXAMPLE
    ./tools/build.ps1 image --release
#>
[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [string]$Command = 'help',

    [Parameter(Position = 1, ValueFromRemainingArguments = $true)]
    [string[]]$Rest = @()
)

$ErrorActionPreference = 'Stop'

# rustup's proxies are not always on PATH in a non-login shell.
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
if (Test-Path $cargoBin) { $env:Path = "$cargoBin;$env:Path" }

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "Rust is not installed, or cargo is not on PATH. See README.md for setup."
}

# Does the default toolchain have a linker it can use?
$hasMsvcLinker = $null -ne (Get-Command link.exe -ErrorAction SilentlyContinue)

$toolchainArg = @()
if (-not $hasMsvcLinker) {
    $fallback = 'nightly-x86_64-pc-windows-gnu'
    $installed = (rustup toolchain list) | ForEach-Object { ($_ -split '\s+')[0] }
    if ($installed -notcontains $fallback) {
        Write-Host "This machine has no MSVC linker, so the default Windows toolchain" -ForegroundColor Yellow
        Write-Host "cannot link. Installing the GNU-host toolchain instead." -ForegroundColor Yellow
        rustup toolchain install $fallback --profile minimal -c rust-src
        if ($LASTEXITCODE -ne 0) { throw "Could not install $fallback." }
    }
    $toolchainArg = @("+$fallback")
    # xtask makes the same choice for the builds it drives; telling it
    # explicitly keeps the two from disagreeing.
    $env:LOKO_TOOLCHAIN = $fallback
}

$cargoArgs = $toolchainArg + @('run', '--quiet', '--package', 'xtask', '--', $Command) + $Rest

# cargo writes progress and warnings to stderr. Under `ErrorActionPreference =
# Stop`, PowerShell treats any stderr output from a native command as a
# terminating error, which would abort a perfectly successful build on a
# harmless incremental-cache warning. The exit code is the only signal worth
# acting on here.
$ErrorActionPreference = 'Continue'
& cargo @cargoArgs
exit $LASTEXITCODE
