<#
.SYNOPSIS
  Installs the app with its NSIS installer silently, starts the installed program (smoke test) and
  uninstalls it again – on a disposable machine (CI runner).

.DESCRIPTION
  Checks what winget and administrators rely on: silent install (/S) per user without a download,
  an entry under "Apps & features", the installed program starts and passes its self-check, and the
  silent uninstall removes program and entry.
#>
param([Parameter(Mandatory = $true)] [string] $Setup)
$ErrorActionPreference = 'Stop'
$product = Get-Content -Raw (Join-Path $PSScriptRoot '..\..\product.json') | ConvertFrom-Json

function Find-Entry {
  foreach ($hive in 'HKCU:', 'HKLM:') {
    $entry = Get-ChildItem "$hive\Software\Microsoft\Windows\CurrentVersion\Uninstall" -ErrorAction SilentlyContinue |
      Get-ItemProperty | Where-Object { $_.DisplayName -eq $product.name } | Select-Object -First 1
    if ($entry) { return $entry }
  }
}

if (Find-Entry) { throw "$($product.name) is already installed on this machine" }
$install = Start-Process -FilePath $Setup -ArgumentList '/S' -PassThru -Wait
if ($install.ExitCode -ne 0) { throw "installer failed with exit code $($install.ExitCode)" }

$entry = Find-Entry
if (-not $entry) { throw 'no uninstall entry after installation' }
$dir = $entry.InstallLocation.Trim('"')
if (-not $dir -or -not (Test-Path -LiteralPath $dir)) { throw "install location missing: '$dir'" }
$exe = Get-ChildItem -LiteralPath $dir -Filter *.exe | Where-Object { $_.Name -notmatch '^uninstall' } | Select-Object -First 1
if (-not $exe) { throw "no program in $dir" }
"installed: $($exe.FullName) (version $($entry.DisplayVersion), publisher $($entry.Publisher))"

$run = Start-Process -FilePath $exe.FullName -ArgumentList '--smoke-test=90' -PassThru
$null = $run.Handle
if (-not $run.WaitForExit(150000)) { $run.Kill(); throw 'installed app: smoke test timed out' }
if ($run.ExitCode -ne 0) { throw "installed app: smoke test failed with exit code $($run.ExitCode)" }
'installed app: smoke test passed'

# `_?=` runs the NSIS uninstaller in place, so the step waits for it.
$uninstaller = Join-Path $dir 'uninstall.exe'
$remove = Start-Process -FilePath $uninstaller -ArgumentList '/S', "_?=$dir" -PassThru -Wait
if ($remove.ExitCode -ne 0) { throw "uninstaller failed with exit code $($remove.ExitCode)" }
if (Test-Path -LiteralPath $exe.FullName) { throw 'program still present after uninstall' }
if (Find-Entry) { throw 'uninstall entry still present' }
'uninstalled cleanly'
