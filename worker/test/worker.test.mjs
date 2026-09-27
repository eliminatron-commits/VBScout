// Tests of the key service with Node's test runner and WebCrypto – no Cloudflare runtime needed:
// the Worker is called with an in-memory KV namespace and a stubbed fetch for the Paddle API.
//
//   cd worker && npm test

import assert from 'node:assert/strict';
import { test } from 'node:test';

import worker, { mspExpiry } from '../src/index.js';
import { cleanLicensee, importSigningKey, issueKey, validateLicense, verifyKey } from '../src/license.js';
import { licenseRequest, signPaddleBody, verifyPaddleSignature } from '../src/paddle.js';
import { buildFixture, TEST_SEED } from './fixture.mjs';

const NOW = new Date('2026-09-27T10:00:00Z');
const NOW_SECONDS = Math.floor(NOW.getTime() / 1000);
const WEBHOOK_SECRET = 'pdl_ntfset_test_secret';
const PRICE_ORG = 'pri_01organization00000000000';
const PRICE_MSP = 'pri_01msp000000000000000000000';
const TXN = 'txn_01k6abcdefghjkmnpqrstvwxyz';
const TXN_MSP = 'txn_01k6zyxwvtsrqpnmkjhgfedcba';

class MemoryKv {
  constructor() {
    this.data = new Map();
  }
  async get(key) {
    return this.data.get(key) ?? null;
  }
  async put(key, value) {
    this.data.set(key, value);
  }
}

async function environment(overrides = {}) {
  const { publicKey } = await importSigningKey(TEST_SEED);
  const calls = [];
  return {
    LICENSES: new MemoryKv(),
    LICENSE_SIGNING_KEY: TEST_SEED,
    LICENSE_PUBLIC_KEY: publicKey,
    PADDLE_WEBHOOK_SECRET: WEBHOOK_SECRET,
    PADDLE_PRICE_ORGANIZATION: PRICE_ORG,
    PADDLE_PRICE_MSP: PRICE_MSP,
    PADDLE_ENVIRONMENT: 'sandbox',
    MSP_GRACE_DAYS: '14',
    now: () => NOW,
    calls,
    fetch: async (url, init) => {
      calls.push({ url, init });
      if (url.endsWith('/customers/ctm_1/businesses/biz_1')) return Response.json({ data: { name: '  Muster\u0007 AG  ' } });
      if (url.endsWith('/customers/ctm_1')) return Response.json({ data: { email: 'buyer@example.com' } });
      if (url === 'https://api.resend.com/emails') return Response.json({ id: 'mail' });
      return new Response('not found', { status: 404 });
    },
    ...overrides,
  };
}

function completed({ id = TXN, price = PRICE_ORG, customData = { licensee: 'ACME GmbH' }, extra = {} } = {}) {
  return {
    event_id: 'evt_1',
    event_type: 'transaction.completed',
    occurred_at: NOW.toISOString(),
    data: { id, status: 'completed', customer_id: 'ctm_1', custom_data: customData, items: [{ price: { id: price }, quantity: 1 }], ...extra },
  };
}

async function webhook(env, event, { secret = WEBHOOK_SECRET, ts = NOW_SECONDS } = {}) {
  const body = JSON.stringify(event);
  const request = new Request('https://keys.example/paddle/webhook', {
    method: 'POST',
    headers: { 'paddle-signature': await signPaddleBody(body, secret, ts), 'content-type': 'application/json' },
    body,
  });
  return worker.fetch(request, env);
}

const getLicense = (env, id) => worker.fetch(new Request(`https://keys.example/license/${id}`), env);

test('fixture for the Rust verifier is reproducible', async () => {
  const { readFileSync } = await import('node:fs');
  assert.equal(await buildFixture(), readFileSync(new URL('./fixtures/issued.json', import.meta.url), 'utf8'));
});

