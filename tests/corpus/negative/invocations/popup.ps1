# Uses the Windows Script Host object model (wshom), not the VBScript engine
$ws = New-Object -ComObject WScript.Shell
$null = $ws.Popup("Backup finished", 5, "Backup", 64)
Get-Content C:\Logs\cscript.txt | Select-Object -First 5
