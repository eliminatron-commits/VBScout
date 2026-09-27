#!/usr/bin/env node
// Third-party notices for the release files: every crate compiled into the collector or the app for
// Windows x64 (resolved Cargo graph, normal dependencies), the npm packages bundled into the frontend
// and the embedded font – each with its license expression and the license texts it ships.
//
//   node scripts/release/third-party.mjs --out <file>      (needs the crate sources: cargo fetch)

import { execFileSync } from 'node:child_process';
import { existsSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../../', import.meta.url));
const TARGET = 'x86_64-pc-windows-msvc';
const SHIPPED = ['vbs-app', 'vbs-collector'];
// Bundled into dist/ by Vite (the Svelte runtime and its runtime helpers, the Tauri IPC API).
const NPM_BUNDLED = ['svelte', 'clsx', 'esm-env', '@tauri-apps/api'];
const LICENSE_FILE = /^(licen[cs]e|copying|notice)([-._].*)?$/i;

function licenseTexts(dir) {
  if (!existsSync(dir)) return [];
  return readdirSync(dir)
    .filter((name) => LICENSE_FILE.test(name))
    .sort()
    .map((name) => ({ name, text: readFileSync(join(dir, name), 'utf8').trim() }));
}

function cratePackages() {
  const metadata = JSON.parse(
    execFileSync('cargo', ['metadata', '--format-version', '1', '--locked', '--filter-platform', TARGET], {
      cwd: root,
      encoding: 'utf8',
      maxBuffer: 256 * 1024 * 1024,
    }),
  );
  const packages = new Map(metadata.packages.map((p) => [p.id, p]));
  const nodes = new Map(metadata.resolve.nodes.map((n) => [n.id, n]));
  const members = new Set(metadata.workspace_members);
  const seen = new Set();
  const queue = metadata.packages.filter((p) => SHIPPED.includes(p.name) && members.has(p.id)).map((p) => p.id);
  if (queue.length !== SHIPPED.length) throw new Error('shipped crates not found in cargo metadata');
  while (queue.length) {
    const id = queue.pop();
    if (seen.has(id)) continue;
    seen.add(id);
    for (const dep of nodes.get(id)?.deps ?? []) {
      // Normal dependencies only (no build scripts, no dev dependencies).
      if (dep.dep_kinds.some((kind) => kind.kind === null)) queue.push(dep.pkg);
    }
  }
  return [...seen]
    .filter((id) => !members.has(id))
    .map((id) => packages.get(id))
    // Procedural macros run at compile time only; nothing of them ends up in the programs.
    .filter((p) => !p.targets.every((target) => target.kind.includes('proc-macro')))
    .map((p) => ({
      name: p.name,
      version: p.version,
      license: p.license ?? (p.license_file ? `see ${p.license_file}` : 'unknown'),
      source: p.repository ?? '',
      texts: licenseTexts(dirname(p.manifest_path)),
    }));
}

function npmPackages() {
  return NPM_BUNDLED.map((name) => {
    const dir = join(root, 'node_modules', name);
    const pkg = JSON.parse(readFileSync(join(dir, 'package.json'), 'utf8'));
    const repository = typeof pkg.repository === 'string' ? pkg.repository : (pkg.repository?.url ?? '');
    return { name, version: pkg.version, license: pkg.license ?? 'unknown', source: repository, texts: licenseTexts(dir) };
  });
}

export function notices() {
  const product = JSON.parse(readFileSync(join(root, 'product.json'), 'utf8'));
  const components = [...cratePackages(), ...npmPackages()].sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version));
  const font = join(root, 'crates/vbs-evaluation/assets/fonts');
  const lines = [
    `# Third-party notices – ${product.name}`,
    '',
    `${product.name} (collector and evaluation app for Windows x64) contains the following third-party components.`,
    'Their licenses apply to those components; the product itself is licensed under FSL-1.1-ALv2 (see LICENSE).',
    '',
    '## Liberation Sans (font embedded in the PDF reports)',
    '',
    '```text',
    readFileSync(join(font, 'OFL.txt'), 'utf8').trim(),
    '```',
    '',
    `## Components (${components.length})`,
    '',
    '| Component | Version | License | Source |',
    '|---|---|---|---|',
    ...components.map((c) => `| ${c.name} | ${c.version} | ${c.license} | ${c.source} |`),
    '',
    '## License texts shipped with the components',
    '',
  ];
  const seenTexts = new Map();
  for (const component of components) {
    for (const { name, text } of component.texts) {
      const first = seenTexts.get(text);
      lines.push(`### ${component.name} ${component.version} – ${name}`, '');
      if (first) {
        lines.push(`Same text as ${first}.`, '');
      } else {
        seenTexts.set(text, `${component.name} ${component.version} – ${name}`);
        lines.push('```text', text.replaceAll('```', "'''"), '```', '');
      }
    }
  }
  return `${lines.join('\n')}\n`;
}

const args = process.argv.slice(2);
const out = args.includes('--out') ? args[args.indexOf('--out') + 1] : null;
const text = notices();
if (out) {
  writeFileSync(out, text);
  console.log(`✓ third-party notices written to ${out}`);
} else {
  process.stdout.write(text);
}
