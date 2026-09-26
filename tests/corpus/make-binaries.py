#!/usr/bin/env python3
"""Creates the binary and specially encoded files of the test collections.

The files are committed; run this script only to recreate or change them. Shortcuts and
installer packages are written by independent implementations – pylnk3 and msitools
(libmsi) – so the collector's readers are tested against files it did not write itself.
Office documents and Access databases: `office_fixtures.py` (real files made by Office, and
VBA projects placed by Apache POI and Jackcess, read back with oletools and Apache POI).
The Windows CI job additionally checks shortcuts and packages made by Windows itself
(scripts/systemtest/windows.ps1).

Requirements: python3, msitools (`msibuild`, `msiinfo`), hivex (`hivexsh`), pylnk3 and olefile
(`pip install pylnk3 olefile`); for the Office files also Java 17+, mdbtools, oletools and
msoffcrypto-tool (see office_fixtures.py).
Registry hives start from hivex's empty test hive ("images/minimal", downloaded when needed, or
given with VBS_MINIMAL_HIVE=<file>); hivexsh adds the keys and values.
Usage: python3 tests/corpus/make-binaries.py
"""

import os
import re
import shutil
import subprocess
import tempfile

ROOT = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(ROOT))


def path(*parts):
    full = os.path.join(ROOT, *parts)
    os.makedirs(os.path.dirname(full), exist_ok=True)
    return full


def crlf(text):
    return text.replace('\r\n', '\n').replace('\n', '\r\n')


def utf16(name, text):
    with open(path(*name.split('/')), 'wb') as f:
        f.write(b'\xff\xfe' + crlf(text).encode('utf-16-le'))


# --- encoded VBScript (.vbe): the inverse of the collector's decoder tables ----------------

def encoder():
    source = open(os.path.join(REPO, 'crates/vbs-collector/src/analysis/vbe.rs'), encoding='utf-8').read()
    table = source[source.index('const DECODE'):source.index('const PICK')]
    rows = [[int(x, 16) for x in re.findall(r'0x([0-9A-F]{2})', row)] for row in re.findall(r'\[(0x[^\]]*)\]', table)]
    pick_text = source[source.index('const PICK'):]
    pick_text = pick_text[pick_text.index('= [') + 3:]
    pick = [int(x) for x in re.findall(r'\b([012])\b', pick_text[:pick_text.index(']')])]
    assert len(rows) == 128 and len(pick) == 64

    def encode(plain):
        out = []
        for index, c in enumerate(plain):
            escapes = {'\n': '@&', '\r': '@#', '<': '@!', '>': '@*', '@': '@$'}
            if c in escapes:
                out.append(escapes[c])
                continue
            k = pick[index % 64]
            candidates = [e for e in range(9, 128) if (e == 9 or e > 31) and chr(e) not in '<>@' and rows[e][k] == ord(c)]
            out.append(chr(candidates[0]))
        return '#@~^AAAAAA==' + ''.join(out) + 'AAAAAA==^#~@'
    return encode


def vbe():
    plain = crlf('Set wmi = GetObject("winmgmts:\\\\.\\root\\cimv2")\n'
                 'For Each os In wmi.ExecQuery("SELECT * FROM Win32_OperatingSystem")\n'
                 '  WScript.Echo os.Caption\n'
                 'Next\n')
    with open(path('positive', 'scripts', 'inventory.vbe'), 'wb') as f:
        f.write(encoder()(plain).encode('ascii'))


# --- shortcuts (pylnk3) ----------------------------------------------------------------

def shortcut(name, target, arguments=None, work_dir=None):
    import pylnk3
    pylnk3.for_file(target, path(*name.split('/')), arguments=arguments, work_dir=work_dir)


