<#
.SYNOPSIS
  System test, Windows: the collector finds VBScript dependencies that Windows itself created.

.DESCRIPTION
  The test collections (tests/corpus) check the modules with fixtures – files written by
  independent tools and in-memory registry/WMI/event views. This script checks the same modules
  against the real thing on a disposable CI machine:

    * a scheduled task registered with schtasks,
    * a Run value and a service (srvany-style Parameters\Application) in the registry,
    * a permanent WMI subscription (ActiveScriptEventConsumer with VBScript, bound to a filter
      that never fires),
    * a shortcut written by the Windows shell (WScript.Shell.CreateShortcut),
    * an installer package written by msi.dll (WindowsInstaller.Installer) with a VBScript
      custom action,
    * a script encoded by Microsoft's Script Encoder (Scripting.Encoder), where available,
    * the hive file of a user who is not logged on (NTUSER.DAT written by `reg save`, registered
      in ProfileList) with a Run value,
    * an Office Open XML workbook written by Microsoft's packaging code (System.IO.Packaging, or
      .NET's ZipArchive) around a VBA project that uses VBScript regular expressions (the project
      from the test collection),
    * an Access database created by Windows' own Jet engine (32-bit ADOX), which must be read
      without a finding, where the Jet provider exists,
    * the VBScript deprecation alert (event 4096), if this Windows build logs it.

  It then runs the collector (system sources + the test folder), reads the result file and checks
  the expected findings, that nothing failed with an internal error, and that the encoded
  script's secret was masked. Everything created is removed at the end. Requires an elevated
  shell and a machine that may be changed (CI runner).
#>
param(
  [Parameter(Mandatory)] [string] $Exe,
  [string] $OutDir = 'systemtest-out'
)

$ErrorActionPreference = 'Stop'

$principal = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Run this script elevated.' }

$exePath = (Resolve-Path $Exe).Path
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$OutDir = (Resolve-Path $OutDir).Path
$base = Join-Path $env:SystemDrive 'VBScoutSystemTest'
$scan = Join-Path $base 'scan'
Remove-Item -LiteralPath $base -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $scan | Out-Null

$taskName = 'VBScoutSystemTest\Nightly'
$runKey = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Run'
$serviceName = 'VBScoutTestSvc'
$wmiNamespace = 'root\subscription'
$consumerName = 'VBScoutTestConsumer'
$filterName = 'VBScoutTestFilter'
$offlineSid = 'S-1-5-21-1111111111-2222222222-3333333333-4242'
$profileList = "HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList\$offlineSid"
$hiveSource = 'HKLM\SOFTWARE\VBScoutSystemTestHive'
$notes = [System.Collections.Generic.List[string]]::new()

function Invoke-Com($object, [string] $method, [object[]] $arguments) {
  # Cmdlet output (e.g. Join-Path) arrives wrapped in PSObject, which COM rejects (DISP_E_TYPEMISMATCH).
  $plain = [object[]]@($arguments | ForEach-Object { $_.PSObject.BaseObject })
  $object.GetType().InvokeMember($method, [Reflection.BindingFlags]::InvokeMethod, $null, $object, $plain)
}

function Remove-Artifacts {
  schtasks /delete /tn $taskName /f 2>$null | Out-Null
  schtasks /delete /tn 'VBScoutSystemTest' /f 2>$null | Out-Null
  Remove-ItemProperty -Path $runKey -Name 'VBScoutSystemTest' -ErrorAction SilentlyContinue
  sc.exe delete $serviceName 2>$null | Out-Null
  Get-CimInstance -Namespace $wmiNamespace -ClassName __FilterToConsumerBinding -ErrorAction SilentlyContinue |
    Where-Object { $_.Consumer.Name -eq $consumerName } | Remove-CimInstance -ErrorAction SilentlyContinue
  Get-CimInstance -Namespace $wmiNamespace -ClassName ActiveScriptEventConsumer -Filter "Name='$consumerName'" -ErrorAction SilentlyContinue |
    Remove-CimInstance -ErrorAction SilentlyContinue
  Get-CimInstance -Namespace $wmiNamespace -ClassName __EventFilter -Filter "Name='$filterName'" -ErrorAction SilentlyContinue |
    Remove-CimInstance -ErrorAction SilentlyContinue
  Remove-Item -Path $profileList -Recurse -Force -ErrorAction SilentlyContinue
  reg delete $hiveSource /f 2>$null | Out-Null
}

