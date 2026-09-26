' Nightly backup of the finance share (legacy)
Option Explicit
Dim fso, shell, strPassword
Set fso = CreateObject("Scripting.FileSystemObject")
Set shell = CreateObject("WScript.Shell")
strPassword = "Sommer2024!"
shell.Run "robocopy \\fs01\finance D:\Backup\finance /MIR", 0, True
WScript.Echo "done"
