<#
.SYNOPSIS
  Builds the release files for Windows x64 into one folder.

.DESCRIPTION
  • <Name>-Collector-<version>-x64.exe   collector, portable single file (static CRT)
  • <Name>-<version>-x64-setup.exe       evaluation app, NSIS installer (per user, no download at install)
  • <Name>-<version>-x64-portable.exe    evaluation app without installation (needs the WebView2 runtime)
  • <Name>-<version>-SHA256SUMS.txt       checksums of the three programs
  • <Name>-<version>-winget-manifests.zip manifests for microsoft/winget-pkgs (packaging/winget/)
  • <Name>-<version>-THIRD-PARTY-NOTICES.md licenses of the components compiled into the programs
  Names come from product.json (scripts/release/names.mjs). With a code-signing certificate in the
  environment (see sign.ps1) the programs and the installer are signed; without one they stay unsigned.

.PARAMETER SkipBuild
  Use the programs already built (CI builds them in earlier steps; the app with `--bundles nsis`).

.EXAMPLE
  ./scripts/release/package.ps1 -OutDir dist-release
#>
param(
  [string] $OutDir = 'dist-release',
  [switch] $SkipBuild
)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
Set-Location $root

$names = node scripts/release/names.mjs | ConvertFrom-Json
if ($LASTEXITCODE) { throw 'names.mjs failed' }
$product = Get-Content -Raw product.json | ConvertFrom-Json
$sign = Join-Path $root 'scripts\release\sign.ps1'

if (-not $SkipBuild) {
  cargo build --release --locked -p vbs-collector
  if ($LASTEXITCODE) { throw 'collector build failed' }
  $tauri = @('tauri', 'build', '--bundles', 'nsis')
  if ($env:VBS_SIGN_CERT_BASE64) {
    # Tauri signs the app and the installer with this command (object form: no quoting issues).
    $config = @{ bundle = @{ windows = @{ signCommand = @{
      cmd = 'pwsh'; args = @('-NoProfile', '-File', $sign, '-Description', $product.name, '%1') } } } }
    $configFile = Join-Path $root 'target\release-sign.conf.json'
    $config | ConvertTo-Json -Depth 6 | Set-Content -Encoding utf8 $configFile
    $tauri += @('--config', $configFile)
  }
  npx @tauri
  if ($LASTEXITCODE) { throw 'app build failed' }
}

New-Item -ItemType Directory -Force $OutDir | Out-Null
$out = (Resolve-Path $OutDir).Path

$collector = Join-Path $out $names.collector
Copy-Item -Force target\release\vbs-collector.exe $collector
& $sign -Description "$($product.name) Collector" -Path $collector

Copy-Item -Force target\release\vbs-app.exe (Join-Path $out $names.portable)
$setup = Get-ChildItem target\release\bundle\nsis\*_x64-setup.exe | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $setup) { throw 'NSIS installer not found (build the app with --bundles nsis)' }
Copy-Item -Force $setup.FullName (Join-Path $out $names.setup)
if ($SkipBuild) {
  # The CI build ran without signCommand: sign here if a certificate is configured.
  & $sign -Description $product.name -Path (Join-Path $out $names.portable), (Join-Path $out $names.setup)
}

$lines = foreach ($file in @($names.collector, $names.setup, $names.portable)) {
  $hash = (Get-FileHash -Algorithm SHA256 (Join-Path $out $file)).Hash.ToLowerInvariant()
  "$hash  $file"
}
Set-Content -Encoding ascii -Path (Join-Path $out $names.checksums) -Value $lines

$winget = Join-Path $out 'winget'
node scripts/release/winget.mjs --dir $out --out $winget
if ($LASTEXITCODE) { throw 'winget manifests failed' }
Compress-Archive -Force -Path (Join-Path $winget '*') -DestinationPath (Join-Path $out $names.winget)
Remove-Item -Recurse -Force $winget

node scripts/release/third-party.mjs --out (Join-Path $out $names.notices)
if ($LASTEXITCODE) { throw 'third-party notices failed' }

Get-ChildItem $out | Format-Table Name, Length
