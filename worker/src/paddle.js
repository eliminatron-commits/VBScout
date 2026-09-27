// Paddle Billing webhooks: signature check and what a completed transaction means for a license.
// Paddle signs `<ts>:<raw body>` with HMAC-SHA256 and the notification destination's secret key and
// sends `Paddle-Signature: ts=<unix seconds>;h1=<hex>` (several h1 values while a secret rotates).

const encoder = new TextEncoder();

function hex(bytes) {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('');
}

/** Constant-time comparison of two hex strings. */
function sameHex(a, b) {
  if (a.length !== b.length) return false;
  let difference = 0;
  for (let i = 0; i < a.length; i += 1) difference |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return difference === 0;
}

async function hmacHex(secret, text) {
  const key = await crypto.subtle.importKey('raw', encoder.encode(secret), { name: 'HMAC', hash: 'SHA-256' }, false, ['sign']);
  return hex(new Uint8Array(await crypto.subtle.sign('HMAC', key, encoder.encode(text))));
}

/**
 * Checks the `Paddle-Signature` header of a webhook.
 * @returns {Promise<boolean>}
 */
export async function verifyPaddleSignature(header, rawBody, secret, nowSeconds, toleranceSeconds = 300) {
  if (!header || !secret) return false;
  const parts = header.split(';').map((part) => part.trim().split('='));
  const ts = parts.find(([name]) => name === 'ts')?.[1];
  const signatures = parts.filter(([name]) => name === 'h1').map(([, value]) => value ?? '');
  if (!ts || !/^\d+$/.test(ts) || signatures.length === 0) return false;
  if (Math.abs(nowSeconds - Number(ts)) > toleranceSeconds) return false;
  const expected = await hmacHex(secret, `${ts}:${rawBody}`);
  return signatures.some((signature) => sameHex(signature.toLowerCase(), expected));
}

/** Header value Paddle would send (used by the tests and for local trials). */
export async function signPaddleBody(rawBody, secret, ts) {
  return `ts=${ts};h1=${await hmacHex(secret, `${ts}:${rawBody}`)}`;
}

/**
 * What a `transaction.completed` event asks for: the license type per configured price, the
 * licensee from the checkout's custom data, and – for the yearly MSP license – the end of the paid
 * period. Returns null for transactions without a license price (other products of the vendor).
 */
export function licenseRequest(event, env) {
  if (event?.event_type !== 'transaction.completed') return null;
  const transaction = event.data ?? {};
  const prices = transaction.items?.map((item) => item?.price?.id) ?? [];
  let type = null;
  if (env.PADDLE_PRICE_MSP && prices.includes(env.PADDLE_PRICE_MSP)) type = 'msp';
  else if (env.PADDLE_PRICE_ORGANIZATION && prices.includes(env.PADDLE_PRICE_ORGANIZATION)) type = 'organization';
  if (!type || typeof transaction.id !== 'string') return null;
  return {
    transactionId: transaction.id,
    type,
    customerId: transaction.customer_id ?? null,
    businessId: transaction.business_id ?? null,
    subscriptionId: transaction.subscription_id ?? null,
    licensee: transaction.custom_data?.licensee ?? null,
    periodEnd: transaction.billing_period?.ends_at ?? null,
    occurredAt: event.occurred_at ?? null,
  };
}

/** Base URL of the Paddle API for the configured environment (sandbox until the user approves). */
export function paddleApiBase(env) {
  return env.PADDLE_ENVIRONMENT === 'production' ? 'https://api.paddle.com' : 'https://sandbox-api.paddle.com';
}

/** Reads one Paddle API entity (customer or business); null when not configured or not found. */
export async function paddleGet(env, path) {
  if (!env.PADDLE_API_KEY) return null;
  const doFetch = env.fetch ?? fetch;
  const response = await doFetch(`${paddleApiBase(env)}${path}`, {
    headers: { Authorization: `Bearer ${env.PADDLE_API_KEY}` },
  });
  if (!response.ok) return null;
  return (await response.json())?.data ?? null;
}
