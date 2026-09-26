<#
.SYNOPSIS
  Definition of Done #3 (second half): without administrator rights the collector runs a limited
  scan and says so – on the console and in the result file.

.DESCRIPTION
  Starts the collector with a "basic user" token (runas /trustlevel:0x20000: the Administrators
  group is deny-only and all privileges are removed), waits for it, and checks:
    * exit code 0 and a written result file,
    * coverage.elevated = false, coverage.mode = limited, limitation notElevated,
    * admin-only system sources marked partial/failed rather than complete,
    * the console hint that the scan is limited.
  Must be started from an elevated shell (the CI runner is).
#>
param(
  [Parameter(Mandatory)] [string] $Exe,
  [Parameter(Mandatory)] [string] $ScanPath,
  [string] $OutDir = 'nonadmin-out',
  [int] $TimeoutSeconds = 900
)

$ErrorActionPreference = 'Stop'

$exePath = (Resolve-Path $Exe).Path
$scan = (Resolve-Path $ScanPath).Path
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$OutDir = (Resolve-Path $OutDir).Path
$extension = (Get-Content -Raw (Join-Path $PSScriptRoot '..\..\product.json') | ConvertFrom-Json).resultFile.extension
$result = Join-Path $OutDir "nonadmin.$extension"
$console = Join-Path $OutDir 'nonadmin-console.txt'
$done = Join-Path $OutDir 'nonadmin-exit.txt'
$wrapper = Join-Path $OutDir 'nonadmin-run.cmd'
Remove-Item -LiteralPath $result, $console, $done -ErrorAction SilentlyContinue
# The basic-user token must be able to write here.
icacls $OutDir /grant '*S-1-5-11:(OI)(CI)M' | Out-Null
Set-Content -LiteralPath $wrapper -Encoding ascii -Value @(
  '@echo off'
  "`"$exePath`" --path `"$scan`" --out `"$result`" > `"$console`" 2>&1"
  # (echo …)> keeps a single-digit exit code from being read as a handle redirection (`echo 0> file`).
  "(echo %ERRORLEVEL%)> `"$done`""
)

runas /trustlevel:0x20000 "`"$wrapper`""
$clock = [Diagnostics.Stopwatch]::StartNew()
while (-not (Test-Path -LiteralPath $done)) {
  if ($clock.Elapsed.TotalSeconds -gt $TimeoutSeconds) { throw 'the collector did not finish in time' }
  Start-Sleep -Seconds 2
}
Start-Sleep -Seconds 1
$exitCode = "$(Get-Content -Raw -LiteralPath $done)".Trim()
$consoleText = if (Test-Path -LiteralPath $console) { Get-Content -Raw -LiteralPath $console } else { '' }

$failures = [System.Collections.Generic.List[string]]::new()
if ($exitCode -ne '0') { $failures.Add("exit code $exitCode") }
if (-not (Test-Path -LiteralPath $result)) {
  $failures.Add('no result file')
} else {
  Add-Type -AssemblyName System.IO.Compression.FileSystem
  $zip = [IO.Compression.ZipFile]::OpenRead($result)
  try {
    $reader = [IO.StreamReader]::new($zip.GetEntry('result.json').Open())
    $scanResult = $reader.ReadToEnd() | ConvertFrom-Json
    $reader.Dispose()
  } finally { $zip.Dispose() }
  if ($scanResult.coverage.elevated -ne $false) { $failures.Add('coverage.elevated is not false') }
  if ($scanResult.coverage.mode -ne 'limited') { $failures.Add("coverage.mode is $($scanResult.coverage.mode)") }
  if (-not ($scanResult.coverage.limitations | Where-Object { $_.code -eq 'notElevated' })) { $failures.Add('limitation notElevated missing') }
  $adminOnly = @($scanResult.coverage.sources | Where-Object { $_.source -in 'tasks.scheduled', 'installer.packages' })
  if (-not $adminOnly.Count) { $failures.Add('admin-only sources missing from the coverage') }
  $complete = @($adminOnly | Where-Object { $_.status -eq 'complete' })
  if ($complete.Count) { $failures.Add("admin-only sources reported complete: $(($complete | ForEach-Object source) -join ', ')") }
  $sources = ($scanResult.coverage.sources | ForEach-Object { "$($_.source)=$($_.status)$(if ($_.reason) { "($($_.reason))" })" }) -join ', '
}
if ($consoleText -notmatch 'administrator') { $failures.Add('the console does not mention the missing administrator rights') }

$report = @(
  "Non-admin run (Windows) - $((Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ'))"
  "exit code: $exitCode"
  "sources:   $sources"
  'console:'
  $consoleText
)
if ($failures.Count) { $report += 'Failures:'; $report += $failures }
$report | Tee-Object -FilePath (Join-Path $OutDir 'nonadmin-windows.txt')
if ($failures.Count) { Write-Error 'FAIL: the non-admin scan is not marked as limited'; exit 1 }
Write-Output 'PASS: without administrator rights the scan is limited and says so'