def shortcuts():
    shortcut('positive/shortcuts/Inventory.lnk', r'C:\Windows\System32\wscript.exe', r'"C:\Scripts\inventory.vbs"', r'C:\Scripts')
    shortcut('positive/shortcuts/Monthly Report.lnk', r'C:\Reports\monthly.vbs')
    shortcut('positive/shortcuts/Setup Wizard.lnk', r'C:\Windows\System32\mshta.exe', r'C:\Tools\wizard.hta')
    shortcut('positive/system/ProgramData/Microsoft/Windows/Start Menu/Programs/StartUp/Helpdesk.lnk',
             r'C:\Windows\System32\wscript.exe', r'//B "C:\Helpdesk\agent.vbs"')
    with open(path('positive', 'shortcuts', 'broken.lnk'), 'wb') as f:
        f.write(b'\x4c\x00\x00\x00\x01\x14\x02\x00\x00\x00')  # header cut off
    shortcut('negative/shortcuts/Edit Script.lnk', r'C:\Windows\System32\notepad.exe', r'C:\Scripts\backup.vbs')
    shortcut('negative/shortcuts/Cleanup JS.lnk', r'C:\Windows\System32\wscript.exe', r'C:\Tools\clean.js')
    shortcut('negative/shortcuts/Command Prompt.lnk', r'C:\Windows\System32\cmd.exe')
    shortcut('negative/system/Users/bob/AppData/Roaming/Microsoft/Windows/Start Menu/Programs/Startup/Notepad.lnk',
             r'C:\Windows\System32\notepad.exe')


# --- installer packages (msitools) --------------------------------------------------------

def idt(folder, name, columns, types, keys, rows):
    with open(os.path.join(folder, f'{name}.idt'), 'w', newline='') as f:
        f.write('\t'.join(columns) + '\r\n' + '\t'.join(types) + '\r\n' + '\t'.join([name] + keys) + '\r\n')
        for row in rows:
            f.write('\t'.join(row) + '\r\n')


def windows_long_strings(package_file):
    """Converts the string pool entries of strings of 64 KiB or more to the layout of Windows
    Installer: (0, reference count), then the length as (low word, high word) - as Wine reads and
    writes them (dlls/msi/string.c). msitools writes (0, high word), (low word, reference count)
    and cannot read that back itself ("string table load failed")."""
    import olefile
    import struct

    def table_name(name):
        chars = '0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz._'
        out = ''
        for c in name[1:] if name and ord(name[0]) == 0x4840 else name:
            code = ord(c)
            if 0x3800 <= code < 0x4800:
                out += chars[(code - 0x3800) & 0x3F] + chars[((code - 0x3800) >> 6) & 0x3F]
            elif 0x4800 <= code < 0x4840:
                out += chars[code - 0x4800]
            else:
                out += c
        return out

    ole = olefile.OleFileIO(package_file, write_mode=True)
    try:
        entry = next(e for e in ole.listdir() if len(e) == 1 and table_name(e[0]) == '_StringPool')
        words = list(struct.unpack(f'<{ole.get_size(entry) // 2}H', ole.openstream(entry).read()))
        i = 2
        while i + 3 < len(words):
            if words[i] == 0 and words[i + 1] != 0:
                words[i + 1], words[i + 3] = words[i + 3], words[i + 1]
                i += 4
            else:
                i += 2
        ole.write_stream(entry, struct.pack(f'<{len(words)}H', *words))
    finally:
        ole.close()


