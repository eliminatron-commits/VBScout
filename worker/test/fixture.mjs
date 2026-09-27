// Keys made by the key service's own code with the published TEST signing key (never used for
// real keys). crates/vbs-license verifies them (`key_from_the_key_service_verifies`), which proves
// that the JavaScript signer and the Rust verifier agree on the format. Ed25519 is deterministic,
// so the file is reproducible:
//
//   node test/fixture.mjs           rewrite test/fixtures/issued.json
//   node test/fixture.mjs --check   fail if the code no longer produces exactly that file (CI)

import { readFileSync, writeFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { importSigningKey, issueKey } from '../src/license.js';

/** Test seed 0x07 × 32, base64url. Public knowledge – a key signed with it proves nothing. */
export const TEST_SEED = 'BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc';

export const TEST_LICENSES = [
  { id: 'L-FIXTURE1', type: 'organization', licensee: 'ACME GmbH', issued: '2026-09-27' },
  { id: 'L-FIXTURE2', type: 'msp', licensee: 'Service Nord Sp. z o.o. – Łódź', issued: '2026-09-27', expires: '2027-10-11' },
];

export async function buildFixture() {
  const { privateKey, publicKey } = await importSigningKey(TEST_SEED);
  const keys = [];
  for (const license of TEST_LICENSES) keys.push({ ...license, key: await issueKey(privateKey, license) });
  return `${JSON.stringify({ note: 'TEST key pair – see worker/test/fixture.mjs', publicKey, keys }, null, 2)}\n`;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const path = new URL('./fixtures/issued.json', import.meta.url);
  const fixture = await buildFixture();
  if (process.argv.includes('--check')) {
    if (readFileSync(path, 'utf8') !== fixture) {
      console.error('✗ test/fixtures/issued.json differs from what the key service produces – run "node test/fixture.mjs"');
      process.exit(1);
    }
    console.log('✓ key service fixture is current');
  } else {
    writeFileSync(path, fixture);
    console.log('updated test/fixtures/issued.json');
  }
}
