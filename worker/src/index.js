// Key service (Cloudflare Worker): turns completed Paddle transactions into signed license keys.
//
//   POST /paddle/webhook   Paddle Billing notification (transaction.completed); signature checked
//   GET  /license/<txn>    the key of a transaction for the checkout's success page
//   POST /license/<txn>    { "licensee": "…" } – completes a transaction whose checkout carried no
//                          licensee name (only while no key exists yet)
//
// The app never talks to this service: keys are verified offline (crates/vbs-license). Bindings and
// secrets: see wrangler.toml and docs/licensing.md.

import { cleanLicensee, importSigningKey, isoDate, issueKey, newKeyId, verifyKey } from './license.js';
import { licenseRequest, paddleGet, verifyPaddleSignature } from './paddle.js';

const TRANSACTION_ID = /^txn_[a-z0-9]{26}$/;

function json(body, status = 200, headers = {}) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json; charset=utf-8', 'cache-control': 'no-store', ...headers },
  });
}

function cors(env) {
  return env.ALLOWED_ORIGIN
    ? { 'access-control-allow-origin': env.ALLOWED_ORIGIN, 'access-control-allow-methods': 'GET, POST', 'access-control-allow-headers': 'content-type', vary: 'origin' }
    : {};
}

/** Last day of an MSP license: end of the paid period plus the grace days (renewals issue a new key). */
export function mspExpiry(periodEnd, issued, graceDays) {
  const base = periodEnd ? new Date(periodEnd) : new Date(`${issued}T00:00:00Z`);
  if (Number.isNaN(base.getTime())) return null;
  if (!periodEnd) base.setUTCFullYear(base.getUTCFullYear() + 1);
  base.setUTCDate(base.getUTCDate() + graceDays);
  return isoDate(base);
}

function graceDays(env) {
  const days = Number.parseInt(env.MSP_GRACE_DAYS ?? '14', 10);
  return Number.isInteger(days) && days >= 0 && days <= 60 ? days : 14;
}

/** Signs and stores the key of a transaction; returns the stored record. */
async function issueFor(env, record, licensee, now) {
  const { privateKey, publicKey } = await importSigningKey(env.LICENSE_SIGNING_KEY);
  // The key must verify with the public key built into the app (product.json license.publicKey).
  if (env.LICENSE_PUBLIC_KEY && env.LICENSE_PUBLIC_KEY !== publicKey) {
    throw new Error('LICENSE_SIGNING_KEY does not belong to LICENSE_PUBLIC_KEY');
  }
  const issued = isoDate(now);
  const license = { id: newKeyId(), type: record.type, licensee, issued };
  if (record.type === 'msp') license.expires = mspExpiry(record.periodEnd, issued, graceDays(env));
  const key = await issueKey(privateKey, license);
  if (!(await verifyKey(publicKey, key))) throw new Error('self-check of the new key failed');
  const stored = { ...record, status: 'ready', key, ...license };
  await env.LICENSES.put(`txn:${record.transactionId}`, JSON.stringify(stored));
  if (record.subscriptionId) await env.LICENSES.put(`sub:${record.subscriptionId}`, record.transactionId);
  await deliverByEmail(env, stored);
  return stored;
}

/** Sends the key to the buyer when e-mail delivery is configured (Resend API); never fails the request. */
async function deliverByEmail(env, record) {
  if (!env.RESEND_API_KEY || !env.EMAIL_FROM || !record.customerId) return;
  try {
    const customer = await paddleGet(env, `/customers/${record.customerId}`);
    if (!customer?.email) return;
    const product = env.PRODUCT_NAME ?? 'VBScout';
    const lines = [
      `Thank you for your purchase of ${product}.`,
      '',
      `License: ${record.type === 'msp' ? 'MSP (yearly)' : 'Organization'} – ${record.licensee}`,
      ...(record.expires ? [`Valid through: ${record.expires}`] : []),
      '',
      'Your license key (paste it into the app under Settings → License):',
      '',
      record.key,
      '',
      `Key ID for support requests: ${record.id}`,
    ];
    const doFetch = env.fetch ?? fetch;
    await doFetch('https://api.resend.com/emails', {
      method: 'POST',
      headers: { Authorization: `Bearer ${env.RESEND_API_KEY}`, 'content-type': 'application/json' },
      body: JSON.stringify({ from: env.EMAIL_FROM, to: [customer.email], subject: `Your ${product} license key`, text: lines.join('\n') }),
    });
  } catch (error) {
    console.error('e-mail delivery failed', error?.message);
  }
}

