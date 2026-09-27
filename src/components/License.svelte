<script lang="ts">
  import { api, type LicenseFailure, type LicenseView } from '../lib/api';
  import { formatDay, formatPrice } from '../lib/format';
  import { key, t, type MessageKey } from '../lib/i18n.svelte';
  import { product } from '../lib/product';

  let { license, onchange }: { license: LicenseView; onchange: (view: LicenseView) => void } = $props();

  let keyText = $state('');
  let busy = $state(false);
  let message = $state<string | null>(null);
  let error = $state<string | null>(null);

  const failureKeys: Record<LicenseFailure['code'], MessageKey> = {
    malformed: key('license.error.malformed'),
    invalidSignature: key('license.error.invalidSignature'),
    wrongProduct: key('license.error.wrongProduct'),
    unsupportedVersion: key('license.error.unsupportedVersion'),
    expired: key('license.error.expired'),
    unavailable: key('license.error.unavailable'),
    io: key('error.generic'),
  };

  const kindKeys: Record<'organization' | 'msp', MessageKey> = {
    organization: key('license.kind.organization'),
    msp: key('license.kind.msp'),
  };

  function explain(e: unknown): string {
    const failure = e as Partial<LicenseFailure> | null;
    const code = failure?.code;
    if (code && code in failureKeys) {
      return t(failureKeys[code], {
        date: failure?.date ? formatDay(failure.date) : '',
        product: product.name,
        message: failure?.message ?? '',
      });
    }
    return t('error.generic', { message: String(e) });
  }

  async function activate() {
    busy = true;
    message = null;
    error = null;
    try {
      const view = await api.activateLicense(keyText);
      keyText = '';
      message = t('license.activated');
      onchange(view);
    } catch (e) {
      error = explain(e);
    } finally {
      busy = false;
    }
  }

  async function remove() {
    busy = true;
    message = null;
    error = null;
    try {
      onchange(await api.removeLicense());
    } catch (e) {
      error = t('error.generic', { message: String(e) });
    } finally {
      busy = false;
    }
  }

  const prices = $derived({
    organization: formatPrice(product.pricing.organization.amountMinor, product.pricing.currency),
    msp: formatPrice(product.pricing.msp.amountMinor, product.pricing.currency),
  });
</script>

<div class="license">
  <section class="panel" aria-labelledby="license-status">
    <h2 id="license-status">{t('license.title')}</h2>
    {#if license.status === 'none' || !license.kind}
      <p>{t('license.none')}</p>
    {:else}
      <dl>
        <dt>{t('license.type')}</dt>
        <dd>{t(kindKeys[license.kind])}</dd>
        <dt>{t('license.licensee')}</dt>
        <dd>{license.licensee}</dd>
        <dt>{t('license.validThrough')}</dt>
        <dd>{license.expires ? formatDay(license.expires) : t('license.noExpiry')}</dd>
        <dt>{t('license.keyId')}</dt>
        <dd class="mono">{license.keyId}</dd>
      </dl>
      {#if license.status === 'expired' && license.expires}
        <p class="error" role="alert">{t('license.expiredNotice', { date: formatDay(license.expires) })}</p>
      {/if}
      <div class="actions">
        <button type="button" class="quiet" disabled={busy} onclick={remove}>{t('license.remove')}</button>
      </div>
    {/if}
  </section>

  <section class="panel" aria-labelledby="license-enter">
    <h2 id="license-enter">{license.status === 'active' ? t('license.replace') : t('license.enter')}</h2>
    {#if !license.verifiable}
      <p class="error" role="note">{t('license.error.unavailable')}</p>
    {/if}
    <label class="key">
      <span>{t('license.keyLabel')}</span>
      <textarea
        rows="4"
        spellcheck="false"
        autocomplete="off"
        placeholder="VBS1-…"
        bind:value={keyText}
        disabled={busy}
      ></textarea>
    </label>
    <div class="actions">
      <button type="button" class="primary" disabled={busy || !keyText.trim()} onclick={activate}>
        {t('license.activate')}
      </button>
    </div>
    {#if message}<p class="success" role="status">{message}</p>{/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <p class="hint">{t('license.offline')}</p>
  </section>

  <section class="panel" aria-labelledby="license-buy">
    <h2 id="license-buy">{t('license.buyTitle')}</h2>
    <ul>
      <li>{t('license.buyOrganization', { price: prices.organization })}</li>
      <li>{t('license.buyMsp', { price: prices.msp })}</li>
    </ul>
    <p class="hint">{t('license.buyWhere', { website: product.website })}</p>
  </section>
</div>

<style>
  .license {
    display: grid;
    gap: 1.25rem;
  }

  dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 0.35rem 1.25rem;
    margin: 0 0 1rem;
  }

  dt {
    color: var(--muted);
  }

  dd {
    margin: 0;
    font-weight: 600;
    overflow-wrap: anywhere;
  }

  .mono,
  textarea {
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
  }

  .key {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    margin-bottom: 0.75rem;
    color: var(--muted);
    font-size: 0.9rem;
  }

  textarea {
    width: 100%;
    box-sizing: border-box;
    padding: 0.5rem 0.65rem;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface);
    color: var(--text);
    font-size: 0.85rem;
    resize: vertical;
    word-break: break-all;
  }

  .success {
    margin: 0.75rem 0 0;
    color: var(--success-text, var(--text));
    font-weight: 600;
  }

  .error {
    margin-top: 0.75rem;
  }

  ul {
    margin: 0;
    padding-left: 1.25rem;
  }
</style>
