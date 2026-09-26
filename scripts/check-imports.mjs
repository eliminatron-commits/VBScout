#!/usr/bin/env node
// Definition of Done #3 (single file that runs without installation on Windows 10/11 and Server
// 2016–2025) and #2 (no network), checked on the finished program file: the collector imports only
// libraries of Windows itself – no C/C++ runtime that would have to be installed, no network,
// directory or web library. Reads the import and delay-load tables of the PE file and lists them.
//   node scripts/check-imports.mjs <program.exe> [--out <list.txt>]
//   node scripts/check-imports.mjs --self-test

import { readFileSync, writeFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

// Libraries of Windows itself, all present on Windows 10 1607 / Server 2016 and later. A library
// outside this list fails the check until it has been reviewed and added.
const SYSTEM = new Set([
  'kernel32.dll', 'kernelbase.dll', 'ntdll.dll', 'advapi32.dll', 'bcrypt.dll', 'bcryptprimitives.dll', 'crypt32.dll',
  'ole32.dll', 'oleaut32.dll', 'combase.dll', 'wevtapi.dll', 'userenv.dll', 'shell32.dll', 'shlwapi.dll', 'user32.dll',
  'version.dll', 'psapi.dll',
  // API sets (Windows 8 / Server 2012 and later)
  'api-ms-win-core-synch-l1-2-0.dll', 'api-ms-win-core-path-l1-1-0.dll', 'api-ms-win-core-winrt-l1-1-0.dll',
  'api-ms-win-core-winrt-error-l1-1-0.dll', 'api-ms-win-core-winrt-error-l1-1-1.dll',
  'api-ms-win-core-winrt-string-l1-1-0.dll',
]);
// Runtime libraries that are not part of every Windows (Visual C++ redistributable, UCRT on old systems).
const RUNTIME = [/^vcruntime/, /^msvcp/, /^msvcr/, /^ucrtbase/, /^api-ms-win-crt-/, /^concrt/, /^vccorlib/];
// Network, name resolution, directory and web libraries – the collector never connects anywhere.
const NETWORK = new Set([
  'ws2_32.dll', 'wsock32.dll', 'mswsock.dll', 'winhttp.dll', 'wininet.dll', 'urlmon.dll', 'dnsapi.dll', 'iphlpapi.dll',
  'netapi32.dll', 'wldap32.dll', 'activeds.dll', 'mpr.dll', 'websocket.dll', 'httpapi.dll', 'rasapi32.dll',
  'wlanapi.dll', 'fwpuclnt.dll', 'rpcrt4.dll',
]);

/** Imports of a PE file: Map<dll (lower case), { functions: string[], delayed: boolean }>. */
export function readImports(buf) {
  if (buf.length < 0x40 || buf.readUInt16LE(0) !== 0x5a4d) throw new Error('not a Windows program (no MZ header)');
  const pe = buf.readUInt32LE(0x3c);
  if (pe + 24 > buf.length || buf.readUInt32LE(pe) !== 0x4550) throw new Error('no PE header');
  const sectionCount = buf.readUInt16LE(pe + 6);
  const optionalSize = buf.readUInt16LE(pe + 20);
  const opt = pe + 24;
  const magic = buf.readUInt16LE(opt);
  if (magic !== 0x20b && magic !== 0x10b) throw new Error(`unknown optional header 0x${magic.toString(16)}`);
  const pe64 = magic === 0x20b;
  const directoryCount = buf.readUInt32LE(opt + (pe64 ? 108 : 92));
  const directory = (index) => {
    if (index >= directoryCount) return 0;
    return buf.readUInt32LE(opt + (pe64 ? 112 : 96) + index * 8);
  };
  const sections = [];
  for (let i = 0; i < sectionCount; i++) {
    const at = opt + optionalSize + i * 40;
    sections.push({
      va: buf.readUInt32LE(at + 12),
      size: Math.max(buf.readUInt32LE(at + 8), buf.readUInt32LE(at + 16)),
      raw: buf.readUInt32LE(at + 20),
    });
  }
  const offset = (rva) => {
    const section = sections.find((s) => rva >= s.va && rva < s.va + s.size);
    if (!section) throw new Error(`address 0x${rva.toString(16)} outside all sections`);
    return section.raw + (rva - section.va);
  };
  const text = (rva) => {
    const start = offset(rva);
    const end = buf.indexOf(0, start);
    return buf.toString('latin1', start, end < 0 ? buf.length : end);
  };
  const width = pe64 ? 8 : 4;
  const ordinalFlag = pe64 ? 1n << 63n : 1n << 31n;
  const functions = (rva) => {
    const names = [];
    for (let at = rva ? offset(rva) : buf.length; at + width <= buf.length; at += width) {
      const entry = pe64 ? buf.readBigUInt64LE(at) : BigInt(buf.readUInt32LE(at));
      if (entry === 0n) break;
      names.push(entry & ordinalFlag ? `#${Number(entry & 0xffffn)}` : text(Number(entry & 0x7fffffffn) + 2));
    }
    return names;
  };
  const imports = new Map();
  const add = (dll, names, delayed) => {
    const key = dll.toLowerCase();
    const entry = imports.get(key) ?? { functions: [], delayed };
    entry.functions.push(...names);
    imports.set(key, entry);
  };
  const importTable = directory(1);
  if (importTable) {
    for (let at = offset(importTable); ; at += 20) {
      const [lookup, name, addresses] = [buf.readUInt32LE(at), buf.readUInt32LE(at + 12), buf.readUInt32LE(at + 16)];
      if (!lookup && !name && !addresses) break;
      add(text(name), functions(lookup || addresses), false);
    }
  }
  const delayTable = directory(13);
  if (delayTable) {
    for (let at = offset(delayTable); ; at += 32) {
      const [attributes, name, names] = [buf.readUInt32LE(at), buf.readUInt32LE(at + 4), buf.readUInt32LE(at + 16)];
      if (!name) break;
      if (!(attributes & 1)) throw new Error('delay-load table with absolute addresses (not supported)');
      add(text(name), functions(names), true);
    }
  }
  return imports;
}

/** Policy verdicts for the imported libraries. */
export function judge(imports) {
  const problems = [];
  for (const dll of imports.keys()) {
    if (RUNTIME.some((pattern) => pattern.test(dll))) problems.push(`${dll}: C/C++ runtime – would have to be installed`);
    else if (NETWORK.has(dll)) problems.push(`${dll}: network/directory library – the collector never connects`);
    else if (!SYSTEM.has(dll)) problems.push(`${dll}: not a reviewed Windows system library (check, then add it)`);
  }
  return problems;
}

function selfTest() {
  // A minimal PE32+ image: one section at RVA 0x1000 holding an import and a delay-load table.
  const image = Buffer.alloc(0x600);
  image.writeUInt16LE(0x5a4d, 0);
  image.writeUInt32LE(0x80, 0x3c);
  image.writeUInt32LE(0x4550, 0x80);
  image.writeUInt16LE(0x8664, 0x84);
  image.writeUInt16LE(1, 0x86);
  image.writeUInt16LE(240, 0x94);
  const opt = 0x98;
  image.writeUInt16LE(0x20b, opt);
  image.writeUInt32LE(16, opt + 108);
  image.writeUInt32LE(0x1000, opt + 112 + 8); // import table
  image.writeUInt32LE(0x1100, opt + 112 + 13 * 8); // delay-load table
  const section = opt + 240;
  image.write('.idata', section, 'latin1');
  image.writeUInt32LE(0x400, section + 8);
  image.writeUInt32LE(0x1000, section + 12);
  image.writeUInt32LE(0x400, section + 16);
  image.writeUInt32LE(0x200, section + 20);
  const at = (rva) => 0x200 + rva - 0x1000;
  const names = { 0x1200: 'KERNEL32.dll', 0x1210: 'WS2_32.dll', 0x1220: 'wevtapi.dll' };
  for (const [rva, name] of Object.entries(names)) image.write(`${name}\0`, at(Number(rva)), 'latin1');
  image.write('\0\0CreateFileW\0', at(0x1300), 'latin1');
  image.write('\0\0WSAStartup\0', at(0x1320), 'latin1');
  image.write('\0\0EvtQuery\0', at(0x1340), 'latin1');
  // Import descriptors: kernel32 (by name), ws2_32 (by ordinal 115).
  image.writeBigUInt64LE(0x1300n, at(0x1380));
  image.writeBigUInt64LE((1n << 63n) | 115n, at(0x1390));
  image.writeUInt32LE(0x1380, at(0x1000));
  image.writeUInt32LE(0x1200, at(0x1000) + 12);
  image.writeUInt32LE(0x1380, at(0x1000) + 16);
  image.writeUInt32LE(0x1390, at(0x1014));
  image.writeUInt32LE(0x1210, at(0x1014) + 12);
  image.writeUInt32LE(0x1390, at(0x1014) + 16);
  // Delay-load descriptor: wevtapi (RVA-based).
  image.writeBigUInt64LE(0x1340n, at(0x13a0));
  image.writeUInt32LE(1, at(0x1100));
  image.writeUInt32LE(0x1220, at(0x1100) + 4);
  image.writeUInt32LE(0x13a0, at(0x1100) + 16);

  const imports = readImports(image);
  const expect = (condition, message) => {
    if (!condition) throw new Error(`self-test: ${message}`);
  };
  expect(imports.size === 3, `3 libraries expected, got ${[...imports.keys()]}`);
  expect(imports.get('kernel32.dll')?.functions.join() === 'CreateFileW', 'kernel32!CreateFileW by name');
  expect(imports.get('ws2_32.dll')?.functions.join() === '#115', 'ws2_32 by ordinal');
  expect(imports.get('wevtapi.dll')?.delayed && imports.get('wevtapi.dll').functions.join() === 'EvtQuery', 'delay-load');
  const verdicts = judge(new Map([
    ['kernel32.dll', {}], ['vcruntime140.dll', {}], ['api-ms-win-crt-runtime-l1-1-0.dll', {}], ['ws2_32.dll', {}],
    ['winhttp.dll', {}], ['foo.dll', {}], ['api-ms-win-core-synch-l1-2-0.dll', {}],
  ]));
  expect(verdicts.length === 5, `5 problems expected: ${verdicts.join('; ')}`);
  for (const dll of ['vcruntime140', 'api-ms-win-crt', 'ws2_32', 'winhttp', 'foo']) {
    expect(verdicts.some((v) => v.startsWith(dll)), `${dll} must be reported`);
  }
  for (const [bytes, what] of [[Buffer.alloc(0x100), 'no MZ header'], [Buffer.from(`MZ${'x'.repeat(0x50)}`), 'no PE header']]) {
    let rejected = false;
    try {
      readImports(bytes);
    } catch {
      rejected = true;
    }
    expect(rejected, `a file with ${what} is rejected`);
  }
  console.log('✓ import check self-test: parser (names, ordinals, delay-load) and policy recognised correctly');
}

const args = import.meta.url === pathToFileURL(process.argv[1] ?? '').href ? process.argv.slice(2) : null;
if (args === null) {
  // imported (tests)
} else if (args[0] === '--self-test') {
  selfTest();
} else if (args[0]) {
  const file = args[0];
  const outIndex = args.indexOf('--out');
  const imports = readImports(readFileSync(file));
  const lines = [...imports.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([dll, { functions, delayed }]) => `${dll}${delayed ? ' (delay-load)' : ''}: ${[...functions].sort().join(', ')}`);
  if (outIndex >= 0) writeFileSync(args[outIndex + 1], `${lines.join('\n')}\n`);
  const problems = judge(imports);
  if (problems.length) {
    console.error(`✗ ${file} imports libraries that are not allowed:\n  ${problems.join('\n  ')}`);
    console.error(`all imports:\n  ${lines.join('\n  ')}`);
    process.exit(1);
  }
  const count = [...imports.values()].reduce((sum, entry) => sum + entry.functions.length, 0);
  console.log(`✓ ${file}: ${count} functions from ${imports.size} Windows system libraries (${[...imports.keys()].sort().join(', ')})`);
} else {
  console.error('usage: check-imports.mjs <program.exe> [--out <list.txt>] | --self-test');
  process.exit(2);
}
