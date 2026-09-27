#!/usr/bin/env node
// Propagates product.json – the single source of truth for name, identifier,
// URLs and prices – into files that cannot read it directly, and checks that
// version numbers agree. (Taken over from Stepwright.)
//
//   node scripts/sync-config.mjs          update derived files
//   node scripts/sync-config.mjs --check  fail if anything is out of sync (CI)

import { readFileSync, writeFileSync } from 'node:fs';

const root = new URL('../', import.meta.url);
const check = process.argv.includes('--check');
const problems = [];

const readText = (path) => readFileSync(new URL(path, root), 'utf8');
const readJson = (path) => JSON.parse(readText(path));
const product = readJson('product.json');

/** Applies `update` to a JSON file and writes it (or reports drift in --check mode). */
function syncJson(path, update) {
  const before = readText(path);
  const data = JSON.parse(before);
  update(data);
  const after = `${JSON.stringify(data, null, 2)}\n`;
  if (after === before) return;
  if (check) {
    problems.push(`${path} is out of sync with product.json – run "npm run sync:config"`);
  } else {
    writeFileSync(new URL(path, root), after);
    console.log(`updated ${path}`);
  }
}

syncJson('src-tauri/tauri.conf.json', (conf) => {
  conf.productName = product.name;
  conf.identifier = product.identifier;
  for (const window of conf.app.windows) window.title = product.name;
  conf.bundle.publisher = product.publisher;
});

// One version for the whole product: Cargo workspace ⇔ package.json.
const cargoVersion = readText('Cargo.toml').match(/\[workspace\.package\][^[]*?\nversion\s*=\s*"([^"]+)"/)?.[1];
const packageVersion = readJson('package.json').version;
if (!cargoVersion) problems.push('Cargo.toml: [workspace.package] version not found');
else if (cargoVersion !== packageVersion) {
  problems.push(`version mismatch: Cargo.toml ${cargoVersion} ≠ package.json ${packageVersion}`);
}

if (problems.length) {
  for (const problem of problems) console.error(`✗ ${problem}`);
  process.exit(1);
}
console.log(check ? '✓ derived configuration is in sync with product.json' : '✓ configuration synced');