async function handleWebhook(request, env, now) {
  const rawBody = await request.text();
  const signed = await verifyPaddleSignature(request.headers.get('paddle-signature'), rawBody, env.PADDLE_WEBHOOK_SECRET, Math.floor(now.getTime() / 1000));
  if (!signed) return json({ error: 'invalid signature' }, 401);
  let event;
  try {
    event = JSON.parse(rawBody);
  } catch {
    return json({ error: 'invalid JSON' }, 400);
  }
  const wanted = licenseRequest(event, env);
  if (!wanted) return json({ status: 'ignored' });
  // Paddle retries notifications: one key per transaction.
  if (await env.LICENSES.get(`txn:${wanted.transactionId}`)) return json({ status: 'exists' });

  let licensee = cleanLicensee(wanted.licensee);
  if (!licensee && wanted.customerId && wanted.businessId) {
    const business = await paddleGet(env, `/customers/${wanted.customerId}/businesses/${wanted.businessId}`);
    licensee = cleanLicensee(business?.name);
  }
  if (!licensee) {
    const pending = { ...wanted, status: 'pending', reason: 'licensee' };
    await env.LICENSES.put(`txn:${wanted.transactionId}`, JSON.stringify(pending));
    return json({ status: 'pending' });
  }
  await issueFor(env, wanted, licensee, now);
  return json({ status: 'issued' });
}

function publicView(record) {
  if (record.status !== 'ready') return { status: record.status, reason: record.reason ?? null, type: record.type };
  return { status: 'ready', key: record.key, id: record.id, type: record.type, licensee: record.licensee, expires: record.expires ?? null };
}

async function handleLicense(request, env, transactionId, now) {
  const headers = cors(env);
  const stored = await env.LICENSES.get(`txn:${transactionId}`);
  // Paddle may call the success page before the webhook arrived.
  if (!stored) return json({ status: 'unknown' }, 404, headers);
  const record = JSON.parse(stored);
  if (request.method === 'GET') return json(publicView(record), 200, headers);

  if (record.status !== 'pending') return json({ error: 'the key has already been issued' }, 409, headers);
  let body;
  try {
    body = await request.json();
  } catch {
    return json({ error: 'invalid JSON' }, 400, headers);
  }
  const licensee = cleanLicensee(body?.licensee);
  if (!licensee) return json({ error: 'licensee required' }, 400, headers);
  return json(publicView(await issueFor(env, record, licensee, now)), 200, headers);
}

export default {
  async fetch(request, env) {
    const now = env.now ? env.now() : new Date();
    const url = new URL(request.url);
    try {
      if (url.pathname === '/paddle/webhook' && request.method === 'POST') return await handleWebhook(request, env, now);
      const match = url.pathname.match(/^\/license\/([^/]+)$/);
      if (match) {
        if (request.method === 'OPTIONS') return new Response(null, { status: 204, headers: cors(env) });
        if (!TRANSACTION_ID.test(match[1])) return json({ error: 'unknown transaction' }, 404, cors(env));
        if (request.method === 'GET' || request.method === 'POST') return await handleLicense(request, env, match[1], now);
      }
      if (url.pathname === '/' && request.method === 'GET') return new Response('ok');
      return json({ error: 'not found' }, 404);
    } catch (error) {
      console.error('request failed', error?.message);
      return json({ error: 'internal error' }, 500);
    }
  },
};
