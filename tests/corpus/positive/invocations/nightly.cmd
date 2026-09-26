@echo off
rem Nightly maintenance - calls the legacy VBScript
cscript //nologo "%~dp0..\scripts\backup.vbs"
if errorlevel 1 echo Backup failed