def package(name, product, product_code, actions, sequence, binaries, properties=()):
    target = path(*name.split('/'))
    with tempfile.TemporaryDirectory() as work:
        os.makedirs(os.path.join(work, 'Binary'))
        for key, content in binaries.items():
            with open(os.path.join(work, 'Binary', f'{key}.ibd'), 'wb') as f:
                f.write(crlf(content).encode('utf-8'))
        idt(work, 'Property', ['Property', 'Value'], ['s72', 'l0'], ['Property'], [
            ['ProductName', product], ['ProductCode', product_code], ['ProductVersion', '3.2.0'],
            ['Manufacturer', 'Contoso'], ['ProductLanguage', '1033'], *properties,
        ])
        idt(work, 'Binary', ['Name', 'Data'], ['s72', 'v0'], ['Name'], [[key, f'{key}.ibd'] for key in binaries])
        idt(work, 'CustomAction', ['Action', 'Type', 'Source', 'Target'], ['s72', 'i2', 'S72', 'S255'], ['Action'], actions)
        idt(work, 'InstallExecuteSequence', ['Action', 'Condition', 'Sequence'], ['s72', 'S255', 'I2'], ['Action'], sequence)
        package_file = os.path.join(work, 'package.msi')
        subprocess.run(['msibuild', package_file, '-s', product, 'Contoso', ';1033', product_code], check=True)
        tables = ['Property', 'Binary', 'CustomAction', 'InstallExecuteSequence']
        subprocess.run(['msibuild', package_file] + [arg for t in tables for arg in ('-i', os.path.join(work, f'{t}.idt'))], check=True, cwd=work)
        if any(len(value) >= 0x10000 for _, value in properties):
            windows_long_strings(package_file)
            # msitools' own reader (the Windows layout) must now read every value in full.
            check = subprocess.run(['msiinfo', 'export', package_file, 'Property'], capture_output=True, text=True, check=True)
            lengths = {line.split('\t')[0]: len(line.split('\t')[1]) for line in check.stdout.splitlines() if '\t' in line}
            assert 'string table load failed' not in check.stderr, check.stderr
            assert all(lengths.get(key) == len(value) for key, value in properties), lengths
        shutil.copyfile(package_file, target)


LICENSE_SCRIPT = '''Function Main()
  Set shell = CreateObject("WScript.Shell")
  strPassword = "Sommer2024!"
  shell.RegWrite "HKLM\\Software\\Contoso\\Licensed", 1, "REG_DWORD"
  Main = 1
End Function

Function Cleanup()
  Cleanup = 1
End Function
'''
JS_SCRIPT = 'function main() { return 1; }\n'


def packages():
    package('positive/installer/legacy-inventory.msi', 'Legacy Inventory', '{3E5C1F6A-9B2D-4C8E-A7F1-0D2B4C6E8A10}', [
        ['CheckLicense', '6', 'LicenseScript', 'Main'],
        ['SetDefaults', '38', '', 'Set shell = CreateObject("WScript.Shell") : shell.RegWrite "HKCU\\Software\\Contoso\\Mode", "legacy"'],
        ['CleanupOnRemove', '70', 'LicenseScript', 'Cleanup'],
        ['JsHelper', '5', 'JsScript', 'main'],
        ['InstalledScript', '22', 'helper.vbs', ''],
    ], [
        ['CheckLicense', 'NOT Installed', '1001'],
        ['SetDefaults', '', '1002'],
        ['CleanupOnRemove', 'REMOVE="ALL"', '1003'],
        ['JsHelper', '', '1004'],
    ], {'LicenseScript': LICENSE_SCRIPT, 'JsScript': JS_SCRIPT})
    package('positive/system/Windows/Installer/2f4a1c.msi', 'Helpdesk Agent', '{0B1E2F7B-3F4A-1D2E-1C2B-3A4958677685}', [
        ['RegisterAgent', '38', '', 'Set shell = CreateObject("WScript.Shell") : shell.Run "sc config HelpdeskAgent start= auto"'],
    ], [['RegisterAgent', '', '1001']], {})
    package('negative/installer/jscript-only.msi', 'JScript Tool', '{6A7B8C9D-0E1F-4A2B-8C3D-4E5F6A7B8C9D}', [
        ['JsHelper', '5', 'JsScript', 'main'],
        ['JsInline', '37', '', 'var x = 1;'],
        ['NativeHelper', '1', 'JsScript', 'Entry'],
    ], [['JsHelper', '', '1001'], ['JsInline', '', '1002']], {'JsScript': JS_SCRIPT})
    # A string of 128 KiB or more: its pool entry is (0, reference count) followed by its length
    # as (low, high) word - high word 2, so a reader that took the reference count (1) for the
    # high word would misread every later string.
    eula = ('This licence text is long. ' * 5200)[:140_000]
    package('positive/installer/long-strings.msi', 'Long Strings', '{5D2C1B0A-9F8E-4D7C-B6A5-948372615049}', [
        ['ShowNotice', '38', '', 'MsgBox "Contoso Long Strings"'],
    ], [['ShowNotice', '', '1001']], {}, [['EulaText', eula]])
    shutil.copyfile(path('negative', 'installer', 'jscript-only.msi'), path('negative', 'system', 'Windows', 'Installer', '7c9e2d.msi'))
    with open(path('positive', 'installer', 'broken.msi'), 'wb') as f:
        f.write(b'\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1' + b'\x00' * 40)  # compound file header, nothing else


