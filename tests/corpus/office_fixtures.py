"""Office documents and Access databases with VBA macros for the test collections.

* Real files made by Office (Apache POI, oletools and Jackcess test data) are downloaded with a
  pinned SHA-256 into `negative/office/real/` (no VBScript) and `positive/office/real/` (macro
  documents that need a password to open). Their origin and licence: `THIRD-PARTY.md`.
* Generated cases: the VBA project streams ([MS-OVBA]: compressed `dir` stream, `PROJECT`,
  `PROJECTwm`, `_VBA_PROJECT`, module streams) are written here; the containers by independent
  libraries - compound files by Apache POI (POIFS), Access databases by Jackcess (starting from the
  empty databases Access made that ship with it), packages by zipfile starting from real Office
  packages, password encryption by msoffcrypto-tool. `tools/OfficeFixtures.java` does the POI and
  Jackcess part.
* Every generated project is read back with oletools (olevba) and Apache POI's VBAMacroReader;
  both must return each module's source unchanged, or generation fails.

Requirements (in addition to make-binaries.py): Java 17+, oletools and msoffcrypto-tool
(`pip install oletools msoffcrypto-tool`), mdbtools (`mdb-tables`); the Java libraries are
downloaded from Maven Central (pinned SHA-256) into VBS_JARS or ~/.cache/vbscout-corpus/jars.
"""

import hashlib
import io
import os
import shutil
import struct
import subprocess
import tempfile
import urllib.request
import zipfile

ROOT = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(ROOT, 'tools', 'OfficeFixtures.java')

MAVEN = ['https://repo1.maven.org/maven2/', 'https://maven-central.storage-download.googleapis.com/maven2/']
JARS = {
    'jackcess-5.0.2.jar': ('com/healthmarketscience/jackcess/jackcess/5.0.2/jackcess-5.0.2.jar',
                           'f8ebe0fe664fd12e679dece335872f00f7b0458d230545e108ad5a3d19011e8c'),
    'poi-5.5.1.jar': ('org/apache/poi/poi/5.5.1/poi-5.5.1.jar',
                      '6c52e876ca75775a11b56e4b36a7541f682827f56406725fcd87560b792ee3d8'),
    'commons-io-2.21.0.jar': ('commons-io/commons-io/2.21.0/commons-io-2.21.0.jar',
                              '7d643a2afea8b058b762aa6fb90e5b256f6c729739f8b3784c3370ddc609e88d'),
    'commons-collections4-4.5.0.jar': ('org/apache/commons/commons-collections4/4.5.0/commons-collections4-4.5.0.jar',
                                       '00f93263c267be201b8ae521b44a7137271b16688435340bf629db1bac0a5845'),
    'commons-codec-1.20.0.jar': ('commons-codec/commons-codec/1.20.0/commons-codec-1.20.0.jar',
                                 '6af66595f9f6a7bb58ce66518d6888d40b547c366d2262f06676eee19528ff66'),
    'commons-math3-3.6.1.jar': ('org/apache/commons/commons-math3/3.6.1/commons-math3-3.6.1.jar',
                                '1e56d7b058d28b65abd256b8458e3885b674c1d588fa43cd7d1cbb9c7ef2b308'),
    'SparseBitSet-1.3.jar': ('com/zaxxer/SparseBitSet/1.3/SparseBitSet-1.3.jar',
                             'f76b85adb0c00721ae267b7cfde4da7f71d3121cc2160c9fc00c0c89f8c53c8a'),
    'log4j-api-2.24.3.jar': ('org/apache/logging/log4j/log4j-api/2.24.3/log4j-api-2.24.3.jar',
                             '5b4a0a0cd0e751ded431c162442bdbdd53328d1f8bb2bae5fc1bbeee0f66d80f'),
}

