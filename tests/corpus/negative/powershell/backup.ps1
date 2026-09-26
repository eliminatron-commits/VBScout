# Backup without VBScript
$source = "C:\Data"
Copy-Item -Path $source -Destination "D:\Backup" -Recurse
Write-Output "done"