# --- registry hives of profiles that are not logged on (hivex) ---------------------------------

MINIMAL_HIVE_URL = 'https://raw.githubusercontent.com/libguestfs/hivex/master/images/minimal'


def minimal_hive(work):
    given = os.environ.get('VBS_MINIMAL_HIVE')
    if given:
        return given
    target = os.path.join(work, 'minimal')
    import urllib.request
    urllib.request.urlretrieve(MINIMAL_HIVE_URL, target)
    return target


def hive(name, keys):
    """keys: {r'Software\\Microsoft\\…': [(value name, 'string:…' | 'expandstring:…' | 'dword:0x…'), …]}"""
    with tempfile.TemporaryDirectory() as work:
        file = os.path.join(work, 'NTUSER.DAT')
        shutil.copyfile(minimal_hive(work), file)
        script = []
        created = set()
        for key, values in keys.items():
            script.append('cd \\')
            path = []
            for part in key.split('\\'):
                path.append(part.lower())
                if tuple(path) not in created:
                    script.append(f'add {part}')
                    created.add(tuple(path))
                script.append(f'cd {part}')
            script.append(f'setval {len(values)}')
            for value_name, value in values:
                script += [value_name, value]
        script.append('commit')
        subprocess.run(['hivexsh', '-w', file], input='\n'.join(script) + '\n', text=True, check=True)
        shutil.copyfile(file, path_of(name))


def path_of(name):
    return path(*name.split('/'))


def hives():
    hive('positive/system/Users/carol/NTUSER.DAT', {
        r'Software\Microsoft\Windows\CurrentVersion\Run': [
            ('LegacyOffline', r'string:wscript.exe //B C:\Offline\offline-agent.vbs'),
            ('Tray', r'expandstring:"%ProgramFiles%\Tray\tray.exe" /min'),
        ],
        r'Environment': [('UserInitMprLogonScript', r'string:cscript //B C:\Login\carol-logon.vbs')],
    })
    hive('negative/system/Users/dave/NTUSER.DAT', {
        r'Software\Microsoft\Windows\CurrentVersion\Run': [
            ('OneDrive', r'string:"C:\Users\dave\AppData\Local\Microsoft\OneDrive\OneDrive.exe" /background'),
        ],
        r'Environment': [('TEMP', r'expandstring:%USERPROFILE%\AppData\Local\Temp')],
    })


# --- UTF-16 files as Windows writes them --------------------------------------------------

