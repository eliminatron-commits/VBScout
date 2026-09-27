<#
.SYNOPSIS
  Signs Windows program files with Authenticode – prepared, used as soon as a certificate exists.

.DESCRIPTION
  Reads the code-signing certificate from the environment (GitHub secrets in the release pipeline):
    VBS_SIGN_CERT_BASE64    PFX file, base64
    VBS_SIGN_CERT_PASSWORD  its password
    VBS_SIGN_TIMESTAMP_URL  optional RFC 3161 time stamp server (default: DigiCert)
  Without a certificate the files stay unsigned and a warning says so; -Required turns that into an
  error. Also used as Tauri's signCommand for the app and its installer (package.ps1).

.EXAMPLE
  ./scripts/release/sign.ps1 -Description Product -Path dist-release\Product-Collector-0.1.0-x64.exe
#>
param(
  [Parameter(Mandatory = $true, ValueFromRemainingArguments = $true)] [string[]] $Path,
  # Shown in the Windows signature dialog; package.ps1 passes the product name from product.json.
  [string] $Description = '',
  [switch] $Required
)
$ErrorActionPreference = 'Stop'

if (-not $env:VBS_SIGN_CERT_BASE64) {
  $message = "not signed (no code-signing certificate configured): $($Path -join ', ')"
  if ($Required) { throw $message }
  Write-Host "::warning::$message"
  exit 0
}

$signtool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" -ErrorAction SilentlyContinue |
  Sort-Object FullName -Descending | Select-Object -First 1
if (-not $signtool) { throw 'signtool.exe not found (Windows SDK)' }

$pfx = Join-Path ([IO.Path]::GetTempPath()) ("sign-" + [guid]::NewGuid() + ".pfx")
try {
  [IO.File]::WriteAllBytes($pfx, [Convert]::FromBase64String($env:VBS_SIGN_CERT_BASE64))
  $timestamp = if ($env:VBS_SIGN_TIMESTAMP_URL) { $env:VBS_SIGN_TIMESTAMP_URL } else { 'http://timestamp.digicert.com' }
  foreach ($file in $Path) {
    $describe = if ($Description) { @('/d', $Description) } else { @() }
    & $signtool.FullName sign /f $pfx /p $env:VBS_SIGN_CERT_PASSWORD /fd SHA256 /tr $timestamp /td SHA256 @describe $file
    if ($LASTEXITCODE) { throw "signing failed: $file" }
    & $signtool.FullName verify /pa $file
    if ($LASTEXITCODE) { throw "signature does not verify: $file" }
  }
} finally {
  Remove-Item -LiteralPath $pfx -Force -ErrorAction SilentlyContinue
}