test('Paddle signatures: valid, wrong secret, changed body, old timestamp, rotated secrets', async () => {
  const body = '{"event_type":"transaction.completed"}';
  const header = await signPaddleBody(body, WEBHOOK_SECRET, NOW_SECONDS);
  assert.equal(await verifyPaddleSignature(header, body, WEBHOOK_SECRET, NOW_SECONDS), true);
  assert.equal(await verifyPaddleSignature(header, body, 'other', NOW_SECONDS), false);
  assert.equal(await verifyPaddleSignature(header, `${body} `, WEBHOOK_SECRET, NOW_SECONDS), false);
  assert.equal(await verifyPaddleSignature(header, body, WEBHOOK_SECRET, NOW_SECONDS + 301), false);
  assert.equal(await verifyPaddleSignature(null, body, WEBHOOK_SECRET, NOW_SECONDS), false);
  assert.equal(await verifyPaddleSignature('ts=abc;h1=00', body, WEBHOOK_SECRET, NOW_SECONDS), false);
  const rotated = `${header};h1=${'0'.repeat(64)}`;
  assert.equal(await verifyPaddleSignature(rotated, body, WEBHOOK_SECRET, NOW_SECONDS), true);
});

test('organization purchase issues a key the app accepts', async () => {
  const env = await environment();
  const response = await webhook(env, completed());
  assert.deepEqual(await response.json(), { status: 'issued' });
  const license = await (await getLicense(env, TXN)).json();
  assert.equal(license.status, 'ready');
  assert.equal(license.type, 'organization');
  assert.equal(license.licensee, 'ACME GmbH');
  assert.equal(license.expires, null);
  const payload = await verifyKey(env.LICENSE_PUBLIC_KEY, license.key);
  assert.deepEqual(payload, { v: 1, product: 'vbs', id: license.id, type: 'organization', licensee: 'ACME GmbH', issued: '2026-09-27' });
});

test('webhook retries do not issue a second key', async () => {
  const env = await environment();
  await webhook(env, completed());
  const first = (await (await getLicense(env, TXN)).json()).key;
  assert.deepEqual(await (await webhook(env, completed())).json(), { status: 'exists' });
  assert.equal((await (await getLicense(env, TXN)).json()).key, first);
});

test('MSP subscription: expiry = end of the paid period + grace days; renewal issues a new key', async () => {
  const env = await environment();
  const event = completed({ id: TXN_MSP, price: PRICE_MSP, customData: { licensee: 'IT Service Nord' }, extra: { subscription_id: 'sub_1', billing_period: { starts_at: '2026-09-27T10:00:00Z', ends_at: '2027-09-27T10:00:00Z' } } });
  await webhook(env, event);
  const license = await (await getLicense(env, TXN_MSP)).json();
  assert.equal(license.type, 'msp');
  assert.equal(license.expires, '2027-10-11');
  assert.equal((await verifyKey(env.LICENSE_PUBLIC_KEY, license.key)).expires, '2027-10-11');
  assert.equal(await env.LICENSES.get('sub:sub_1'), TXN_MSP);
  assert.equal(mspExpiry(null, '2026-09-27', 14), '2027-10-11');
  assert.equal(mspExpiry('2028-02-20T00:00:00Z', '2027-09-27', 14), '2028-03-05');
});

test('unknown prices, other events and bad signatures issue nothing', async () => {
  const env = await environment();
  assert.deepEqual(await (await webhook(env, completed({ price: 'pri_other' }))).json(), { status: 'ignored' });
  assert.deepEqual(await (await webhook(env, { ...completed(), event_type: 'transaction.created' })).json(), { status: 'ignored' });
  assert.equal((await webhook(env, completed(), { secret: 'forged' })).status, 401);
  assert.equal((await webhook(env, completed(), { ts: NOW_SECONDS - 3600 })).status, 401);
  assert.equal(env.LICENSES.data.size, 0);
});

test('licensee from the Paddle business when the checkout had none, cleaned', async () => {
  const env = await environment({ PADDLE_API_KEY: 'pdl_sdbx_apikey_test' });
  await webhook(env, completed({ customData: null, extra: { business_id: 'biz_1' } }));
  const license = await (await getLicense(env, TXN)).json();
  assert.equal(license.licensee, 'Muster AG');
  assert.ok(env.calls.every((call) => call.url.startsWith('https://sandbox-api.paddle.com/')), 'sandbox API until production is switched on');
});

