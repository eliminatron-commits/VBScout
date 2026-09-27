// Names of the release files, derived from product.json (the product name is swappable, the internal
// `vbs-` names of the build are not user-visible). Used by package.ps1, winget.mjs and release.yml.
//
//   node scripts/release/names.mjs            JSON with version and file names (for PowerShell)

import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

const root = new URL('../../', import.meta.url);
export const product = JSON.parse(readFileSync(new URL('product.json', root), 'utf8'));

/** The single product version (Cargo workspace = package.json, checked by sync-config). */
export const version = JSON.parse(readFileSync(new URL('package.json', root), 'utf8')).version;

const slug = product.name.replace(/[^A-Za-z0-9]/g, '');

export function releaseFiles(v = version) {
  return {
    version: v,
    collector: `${slug}-Collector-${v}-x64.exe`,
    setup: `${slug}-${v}-x64-setup.exe`,
    portable: `${slug}-${v}-x64-portable.exe`,
    checksums: `${slug}-${v}-SHA256SUMS.txt`,
    winget: `${slug}-${v}-winget-manifests.zip`,
    notices: `${slug}-${v}-THIRD-PARTY-NOTICES.md`,
    collectorCommand: `${slug.toLowerCase()}-collector`,
    wingetPublisher: product.publisher.replace(/[^A-Za-z0-9]/g, ''),
    wingetName: slug,
  };
}

export function downloadUrl(file, v = version) {
  return product.releaseUrl.replaceAll('{version}', v).replaceAll('{file}', file);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  console.log(JSON.stringify(releaseFiles()));
}
