' Maps the department drives at logon
Set net = CreateObject("WScript.Network")
net.MapNetworkDrive "S:", "\\fs01\shared"