test('without a licensee the transaction waits; the success page completes it once', async () => {
  const env = await environment();
  assert.deepEqual(await (await webhook(env, completed({ customData: {} }))).json(), { status: 'pending' });
  assert.deepEqual(await (await getLicense(env, TXN)).json(), { status: 'pending', reason: 'licensee', type: 'organization' });
  const post = (licensee) => worker.fetch(new Request(`https://keys.example/license/${TXN}`, { method: 'POST', body: JSON.stringify({ licensee }) }), env);
  assert.equal((await post('   ')).status, 400);
  const issued = await (await post('Beispiel e.V.')).json();
  assert.equal(issued.status, 'ready');
  assert.equal(issued.licensee, 'Beispiel e.V.');
  assert.equal((await post('Someone Else')).status, 409, 'the name cannot be changed afterwards');
});

test('e-mail delivery when configured', async () => {
  const env = await environment({ PADDLE_API_KEY: 'pdl_sdbx_apikey_test', RESEND_API_KEY: 're_test', EMAIL_FROM: 'VBScout <keys@vbscout.example>' });
  await webhook(env, completed());
  const mail = env.calls.find((call) => call.url === 'https://api.resend.com/emails');
  const body = JSON.parse(mail.init.body);
  assert.deepEqual(body.to, ['buyer@example.com']);
  assert.match(body.text, /VBS1-/);
});

test('a signing key that does not match the app\'s public key issues nothing', async () => {
  const env = await environment({ LICENSE_PUBLIC_KEY: 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA' });
  assert.equal((await webhook(env, completed())).status, 500);
  assert.equal(await env.LICENSES.get(`txn:${TXN}`), null);
});

test('license lookups: unknown and malformed transaction ids', async () => {
  const env = await environment();
  assert.equal((await getLicense(env, TXN)).status, 404);
  assert.equal((await getLicense(env, '../secret')).status, 404);
  assert.equal((await worker.fetch(new Request('https://keys.example/'), env)).status, 200);
});

test('validation matches the app: names, dates, types', async () => {
  const good = { id: 'L-1', type: 'organization', licensee: 'A', issued: '2026-09-27' };
  assert.deepEqual(validateLicense(good), []);
  assert.deepEqual(validateLicense({ ...good, licensee: ' A' }), ['licensee']);
  assert.deepEqual(validateLicense({ ...good, licensee: 'x'.repeat(121) }), ['licensee']);
  assert.deepEqual(validateLicense({ ...good, issued: '2026-02-30' }), ['issued']);
  assert.deepEqual(validateLicense({ ...good, type: 'msp' }), ['expires']);
  assert.deepEqual(validateLicense({ ...good, type: 'msp', expires: '2026-09-26' }), ['expires']);
  assert.deepEqual(validateLicense({ ...good, type: 'team' }), ['type']);
  assert.equal(cleanLicensee(' ACME\n\tGmbH '), 'ACME GmbH');
  assert.equal(cleanLicensee('\u0000'), null);
  assert.equal([...cleanLicensee('Ł'.repeat(200))].length, 120);
  const { privateKey } = await importSigningKey(TEST_SEED);
  await assert.rejects(issueKey(privateKey, { ...good, licensee: '' }));
});

test('tampered keys fail verification', async () => {
  const env = await environment();
  const { privateKey } = await importSigningKey(TEST_SEED);
  const key = await issueKey(privateKey, { id: 'L-1', type: 'organization', licensee: 'ACME GmbH', issued: '2026-09-27' });
  const [payload, signature] = key.slice(5).split('.');
  const forged = btoa(atob(payload.replaceAll('-', '+').replaceAll('_', '/')).replace('ACME', 'EVIL')).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '');
  assert.equal(await verifyKey(env.LICENSE_PUBLIC_KEY, `VBS1-${forged}.${signature}`), null);
  assert.equal(await verifyKey(env.LICENSE_PUBLIC_KEY, 'VBS1-abc'), null);
});

test('license request mapping', () => {
  const env = { PADDLE_PRICE_ORGANIZATION: PRICE_ORG, PADDLE_PRICE_MSP: PRICE_MSP };
  assert.equal(licenseRequest(completed(), env).type, 'organization');
  assert.equal(licenseRequest(completed({ price: PRICE_MSP }), env).type, 'msp');
  assert.equal(licenseRequest(completed(), {}), null, 'no configured prices → nothing');
});
