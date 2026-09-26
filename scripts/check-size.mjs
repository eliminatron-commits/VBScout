#!/usr/bin/env node
// Definition of Done #3: the collector stays below 10 MB (decimal megabytes); the app is checked too.
//   node scripts/check-size.mjs <file> <maxMegabytes>

import { statSync } from 'node:fs';

const [file, limitArg] = process.argv.slice(2);
if (!file) {
  console.error('usage: check-size.mjs <file> <maxMegabytes>');
  process.exit(2);
}
const limit = Number(limitArg) * 1_000_000;
const size = statSync(file).size;
const mb = (bytes) => (bytes / 1_000_000).toFixed(2);
if (size >= limit) {
  console.error(`✗ ${file}: ${mb(size)} MB – limit ${mb(limit)} MB`);
  process.exit(1);
}
console.log(`✓ ${file}: ${mb(size)} MB (limit ${mb(limit)} MB)`);
