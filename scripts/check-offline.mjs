#!/usr/bin/env node
// Static half of the offline guarantee (Definition of Done #2; the dynamic half is scripts/nettest/).
// Taken over from Stepwright and extended by the collector:
//
//  • Tauri config: restrictive CSP (IPC only, no remote sources), mandatory WebView2 switches, no network plugins
//  • capabilities: no http/updater/websocket/upload/shell/opener permissions, no remote capabilities
//  • dependencies: no HTTP clients, websocket, TLS or telemetry packages (npm + Cargo)
//  • sources: no fetch/XMLHttpRequest/WebSocket/window.open/… in the frontend, no remote URLs,
//    no std::net, HTTP crates or Win32 networking APIs in Rust
//  • the collector's resolved dependency graph contains no async runtime or network stack at all
//
//   node scripts/check-offline.mjs            static checks
//   node scripts/check-offline.mjs --cargo    additionally inspect the resolved
//                                             Cargo dependency graph (needs cargo)

import { execFileSync } from 'node:child_process';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const errors = [];
const readText = (path) => readFileSync(join(root, path), 'utf8');
const readJson = (path) => JSON.parse(readText(path));
const rel = (path) => relative(root, path).replaceAll('\\', '/');

function* walk(dir) {
  for (const entry of readdirSync(dir)) {
    if (['node_modules', 'target', 'gen', 'dist'].includes(entry)) continue;
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) yield* walk(path);
    else yield path;
  }
}

const FORBIDDEN_CRATES = [
  'reqwest', 'hyper', 'hyper-util', 'h2', 'ureq', 'isahc', 'curl', 'curl-sys', 'attohttpc', 'surf',
  'tungstenite', 'tokio-tungstenite', 'async-tungstenite', 'native-tls', 'openssl', 'openssl-sys',
  'rustls', 'sentry', 'tauri-plugin-http', 'tauri-plugin-updater', 'tauri-plugin-websocket',
  'tauri-plugin-upload', 'tauri-plugin-shell', 'tauri-plugin-opener',
];
// The collector is a plain synchronous program: no async runtime, no sockets, no GUI stack.
const FORBIDDEN_COLLECTOR_CRATES = [...FORBIDDEN_CRATES, 'tokio', 'mio', 'socket2', 'async-std', 'smol', 'tauri', 'wry'];
const FORBIDDEN_NPM = [
  /^@tauri-apps\/plugin-(http|updater|websocket|upload|shell|opener)$/, /^axios$/, /^ky$/, /^got$/,
  /^node-fetch$/, /^cross-fetch$/, /^superagent$/, /^socket\.io-client$/, /^@sentry\//, /^posthog/,
  /^mixpanel/, /^@segment\//, /^@amplitude\//, /^@datadog\//, /^logrocket/, /^@vercel\/analytics/,
  /^plausible/,
];
const FORBIDDEN_PLUGINS = ['http', 'updater', 'websocket', 'upload', 'shell', 'opener'];
const FORBIDDEN_PERMISSIONS = [/^(http|updater|websocket|upload|shell|opener):/];

