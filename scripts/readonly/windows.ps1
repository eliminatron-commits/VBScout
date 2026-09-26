<#
.SYNOPSIS
  Read-only test, Windows (Definition of Done #1) - kernel trace level.

.DESCRIPTION
  Records the collector's file and registry changes with the kernel's own event providers
  (Microsoft-Windows-Kernel-File, Microsoft-Windows-Kernel-Registry) while it runs a complete scan:
  all system sources plus the file scan of -ScanPath. Recorded for the collector's process:

    * every file it creates (CreateNewFile),
    * every open with create/overwrite semantics (Create with a disposition other than FILE_OPEN),
    * every delete, rename and hard link (DeletePath, RenamePath, SetLinkPath),
    * every registry change (all write-type keywords of Kernel-Registry).

  Listed but not counted as changes: create calls that only opened an existing key, and handle tags
  (SetInformationKey, KeySetHandleTagsInformation) - state of an open handle, nothing stored in the registry.

  Passes only if the collector exits with 0, the only file it created or opened for creation is its
  result file, it made no other file change and no registry change, the scanned folder is unchanged
  (content hash, size, times, attributes), its working and temp folders stay empty and the output
  folder holds exactly the result file.

  A positive control - a helper process that creates a file and writes a registry value while the
  trace runs - proves that the trace records such changes; otherwise "no changes" would mean nothing.
  Requires an elevated shell (GitHub runners are elevated).

.EXAMPLE
  pwsh scripts/readonly/windows.ps1 -Exe target\release\vbs-collector.exe -ScanPath tests\corpus
#>
param(
  [Parameter(Mandatory)] [string] $Exe,
  [Parameter(Mandatory)] [string] $ScanPath,
  [string] $OutDir = 'readonly-out',
  [int] $TimeoutSeconds = 1800
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$principal = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  throw 'Run this script in an elevated PowerShell (Administrator).'
}

$exePath = (Resolve-Path $Exe).Path
$scan = (Resolve-Path $ScanPath).Path
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$OutDir = (Resolve-Path $OutDir).Path
# Below the output folder, not %TEMP%: kernel events carry long path names, %TEMP% may be an 8.3 short name.
$work = Join-Path $OutDir 'work'
Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
$cwd = Join-Path $work 'cwd'; $temp = Join-Path $work 'temp'; $results = Join-Path $work 'results'
foreach ($dir in $cwd, $temp, $results) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
$extension = (Get-Content -Raw (Join-Path $PSScriptRoot '..\..\product.json') | ConvertFrom-Json).resultFile.extension
$result = Join-Path $results "result.$extension"
$controlFile = Join-Path $work 'control.txt'
$controlKey = 'HKCU:\Software\VBScoutReadOnlyControl'
$session = 'VBScoutReadOnlyTest'
$etl = Join-Path $OutDir 'readonly-trace.etl'

# --- keywords, resolved by name from the providers' manifests ------------------------------------
function Get-ProviderKeywords([string] $Provider) {
  $keywords = [ordered]@{}
  foreach ($line in (logman query providers $Provider)) {
    if ($line -match '^\s*0x([0-9A-Fa-f]{16})\s+(\S+)') { $keywords[$Matches[2]] = [Convert]::ToUInt64($Matches[1], 16) }
  }
  $keywords
}
$fileKeywords = Get-ProviderKeywords 'Microsoft-Windows-Kernel-File'
$fileWanted = 'KERNEL_FILE_KEYWORD_CREATE', 'KERNEL_FILE_KEYWORD_CREATE_NEW_FILE', 'KERNEL_FILE_KEYWORD_DELETE_PATH',
  'KERNEL_FILE_KEYWORD_RENAME_SETLINK_PATH'
$fileMask = [UInt64]0
foreach ($name in $fileWanted) {
  if (-not $fileKeywords.Contains($name)) { throw "Kernel-File keyword $name missing; available: $($fileKeywords.Keys -join ', ')" }
  $fileMask = $fileMask -bor $fileKeywords[$name]
}
$registryKeywords = Get-ProviderKeywords 'Microsoft-Windows-Kernel-Registry'
$registryWrites = 'createkey', 'deletekey', 'setvaluekey', 'deletevaluekey', 'setinformationkey', 'setsecuritykey', 'renamekey'
$registryMask = [UInt64]0
$registryUsed = @()
foreach ($entry in $registryKeywords.GetEnumerator()) {
  $normalized = ($entry.Key -replace '[^A-Za-z]', '').ToLowerInvariant()
  if ($registryWrites | Where-Object { $normalized.EndsWith($_) }) { $registryMask = $registryMask -bor $entry.Value; $registryUsed += $entry.Key }
}
if ($registryMask -eq 0) { throw "no write keywords found for Kernel-Registry; available: $($registryKeywords.Keys -join ', ')" }

# --- snapshot of the scanned folder -------------------------------------------------------------
function Get-TreeState([string] $Root) {
  Get-ChildItem -LiteralPath $Root -Recurse -Force | Sort-Object FullName | ForEach-Object {
    $hash = if (-not $_.PSIsContainer) { (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash } else { '' }
    $length = if (-not $_.PSIsContainer) { $_.Length } else { 0 }
    '{0}|{1}|{2}|{3}|{4}|{5}' -f $_.FullName.Substring($Root.Length), $length, $_.LastWriteTimeUtc.Ticks, $_.CreationTimeUtc.Ticks, [int]$_.Attributes, $hash
  }
}

# Named data fields of an event; empty fields and events without EventData are fine under StrictMode.
function Get-EventData($record) {
  $fields = @{}
  $xml = [xml]$record.ToXml()
  foreach ($item in $xml.SelectNodes("//*[local-name()='EventData']/*[local-name()='Data']")) {
    $fields[$item.GetAttribute('Name')] = $item.InnerText
  }
  $fields
}

function Get-ProcessEvents([int] $ProcessId) {
  try {
    @(Get-WinEvent -Path $etl -Oldest -FilterXPath "*[System[Execution[@ProcessID=$ProcessId]]]" -ErrorAction Stop)
  } catch {
    if ($_.Exception.Message -match 'No events were found') { return @() }
    Write-Warning "XPath query on the trace failed ($($_.Exception.Message)); reading all events"
    @(Get-WinEvent -Path $etl -Oldest -ErrorAction SilentlyContinue | Where-Object { $_.ProcessId -eq $ProcessId })
  }
}

# The file path of a Kernel-File event (FileName or FilePath, depending on the event).
function Get-EventPath($data) {
  foreach ($name in 'FileName', 'FilePath') { if ($data[$name]) { return $data[$name] } }
  $data.Values | Where-Object { $_ -is [string] -and $_ -match '^\\(Device|\?\?)\\' } | Select-Object -First 1
}

# `\Device\HarddiskVolume3\a\b` ends with the drive-less tail of `C:\a\b`.
function Test-SamePath([string] $DevicePath, [string] $Path) {
  $tail = $Path.Substring(2)
  $DevicePath -and $DevicePath.EndsWith($tail, [StringComparison]::OrdinalIgnoreCase)
}

$FILE_OPEN = 1
$changes = [System.Collections.Generic.List[object]]::new()
$histogram = @{}
$openedExisting = @{}
$handleTags = 0
$controlFileSeen = 0; $controlRegistrySeen = 0; $controlDisposition = $null; $resultDisposition = $null
$exitCode = 'not run'
$before = Get-TreeState $scan
$providers = Join-Path $work 'providers.txt'
@(
  ('"Microsoft-Windows-Kernel-File" 0x{0:X} 0xff' -f $fileMask)
  ('"Microsoft-Windows-Kernel-Registry" 0x{0:X} 0xff' -f $registryMask)
) | Set-Content -Path $providers -Encoding ascii

try {
  logman stop $session -ets 2>$null | Out-Null
  Remove-Item -LiteralPath $etl -ErrorAction SilentlyContinue
  logman create trace $session -ets -o $etl -pf $providers -bs 1024 -nb 64 512 | Out-Null
  if ($LASTEXITCODE -ne 0) { throw "logman could not start the trace session (exit $LASTEXITCODE)" }

  # Positive control: another process creates a file and writes a registry value.
  $control = Start-Process -FilePath 'powershell.exe' -PassThru -Wait -WindowStyle Hidden -ArgumentList @(
    '-NoProfile', '-NonInteractive', '-Command',
    "Set-Content -LiteralPath '$controlFile' -Value x; New-Item -Path '$controlKey' -Force | Out-Null; Set-ItemProperty -Path '$controlKey' -Name Probe -Value 1; Remove-Item -Path '$controlKey' -Recurse -Force"
  )

  # The collector: complete scan, files restricted to the scan folder, private working and temp folders.
  $savedTemp = $env:TEMP; $savedTmp = $env:TMP
  $env:TEMP = $temp; $env:TMP = $temp
  try {
    $process = Start-Process -FilePath $exePath -WorkingDirectory $cwd -PassThru -NoNewWindow `
      -RedirectStandardOutput (Join-Path $OutDir 'collector-stdout.log') -RedirectStandardError (Join-Path $OutDir 'collector-stderr.log') `
      -ArgumentList @('--path', "`"$scan`"", '--out', "`"$result`"", '--quiet')
  } finally {
    $env:TEMP = $savedTemp; $env:TMP = $savedTmp
  }
  $null = $process.Handle
  if ($process.WaitForExit($TimeoutSeconds * 1000)) { $exitCode = $process.ExitCode } else { $process.Kill(); $exitCode = 'timeout' }
  Start-Sleep -Seconds 2 # let the kernel flush its buffers
}
finally {
  logman stop $session -ets | Out-Null
}

# --- analysis -------------------------------------------------------------------------------------
foreach ($record in Get-ProcessEvents $control.Id) {
  if ($record.ProviderName -eq 'Microsoft-Windows-Kernel-Registry') { $controlRegistrySeen++; continue }
  if ($record.ProviderName -ne 'Microsoft-Windows-Kernel-File' -or $record.Id -notin 12, 30) { continue }
  $data = Get-EventData $record
  if (-not (Test-SamePath (Get-EventPath $data) $controlFile)) { continue }
  if ($record.Id -eq 30) { $controlFileSeen++ }
  else { $controlDisposition = ([UInt32]$data['CreateOptions'] -shr 24) -band 0xFF }
}

foreach ($record in Get-ProcessEvents $process.Id) {
  $key = "$($record.ProviderName) #$($record.Id)"
  $histogram[$key] = 1 + ($histogram[$key] ?? 0)
  if ($record.ProviderName -eq 'Microsoft-Windows-Kernel-Registry') {
    $data = Get-EventData $record
    # A CreateKey that only opened an existing key (REG_OPENED_EXISTING_KEY = 2) changes nothing;
    # the keys are listed in the report (Windows components in the process, e.g. COM, use RegCreateKeyEx to open).
    if ($data['Disposition'] -and [UInt32]$data['Disposition'] -eq 2) {
      $name = [string]$data['RelativeName']
      $openedExisting[$name] = 1 + ($openedExisting[$name] ?? 0)
      continue
    }
    # SetInformationKey with class 5 (KeySetHandleTagsInformation) sets tags on an open key handle - state of
    # the handle that ends when it is closed, nothing stored in the registry (phnt ntregapi.h:
    # KEY_HANDLE_TAGS_INFORMATION, "tags associated with the key handle"). The registry API sets them for WOW64
    # view flags; the collector's own code opens keys without such flags, Windows' COM/WMI client code uses them.
    if ($record.Id -eq 11 -and $data.ContainsKey('InfoClass') -and [UInt32]$data['InfoClass'] -eq 5) {
      $handleTags++
      continue
    }
    $fields = ($data.GetEnumerator() | Sort-Object Name | ForEach-Object { "$($_.Name)=$($_.Value)" }) -join ' '
    $changes.Add([pscustomobject]@{ Kind = "registry event $($record.Id) $($record.TaskDisplayName)"; Target = $fields })
    continue
  }
  if ($record.ProviderName -ne 'Microsoft-Windows-Kernel-File') { continue }
  switch ($record.Id) {
    12 {
      $data = Get-EventData $record
      $disposition = ([UInt32]$data['CreateOptions'] -shr 24) -band 0xFF
      $path = Get-EventPath $data
      if (Test-SamePath $path $result) { $resultDisposition = $disposition }
      elseif ($disposition -ne $FILE_OPEN) {
        $changes.Add([pscustomobject]@{ Kind = "open with disposition $disposition"; Target = $path })
      }
    }
    30 {
      $path = Get-EventPath (Get-EventData $record)
      if (-not (Test-SamePath $path $result)) { $changes.Add([pscustomobject]@{ Kind = 'file created'; Target = $path }) }
    }
    { $_ -in 26, 27, 28 } {
      $what = @{ 26 = 'file deleted'; 27 = 'file renamed'; 28 = 'hard link created' }[$record.Id]
      $changes.Add([pscustomobject]@{ Kind = $what; Target = Get-EventPath (Get-EventData $record) })
    }
  }
}

$after = Get-TreeState $scan
$treeDiff = @(Compare-Object -ReferenceObject @($before) -DifferenceObject @($after))
$resultsListing = @(Get-ChildItem -LiteralPath $results -Force | ForEach-Object Name)
$cwdListing = @(Get-ChildItem -LiteralPath $cwd -Force | ForEach-Object Name)
$tempListing = @(Get-ChildItem -LiteralPath $temp -Force | ForEach-Object Name)

$report = @(
  "Read-only test (Windows) - $((Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ'))"
  "collector:                  $exePath"
  "scanned folder:             $scan"
  "exit code:                  $exitCode"
  "file keywords:              $($fileWanted -join ', ')"
  "registry keywords:          $($registryUsed -join ', ')"
  "collector trace events:     $(($histogram.GetEnumerator() | Sort-Object Name | ForEach-Object { "$($_.Name)=$($_.Value)" }) -join ', ')"
  "changes by the collector:   $($changes.Count)"
  "existing keys opened by create calls (unchanged): $(($openedExisting.GetEnumerator() | Sort-Object Name | ForEach-Object { "$($_.Name) ($($_.Value))" }) -join '; ')"
  "handle tags on open keys (not stored in the registry): $handleTags"
  "scanned folder unchanged:   $(if ($treeDiff.Count) { 'NO' } else { 'yes' })"
  "output folder:              $($resultsListing -join ', ')"
  "working folder:             $($cwdListing -join ', ')"
  "temp folder:                $($tempListing -join ', ')"
  "positive control seen:      file $controlFileSeen, registry $controlRegistrySeen, create disposition $controlDisposition"
  "result file disposition:    $resultDisposition (2 = FILE_CREATE: created new, never overwriting)"
)
if ($changes.Count) { $report += ''; $report += 'Changes:'; $report += ($changes | Format-Table -AutoSize | Out-String -Width 400).TrimEnd() }
if ($treeDiff.Count) { $report += ''; $report += 'Differences in the scanned folder:'; $report += ($treeDiff | Format-Table -AutoSize | Out-String -Width 400).TrimEnd() }
$report | Tee-Object -FilePath (Join-Path $OutDir 'readonly-windows.txt')

if ($controlFileSeen -lt 1 -or $controlRegistrySeen -lt 1) { Write-Error 'FAIL: the positive control was not recorded - the trace does not work'; exit 1 }
# The control creates its file with CREATE_ALWAYS: a decoded disposition of FILE_OPEN would mean overwrites go unnoticed.
if ($null -eq $controlDisposition -or $controlDisposition -eq $FILE_OPEN) { Write-Error "FAIL: create dispositions are not decoded correctly ($controlDisposition)"; exit 1 }
if ($exitCode -ne 0) { Write-Error "FAIL: the collector exited with $exitCode"; exit 1 }
$FILE_CREATE = 2
if ($resultDisposition -ne $FILE_CREATE) { Write-Error "FAIL: the result file was not created with FILE_CREATE ($resultDisposition)"; exit 1 }
if ($changes.Count) { Write-Error 'FAIL: the collector changed files or the registry'; exit 1 }
if ($treeDiff.Count) { Write-Error 'FAIL: the scanned folder changed'; exit 1 }
if ($resultsListing.Count -ne 1 -or $resultsListing[0] -ne "result.$extension") { Write-Error 'FAIL: the output folder must contain exactly the result file'; exit 1 }
if ($cwdListing.Count -or $tempListing.Count) { Write-Error 'FAIL: the collector left files in its working or temp folder'; exit 1 }
Write-Output 'PASS: the collector changed nothing and wrote only its result file'
