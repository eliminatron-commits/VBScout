// License keys, format version 1 – the JavaScript counterpart of crates/vbs-license (which verifies
// them offline in the app). A key is `VBS1-<payload>.<signature>`: the payload is compact JSON, the
// signature Ed25519 (RFC 8032) over exactly the payload bytes, both base64url without padding.
// See docs/licensing.md.

export const KEY_PREFIX = 'VBS1-';
export const PRODUCT_CODE = 'vbs';
export const FORMAT_VERSION = 1;
export const MAX_LICENSEE_CHARS = 120;

// PKCS#8 wrapper of a raw 32-byte Ed25519 seed (RFC 8410), so WebCrypto can import the seed that
// tools/license-keys writes (the Worker secret LICENSE_SIGNING_KEY).
const PKCS8_ED25519_PREFIX = [0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20];

export function base64url(bytes) {
  let binary = '';
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '');
}

export function fromBase64url(text) {
  if (!/^[A-Za-z0-9_-]*$/.test(text)) throw new Error('not base64url');
  const binary = atob(text.replaceAll('-', '+').replaceAll('_', '/'));
  return Uint8Array.from(binary, (c) => c.charCodeAt(0));
}

/** Imports the signing key from its base64url seed; also returns the public key (base64url). */
export async function importSigningKey(seedBase64url) {
  const seed = fromBase64url(String(seedBase64url ?? '').trim());
  if (seed.length !== 32) throw new Error('LICENSE_SIGNING_KEY must be a base64url Ed25519 seed (32 bytes)');
  const pkcs8 = new Uint8Array([...PKCS8_ED25519_PREFIX, ...seed]);
  const privateKey = await crypto.subtle.importKey('pkcs8', pkcs8, { name: 'Ed25519' }, true, ['sign']);
  const { x } = await crypto.subtle.exportKey('jwk', privateKey);
  return { privateKey, publicKey: x };
}

/** `YYYY-MM-DD` of a Date (UTC). */
export function isoDate(date) {
  return date.toISOString().slice(0, 10);
}

/** Random key identifier for support requests, e.g. `L-7Q3K9M2X` (Crockford base32). */
export function newKeyId() {
  const alphabet = '0123456789ABCDEFGHJKMNPQRSTVWXYZ';
  const bytes = crypto.getRandomValues(new Uint8Array(8));
  return `L-${Array.from(bytes, (b) => alphabet[b % 32]).join('')}`;
}

/**
 * Checks a license the same way the app does (crates/vbs-license, `Payload::into_license`), so the
 * service never hands out a key the app would refuse.
 */
export function validateLicense({ id, type, licensee, issued, expires }) {
  const problems = [];
  if (!/^[A-Za-z0-9_-]{1,40}$/.test(id ?? '')) problems.push('id');
  if (typeof licensee !== 'string' || licensee.trim() !== licensee || licensee.length === 0 ||
      [...licensee].length > MAX_LICENSEE_CHARS || /\p{Cc}/u.test(licensee)) problems.push('licensee');
  const date = /^\d{4}-\d{2}-\d{2}$/;
  const valid = (text) => date.test(text ?? '') && isoDate(new Date(`${text}T00:00:00Z`)) === text;
  if (!valid(issued)) problems.push('issued');
  if (type === 'organization') {
    if (expires !== undefined) problems.push('expires');
  } else if (type === 'msp') {
    if (!valid(expires) || expires < issued) problems.push('expires');
  } else {
    problems.push('type');
  }
  return problems;
}

/** Cleans a licensee name from a checkout form: collapses whitespace, drops control characters. */
export function cleanLicensee(text) {
  if (typeof text !== 'string') return null;
  const cleaned = text.replace(/\p{Cc}/gu, ' ').replace(/\s+/g, ' ').trim();
  if (!cleaned) return null;
  return [...cleaned].slice(0, MAX_LICENSEE_CHARS).join('').trim();
}

/** Signs a license; throws if the app would refuse it. Returns the key text. */
export async function issueKey(privateKey, license) {
  const problems = validateLicense(license);
  if (problems.length) throw new Error(`invalid license: ${problems.join(', ')}`);
  const payload = {
    v: FORMAT_VERSION,
    product: PRODUCT_CODE,
    id: license.id,
    type: license.type,
    licensee: license.licensee,
    issued: license.issued,
    ...(license.type === 'msp' ? { expires: license.expires } : {}),
  };
  const bytes = new TextEncoder().encode(JSON.stringify(payload));
  const signature = new Uint8Array(await crypto.subtle.sign('Ed25519', privateKey, bytes));
  return `${KEY_PREFIX}${base64url(bytes)}.${base64url(signature)}`;
}

/** Verifies a key against a public key (base64url); returns the payload or null. */
export async function verifyKey(publicKeyBase64url, key) {
  const text = String(key).replace(/\s+/g, '');
  if (!text.startsWith(KEY_PREFIX)) return null;
  const [payload, signature, ...rest] = text.slice(KEY_PREFIX.length).split('.');
  if (!payload || !signature || rest.length) return null;
  try {
    const publicKey = await crypto.subtle.importKey('raw', fromBase64url(publicKeyBase64url), { name: 'Ed25519' }, false, ['verify']);
    const bytes = fromBase64url(payload);
    const ok = await crypto.subtle.verify('Ed25519', publicKey, fromBase64url(signature), bytes);
    return ok ? JSON.parse(new TextDecoder().decode(bytes)) : null;
  } catch {
    return null;
  }
}