// --- Tauri configuration ------------------------------------------------------
const conf = readJson('src-tauri/tauri.conf.json');
const security = conf.app?.security ?? {};
const csp = security.csp;
const ALLOWED_SOURCES = new Set([
  "'self'", "'none'", 'ipc:', 'http://ipc.localhost', 'data:', 'blob:', 'asset:', 'http://asset.localhost', 'tauri:',
]);
if (!csp) errors.push('tauri.conf.json: app.security.csp must be set');
else {
  const directives = typeof csp === 'string'
    ? Object.fromEntries(csp.split(';').map((d) => d.trim()).filter(Boolean).map((d) => {
        const [name, ...sources] = d.split(/\s+/);
        return [name, sources.join(' ')];
      }))
    : csp;
  for (const required of ['default-src', 'connect-src', 'object-src']) {
    if (!(required in directives)) errors.push(`CSP: "${required}" must be set`);
  }
  for (const [directive, value] of Object.entries(directives)) {
    const sources = (Array.isArray(value) ? value : String(value).split(/\s+/)).filter(Boolean);
    for (const source of sources) {
      const ok = ALLOWED_SOURCES.has(source) || (directive === 'style-src' && source === "'unsafe-inline'");
      if (!ok) errors.push(`CSP ${directive}: source "${source}" is not allowed (no remote or wildcard sources)`);
    }
  }
}
if (security.dangerousDisableAssetCspModification) errors.push('tauri.conf.json: CSP modification must stay enabled');
// WebView2 (Windows) runs Chromium services that would otherwise talk to the network on their own –
// found by Stepwright's network block test: proxy auto-discovery (DNS "wpad") and a TCP preconnect to
// http://tauri.localhost (= loopback :80). Name resolution is disabled for the web layer entirely;
// app content arrives through Tauri's protocol handler without DNS or sockets.
const REQUIRED_BROWSER_ARGS = [
  '--no-proxy-server',
  '--host-resolver-rules="MAP * ~NOTFOUND"',
  '--disable-background-networking',
  '--disable-component-update',
];
for (const window of conf.app?.windows ?? []) {
  const args = ` ${window.additionalBrowserArgs ?? ''} `;
  for (const required of REQUIRED_BROWSER_ARGS) {
    if (!args.includes(` ${required} `)) {
      errors.push(`tauri.conf.json: window "${window.label}" needs browser argument ${required}`);
    }
  }
}
if (conf.bundle?.createUpdaterArtifacts) errors.push('tauri.conf.json: updater artifacts are forbidden');
// The installer must not download anything either: no WebView2 bootstrapper (the runtime ships with
// Windows 10/11; `offlineInstaller` or `fixedRuntime` would embed it instead).
const webviewInstall = conf.bundle?.windows?.webviewInstallMode?.type ?? 'downloadBootstrapper';
if (!['skip', 'offlineInstaller', 'fixedRuntime'].includes(webviewInstall)) {
  errors.push(`tauri.conf.json: bundle.windows.webviewInstallMode "${webviewInstall}" downloads during installation`);
}
for (const plugin of Object.keys(conf.plugins ?? {})) {
  if (FORBIDDEN_PLUGINS.includes(plugin)) errors.push(`tauri.conf.json: plugin "${plugin}" is forbidden`);
}

// --- capabilities -------------------------------------------------------------
for (const path of walk(join(root, 'src-tauri/capabilities'))) {
  if (!path.endsWith('.json')) continue;
  const capability = JSON.parse(readFileSync(path, 'utf8'));
  if (capability.remote) errors.push(`${rel(path)}: remote capabilities are forbidden`);
  for (const permission of capability.permissions ?? []) {
    const id = typeof permission === 'string' ? permission : permission.identifier;
    if (FORBIDDEN_PERMISSIONS.some((pattern) => pattern.test(id))) {
      errors.push(`${rel(path)}: permission "${id}" is forbidden`);
    }
  }
}

// --- declared dependencies ------------------------------------------------------
const pkg = readJson('package.json');
for (const name of Object.keys({ ...pkg.dependencies, ...pkg.devDependencies })) {
  if (FORBIDDEN_NPM.some((pattern) => pattern.test(name))) errors.push(`package.json: "${name}" is forbidden`);
}
const manifests = [
  'Cargo.toml',
  'src-tauri/Cargo.toml',
  ...readdirSync(join(root, 'crates')).map((c) => `crates/${c}/Cargo.toml`),
  ...readdirSync(join(root, 'tools')).map((c) => `tools/${c}/Cargo.toml`),
];
for (const manifest of manifests) {
  let table = '';
  for (const line of readText(manifest).split('\n')) {
    const header = line.match(/^\s*\[([^\]]+)\]/);
    if (header) {
      table = header[1];
      continue;
    }
    if (!/dependencies/.test(table)) continue;
    const name = line.match(/^\s*([A-Za-z0-9_-]+)\s*=/)?.[1];
    if (name && FORBIDDEN_CRATES.includes(name)) errors.push(`${manifest}: dependency "${name}" is forbidden`);
  }
}

