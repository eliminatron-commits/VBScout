#!/usr/bin/env node
// What a public release needs besides green CI – the release pipeline stops on any of these:
//
//  • product.json license.publicKey is set (otherwise the app accepts no license key at all)
//  • no placeholders left: example domains, the example publisher, the LICENSE licensor
//  • the tag matches the product version (v<version>)
//
//   node scripts/release/check-release.mjs [--tag v1.2.3]   fail on problems (release pipeline)
//   node scripts/release/check-release.mjs --warn           only list them (CI, development builds)

import { readFileSync } from 'node:fs';

import { product, version } from './names.mjs';

const args = process.argv.slice(2);
const warnOnly = args.includes('--warn');
const tag = args.includes('--tag') ? args[args.indexOf('--tag') + 1] : undefined;
const problems = [];

if (!product.license?.publicKey) {
  problems.push('product.json license.publicKey is null – create the signing key (cargo run -p license-keys -- keygen, docs/licensing.md)');
}
for (const [field, value] of [['website', product.website], ['supportEmail', product.supportEmail], ['releaseUrl', product.releaseUrl]]) {
  if (/\.example\b/.test(value)) problems.push(`product.json ${field} is still a placeholder (${value})`);
}
if (/^example\b/i.test(product.publisher)) problems.push(`product.json publisher is still a placeholder (${product.publisher})`);
const license = readFileSync(new URL('../../LICENSE', import.meta.url), 'utf8');
if (/\[Licensor/i.test(license) || /Licensor: *$/m.test(license)) problems.push('LICENSE: the licensor is not filled in');
if (tag !== undefined && tag !== `v${version}`) problems.push(`tag ${tag} does not match the product version v${version}`);

if (problems.length === 0) {
  console.log('✓ release configuration complete');
} else {
  for (const problem of problems) console[warnOnly ? 'log' : 'error'](`${warnOnly ? '⚠' : '✗'} ${problem}`);
  if (!warnOnly) process.exit(1);
  console.log(`(${problems.length} open item(s) before a public release – see PROGRESS.md)`);
}