POI = 'https://raw.githubusercontent.com/apache/poi/trunk/test-data/'
OLETOOLS = 'https://raw.githubusercontent.com/decalage2/oletools/master/tests/test-data/'
JACKCESS = 'https://raw.githubusercontent.com/jahlborn/jackcess/master/src/test/data/'
REAL = [
    # (collection path, source URL, SHA-256)
    ('negative/office/real/poi-SimpleMacro.xls', POI + 'spreadsheet/SimpleMacro.xls',
     '0e92c9bb018abd8a5f9121d65827c9e3bd280777219cb77a2efd70635143c00a'),
    ('negative/office/real/poi-SimpleMacro.xlsm', POI + 'spreadsheet/SimpleMacro.xlsm',
     'f76c986f4ebc25c2cc57c088b2511a1269f4bd61d6223a2ab58db351da348ba6'),
    ('negative/office/real/poi-SimpleMacro.doc', POI + 'document/SimpleMacro.doc',
     '39e9608c711d38f298ed20a5bfae12fcdaed6f4be2be4d26db670475216ca393'),
    ('negative/office/real/poi-SimpleMacro.docm', POI + 'document/SimpleMacro.docm',
     'fd591958fcf5322f72c0a740e9606309c949254bda4c3d9bd966481ddf220563'),
    ('negative/office/real/poi-SimpleMacro.pptm', POI + 'slideshow/SimpleMacro.pptm',
     '8a3573c82fd07a301d7f175b8bee646c0b58eb8a945bd0ff408225b6e2b89b15'),
    ('negative/office/real/poi-60158.docm', POI + 'document/60158.docm',
     '51377b41d9843b9418ef27be036fdbe74fa13c1dfa2b2a501191b1a24ca586b0'),
    ('negative/office/real/poi-60273-mac.xls', POI + 'spreadsheet/60273.xls',
     'd83108a03fff45b0a27cd58094da7d549513ff789fd31dee0f42a38c55cf577e'),
    ('negative/office/real/poi-60279-offset.doc', POI + 'document/60279.doc',
     'acd59256eb12abaa3c376e72d33601230edc19aaf343e16d714fe919e70224f9'),
    ('negative/office/real/poi-59830-modules.xls', POI + 'spreadsheet/59830.xls',
     '432dbf2d81510bdf42eb3fe736fe62fcebbb444d8e7bed16f2dfb047064ff74b'),
    ('negative/office/real/poi-Simple.xlsb', POI + 'spreadsheet/Simple.xlsb',
     '41b0c82bfa682f968d69e05e23137232d6c89ea4f24439786f0b779be319929b'),
    ('negative/office/real/poi-password.xls', POI + 'spreadsheet/password.xls',
     '3ad12a829132d9c157d213c623af9c534b1449c42e93ab39b17ca53c909ac412'),
    ('negative/office/real/poi-xor-encryption-abc.xls', POI + 'spreadsheet/xor-encryption-abc.xls',
     'c61c8714ae9cb77251235606cbfc84e5d289c7ce6bef940da57dd2518f3c90fa'),
    ('negative/office/real/oletools-encrypted.xls', OLETOOLS + 'encrypted/encrypted.xls',
     '0f237a64ff686766f47ec95d4cda2180ec40edc9ec5d625be4194b991eaacc9a'),
    ('negative/office/real/oletools-encrypted.doc', OLETOOLS + 'encrypted/encrypted.doc',
     '27448002fe365f2297bb76904181c1919b0ac8363be209f748d6b20e5a8b8cce'),
    ('negative/office/real/jackcess-testV1997.mdb', JACKCESS + 'V1997/testV1997.mdb',
     '27788b47e17412427830e80c2cb69366885aa11b12c61422de73029194bef135'),
    ('positive/office/real/oletools-encrypted.docm', OLETOOLS + 'encrypted/encrypted.docm',
     'bbcc01aa21de93addddfa7aaad07951fc7b1464a1d1575bde1b17393eee30b1f'),
    ('positive/office/real/oletools-encrypted.xlsm', OLETOOLS + 'encrypted/encrypted.xlsm',
     '05f3e35090741d53a4afcedc8e6614061ca7f1ec4dfb822eb4cd2cfe3fbd6fca'),
    ('positive/office/real/oletools-encrypted.pptm', OLETOOLS + 'encrypted/encrypted.pptm',
     'fd12f906e409b95c0836cac55a2fc1b3fa3884508aa308a002a53a6496e4e26a'),
    ('positive/office/real/oletools-encrypted.xlsb', OLETOOLS + 'encrypted/encrypted.xlsb',
     'c60629e25bf0eb24d208d64e1893dba02009d9dd990e23f587dc28f4d03d4aa1'),
]


def corpus(*parts):
    full = os.path.join(ROOT, *parts)
    os.makedirs(os.path.dirname(full), exist_ok=True)
    return full


def sha256(file):
    with open(file, 'rb') as f:
        return hashlib.sha256(f.read()).hexdigest()


def download(urls, target, digest):
    if os.path.exists(target) and sha256(target) == digest:
        return target
    last = None
    for url in urls:
        try:
            with urllib.request.urlopen(url, timeout=60) as response:
                data = response.read()
            if hashlib.sha256(data).hexdigest() != digest:
                raise ValueError(f'{url}: SHA-256 mismatch')
            with open(target, 'wb') as f:
                f.write(data)
            return target
        except Exception as error:  # try the next mirror
            last = error
    raise RuntimeError(f'cannot download {os.path.basename(target)}: {last}')


def real_files():
    for name, url, digest in REAL:
        download([url], corpus(*name.split('/')), digest)


def jars():
    folder = os.environ.get('VBS_JARS') or os.path.expanduser('~/.cache/vbscout-corpus/jars')
    os.makedirs(folder, exist_ok=True)
    for name, (path, digest) in JARS.items():
        download([mirror + path for mirror in MAVEN], os.path.join(folder, name), digest)
    return folder


def java(*args):
    result = subprocess.run(['java', '-cp', os.path.join(jars(), '*'), TOOL, *args],
                            capture_output=True, text=True)
    if result.returncode != 0:
        raise RuntimeError(f'OfficeFixtures {args[0]} failed:\n{result.stderr}')
    return result.stdout


# --- VBA project streams ([MS-OVBA]) ------------------------------------------------------------

def compress(data):
    """CompressedContainer (MS-OVBA 2.4.1.3.6): per 4096-byte chunk, flag bytes followed by literal
    and copy tokens; the longest earlier match in the chunk wins. A chunk that does not get smaller
    is stored raw."""
    out = bytearray(b'\x01')
    for start in range(0, len(data), 4096):
        chunk = data[start:start + 4096]
        body = bytearray()
        pos = 0
        while pos < len(chunk):
            flag_at = len(body)
            body.append(0)
            for bit in range(8):
                if pos >= len(chunk):
                    break
                bit_count = max((pos - 1).bit_length() if pos > 1 else 0, 4)
                max_length = (0xFFFF >> bit_count) + 3
                best_length, best_offset = 0, 0
                for candidate in range(pos - 1, -1, -1):
                    length = 0
                    while (length < max_length and pos + length < len(chunk)
                           and chunk[candidate + length] == chunk[pos + length]):
                        length += 1
                    if length > best_length:
                        best_length, best_offset = length, pos - candidate
                if best_length >= 3:
                    token = ((best_offset - 1) << (16 - bit_count)) | (best_length - 3)
                    body += struct.pack('<H', token)
                    body[flag_at] |= 1 << bit
                    pos += best_length
                else:
                    body.append(chunk[pos])
                    pos += 1
        if len(body) > 4096:
            out += struct.pack('<H', 0x3FFF) + chunk.ljust(4096, b'\0')
        else:
            out += struct.pack('<H', 0xB000 | (len(body) - 1)) + body
    return bytes(out)


