@echo off
rem cscript //nologo old.vbs is no longer used
echo Copying scripts - do not run cscript here
copy \\srv\share\logon.vbs C:\Temp\
del /q C:\Temp\old.vbs
if exist C:\Temp\logon.vbs del C:\Temp\logon.vbs
