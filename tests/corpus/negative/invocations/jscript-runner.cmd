@if (@X)==(@Y) @end /* batch/JScript hybrid
@cscript //nologo //E:JScript "%~f0" %*
@wscript C:\Tools\clean.js
@exit /b
*/
WScript.Echo("JScript only");