def record(record_id, data):
    return struct.pack('<HI', record_id, len(data)) + data


def utf16(text):
    return text.encode('utf-16-le')


STDOLE = r'*\G{00020430-0000-0000-C000-000000000046}#2.0#0#C:\Windows\System32\stdole2.tlb#OLE Automation'
OFFICE = r'*\G{2DF8D04C-5BFA-101B-BDE5-00AA0044DE52}#2.0#0#C:\Program Files\Common Files\Microsoft Shared\OFFICE16\MSO.DLL#Microsoft Office 16.0 Object Library'
ADODB = r'*\G{B691E011-1797-432E-907A-4D8C69339129}#6.1#0#C:\Program Files\Common Files\System\ado\msado15.dll#Microsoft ActiveX Data Objects 6.1 Library'
REGEXP_55 = r'*\G{3F4DACA7-160D-11D2-A8E9-00104B365C9F}#5.5#0#C:\Windows\System32\vbscript.dll\3#Microsoft VBScript Regular Expressions 5.5'
SCRIPTING = r'*\G{420B2830-E718-11CF-893D-00A0C9054228}#1.0#0#C:\Windows\System32\scrrun.dll#Microsoft Scripting Runtime'


def dir_stream(name, modules, references, compat=True):
    out = record(0x0001, struct.pack('<I', 3))  # SysKind: 64-bit Windows
    if compat:
        out += record(0x004A, struct.pack('<I', 0x0000000A))
    out += record(0x0002, struct.pack('<I', 0x409)) + record(0x0014, struct.pack('<I', 0x409))
    out += record(0x0003, struct.pack('<H', 1252)) + record(0x0004, name.encode('cp1252'))
    out += record(0x0005, b'') + record(0x0040, b'') + record(0x0006, b'') + record(0x003D, b'')
    out += record(0x0007, struct.pack('<I', 0)) + record(0x0008, struct.pack('<I', 0))
    out += struct.pack('<HIIH', 0x0009, 4, 0x6C3FA5CF, 0x0011)  # PROJECTVERSION: reserved 4, major, minor
    out += record(0x000C, b'') + record(0x003C, b'')
    for ref_name, libid in references:
        out += record(0x0016, ref_name.encode('cp1252')) + record(0x003E, utf16(ref_name))
        encoded = libid.encode('cp1252')
        out += record(0x000D, struct.pack('<I', len(encoded)) + encoded + struct.pack('<IH', 0, 0))
    out += record(0x000F, struct.pack('<H', len(modules))) + record(0x0013, struct.pack('<H', 0xFFFF))
    for module in modules:
        out += record(0x0019, module['name'].encode('cp1252')) + record(0x0047, utf16(module['name']))
        out += record(0x001A, module['name'].encode('cp1252')) + record(0x0032, utf16(module['name']))
        out += record(0x001C, b'') + record(0x0048, b'')
        out += record(0x0031, struct.pack('<I', len(module['pcode'])))
        out += record(0x001E, struct.pack('<I', 0)) + record(0x002C, struct.pack('<H', 0xFFFF))
        out += record(0x0021 if module['kind'] == 'standard' else 0x0022, b'')
        out += record(0x002B, b'')
    out += record(0x0010, b'')
    return out


