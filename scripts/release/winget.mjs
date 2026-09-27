#!/usr/bin/env node
// Fills the winget templates (packaging/winget/) for the release files in a folder: version, download
// URL (product.json releaseUrl) and SHA-256 of the files actually built. Output layout as in
// microsoft/winget-pkgs: <out>/manifests/<letter>/<Publisher>/<Name>[/Collector]/<version>/*.yaml
//
//   node scripts/release/winget.mjs --dir <release folder> [--out <folder>] [--date YYYY-MM-DD]
//   node scripts/release/winget.mjs --self-test

import { createHash } from 'node:crypto';
import { mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { downloadUrl, product, releaseFiles, version } from './names.mjs';

const templates = fileURLToPath(new URL('../../packaging/winget/', import.meta.url));

function sha256(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex').toUpperCase();
}

function render(text, values) {
  const out = text.replace(/\{\{(\w+)\}\}/g, (match, name) => {
    if (!(name in values)) throw new Error(`template placeholder ${match} has no value`);
    return values[name];
  });
  return out;
}

/** Writes the manifests of both packages; returns the written files. */
export function writeManifests(dir, out, releaseDate) {
  const names = releaseFiles(version);
  const packages = [
    { kind: 'app', id: `${names.wingetPublisher}.${names.wingetName}`, file: names.setup, path: [names.wingetName] },
    { kind: 'collector', id: `${names.wingetPublisher}.${names.wingetName}.Collector`, file: names.collector, path: [names.wingetName, 'Collector'] },
  ];
  const written = [];
  for (const pkg of packages) {
    const values = {
      id: pkg.id,
      version,
      releaseDate,
      publisher: product.publisher,
      website: product.website,
      name: product.name,
      command: names.collectorCommand,
      resultExtension: product.resultFile.extension,
      installerUrl: downloadUrl(pkg.file, version),
      installerSha256: sha256(join(dir, pkg.file)),
    };
    const target = join(out, 'manifests', names.wingetPublisher[0].toLowerCase(), names.wingetPublisher, ...pkg.path, version);
    mkdirSync(target, { recursive: true });
    for (const template of readdirSync(join(templates, pkg.kind))) {
      const name = template === 'version.yaml' ? `${pkg.id}.yaml` : `${pkg.id}.${template}`;
      const text = render(readFileSync(join(templates, pkg.kind, template), 'utf8'), values);
      writeFileSync(join(target, name), text);
      written.push(join(target, name));
    }
  }
  return written;
}

/** Structural checks a winget validator would do first. */
export function checkManifest(text) {
  const problems = [];
  if (/\{\{|\}\}/.test(text)) problems.push('unfilled placeholder');
  for (const field of ['PackageIdentifier', 'PackageVersion', 'ManifestType', 'ManifestVersion']) {
    if (!new RegExp(`^${field}: \\S`, 'm').test(text)) problems.push(`missing ${field}`);
  }
  const id = text.match(/^PackageIdentifier: (.+)$/m)?.[1] ?? '';
  if (!/^[A-Za-z0-9]+(\.[A-Za-z0-9]+){1,7}$/.test(id)) problems.push(`invalid PackageIdentifier ${id}`);
  const hash = text.match(/InstallerSha256: (.+)$/m)?.[1];
  if (hash !== undefined && !/^[0-9A-F]{64}$/.test(hash)) problems.push('invalid InstallerSha256');
  const url = text.match(/InstallerUrl: (.+)$/m)?.[1];
  if (url !== undefined && !url.startsWith('https://')) problems.push('InstallerUrl must be https');
  return problems;
}

function selfTest() {
  const dir = mkdtempSync(join(tmpdir(), 'winget-'));
  try {
    const names = releaseFiles(version);
    writeFileSync(join(dir, names.setup), 'setup');
    writeFileSync(join(dir, names.collector), 'collector');
    const files = writeManifests(dir, join(dir, 'out'), '2026-09-27');
    if (files.length !== 6) throw new Error(`expected 6 manifests, got ${files.length}`);
    for (const file of files) {
      const problems = checkManifest(readFileSync(file, 'utf8'));
      if (problems.length) throw new Error(`${file}: ${problems.join(', ')}`);
    }
    const installer = readFileSync(files.find((f) => f.endsWith('.Collector.installer.yaml')), 'utf8');
    if (!installer.includes(sha256(join(dir, names.collector)))) throw new Error('collector hash missing');
    if (!installer.includes(downloadUrl(names.collector))) throw new Error('collector URL missing');
    if (checkManifest('PackageIdentifier: bad id\n').length === 0) throw new Error('checker accepts a bad manifest');
    console.log('✓ winget manifests: templates render completely (self-test)');
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const args = process.argv.slice(2);
  const option = (name) => {
    const index = args.indexOf(name);
    return index >= 0 ? args[index + 1] : undefined;
  };
  if (args.includes('--self-test')) {
    selfTest();
  } else {
    const dir = option('--dir');
    if (!dir) {
      console.error('usage: winget.mjs --dir <release folder> [--out <folder>] [--date YYYY-MM-DD] | --self-test');
      process.exit(2);
    }
    const date = option('--date') ?? new Date().toISOString().slice(0, 10);
    const files = writeManifests(dir, option('--out') ?? join(dir, 'winget'), date);
    let failed = false;
    for (const file of files) {
      const problems = checkManifest(readFileSync(file, 'utf8'));
      if (problems.length) {
        failed = true;
        console.error(`✗ ${file}: ${problems.join(', ')}`);
      }
    }
    if (failed) process.exit(1);
    console.log(`✓ ${files.length} winget manifests written`);
  }
}
