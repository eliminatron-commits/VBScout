<#
.SYNOPSIS
  Network block test, Windows (Definition of Done #2) - for the collector and the evaluation app.

.DESCRIPTION
  Taken over from Stepwright and generalised. Blocks all outbound traffic of the program (and, with
  -WebView, of the WebView2 runtime) with Windows Firewall rules, enables Windows Filtering Platform
  connection auditing (Security events 5156 = allowed, 5157 = blocked) and DNS client logging, runs
  the program with the given arguments and lists every connection attempt and DNS query of its
  process tree. Passes only if the program exits with 0 and there are zero attempts.

  A positive control (a deliberate loopback connection from this script) proves that auditing
  records connections - otherwise "0" would be meaningless.

  Requires an elevated shell (GitHub runners are elevated). All changes (firewall, audit policy,
  DNS log) are restored afterwards.

.EXAMPLE
  pwsh scripts/nettest/windows.ps1 -Exe target/release/vbs-collector.exe -Arguments '--path','tests\corpus','--out','nettest-out\scan.vbscout'
  pwsh scripts/nettest/windows.ps1 -Exe target/release/vbs-app.exe -Arguments '--smoke-test=90' -WebView
#>
param(
  [Parameter(Mandatory)] [string] $Exe,
  [string[]] $Arguments = @(),
  [switch] $WebView,
  [string] $OutDir = 'nettest-out',
  [int] $TimeoutSeconds = 900
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$principal = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  throw 'Run this script in an elevated PowerShell (Administrator).'
}

$exePath = (Resolve-Path $Exe).Path
$name = [IO.Path]::GetFileNameWithoutExtension($exePath)
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$OutDir = (Resolve-Path $OutDir).Path
$group = 'VBS-NetTest'
$wfpConnection = '{0CCE9226-69AE-11D9-BED3-505054503030}' # audit subcategory "Filtering Platform Connection"
$dnsLog = 'Microsoft-Windows-DNS-Client/Operational'
$auditBackup = Join-Path $OutDir 'auditpol-backup.csv'

# Not `$webView`: PowerShell variable names ignore case, that would be the -WebView switch.
$webViewPrograms = @()
if ($WebView) {
  $webViewPrograms = @(
    Get-ChildItem "${env:ProgramFiles(x86)}\Microsoft\EdgeWebView\Application\*\msedgewebview2.exe",
    "$env:ProgramFiles\Microsoft\EdgeWebView\Application\*\msedgewebview2.exe" -ErrorAction SilentlyContinue |
      ForEach-Object FullName
  )
}
$dnsWasEnabled = (Get-WinEvent -ListLog $dnsLog).IsEnabled
$profilesBefore = @(Get-NetFirewallProfile | Select-Object Name, Enabled)

# Named data fields of an event; empty fields and events without EventData are fine under StrictMode.
function Get-EventFields($record) {
  $fields = @{}
  $xml = [xml]$record.ToXml()
  foreach ($item in $xml.SelectNodes("//*[local-name()='EventData']/*[local-name()='Data']")) {
    $fields[$item.GetAttribute('Name')] = $item.InnerText
  }
  $fields
}

$tree = [System.Collections.Generic.HashSet[int]]::new()
$treePrograms = [System.Collections.Generic.SortedSet[string]]::new()
$attempts = [System.Collections.Generic.List[object]]::new()
$dnsQueries = @()
$controlSeen = 0
$exitCode = 'not run'
try {
  auditpol /backup /file:"$auditBackup" | Out-Null
  auditpol /set /subcategory:"$wfpConnection" /success:enable /failure:enable | Out-Null
  wevtutil sl $dnsLog /e:true
  Set-NetFirewallProfile -All -Enabled True
  foreach ($program in @($exePath) + $webViewPrograms) {
    New-NetFirewallRule -DisplayName "$group $(Split-Path $program -Leaf)" -Group $group -Direction Outbound `
      -Action Block -Program $program -Profile Any | Out-Null
  }

  $start = Get-Date
  $startArgs = @{ FilePath = $exePath; PassThru = $true; NoNewWindow = $true
    RedirectStandardOutput = (Join-Path $OutDir "stdout-$name.log"); RedirectStandardError = (Join-Path $OutDir "stderr-$name.log") }
  if ($Arguments.Count) { $startArgs.ArgumentList = $Arguments }
  $process = Start-Process @startArgs
  $null = $process.Handle # keeps the exit code available
  [void]$tree.Add($process.Id)
  $deadline = $start.AddSeconds($TimeoutSeconds)
  while (-not $process.HasExited -and (Get-Date) -lt $deadline) {
    foreach ($p in Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId, ExecutablePath) {
      if ($tree.Contains([int]$p.ParentProcessId) -or $tree.Contains([int]$p.ProcessId)) {
        [void]$tree.Add([int]$p.ProcessId)
        if ($p.ExecutablePath) { [void]$treePrograms.Add($p.ExecutablePath) }
      }
    }
    Start-Sleep -Milliseconds 150
  }
  if ($process.HasExited) { $exitCode = $process.ExitCode } else { Stop-Process -Id $process.Id -Force; $exitCode = 'timeout' }

  # Positive control: a loopback connection attempt by this script must be audited.
  $client = [System.Net.Sockets.TcpClient]::new()
  try { $client.Connect('127.0.0.1', 9) } catch { } finally { $client.Dispose() }

  Start-Sleep -Seconds 3 # let the event logs flush
  $end = Get-Date

  $appName = Split-Path $exePath -Leaf
  $wfpEvents = Get-WinEvent -FilterHashtable @{ LogName = 'Security'; Id = 5156, 5157; StartTime = $start; EndTime = $end } -ErrorAction SilentlyContinue
  foreach ($record in @($wfpEvents)) {
    if ($null -eq $record) { continue }
    $f = Get-EventFields $record
    $processId = [int]$f['ProcessID']
    if ($processId -eq $PID -and $f['DestAddress'] -eq '127.0.0.1' -and $f['DestPort'] -eq '9') { $controlSeen++; continue }
    if ($tree.Contains($processId) -or $f['Application'] -like "*\$appName") {
      $attempts.Add([pscustomobject]@{
        Time        = $record.TimeCreated.ToString('HH:mm:ss.fff')
        Result      = if ($record.Id -eq 5157) { 'blocked' } else { 'allowed' }
        ProcessId   = $processId
        Application = $f['Application']
        Protocol    = $f['Protocol']
        Destination = "$($f['DestAddress']):$($f['DestPort'])"
      })
    }
  }
  $dnsQueries = @(
    Get-WinEvent -FilterHashtable @{ LogName = $dnsLog; StartTime = $start; EndTime = $end } -ErrorAction SilentlyContinue |
      Where-Object { $null -ne $_ -and $tree.Contains([int]$_.ProcessId) } |
      ForEach-Object {
        [pscustomobject]@{
          Time      = $_.TimeCreated.ToString('HH:mm:ss.fff')
          ProcessId = $_.ProcessId
          Event     = $_.Id
          Query     = if ($_.Properties.Count) { $_.Properties[0].Value } else { '' }
        }
      }
  )
}
finally {
  Get-NetFirewallRule -Group $group -ErrorAction SilentlyContinue | Remove-NetFirewallRule
  foreach ($firewallProfile in $profilesBefore) { Set-NetFirewallProfile -Name $firewallProfile.Name -Enabled $firewallProfile.Enabled }
  if (Test-Path $auditBackup) { auditpol /restore /file:"$auditBackup" | Out-Null; Remove-Item $auditBackup }
  if (-not $dnsWasEnabled) { wevtutil sl $dnsLog /e:false }
}

$report = @(
  "Network block test (Windows) - $((Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ'))"
  "program:                    $exePath $($Arguments -join ' ')"
  "blocked programs:           $(1 + $webViewPrograms.Count) (program$(if ($WebView) { ' + WebView2 runtime' }))"
  "exit code:                  $exitCode"
  "processes in program tree:  $($tree.Count)"
  "programs in program tree:   $(@($treePrograms) -join ', ')"
  "connection attempts (WFP):  $($attempts.Count)"
  "DNS queries:                $($dnsQueries.Count)"
  "positive control seen:      $controlSeen"
)
$columns = 'Time', 'Result', 'ProcessId', 'Protocol', 'Destination', 'Application'
if ($attempts.Count) { $report += ''; $report += 'Connection attempts:'; $report += ($attempts | Format-Table -Property $columns -AutoSize | Out-String -Width 400).TrimEnd() }
if ($dnsQueries.Count) { $report += ''; $report += 'DNS queries:'; $report += ($dnsQueries | Format-Table -AutoSize | Out-String -Width 400).TrimEnd() }
$report | Tee-Object -FilePath (Join-Path $OutDir "nettest-windows-$name.txt")

if ($controlSeen -lt 1) { Write-Error 'FAIL: the positive control was not audited - the monitor does not work'; exit 1 }
if ($exitCode -ne 0) { Write-Error "FAIL: $name exited with $exitCode"; exit 1 }
if ($attempts.Count -or $dnsQueries.Count) { Write-Error "FAIL: $name attempted network access"; exit 1 }
Write-Output "PASS: $name made zero network connections"
