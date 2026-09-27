# Licensing – key format, key service, operations

The collector is always free. The evaluation app runs as the **free edition** until a license key is entered:
**organization** (one-time) or **MSP** (yearly). Limits are enforced in Rust (`crates/vbs-evaluation/src/edition.rs`);
keys are verified **offline** (`crates/vbs-license`) – the app never contacts the key service or anything else.

Stepwright, whose license interface this reuses, had no key format or key service yet; both are designed here so
that Stepwright can adopt them (the payload carries a product code, the key prefix names product and format).

## Key format (version 1)

```text
VBS1-<payload>.<signature>
```

* `payload`: compact JSON, base64url without padding. Fields: `v` (1), `product` (`vbs` – an internal code, stable
  across a product rename), `id` (key ID for support, e.g. `L-7Q3K9M2X`), `type` (`organization` | `msp`),
  `licensee` (1–120 characters, printed in reports), `issued` (`YYYY-MM-DD`), `expires` (MSP only: last valid day).
* `signature`: Ed25519 (RFC 8032) over exactly the payload bytes, base64url.
* Pasted keys may contain line breaks and spaces (mail clients wrap long lines); they are removed before checking.
* The app refuses: other prefixes (`wrongProduct`), other format versions (`unsupportedVersion`), any change of
  payload or signature (`invalidSignature`), incomplete fields (`malformed`), and MSP keys past their last day
  (`expired`, checked against the system clock). A stored MSP key that expires later switches the app back to the
  free edition and says so.

Tests: `cargo test -p vbs-license` (every single changed character is refused; foreign signing keys, swapped
signatures, signed but invalid payloads), `cargo test -p vbs-app` (activation, restart, expiry, removal), and the
key service's keys are verified by the Rust code (`worker/test/fixtures/issued.json`, reproduced in CI).

## Keys and secrets

| What | Where |
|---|---|
| Public key (base64url, 43 characters) | `product.json` → `license.publicKey` (embedded in the app) and `worker/wrangler.toml` → `LICENSE_PUBLIC_KEY` |
| Signing key (Ed25519 seed) | Worker secret `LICENSE_SIGNING_KEY` + an offline copy (password manager). **Never in the repository.** |
| Paddle notification secret | Worker secret `PADDLE_WEBHOOK_SECRET` |
| Paddle API key (optional) | Worker secret `PADDLE_API_KEY` – business name as licensee, buyer e-mail for delivery |
| E-mail API key (optional) | Worker secret `RESEND_API_KEY`, sender in `EMAIL_FROM` |

While `license.publicKey` is `null`, the app accepts no key (it says "development build"), and the release
pipeline refuses to build a release (`scripts/release/check-release.mjs`).

## One-time setup (vendor)

1. **Signing key** – on your own computer, outside the repository:
   `cargo run -p license-keys -- keygen --out <safe place>/vbs-license-signing.key`.
   It prints the public key and the next steps. Put the public key into `product.json` (`license.publicKey`) and
   `worker/wrangler.toml` (`LICENSE_PUBLIC_KEY`); run `npm run sync:config`; commit.
2. **Paddle sandbox** (live only after your approval):
   * Products: "Organization license" with a one-time price, "MSP license" with a yearly subscription price
     (amounts as in `product.json` → `pricing`). Enter the two price IDs in `wrangler.toml`
     (`PADDLE_PRICE_ORGANIZATION`, `PADDLE_PRICE_MSP`).
   * Notification destination: URL `https://<worker>/paddle/webhook`, event `transaction.completed`; copy its secret.
   * Checkout (website, phase 6): pass the licensee name as `customData: { licensee: "…" }` and send buyers to a
     success page that reads `GET https://<worker>/license/<transaction id>` (set `ALLOWED_ORIGIN` to the site).
3. **Worker** (`worker/`): `npx wrangler kv namespace create LICENSES` (id into `wrangler.toml`), then
   `npx wrangler secret put LICENSE_SIGNING_KEY` (content of the key file), `… PADDLE_WEBHOOK_SECRET`, optionally
   `… PADDLE_API_KEY`, `… RESEND_API_KEY`; `npx wrangler deploy`. Tests: `cd worker && npm test`.
4. **Trial purchase** in the sandbox with Paddle's test card; the key appears on the success page (and by e-mail
   if configured) and activates in the app under **License**.

## How the key service behaves

* `POST /paddle/webhook` – checks the `Paddle-Signature` (HMAC-SHA256 over `ts:body`, 5 minutes tolerance, rotated
  secrets accepted). For `transaction.completed` with one of the two prices it issues one key per transaction
  (Paddle retries are answered with `exists`, never a second key). Other products and events are ignored.
* Licensee: `custom_data.licensee` of the checkout, else the Paddle business name (API key needed), cleaned
  (control characters removed, 120 characters). Without a name the transaction waits (`pending`) and the success
  page asks for it once (`POST /license/<txn>` with `{ "licensee": "…" }`); afterwards it cannot be changed.
* MSP: valid through the end of the paid billing period **plus `MSP_GRACE_DAYS`** (default 14). Every renewal is a
  new completed transaction and gets a new key with the new end date; the app shows the expiry.
* Before storing a key the service verifies it with `LICENSE_PUBLIC_KEY` – a signing key that does not belong to the
  app's public key issues nothing (HTTP 500 in the log).
* Stored per transaction (KV `LICENSES`): type, licensee, dates, key, key ID, Paddle IDs (transaction, customer,
  subscription). No card data, no addresses; the buyer's e-mail is read from Paddle only for delivery, not stored.

## Manual keys, support

* Issue a key by hand (e.g. invoice customers, replacements): `cargo run -p license-keys -- issue --key <file>
  --type organization --licensee "ACME GmbH"` or `--type msp --licensee "…" --expires 2027-10-11`.
* Check a key a customer sends: `cargo run -p license-keys -- inspect <key>` (uses `product.json`'s public key).
* A lost key: look up the transaction in KV (`txn:<id>`) or issue a new one; keys cannot be revoked offline –
  MSP keys end by themselves, organization keys are perpetual by design.

## Changing the signing key

Old keys stop working once the app ships a new public key. Plan it as a new major version: generate the new key,
update `product.json` and the Worker together, and reissue keys of active customers.