try {
  Remove-Artifacts

  # Scheduled task (Task Scheduler writes the definition file).
  schtasks /create /tn $taskName /tr "wscript.exe //B $base\nightly.vbs" /sc daily /st 03:00 /ru SYSTEM /f | Out-Null
  if ($LASTEXITCODE -ne 0) { throw 'schtasks failed' }

  # Run value.
  New-ItemProperty -Path $runKey -Name 'VBScoutSystemTest' -Value "wscript.exe //B `"$base\agent.vbs`"" -PropertyType String -Force | Out-Null

  # Service with a srvany-style wrapper.
  sc.exe create $serviceName binPath= "$base\srvany.exe" start= demand | Out-Null
  if ($LASTEXITCODE -ne 0) { throw 'sc create failed' }
  $parameters = "HKLM:\SYSTEM\CurrentControlSet\Services\$serviceName\Parameters"
  New-Item -Path $parameters -Force | Out-Null
  New-ItemProperty -Path $parameters -Name Application -Value 'C:\Windows\System32\cscript.exe' -PropertyType String -Force | Out-Null
  New-ItemProperty -Path $parameters -Name AppParameters -Value "//B $base\service.vbs" -PropertyType String -Force | Out-Null

  # Permanent WMI subscription whose filter never fires.
  $filter = New-CimInstance -Namespace $wmiNamespace -ClassName __EventFilter -Property @{
    Name = $filterName; EventNamespace = 'root\cimv2'; QueryLanguage = 'WQL'
    Query = "SELECT * FROM __InstanceModificationEvent WITHIN 86400 WHERE TargetInstance ISA 'Win32_LocalTime' AND TargetInstance.Year = 1999"
  }
  $consumer = New-CimInstance -Namespace $wmiNamespace -ClassName ActiveScriptEventConsumer -Property @{
    Name = $consumerName; ScriptingEngine = 'VBScript'; ScriptText = "Set fso = CreateObject(`"Scripting.FileSystemObject`")`r`nfso.DeleteFile `"C:\VBScoutSystemTest\never.tmp`""
  }
  New-CimInstance -Namespace $wmiNamespace -ClassName __FilterToConsumerBinding -Property @{ Filter = [ref]$filter; Consumer = [ref]$consumer } | Out-Null

  # Shortcut written by the Windows shell.
  $shell = New-Object -ComObject WScript.Shell
  $link = $shell.CreateShortcut((Join-Path $scan 'Tool.lnk'))
  $link.TargetPath = 'C:\Windows\System32\wscript.exe'
  $link.Arguments = "`"$base\tool.vbs`""
  $link.WorkingDirectory = $base
  $link.Save()

  # Installer package written by msi.dll, with a VBScript custom action (type 38).
  $installer = New-Object -ComObject WindowsInstaller.Installer
  $msiPath = Join-Path $scan 'windows-made.msi'
  $database = Invoke-Com $installer 'OpenDatabase' @($msiPath, 3)
  foreach ($sql in @(
      'CREATE TABLE `Property` (`Property` CHAR(72) NOT NULL, `Value` LONGCHAR NOT NULL LOCALIZABLE PRIMARY KEY `Property`)',
      "INSERT INTO ``Property`` (``Property``, ``Value``) VALUES ('ProductName', 'VBScout System Test')",
      'CREATE TABLE `CustomAction` (`Action` CHAR(72) NOT NULL, `Type` SHORT NOT NULL, `Source` CHAR(72), `Target` CHAR(255) PRIMARY KEY `Action`)',
      "INSERT INTO ``CustomAction`` (``Action``, ``Type``, ``Target``) VALUES ('SetDefaults', 38, 'Set shell = CreateObject(""WScript.Shell"")')",
      'CREATE TABLE `InstallExecuteSequence` (`Action` CHAR(72) NOT NULL, `Condition` CHAR(255), `Sequence` SHORT PRIMARY KEY `Action`)',
      "INSERT INTO ``InstallExecuteSequence`` (``Action``, ``Sequence``) VALUES ('SetDefaults', 1500)")) {
    $view = Invoke-Com $database 'OpenView' @($sql)
    Invoke-Com $view 'Execute' @() | Out-Null
    Invoke-Com $view 'Close' @() | Out-Null
  }
  Invoke-Com $database 'Commit' @() | Out-Null
  [Runtime.InteropServices.Marshal]::ReleaseComObject($database) | Out-Null

  # A profile whose user is not logged on: its hive file, written by Windows (reg save).
  $offlineProfile = Join-Path $base 'offline-profile'
  New-Item -ItemType Directory -Force -Path $offlineProfile | Out-Null
  reg add "$hiveSource\Software\Microsoft\Windows\CurrentVersion\Run" /v VBScoutOffline /t REG_SZ /d "wscript.exe //B $base\offline.vbs" /f | Out-Null
  reg save $hiveSource (Join-Path $offlineProfile 'NTUSER.DAT') /y | Out-Null
  if ($LASTEXITCODE -ne 0) { throw 'reg save failed' }
  New-Item -Path $profileList -Force | Out-Null
  New-ItemProperty -Path $profileList -Name ProfileImagePath -Value $offlineProfile -PropertyType ExpandString -Force | Out-Null

  # Office Open XML workbook written by Microsoft's packaging code (System.IO.Packaging, else .NET's
  # ZipArchive with the package parts), with the VBA project of the test collection's regexp-late.xlsm.
  Add-Type -AssemblyName System.IO.Compression
  Add-Type -AssemblyName System.IO.Compression.FileSystem
  $sample = [IO.Compression.ZipFile]::OpenRead((Join-Path $PSScriptRoot '..\..\tests\corpus\positive\office\regexp-late.xlsm'))
  try {
    $stream = $sample.GetEntry('xl/vbaProject.bin').Open()
    $vba = [IO.MemoryStream]::new()
    $stream.CopyTo($vba)
    $stream.Dispose()
  } finally { $sample.Dispose() }
  $packagedPath = Join-Path $scan 'windows-packaged.xlsm'
  $workbookXml = [Text.Encoding]::UTF8.GetBytes('<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"/>')
  try {
    Add-Type -AssemblyName WindowsBase
    $package = [IO.Packaging.Package]::Open($packagedPath, [IO.FileMode]::Create)
    try {
      $workbook = $package.CreatePart([Uri]::new('/xl/workbook.xml', [UriKind]::Relative), 'application/vnd.ms-excel.sheet.macroEnabled.main+xml')
      $workbook.GetStream().Write($workbookXml, 0, $workbookXml.Length)
      $package.CreateRelationship($workbook.Uri, [IO.Packaging.TargetMode]::Internal, 'http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument') | Out-Null
      $vbaPart = $package.CreatePart([Uri]::new('/xl/vbaProject.bin', [UriKind]::Relative), 'application/vnd.ms-office.vbaProject')
      $vba.Position = 0
      $vba.CopyTo($vbaPart.GetStream())
      $workbook.CreateRelationship([Uri]::new('vbaProject.bin', [UriKind]::Relative), [IO.Packaging.TargetMode]::Internal, 'http://schemas.microsoft.com/office/2006/relationships/vbaProject') | Out-Null
    } finally { $package.Close() }
    $notes.Add('workbook package written by System.IO.Packaging')
  } catch {
    Remove-Item -LiteralPath $packagedPath -ErrorAction SilentlyContinue
    $archive = [IO.Compression.ZipFile]::Open($packagedPath, [IO.Compression.ZipArchiveMode]::Create)
    try {
      $parts = [ordered]@{
        '[Content_Types].xml' = '<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="bin" ContentType="application/vnd.ms-office.vbaProject"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.ms-excel.sheet.macroEnabled.main+xml"/></Types>'
        '_rels/.rels' = '<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>'
        'xl/_rels/workbook.xml.rels' = '<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.microsoft.com/office/2006/relationships/vbaProject" Target="vbaProject.bin"/></Relationships>'
      }
      foreach ($name in $parts.Keys) {
        $writer = [IO.StreamWriter]::new($archive.CreateEntry($name).Open())
        $writer.Write($parts[$name])
        $writer.Dispose()
      }
      $entry = $archive.CreateEntry('xl/workbook.xml').Open()
      $entry.Write($workbookXml, 0, $workbookXml.Length)
      $entry.Dispose()
      $entry = $archive.CreateEntry('xl/vbaProject.bin').Open()
      $vba.Position = 0
      $vba.CopyTo($entry)
      $entry.Dispose()
    } finally { $archive.Dispose() }
    $notes.Add("workbook package written by ZipArchive (System.IO.Packaging: $($_.Exception.Message))")
  }

  # Access database created by Windows' own Jet engine (32-bit only), where the provider exists.
  $jetPath = Join-Path $scan 'windows-jet.mdb'
  $powershell32 = Join-Path $env:WINDIR 'SysWOW64\WindowsPowerShell\v1.0\powershell.exe'
  $jetMade = $false
  if (Test-Path $powershell32) {
    $jetOutput = & $powershell32 -NoProfile -NonInteractive -Command "`$c = New-Object -ComObject ADOX.Catalog; `$c.Create('Provider=Microsoft.Jet.OLEDB.4.0;Data Source=$jetPath') | Out-Null; `$c.ActiveConnection.Execute('CREATE TABLE Orders (Id INTEGER, Customer TEXT(50))') | Out-Null; `$c.ActiveConnection.Close()" 2>&1
    $jetMade = Test-Path $jetPath
    if ($jetMade) { $notes.Add('Access database created by the Jet engine (ADOX)') } else { $notes.Add("Jet provider not available: $jetOutput") }
  } else {
    $notes.Add('no 32-bit PowerShell for the Jet provider')
  }

  # Script encoded by Microsoft's Script Encoder, if the object exists on this Windows.
  $encoded = $false
  try {
    $encoder = New-Object -ComObject Scripting.Encoder
    $plain = "Set net = CreateObject(`"WScript.Network`")`r`nstrPassword = `"EncodedSecret42`"`r`nnet.MapNetworkDrive `"Z:`", `"\\fs01\share`"`r`n"
    $text = $encoder.EncodeScriptFile('.vbs', $plain, 0, '')
    [IO.File]::WriteAllText((Join-Path $scan 'encoded.vbe'), $text, [Text.Encoding]::ASCII)
    $encoded = $true
  } catch {
    $notes.Add("Scripting.Encoder not available: $($_.Exception.Message)")
  }

  # VBScript deprecation alert, if this build logs it.
  Set-Content -LiteralPath (Join-Path $base 'hello.vbs') -Value 'WScript.Echo "hello"' -Encoding ascii
  $vbscriptWorks = $true
  $output = & cscript.exe //nologo (Join-Path $base 'hello.vbs') 2>&1
  if ($LASTEXITCODE -ne 0) { $vbscriptWorks = $false; $notes.Add("cscript could not run VBScript: $output") }
  Start-Sleep -Seconds 3
  $alerts = @()
  try {
    $alerts = @(Get-WinEvent -FilterHashtable @{ LogName = 'Application'; ProviderName = 'VBScriptDeprecationAlert'; Id = 4096 } -MaxEvents 5 -ErrorAction Stop)
  } catch { }
  if (-not $alerts.Count) { $notes.Add('this Windows build logged no VBScriptDeprecationAlert (4096) event') }

  # Run the collector: all system sources, files of the test folder only.
  $result = Join-Path $OutDir "systemtest.$((Get-Content -Raw (Join-Path $PSScriptRoot '..\..\product.json') | ConvertFrom-Json).resultFile.extension)"
  Remove-Item -LiteralPath $result -ErrorAction SilentlyContinue
  & $exePath --path $scan --out $result --quiet
  if ($LASTEXITCODE -ne 0) { throw "collector exited with $LASTEXITCODE" }

  Add-Type -AssemblyName System.IO.Compression.FileSystem
  $zip = [IO.Compression.ZipFile]::OpenRead($result)
  try {
    $reader = [IO.StreamReader]::new($zip.GetEntry('result.json').Open())
    $json = $reader.ReadToEnd()
    $reader.Dispose()
  } finally { $zip.Dispose() }
  Set-Content -LiteralPath (Join-Path $OutDir 'systemtest-result.json') -Value $json -Encoding utf8
  $scanResult = $json | ConvertFrom-Json
  $findings = @($scanResult.findings)

  $failures = [System.Collections.Generic.List[string]]::new()
  function Expect([string] $label, [scriptblock] $match) {
    if (-not ($findings | Where-Object $match)) { $failures.Add("missing: $label") }
  }
  Expect 'scheduled task (VBS-301)' { $_.rule -eq 'VBS-301' -and $_.location.path -eq '\VBScoutSystemTest\Nightly' }
  Expect 'Run value (VBS-311)' { $_.rule -eq 'VBS-311' -and $_.location.item -eq 'VBScoutSystemTest' }
  Expect 'service wrapper (VBS-321)' { $_.rule -eq 'VBS-321' -and $_.location.path -eq $serviceName }
  Expect 'WMI subscription (VBS-331, bound)' { $_.rule -eq 'VBS-331' -and $_.location.path -like "*$consumerName*" -and $_.activation -eq 'automatic' }
  Expect 'Run value in the hive file of a user who is not logged on (VBS-311)' { $_.rule -eq 'VBS-311' -and $_.location.path -like "HKU\$offlineSid\*" -and $_.location.item -eq 'VBScoutOffline' }
  Expect 'shell-made shortcut (VBS-211)' { $_.rule -eq 'VBS-211' -and $_.location.path -like '*Tool.lnk' -and $_.target -like '*tool.vbs' }
  Expect 'msi.dll-made package (VBS-401)' { $_.rule -eq 'VBS-401' -and $_.location.path -like '*windows-made.msi' -and $_.location.item -eq 'SetDefaults' }
  if ($encoded) {
    Expect 'encoded script, decoded (VBS-102)' { $_.rule -eq 'VBS-102' -and ($_.evidence | Where-Object { $_.text -like '*MapNetworkDrive*' }) }
    Expect 'secret in the encoded script (VBS-901)' { $_.rule -eq 'VBS-901' -and $_.location.path -like '*encoded.vbe' }
    if ($json -like '*EncodedSecret42*') { $failures.Add('the encoded script''s secret appears in the result file') }
  }
  if ($alerts.Count) { Expect 'deprecation alert (VBS-501)' { $_.rule -eq 'VBS-501' } }
  Expect 'System.IO.Packaging workbook with VBScript RegExp (VBS-601)' { $_.rule -eq 'VBS-601' -and $_.location.path -like '*windows-packaged.xlsm' -and $_.location.item -eq 'Module1' }
  if ($jetMade) {
    $jetFindings = @($findings | Where-Object { $_.location.path -like '*windows-jet.mdb' })
    if ($jetFindings.Count) { $failures.Add("Jet-made database: unexpected findings $(($jetFindings | ForEach-Object { "$($_.rule)/$($_.reason)" }) -join ', ')") }
  }
  $internal = @($findings | Where-Object { $_.reason -eq 'internalError' })
  if ($internal.Count) { $failures.Add("internal errors: $(($internal | ForEach-Object { $_.location.path }) -join ', ')") }
  $failedSources = @($scanResult.coverage.sources | Where-Object { $_.status -eq 'failed' })

  $report = @(
    "System test (Windows) - $((Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ'))"
    "collector:        $exePath"
    "VBScript runs:    $vbscriptWorks"
    "findings:         $($findings.Count)"
    "by rule:          $(($findings | Group-Object rule | Sort-Object Name | ForEach-Object { "$($_.Name)=$($_.Count)" }) -join ', ')"
    "sources:          $(($scanResult.coverage.sources | ForEach-Object { "$($_.source)=$($_.status)$(if ($_.reason) { "($($_.reason))" })" }) -join ', ')"
    "failed sources:   $($failedSources.Count)"
    "notes:            $($notes -join ' | ')"
  )
  if ($failures.Count) { $report += ''; $report += 'Failures:'; $report += $failures }
  $report | Tee-Object -FilePath (Join-Path $OutDir 'systemtest-windows.txt')
  if ($failures.Count) { Write-Error 'FAIL: expected findings are missing'; exit 1 }
  Write-Output 'PASS: the collector found every dependency Windows created'
}
finally {
  Remove-Artifacts
  Remove-Item -LiteralPath $base -Recurse -Force -ErrorAction SilentlyContinue
}
