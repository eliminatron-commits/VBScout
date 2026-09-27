<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type EditionInfo, type ReportSettings } from '../lib/api';
  import { LANGUAGES, key, language, nativeName, t, type MessageKey } from '../lib/i18n.svelte';

  let { edition }: { edition: EditionInfo } = $props();

  let reportLanguage = $state<string>(language());
  let settings = $state<ReportSettings>({ customer: null, logo: null });
  let customer = $state('');
  let busy = $state(false);
  let message = $state<string | null>(null);
  let error = $state<string | null>(null);

  onMount(async () => {
    try {
      settings = await api.reportSettings();
      customer = settings.customer ?? '';
    } catch (e) {
      error = String(e);
    }
  });

  // Error codes of the Rust commands with their own message.
  const errorKeys: Record<string, MessageKey> = {
    notLicensed: key('error.notLicensed'),
    logoFormat: key('error.logoFormat'),
    logoTooLarge: key('error.logoTooLarge'),
    logoInvalid: key('error.logoInvalid'),
  };

  function explain(e: unknown): string {
    const code = String(e);
    const known = errorKeys[code];
    return known ? t(known) : t('error.generic', { message: code });
  }

  async function run(action: () => Promise<void>) {
    busy = true;
    message = null;
    error = null;
    try {
      await action();
    } catch (e) {
      error = explain(e);
    } finally {
      busy = false;
    }
  }

  const saveCustomer = () =>
    run(async () => {
      settings = await api.setReportCustomer(customer.trim() || null);
      customer = settings.customer ?? '';
    });

  const chooseLogo = () => run(async () => void (settings = await api.chooseReportLogo()));
  const removeLogo = () => run(async () => void (settings = await api.clearReportLogo()));

  const exportExcel = () =>
    run(async () => {
      const path = await api.exportExcel(reportLanguage);
      if (path) message = t('export.saved', { path });
    });

  const exportPdf = () =>
    run(async () => {
      const path = await api.exportPdf(reportLanguage);
      if (path) message = t('export.saved', { path });
    });
</script>

<section class="panel" aria-labelledby="reports-title">
  <h2 id="reports-title">{t('nav.reports')}</h2>

  <div class="settings">
    <label>
      <span>{t('export.language')}</span>
      <select bind:value={reportLanguage}>
        {#each LANGUAGES as lang (lang)}
          <option value={lang} {lang}>{nativeName(lang)}</option>
        {/each}
      </select>
    </label>
    <label class="customer">
      <span>{t('export.customer')}</span>
      <span class="row">
        <input type="text" maxlength="120" bind:value={customer} onchange={saveCustomer} />
      </span>
      <span class="hint">{t('export.customerHint')}</span>
    </label>
  </div>

  <div class="exports">
    <div class="export">
      <h3>{t('export.excel')}</h3>
      <p>{t('export.excelHint')}</p>
      {#if !edition.hints}<p class="hint">{t('export.freeExcel')}</p>{/if}
      <button type="button" class="primary" disabled={busy} onclick={exportExcel}>{t('export.excel')}</button>
    </div>
    <div class="export">
      <h3>{t('export.pdf')}</h3>
      <p>{t('export.pdfHint')}</p>
      {#if edition.pdf}
        <button type="button" class="primary" disabled={busy} onclick={exportPdf}>{t('export.pdf')}</button>
      {:else}
        <p class="locked">{t('export.pdfLocked')}</p>
        <button type="button" disabled>{t('export.pdf')}</button>
      {/if}
    </div>
  </div>

  {#if busy}<p class="hint" aria-live="polite">{t('export.working')}</p>{/if}
  {#if message}<p class="saved" aria-live="polite">{message}</p>{/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
</section>

<section class="panel" aria-labelledby="logo-title">
  <h2 id="logo-title">{t('export.logo')}</h2>
  <p class="hint">{t('export.logoHint')}</p>
  {#if settings.logo}
    <img class="logo" src={settings.logo.dataUrl} alt={t('export.logo')} />
  {/if}
  <div class="actions">
    <button type="button" disabled={busy} onclick={chooseLogo}>{t('export.logoChoose')}</button>
    {#if settings.logo}
      <button type="button" class="quiet" disabled={busy} onclick={removeLogo}>{t('export.logoRemove')}</button>
    {/if}
  </div>
  {#if !edition.logo}<p class="locked">{t('export.logoLocked')}</p>{/if}
</section>

<style>
  .settings {
    display: flex;
    flex-wrap: wrap;
    gap: 1rem;
    margin-bottom: 1.25rem;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    font-size: 0.9rem;
    color: var(--muted);
  }

  .customer {
    flex: 1 1 18rem;
  }

  select,
  input {
    padding: 0.4rem 0.55rem;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface);
    color: var(--text);
  }

  .row {
    display: flex;
    gap: 0.5rem;
  }

  .row input {
    flex: 1;
  }

  .exports {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(16rem, 1fr));
    gap: 1rem;
  }

  .export {
    padding: 1rem;
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }

  h3 {
    margin: 0 0 0.4rem;
    font-size: 1rem;
  }

  .export p {
    margin: 0 0 0.6rem;
  }

  .locked {
    color: var(--muted);
    font-style: italic;
  }

  .saved {
    color: var(--ok);
    font-weight: 600;
    overflow-wrap: anywhere;
  }

  .logo {
    display: block;
    max-width: 240px;
    max-height: 90px;
    margin: 0.5rem 0 0.75rem;
    object-fit: contain;
  }

  .actions {
    display: flex;
    gap: 0.75rem;
  }
</style>
