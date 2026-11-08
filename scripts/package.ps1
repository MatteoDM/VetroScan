# Packages a release build into dist/ on Windows: the portable zip and the Inno Setup installer
# (packaging/windows/hp3770.iss). Linux and macOS use scripts/package.sh.
#
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/package.ps1
#        scripts/package.ps1 -Target x86_64-pc-windows-msvc -Version 0.1.0   (as CI calls it)
#
#   Without arguments it builds the release binaries and takes the version from Cargo.toml.
#   With -Version it packages binaries that are already built (in target/<Target>/release).
#
# Needs Inno Setup 6 (https://jrsoftware.org/isinfo.php; installed from Chocolatey if missing
# and choco is there, as on the CI runners) and cargo-about
# (`cargo install --locked --features cli cargo-about`) for the third-party license notices.
param(
    [string]$Target = '',
    [string]$Version = ''
)

$ErrorActionPreference = 'Stop'
Set-Location (Split-Path -Parent $PSScriptRoot)

if (-not $Version) {
    cargo build --release --locked --bin hp3770-app --bin hp3770
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }
    # "path+file:///.../crates/hp3770-app#0.1.0" or "...#hp3770-app@0.1.0": after the last # or @.
    $Version = (cargo pkgid -p hp3770-app) -replace '.*[#@]', ''
    if ($LASTEXITCODE -ne 0 -or -not $Version) { throw "Could not read the hp3770-app version" }
}

$bindir = if ($Target) { "target\$Target\release" } else { 'target\release' }
$app = (Resolve-Path "$bindir\hp3770-app.exe").Path
$cli = (Resolve-Path "$bindir\hp3770.exe").Path

$iscc = @(
    (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'),
    (Join-Path $env:ProgramFiles 'Inno Setup 6\ISCC.exe'),
    (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe')
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc -and (Get-Command choco -ErrorAction SilentlyContinue)) {
    choco install innosetup --no-progress -y
    if ($LASTEXITCODE -ne 0) { throw "Could not install Inno Setup" }
    $iscc = Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'
}
if (-not $iscc -or -not (Test-Path $iscc)) {
    throw "Inno Setup 6 not found: install it from https://jrsoftware.org/isinfo.php"
}

New-Item -ItemType Directory -Force dist, target\package | Out-Null
$name = "hp3770-v$Version-windows-x86_64"

# The notices of the crates built into the binaries (as scripts/third_party_licenses.sh).
$notices = 'target\package\THIRD_PARTY_LICENSES.txt'
# about.toml covers every platform; the build fetched only this one's crates.
cargo fetch --locked
if ($LASTEXITCODE -ne 0) { throw "cargo fetch failed with exit code $LASTEXITCODE" }
cargo about generate --frozen --fail --workspace -c packaging/about.toml -o $notices packaging/about.hbs
if ($LASTEXITCODE -ne 0) { throw "cargo about failed with exit code $LASTEXITCODE" }

# Portable build: unzip anywhere and run, nothing installed.
Compress-Archive -Path $app, $cli, README.md, LICENSE, $notices -DestinationPath "dist\$name-portable.zip" -Force
Write-Host "packaged dist\$name-portable.zip"

& $iscc /Qp "/DAppVersion=$Version" "/DSourceDir=$((Resolve-Path $bindir).Path)" "/DOutputDir=$((Resolve-Path dist).Path)" packaging\windows\hp3770.iss
if ($LASTEXITCODE -ne 0) { throw "ISCC failed with exit code $LASTEXITCODE" }
Write-Host "packaged dist\$name-setup.exe"
