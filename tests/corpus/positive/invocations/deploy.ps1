# Deployment helper
$shell = New-Object -ComObject WScript.Shell
$shell.Run("wscript.exe //B C:\Deploy\post-install.vbs", 0, $true)
Write-Output "done"