// --- sources --------------------------------------------------------------------
const FRONTEND_APIS = [
  /\bfetch\s*\(/, /\bXMLHttpRequest\b/, /\bWebSocket\b/, /\bEventSource\b/, /\bsendBeacon\b/,
  /\bRTCPeerConnection\b/, /\bimportScripts\b/, /\bserviceWorker\b/, /\bwindow\.open\s*\(/, /\blocation\.(href|assign|replace)\b/,
];
const ALLOWED_URLS = [/^https?:\/\/(ipc|asset)\.localhost/, /^http:\/\/www\.w3\.org\//];
const frontendFiles = [join(root, 'index.html'), ...walk(join(root, 'src'))];
for (const path of frontendFiles) {
  if (!/\.(ts|js|svelte|css|html)$/.test(path)) continue;
  const text = readFileSync(path, 'utf8');
  for (const api of FRONTEND_APIS) {
    if (api.test(text)) errors.push(`${rel(path)}: network API ${api} is forbidden`);
  }
  for (const [url] of text.matchAll(/\b(?:https?|wss?):\/\/[^\s'"`)<>]+/g)) {
    if (!ALLOWED_URLS.some((pattern) => pattern.test(url))) errors.push(`${rel(path)}: remote URL ${url}`);
  }
}
const RUST_NETWORK =
  /\bstd::net\b|\bTcpStream\b|\bTcpListener\b|\bUdpSocket\b|\bto_socket_addrs\b|\breqwest::|\bureq::|\bhyper::|\bWinHttp\w*|\bInternet(?:Open|Connect|ReadFile)\w*|\bURLDownloadToFile\w*|\bWSAStartup\b|\bWNet(?:AddConnection|UseConnection)\w*|\bRegConnectRegistry\w*|\bgetaddrinfo\b|\bGetAddrInfo\w*/;
for (const dir of ['crates', 'src-tauri/src', 'tools']) {
  for (const path of walk(join(root, dir))) {
    if (!path.endsWith('.rs')) continue;
    const text = readFileSync(path, 'utf8');
    text.split('\n').forEach((line, index) => {
      if (RUST_NETWORK.test(line)) errors.push(`${rel(path)}:${index + 1}: network API in Rust code`);
    });
  }
}

// --- resolved Cargo graph (optional) ------------------------------------------
function resolvedCrates(args) {
  const tree = execFileSync('cargo', ['tree', ...args, '--edges', 'normal', '--prefix', 'none', '--format', '{p}'], {
    cwd: root,
    encoding: 'utf8',
    maxBuffer: 64 * 1024 * 1024,
  });
  return new Set(tree.split('\n').map((line) => line.trim().split(' ')[0]).filter(Boolean));
}

if (process.argv.includes('--cargo')) {
  for (const target of ['x86_64-pc-windows-msvc', 'x86_64-unknown-linux-gnu']) {
    const crates = resolvedCrates(['--workspace', '--target', target]);
    for (const name of FORBIDDEN_CRATES) {
      if (crates.has(name)) errors.push(`${target}: resolved dependency graph contains "${name}"`);
    }
    // Tauri needs tokio as async runtime – but never its TCP/UDP layer.
    if (crates.has('tokio')) {
      const features = execFileSync(
        'cargo',
        ['tree', '--workspace', '--edges', 'features', '--invert', 'tokio', '--target', target, '--prefix', 'none'],
        { cwd: root, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 },
      );
      if (/tokio feature "net"/.test(features)) errors.push(`${target}: tokio is built with its "net" feature`);
    }
    const collector = resolvedCrates(['--package', 'vbs-collector', '--target', target]);
    for (const name of FORBIDDEN_COLLECTOR_CRATES) {
      if (collector.has(name)) errors.push(`${target}: the collector's dependency graph contains "${name}"`);
    }
    console.log(`  ${target}: ${crates.size} crates checked (collector: ${collector.size})`);
  }
}

if (errors.length) {
  for (const error of errors) console.error(`✗ ${error}`);
  console.error(`\n${errors.length} offline violation(s).`);
  process.exit(1);
}
console.log('✓ offline policy: CSP, capabilities, dependencies and sources are clean');