def encrypt(data, seed, project_key):
    """Data encryption of the PROJECT stream (MS-OVBA 2.4.3.2)."""
    version_enc, key_enc = seed ^ 2, seed ^ project_key
    out = bytearray([seed, version_enc, key_enc])
    unencrypted1, encrypted1, encrypted2 = project_key, key_enc, version_enc
    plain = bytes((seed & 6) // 2) + struct.pack('<I', len(data)) + data
    for byte in plain:
        byte_enc = byte ^ ((encrypted2 + unencrypted1) & 0xFF)
        out.append(byte_enc)
        encrypted2, encrypted1, unencrypted1 = encrypted1, byte_enc, byte
    return out.hex().upper()


def project_stream(project):
    project_id = project.get('id', '{5F1A7C3E-2B4D-4E6F-8A9B-0C1D2E3F4A5B}')
    key = sum(project_id.encode('ascii')) & 0xFF
    lines = [f'ID="{project_id}"']
    for module in project['modules']:
        lines.append({'standard': f'Module={module["name"]}', 'class': f'Class={module["name"]}'}.get(
            module['kind'], f'Document={module["name"]}/&H00000000'))
    lines += [f'Name="{project["name"]}"', 'HelpContextID="0"', 'VersionCompatible32="393222000"']
    locked = project.get('locked', False)
    password = bytes([0xFF, 0x60]) + bytes(range(0x21, 0x3A)) + b'\0' if locked else b'\0'  # 29 bytes when set
    lines += [f'CMG="{encrypt(struct.pack("<I", 1 if locked else 0), 0x0D, key)}"',
              f'DPB="{encrypt(password, 0x41, key)}"',
              f'GC="{encrypt(bytes([0xFF]), 0x16, key)}"', '',
              '[Host Extender Info]', '&H00000001={3832D640-CF90-11CF-8E43-00A0C911005A};VBE;&H00000000', '',
              '[Workspace]']
    lines += [f'{module["name"]}=0, 0, 0, 0, C' for module in project['modules']]
    return ('\r\n'.join(lines) + '\r\n').encode('cp1252')


def project_wm(modules):
    out = b''
    for module in modules:
        out += module['name'].encode('cp1252') + b'\0' + utf16(module['name']) + b'\0\0'
    return out + b'\0\0'


ATTRIBUTES = {
    'standard': [],
    'class': ['VB_Base = "0{FCFB3D2A-A0FA-1068-A738-08002B3371B5}"', 'VB_GlobalNameSpace = False',
              'VB_Creatable = False', 'VB_PredeclaredId = False', 'VB_Exposed = False'],
    'document': ['VB_Base = "0{00020819-0000-0000-C000-000000000046}"', 'VB_GlobalNameSpace = False',
                 'VB_Creatable = False', 'VB_PredeclaredId = True', 'VB_Exposed = True',
                 'VB_TemplateDerived = False', 'VB_Customizable = True'],
}


def module_source(module):
    header = [f'Attribute VB_Name = "{module["name"]}"'] + [f'Attribute {a}' for a in ATTRIBUTES[module['kind']]]
    code = module['code'].replace('\r\n', '\n').strip('\n').replace('\n', '\r\n')
    return '\r\n'.join(header) + '\r\n' + code + '\r\n'


def module(name, code, kind='standard', pcode=0):
    """A module; `pcode` bytes of (fake) compiled code come before the source, like Office writes it."""
    return {'name': name, 'code': code, 'kind': kind, 'pcode': bytes((i * 7 + 3) & 0xFF for i in range(pcode))}


def project_streams(project):
    """The streams of a project storage by path relative to it."""
    modules = project['modules']
    streams = {
        'PROJECT': project_stream(project),
        'PROJECTwm': project_wm(modules),
        'VBA/_VBA_PROJECT': bytes([0xCC, 0x61, 0xFF, 0xFF, 0x00, 0x00, 0x00]),
        'VBA/dir': compress(dir_stream(project['name'], modules, project.get('references', []))),
    }
    for m in modules:
        if m.get('stripped'):
            streams[f'VBA/{m["name"]}'] = m['pcode'] + bytes(64)  # compiled code only, no source
        else:
            streams[f'VBA/{m["name"]}'] = m['pcode'] + compress(module_source(m).encode('cp1252'))
    return streams


# --- containers -----------------------------------------------------------------------------------

def compound(out, streams, base=None, remove=(), work=None, storages=()):
    """A compound file written by Apache POI: a copy of `base` (if any) without the storages in
    `remove`, plus the (empty) `storages` and `streams` (path -> bytes)."""
    spec = []
    if base:
        spec.append(f'base\t{base}')
    for path in remove:
        spec.append(f'remove\t{path}')
    for path in storages:
        spec.append(f'storage\t{path}')
    for index, (path, data) in enumerate(sorted(streams.items())):
        file = os.path.join(work, f'stream{index}.bin')
        with open(file, 'wb') as f:
            f.write(data)
        spec.append(f'stream\t{path}\t{file}')
    spec_file = os.path.join(work, 'spec.txt')
    with open(spec_file, 'w') as f:
        f.write('\n'.join(spec) + '\n')
    java('cfb', out, spec_file)


def vba_project_bin(project, work):
    out = os.path.join(work, 'vbaProject.bin')
    compound(out, project_streams(project), work=work)
    with open(out, 'rb') as f:
        return f.read()


CONTENT_TYPES = {
    'xlsm': 'application/vnd.ms-excel.sheet.macroEnabled.main+xml',
    'xltm': 'application/vnd.ms-excel.template.macroEnabled.main+xml',
    'xlam': 'application/vnd.ms-excel.addin.macroEnabled.main+xml',
    'docm': 'application/vnd.ms-word.document.macroEnabled.main+xml',
    'dotm': 'application/vnd.ms-word.template.macroEnabledTemplate.main+xml',
    'pptm': 'application/vnd.ms-powerpoint.presentation.macroEnabled.main+xml',
    'ppsm': 'application/vnd.ms-powerpoint.slideshow.macroEnabled.main+xml',
    'potm': 'application/vnd.ms-powerpoint.template.macroEnabled.main+xml',
    'ppam': 'application/vnd.ms-powerpoint.addin.macroEnabled.main+xml',
}
MAIN_TYPES = [CONTENT_TYPES[k] for k in ('xlsm', 'docm', 'pptm')]


def package(out, base, project, work, extension=None, add_vba_part=None):
    """A copy of a real Office package with its vbaProject.bin replaced (or added, for .xlsb)."""
    vba = vba_project_bin(project, work)
    with zipfile.ZipFile(base) as source, zipfile.ZipFile(out, 'w', zipfile.ZIP_DEFLATED) as target:
        names = source.namelist()
        for name in names:
            data = source.read(name)
            if name.lower().endswith('vbaproject.bin'):
                data = vba
            elif name == '[Content_Types].xml':
                text = data.decode('utf-8')
                if extension in CONTENT_TYPES:
                    for main in MAIN_TYPES:
                        text = text.replace(main, CONTENT_TYPES[extension])
                if add_vba_part:
                    text = text.replace('</Types>', f'<Override PartName="/{add_vba_part}" '
                                        'ContentType="application/vnd.ms-office.vbaProject"/></Types>')
                data = text.encode('utf-8')
            elif add_vba_part and name == 'xl/_rels/workbook.bin.rels':
                data = data.decode('utf-8').replace('</Relationships>', '<Relationship Id="rIdVba" '
                    'Type="http://schemas.microsoft.com/office/2006/relationships/vbaProject" '
                    'Target="vbaProject.bin"/></Relationships>').encode('utf-8')
            info = zipfile.ZipInfo(name, date_time=(2026, 9, 26, 12, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            target.writestr(info, data)
        if add_vba_part:
            info = zipfile.ZipInfo(add_vba_part, date_time=(2026, 9, 26, 12, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            target.writestr(info, vba)


def binary_document(out, base, storage, project, work, extra=None):
    """A copy of a real .xls/.doc with its VBA storage replaced by `project`."""
    streams = {f'{storage}/{path}': data for path, data in project_streams(project).items()}
    streams.update(extra or {})
    compound(out, streams, base=base, remove=[storage], work=work)


def access(out, file_format, project, work, encode_key=None):
    """An Access database made by Jackcess (from its empty databases made by Access) with the
    project's streams below VBA/VBAProject; Access 2000 keeps them in the chunked compound file."""
    spec = []
    for index, (path, data) in enumerate(sorted(project_streams(project).items())):
        file = os.path.join(work, f'access{index}.bin')
        with open(file, 'wb') as f:
            f.write(data)
        spec.append(f'stream\tVBA/VBAProject/{path}\t{file}')
    spec_file = os.path.join(work, 'access.txt')
    with open(spec_file, 'w') as f:
        f.write('\n'.join(spec) + '\n')
    java('access', file_format, out, spec_file)
    if encode_key is not None:
        encode_jet(out, encode_key)


def access2000(out, project, work):
    """Access 2000 keeps the project in a compound file split into 3992-byte rows of
    MSysAccessObjects - a column type Jackcess reads but cannot write. So Jackcess creates the
    database and reads the rows, Apache POI writes the new compound file, and its chunks replace
    the old ones in place (the header row gets the new length); Jackcess and Apache POI then
    read the project back."""
    java('access', 'V2000', out, os.devnull)
    folder = os.path.join(work, 'objects')
    shutil.rmtree(folder, ignore_errors=True)
    java('objects', out, folder)
    rows = {int(name[4:-4]): open(os.path.join(folder, name), 'rb').read()
            for name in os.listdir(folder) if name.startswith('row-')}
    new_compound = os.path.join(work, 'objects.bin')
    # Written anew (POIFS would keep the space of replaced streams): the storages and streams of
    # the empty database, the project's streams in place of its empty project.
    import olefile
    template = olefile.OleFileIO(os.path.join(folder, 'compound.bin'))
    storages, streams = [], {}
    for entry in template.listdir(streams=True, storages=True):
        name = '/'.join(entry)
        if name.upper().startswith('VBA/VBAPROJECT'):
            continue
        if template.get_type(name) == olefile.STGTY_STORAGE:
            storages.append(name)
        else:
            streams[name] = template.openstream(name).read()
    template.close()
    streams.update({f'VBA/VBAProject/{path}': data for path, data in project_streams(project).items()})
    compound(new_compound, streams, storages=storages, work=work)
    new = open(new_compound, 'rb').read()
    chunks = sorted(k for k in rows if k > 0)
    capacity = 3992 * len(chunks)
    if len(new) > capacity:
        raise RuntimeError(f'Access 2000 project needs {len(new)} bytes, the rows hold {capacity}')
    with open(out, 'rb') as f:
        data = bytearray(f.read())

    def replace(old, replacement):
        at = data.find(old)
        if at < 0 or data.find(old, at + 1) >= 0:
            raise RuntimeError('MSysAccessObjects row not found exactly once')
        data[at:at + len(old)] = replacement

    padded = new.ljust(capacity, b'\0')
    for index, key in enumerate(chunks):
        replace(rows[key], padded[index * 3992:(index + 1) * 3992])
    header = bytearray(rows[0])
    header[4:8] = struct.pack('<I', len(new))
    replace(rows[0], bytes(header))
    with open(out, 'wb') as f:
        f.write(data)
    # Read back: Jackcess reassembles the rows, oletools reads the project from them. (Apache POI's
    # reader takes the top-level VBA storage of Access for a project and cannot check this layout.)
    check = os.path.join(work, 'objects-check')
    shutil.rmtree(check, ignore_errors=True)
    java('objects', out, check)
    verify(os.path.join(check, 'compound.bin'), project, poi=False)


def rc4(key, data):
    state = list(range(256))
    j = 0
    for i in range(256):
        j = (j + state[i] + key[i % len(key)]) & 0xFF
        state[i], state[j] = state[j], state[i]
    i = j = 0
    out = bytearray()
    for byte in data:
        i = (i + 1) & 0xFF
        j = (j + state[i]) & 0xFF
        state[i], state[j] = state[j], state[i]
        out.append(byte ^ state[(state[i] + state[j]) & 0xFF])
    return bytes(out)


def encode_jet(file, key):
    """"Encrypt/Decrypt Database" of Jet 4 (as mdbtools reads it): the database key in the
    RC4-obfuscated header, every page after the first RC4-encoded with key XOR page number."""
    with open(file, 'rb') as f:
        data = bytearray(f.read())
    page_size = 4096
    header = bytearray(rc4(bytes([0xC7, 0xDA, 0x39, 0x6B]), bytes(data[0x18:0x18 + 128])))
    header[0x3E - 0x18:0x42 - 0x18] = struct.pack('<I', key)
    data[0x18:0x18 + 128] = rc4(bytes([0xC7, 0xDA, 0x39, 0x6B]), bytes(header))
    for page in range(1, len(data) // page_size):
        start = page * page_size
        data[start:start + page_size] = rc4(struct.pack('<I', key ^ page), bytes(data[start:start + page_size]))
    with open(file, 'wb') as f:
        f.write(data)
    subprocess.run(['mdb-tables', '-S', file], check=True, capture_output=True)  # mdbtools reads it


def java_string_hash(text):
    h = 0
    for unit in struct.unpack(f'<{len(text.encode("utf-16-le")) // 2}H', text.encode('utf-16-le')):
        h = (31 * h + unit) & 0xFFFFFFFF
    return format(h, 'x')


def verify(file, project, poi=True):
    """oletools and Apache POI must both return every module's source unchanged."""
    from oletools.olevba import VBA_Parser
    expected = {m['name']: module_source(m) for m in project['modules'] if not m.get('stripped')}
    parser = VBA_Parser(file)
    found = {}
    for _, _, vba_filename, code in parser.extract_macros():
        found[os.path.splitext(vba_filename)[0]] = code.replace('\r\n', '\n')
    parser.close()
    for name, source in expected.items():
        if found.get(name, '').strip('\n') != source.replace('\r\n', '\n').strip('\n'):
            raise RuntimeError(f'{file}: olevba reads module {name} differently')
    # (Log4j reports on stdout that it has no logging provider - only the tab-separated lines count.)
    if not poi:
        return
    poi = dict(line.split('\t', 1) for line in java('macros', file).splitlines() if '\t' in line)
    for name, source in expected.items():
        text = source.replace('\r\n', '\n')
        if poi.get(name) != f'{len(text)}\t{java_string_hash(text)}':
            raise RuntimeError(f'{file}: Apache POI reads module {name} differently ({poi.get(name)})')


# --- the cases ----------------------------------------------------------------------------------

def P(*parts):
    return corpus('positive', 'office', *parts)


def N(*parts):
    return corpus('negative', 'office', *parts)


def REAL_FILE(name):
    return corpus('negative', 'office', 'real', name)


THIS_WORKBOOK = module('ThisWorkbook', '''
Private Sub Workbook_Open()
    Module1.CheckAddresses
End Sub''', 'document')

REGEXP_LATE = module('Module1', '''
Option Explicit

' Checks the e-mail addresses in column B
Public Sub CheckAddresses()
    Dim re As Object, cell As Range
    Set re = CreateObject("VBScript.RegExp")
    re.Pattern = "^[^@]+@[^@]+\\.[a-z]{2,}$"
    Dim conn As Object
    Set conn = CreateObject("ADODB.Connection")
    conn.Open "Provider=SQLOLEDB;Data Source=sql01;User ID=report;Password=Sommer2024!"
    For Each cell In ActiveSheet.Range("B2:B500")
        If Not re.Test(cell.Value) Then cell.Interior.Color = vbYellow
    Next
End Sub''', pcode=0x140)

CLEAN = module('Module1', '''
Option Explicit

' Formerly: Set re = CreateObject("VBScript.RegExp") - replaced by VBA's own RegExp
Public Sub Tidy()
    Dim re As New RegExp, fso As Object, names As Object
    re.Pattern = "\\s+"
    Set fso = CreateObject("Scripting.FileSystemObject")
    Set names = CreateObject("Scripting.Dictionary")
    Dim sc As Object
    Set sc = CreateObject("MSScriptControl.ScriptControl")
    sc.Language = "JScript"
    MsgBox "Run update.vbs once IT has migrated it"
    Open "C:\\Temp\\notes.vbs" For Output As #1
    Print #1, "' kept for reference"
    Close #1
    Kill "C:\\Temp\\old.vbs"
    Shell "cscript //E:JScript //nologo C:\\Tools\\report.js", vbHide
    CreateObject("htmlfile").parentWindow.execScript "var total = 1 + 2;"
    Rem Shell "wscript.exe C:\\Scripts\\legacy.vbs"
End Sub''', pcode=0x80)


def generate():
    real_files()
    with tempfile.TemporaryDirectory() as work:
        # Excel Open XML: late-bound RegExp, an auto macro and a connection-string password.
        workbook = {'name': 'VBAProject', 'modules': [THIS_WORKBOOK, REGEXP_LATE], 'references': [('stdole', STDOLE), ('Office', OFFICE)]}
        out = P('regexp-late.xlsm')
        package(out, REAL_FILE('poi-SimpleMacro.xlsm'), workbook, work)
        verify(out, workbook)

        # Excel binary workbook (.xlsb, a real one from Apache POI) with the VBScript RegExp reference.
        early = {'name': 'VBAProject', 'references': [('stdole', STDOLE), ('VBScript_RegExp_55', REGEXP_55)],
                 'modules': [module('Module1', '''
Public Function IsPostcode(value As String) As Boolean
    Dim re As New RegExp
    re.Pattern = "^[0-9]{5}$"
    IsPostcode = re.Test(value)
End Function''', pcode=0x60)]}
        out = P('regexp-reference.xlsb')
        package(out, REAL_FILE('poi-Simple.xlsb'), early, work, add_vba_part='xl/vbaProject.bin')
        verify(out, early)

        # Word: the Script Control with VBScript (document) and with a language set at run time (template).
        control = {'name': 'Project', 'references': [('stdole', STDOLE)], 'modules': [
            module('ThisDocument', 'Private Sub Document_Open()\n    Calc.Evaluate\nEnd Sub', 'document'),
            module('Calc', '''
Public Sub Evaluate()
    Dim sc As Object
    Set sc = CreateObject("MSScriptControl.ScriptControl")
    sc.Language = "VBScript"
    sc.AddCode "Function Twice(x) : Twice = x * 2 : End Function"
    MsgBox sc.Run("Twice", 21)
End Sub''', pcode=0x44)]}
        out = P('script-control.docm')
        package(out, REAL_FILE('poi-SimpleMacro.docm'), control, work)
        verify(out, control)
        unknown = {'name': 'Project', 'modules': [module('Engine', '''
Public Function Run(language As String, code As String)
    Dim sc As New ScriptControl
    sc.Language = language
    Run = sc.Eval(code)
End Function''')]}
        out = P('script-control-unknown.dotm')
        package(out, REAL_FILE('poi-SimpleMacro.docm'), unknown, work, extension='dotm')
        verify(out, unknown)

        # Excel binary workbook: starts a .vbs through wscript and through WScript.Shell.Run.
        starts = {'name': 'VBAProject', 'references': [('stdole', STDOLE)], 'modules': [
            module('ThisWorkbook', '', 'document'),
            module('Sync', '''
Public Sub SyncFolders()
    Shell "wscript.exe ""\\\\fs01\\netlogon\\sync.vbs"" /quiet", vbHide
    Dim sh As Object
    Set sh = CreateObject("WScript.Shell")
    sh.Run "C:\\Scripts\\cleanup.vbe", 0, True
    sh.Run "cscript //nologo """ & ThisWorkbook.Path & "\\export.vbs"""
End Sub''', pcode=0x200)]}
        out = P('starts-vbscript.xls')
        binary_document(out, REAL_FILE('poi-SimpleMacro.xls'), '_VBA_PROJECT_CUR', starts, work)
        verify(out, starts)

        # Excel add-in (.xla): starts an HTML application (language unknown), and runs VBScript through
        # execScript of an HTML document window (the way around the missing Script Control of 64-bit Office).
        hta = {'name': 'VBAProject', 'modules': [module('Menu', '''
Public Sub ShowMenu()
    Shell "mshta.exe " & Chr(34) & "C:\\Tools\\menu.hta" & Chr(34), vbNormalFocus
End Sub'''), module('Html', '''
Public Sub Greet()
    Dim html As Object
    Set html = CreateObject("htmlfile")
    html.parentWindow.execScript "MsgBox ""Hello from VBScript""", "VBScript"
End Sub''')]}
        out = P('starts-hta.xla')
        binary_document(out, REAL_FILE('poi-SimpleMacro.xls'), '_VBA_PROJECT_CUR', hta, work)
        verify(out, hta)

        # PowerPoint: objects of the Windows Script Host only.
        wsh = {'name': 'VBAProject', 'modules': [module('Module1', '''
Public Sub ShowUser()
    Dim net As Object
    Set net = CreateObject("WScript.Network")
    MsgBox net.UserName
End Sub''')]}
        out = P('wsh-objects.pptm')
        package(out, REAL_FILE('poi-SimpleMacro.pptm'), wsh, work)
        verify(out, wsh)

        # Word 97 template (.dot): runs a script with the VBScript engine switch.
        dot = {'name': 'Project', 'modules': [module('ThisDocument', '''
Private Sub Document_New()
    Shell "cscript.exe //E:VBScript //B C:\\Templates\\fill-in.txt"
End Sub''', 'document', pcode=0x30)]}
        out = P('template-engine-switch.dot')
        binary_document(out, REAL_FILE('poi-SimpleMacro.doc'), 'Macros', dot, work)
        verify(out, dot)

        # Other Open XML variants: add-in, template, show, presentation add-in.
        out = P('addin.xlam')
        package(out, REAL_FILE('poi-SimpleMacro.xlsm'), workbook, work, extension='xlam')
        verify(out, workbook)
        out = P('template.xltm')
        package(out, REAL_FILE('poi-SimpleMacro.xlsm'), starts, work, extension='xltm')
        verify(out, starts)
        out = P('show.ppsm')
        package(out, REAL_FILE('poi-SimpleMacro.pptm'), hta, work, extension='ppsm')
        verify(out, hta)
        out = P('addin.ppam')
        package(out, REAL_FILE('poi-SimpleMacro.pptm'), control, work, extension='ppam')
        verify(out, control)

        # Word document with an embedded workbook whose project uses VBScript RegExp.
        embedded = {f'ObjectPool/_1234567890/_VBA_PROJECT_CUR/{path}': data
                    for path, data in project_streams(workbook).items()}
        clean_doc = {'name': 'Project', 'modules': [module('ThisDocument', '', 'document')]}
        out = P('embedded-workbook.doc')
        binary_document(out, REAL_FILE('poi-SimpleMacro.doc'), 'Macros', clean_doc, work, extra=embedded)

        # Locked for viewing: the code is read anyway (and the finding says so).
        locked = dict(workbook, locked=True, id='{9E8D7C6B-5A49-4382-9170-6F5E4D3C2B1A}')
        out = P('locked-project.xls')
        binary_document(out, REAL_FILE('poi-SimpleMacro.xls'), '_VBA_PROJECT_CUR', locked, work)
        verify(out, locked)
        # Locked, and the module holds compiled code only: not checkable.
        stripped = {'name': 'Project', 'locked': True, 'modules': [
            dict(module('ThisDocument', '', 'document')), dict(module('Worker', 'Sub A()\nEnd Sub', pcode=0x100), stripped=True)]}
        out = P('locked-compiled-only.doc')
        binary_document(out, REAL_FILE('poi-SimpleMacro.doc'), 'Macros', stripped, work)

        # Access: 2010 (.accdb) with the RegExp reference, 2002-2003 (.mdb) with a form module that
        # starts a .vbs, 2000 (.mdb, chunked compound file) with the Script Control, and a Jet 4
        # database encoded with RC4.
        access(P('access', 'inventory.accdb'), 'V2010', {'name': 'Inventory', 'references': [
            ('stdole', STDOLE), ('ADODB', ADODB), ('VBScript_RegExp_55', REGEXP_55)], 'modules': [
            module('Validation', '''
Public Function ValidSerial(serial As String) As Boolean
    Static re As VBScript_RegExp_55.RegExp
    If re Is Nothing Then Set re = New VBScript_RegExp_55.RegExp
    re.Pattern = "^[A-Z]{3}-[0-9]{6}$"
    ValidSerial = re.Test(serial)
End Function''', pcode=0x90)]}, work)
        access(P('access', 'orders.mdb'), 'V2003', {'name': 'Orders', 'references': [('stdole', STDOLE)], 'modules': [
            module('Form_Orders', '''
Private Sub cmdExport_Click()
    Shell "wscript.exe //B C:\\Orders\\export-orders.vbs " & Me.OrderID
End Sub''', 'class', pcode=0x70)]}, work)
        access2000(P('access', 'legacy-2000.mdb'), {'name': 'Legacy', 'modules': [
            module('Formulas', '''
Public Function Calculate(expression As String)
    Dim sc As Object
    Set sc = CreateObject("MSScriptControl.ScriptControl")
    sc.Language = "VBS"
    Calculate = sc.Eval(expression)
End Function''')]}, work)
        access(P('access', 'encoded.mdb'), 'V2003', {'name': 'Encoded', 'modules': [
            module('Checks', 'Public Sub Check()\n    Set re = CreateObject("VBScript.RegExp")\nEnd Sub')]}, work,
            encode_key=0x5A3C9E17)

        # Needs a password to open: the RegExp workbook encrypted with msoffcrypto-tool (agile).
        from msoffcrypto.format.ooxml import OOXMLFile
        with open(P('regexp-late.xlsm'), 'rb') as plain, open(P('protected', 'password-to-open.xlsm'), 'wb') as out:
            OOXMLFile(plain).encrypt('Passw0rd!', out)

        # Formats the collector does not read: Excel 5.0/95 module sheet, Word web archive (MHTML).
        bof = struct.pack('<HHHH', 0x0809, 8, 0x0500, 0x0005) + bytes(4)
        sheet = struct.pack('<IH', 0, 0x0600) + bytes([7]) + b'Module1'
        book = bof + struct.pack('<HH', 0x0085, len(sheet)) + sheet + struct.pack('<HH', 0x000A, 0)
        compound(P('unsupported', 'excel95-module.xls'), {'Book': book}, work=work)
        with open(P('unsupported', 'web-archive.doc'), 'w', newline='') as f:
            f.write('MIME-Version: 1.0\r\nContent-Type: multipart/related; boundary="----=_NextPart_01"\r\n\r\n'
                    '------=_NextPart_01\r\nContent-Location: file:///C:/Report_files/editdata.mso\r\n'
                    'Content-Transfer-Encoding: base64\r\nContent-Type: application/x-mso\r\n\r\n'
                    'QWN0aXZlTWltZQAAAfAEAAAA/////wAAB/AAAAAA\r\n------=_NextPart_01--\r\n')
        with open(P('broken', 'damaged.xlsm'), 'wb') as f:
            f.write(b'PK\x03\x04' + bytes(range(256)) * 4)

        # Negative: macros without VBScript, Access databases as Jackcess creates them (the empty
        # VBA project of Access), and files with macro extensions that cannot hold VBA.
        clean = {'name': 'VBAProject', 'references': [('stdole', STDOLE), ('Scripting', SCRIPTING)], 'modules': [CLEAN]}
        out = N('no-vbscript.xlsm')
        package(out, REAL_FILE('poi-SimpleMacro.xlsm'), clean, work)
        verify(out, clean)
        out = N('no-vbscript.doc')
        binary_document(out, REAL_FILE('poi-SimpleMacro.doc'), 'Macros', dict(clean, name='Project'), work)
        verify(out, dict(clean, name='Project'))
        java('access', 'V2010', N('access', 'empty.accdb'), os.devnull)
        java('access', 'V2003', N('access', 'empty.mdb'), os.devnull)
        java('access', 'V2000', N('access', 'empty-2000.mdb'), os.devnull)
        with open(N('formats', 'report-export.xls'), 'w', newline='') as f:
            f.write('<html><body><table><tr><td>Region</td><td>Total</td></tr></table></body></html>\r\n')
        with open(N('formats', 'csv-export.xls'), 'w', newline='') as f:
            f.write('Region;Total\r\nNorth;1200\r\n')
        with open(N('formats', 'letter.doc'), 'w', newline='') as f:
            f.write('{\\rtf1\\ansi\\deff0 {\\fonttbl {\\f0 Calibri;}}\\f0 Dear customer,\\par}\r\n')
        with open(N('formats', '~$Budget.xlsm'), 'wb') as f:
            f.write(bytes([5]) + b'alice'.ljust(53, b' ') + bytes([5, 0]) + utf16('alice').ljust(108, b'\0'))
        with open(N('formats', 'empty.xlsm'), 'wb'):
            pass
        with open(N('formats', 'excel4.xls'), 'wb') as f:
            f.write(struct.pack('<HHHH', 0x0409, 6, 0x0000, 0x0010) + bytes(2) + struct.pack('<HH', 0x000A, 0))


if __name__ == '__main__':
    generate()
    print('office fixtures written')