def utf16_files():
    utf16('positive/scripts/report-utf16.vbs', "' Monthly report (saved as Unicode)\nSet xl = CreateObject(\"Excel.Application\")\nxl.Visible = False\n")
    utf16('positive/policies/GroupPolicy/User/Scripts/scripts.ini',
          '\n[Logon]\n0CmdLine=map-drives.vbs\n0Parameters=/silent\n1CmdLine=printers.cmd\n1Parameters=\n'
          '[Logoff]\n0CmdLine=cleanup.wsf\n0Parameters=\n')
    utf16('negative/policies/GroupPolicy/User/Scripts/psscripts.ini', '\n[Logon]\n0CmdLine=logon.ps1\n0Parameters=\n')
    utf16('positive/system/Windows/System32/Tasks/Contoso/Nightly Backup', '''<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.4" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo><URI>\\Contoso\\Nightly Backup</URI></RegistrationInfo>
  <Triggers>
    <CalendarTrigger><StartBoundary>2024-01-01T02:00:00</StartBoundary><Enabled>true</Enabled><ScheduleByDay><DaysInterval>1</DaysInterval></ScheduleByDay></CalendarTrigger>
  </Triggers>
  <Settings><Enabled>true</Enabled></Settings>
  <Actions Context="Author">
    <Exec>
      <Command>C:\\Windows\\System32\\wscript.exe</Command>
      <Arguments>//B "C:\\Scripts\\backup.vbs" /user:svc_backup /password:Sommer2024!</Arguments>
    </Exec>
  </Actions>
</Task>
''')


# --- Windows' own data under the names of scripts, shortcuts, packages and databases ------
# Seen on real Windows (CI full scans): differentials of the component store (WinSxS\…\f\, r\,
# n\; MSDelta "PA30" behind a CRC-32), compressed payloads of components that are not installed
# ("DCS"/"DCN", version 1) and ESE databases named .mdb (User Access Logging). Microsoft's files
# cannot be redistributed, so these are synthetic: the documented headers, deterministic bodies.

def windows_data():
    import struct
    import zlib

    def body(seed, size):
        return bytes((seed * 131 + n * 197 + (n >> 3)) & 0xFF for n in range(size))

    def differential(seed):
        delta = b'PA30' + struct.pack('<Q', 0x01DB2F3C4A5B6C7D + seed) + body(seed, 180)
        return struct.pack('<I', zlib.crc32(delta)) + delta

    def payload(kind, seed):
        return b'DC' + kind + b'\x01' + struct.pack('<II', 1, 4096) + body(seed, 240)

    sxs = 'negative/system/Windows/WinSxS/'
    files = {
        sxs + 'amd64_microsoft-windows-security-spp-tools_31bf3856ad364e35_10.0.26100.1_none_91938b3e66db829b/r/slmgr.vbs':
            differential(1),
        sxs + 'amd64_microsoft.windows.powershell.common_31bf3856ad364e35_10.0.26100.1_none_7cdc2287c2f1d46b/r/Windows PowerShell.lnk':
            differential(2),
        sxs + 'amd64_microsoft-windows-winrm-winrscmd_31bf3856ad364e35_10.0.26100.1_none_1c3e8a4f0b2d6e7a/f/winrm.cmd':
            differential(3),
        sxs + 'amd64_microsoft.powershell.dsc.pullserver_31bf3856ad364e35_10.0.26100.1_none_e25df1b635c8451d/Devices.mdb':
            payload(b'S', 4),
        sxs + 'amd64_microsoft-windows-example-setup_31bf3856ad364e35_10.0.26100.1_none_0d4c8b2a6e1f3957/setup.msi':
            payload(b'N', 5),
        # ESE database header: checksum, signature 0x89ABCDEF, format version 0x620.
        'negative/system/Windows/System32/LogFiles/Sum/SystemIdentity.mdb':
            struct.pack('<IIII', 0x9E1F5A3C, 0x89ABCDEF, 0x620, 0) + bytes(8192 - 16),
    }
    for name, data in files.items():
        with open(path(*name.split('/')), 'wb') as f:
            f.write(data)


if __name__ == '__main__':
    import office_fixtures
    vbe()
    shortcuts()
    packages()
    hives()
    utf16_files()
    windows_data()
    office_fixtures.generate()
    print('binary fixtures written')
