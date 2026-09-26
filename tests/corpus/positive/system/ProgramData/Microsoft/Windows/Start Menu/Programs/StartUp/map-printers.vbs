' Connects the floor printers for every user
Set net = CreateObject("WScript.Network")
net.AddWindowsPrinterConnection "\\print01\floor2"
