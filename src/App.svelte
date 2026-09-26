<script lang="ts">
  import { onMount } from 'svelte';
  import { getCurrentWebview } from '@tauri-apps/api/webview';
  import { api, type AppInfo, type EditionKind, type ImportSummary, type MachineSummary } from './lib/api';
  import {
    LANGUAGES,
    isLang,
    key,
    language,
    nativeName,
    setLanguage,
    t,
    tc,
    tDynamic,
    type MessageKey,
  } from './lib/i18n.svelte';
  import { product } from './lib/product';
  import Logo from './lib/Logo.svelte';

  let info = $state<AppInfo | null>(null);
  let error = $state<string | null>(null);
  let languageChoice = $state('system');
  let reviewed = $state(true);
  let machines = $state<MachineSummary[]>([]);
  let lastImport = $state<ImportSummary | null>(null);
  let busy = $state(false);
  let dropActive = $state(false);

  const editionLabels: Record<EditionKind, MessageKey> = {
    free: key('edition.free'),
    organization: key('edition.organization'),
    msp: key('edition.msp'),
  };

  const errorKeys: Record<ImportSummary['errors'][number]['code'], MessageKey> = {
    notResultFile: key('import.error.notResultFile'),
    wrongFormat: key('import.error.wrongFormat'),
    newerSchema: key('import.error.newerSchema'),
    invalid: key('import.error.invalid'),
    tooLarge: key('import.error.tooLarge'),
    io: key('import.error.io'),
  };

  const dateFormat = $derived(new Intl.DateTimeFormat(language(), { dateStyle: 'medium', timeStyle: 'short' }));

  function formatDate(iso: string): string {
    const date = new Date(iso);
    return Number.isNaN(date.getTime()) ? iso : dateFormat.format(date);
  }

  function importError(entry: ImportSummary['errors'][number]): string {
    return t(errorKeys[entry.code], {
      format: entry.message ?? '',
      message: entry.message ?? '',
      found: entry.found ?? '?',
      supported: entry.supported ?? '?',
    });
  }

  async function runImport(action: () => Promise<ImportSummary>) {
    busy = true;
    error = null;
    try {
      const summary = await action();
      machines = summary.machines;
      if (!summary.cancelled) lastImport = summary;
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function clearAll() {
    await api.clearResults().catch((e) => (error = String(e)));
    machines = [];
    lastImport = null;
  }

  onMount(() => {
    let unlisten: (() => void) | undefined;
    (async () => {
      try {
        info = await api.appInfo();
        setLanguage(info.language.tag);
        reviewed = info.language.reviewed;
        languageChoice = info.uiLanguageSetting ?? 'system';
        machines = await api.loadedMachines();
        unlisten = await getCurrentWebview().onDragDropEvent((event) => {
          if (event.payload.type === 'enter' || event.payload.type === 'over') {
            dropActive = true;
          } else if (event.payload.type === 'drop') {
            dropActive = false;
            const paths = event.payload.paths;
            if (paths.length) void runImport(() => api.importPaths(paths));
          } else {
            dropActive = false;
          }
        });
      } catch (e) {
        error = String(e);
      } finally {
        await api.frontendReady().catch(() => undefined);
      }
    })();
    return () => unlisten?.();
  });

  async function changeLanguage(event: Event) {
    const value = (event.currentTarget as HTMLSelectElement).value;
    try {
      const effective = await api.setUiLanguage(value === 'system' ? null : value);
      languageChoice = value;
      setLanguage(effective.tag);
      reviewed = effective.reviewed;
    } catch (e) {
      error = String(e);
    }
  }
</script>

<div class="shell" class:drop-active={dropActive}>
  <header class="topbar">
    <div class="brand">
      <Logo size={28} />
      <span class="brand-name">{product.name}</span>
      {#if info}
        <span class="badge" data-kind={info.edition.kind}>{t(editionLabels[info.edition.kind])}</span>
      {/if}
    </div>
    <label class="language">
      <span>{t('settings.language')}</span>
      <select value={languageChoice} onchange={changeLanguage}>
        <option value="system">{t('settings.languageSystem')}</option>
        {#each LANGUAGES as lang (lang)}
          <option value={lang} {lang}>{nativeName(lang)}</option>
        {/each}
      </select>
    </label>
  </header>

  {#if info && !reviewed && isLang(language())}
    <p class="notice" role="note">{t('language.unreviewed', { url: info.translationsUrl })}</p>
  {/if}

  <main class="content">
    <section class="hero" aria-labelledby="hero-title">
      <h1 id="hero-title">{product.name}</h1>
      <p class="tagline">{t('app.tagline')}</p>
      <p class="intro">{t('home.intro')}</p>
      <div class="actions">
        <button type="button" class="primary" disabled={busy} onclick={() => runImport(api.openResultFiles)}>
          {t('home.openFiles')}
        </button>
        <button type="button" disabled={busy} onclick={() => runImport(api.openResultFolder)}>
          {t('home.openFolder')}
        </button>
        {#if machines.length}
          <button type="button" class="quiet" disabled={busy} onclick={clearAll}>{t('home.clear')}</button>
        {/if}
      </div>
      <p class="hint">{busy ? t('import.reading') : t('home.dropHint')}</p>
    </section>

    {#if error}
      <p class="error" role="alert">{t('error.generic', { message: error })}</p>
    {/if}

    {#if lastImport}
      <section class="import-status" aria-live="polite">
        <p>{tc('import.loaded', lastImport.loaded)}</p>
        {#if lastImport.duplicates}
          <p>{tc('import.duplicates', lastImport.duplicates)}</p>
        {/if}
        {#if lastImport.newerValues}
          <p>{t('import.newerValues')}</p>
        {/if}
        {#if lastImport.errors.length}
          <p class="failed">{tc('import.failed', lastImport.errors.length)}</p>
          <ul class="errors">
            {#each lastImport.errors as entry (entry.file)}
              <li><span class="file">{entry.file}</span> – {importError(entry)}</li>
            {/each}
          </ul>
        {/if}
      </section>
    {/if}

    {#if machines.length}
      <section class="machines" aria-labelledby="machines-title">
        <h2 id="machines-title">{t('machines.heading')} · {tc('machine.count', machines.length)}</h2>
        <div class="table-wrap">
          <table>
            <thead>
              <tr>
                <th scope="col">{t('machines.column.machine')}</th>
                <th scope="col">{t('machines.column.os')}</th>
                <th scope="col">{t('machines.column.scanned')}</th>
                <th scope="col">{t('machines.column.coverage')}</th>
                <th scope="col" class="number">{t('machines.column.findings')}</th>
                <th scope="col">{t('machines.column.file')}</th>
              </tr>
            </thead>
            <tbody>
              {#each machines as machine (machine.scanId)}
                <tr>
                  <th scope="row">
                    {machine.hostname}
                    {#if machine.domain}<span class="muted">.{machine.domain}</span>{/if}
                  </th>
                  <td>{machine.os ?? '–'}</td>
                  <td>{formatDate(machine.scannedAt)}</td>
                  <td>
                    <span class="coverage" data-mode={machine.coverage}>
                      {machine.coverage === 'full' ? t('coverage.full') : t('coverage.limited')}
                    </span>
                    {#each machine.limitations as code (code)}
                      <span class="limitation">{tDynamic(`limitation.${code}`, code)}</span>
                    {/each}
                  </td>
                  <td class="number">
                    {machine.findings}
                    {#if machine.notCheckable}
                      <span class="muted">+ {machine.notCheckable} {t('findingStatus.notCheckable')}</span>
                    {/if}
                  </td>
                  <td class="file" title={machine.path}>{machine.fileName}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      </section>
    {/if}

    {#if info && info.edition.kind === 'free' && info.edition.maxMachines !== null}
      <section class="edition" aria-label={t('about.edition')}>
        <p>
          <strong>{t('edition.free')}</strong> · {t('edition.freeLimit', { max: info.edition.maxMachines })}
        </p>
        <p class="hint">{t('edition.freeExcludes')}</p>
      </section>
    {/if}
  </main>

  <footer class="footer">
    <svg class="offline-icon" viewBox="0 0 24 24" aria-hidden="true" focusable="false">
      <path d="M12 2 4 5v6c0 5 3.4 9.4 8 11 4.6-1.6 8-6 8-11V5l-8-3Zm-1 14.2-3.7-3.7 1.4-1.4 2.3 2.3 5.3-5.3 1.4 1.4-6.7 6.7Z" />
    </svg>
    <span>{t('app.offlineBadge')}</span>
    {#if info}
      <span class="muted">· {t('about.version')} {info.version} · {t('about.rules', { date: info.rulesAsOf })}</span>
    {/if}
  </footer>
</div>

<style>
  .shell {
    min-height: 100vh;
    display: grid;
    grid-template-rows: auto auto 1fr auto;
  }

  .shell.drop-active {
    outline: 4px dashed var(--accent);
    outline-offset: -8px;
  }

  .topbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    padding: 0.75rem 1.5rem;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 0.6rem;
  }

  .brand-name {
    font-weight: 650;
    letter-spacing: 0.01em;
  }

  .badge {
    padding: 0.1rem 0.55rem;
    border-radius: 999px;
    font-size: 0.8rem;
    font-weight: 600;
    background: var(--badge-bg);
    color: var(--badge-text);
  }

  .language {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    color: var(--muted);
    font-size: 0.9rem;
  }

  .language select {
    padding: 0.3rem 0.5rem;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface);
    color: var(--text);
  }

  .notice {
    margin: 0;
    padding: 0.5rem 1.5rem;
    background: var(--badge-bg);
    color: var(--badge-text);
    font-size: 0.9rem;
    overflow-wrap: anywhere;
  }

  .content {
    width: min(1100px, 100% - 3rem);
    margin: 0 auto;
    padding: 2.5rem 0 2rem;
    display: flex;
    flex-direction: column;
    gap: 1.75rem;
  }

  h1 {
    margin: 0 0 0.25rem;
    font-size: 2rem;
    line-height: 1.2;
  }

  .tagline {
    margin: 0 0 0.75rem;
    color: var(--muted);
    font-size: 1.1rem;
  }

  .intro {
    margin: 0 0 1.25rem;
    max-width: 60ch;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.75rem;
  }

  button {
    padding: 0.65rem 1.2rem;
    border-radius: var(--radius);
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
    font-weight: 600;
    cursor: pointer;
  }

  button.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-text);
  }

  button.quiet {
    background: transparent;
  }

  button:disabled {
    cursor: not-allowed;
    opacity: 0.55;
  }

  .hint {
    margin: 0.75rem 0 0;
    color: var(--muted);
    font-size: 0.9rem;
  }

  .error {
    margin: 0;
    padding: 0.75rem 1rem;
    border-radius: var(--radius);
    background: var(--error-bg);
    color: var(--error-text);
  }

  .import-status,
  .machines,
  .edition {
    padding: 1.25rem 1.5rem;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  .import-status p,
  .edition p {
    margin: 0 0 0.35rem;
  }

  .failed {
    color: var(--error-text);
    font-weight: 600;
  }

  .errors {
    margin: 0.25rem 0 0;
    padding-left: 1.25rem;
    font-size: 0.9rem;
  }

  .errors .file {
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
    overflow-wrap: anywhere;
  }

  h2 {
    margin: 0 0 0.75rem;
    font-size: 1.05rem;
  }

  .table-wrap {
    overflow-x: auto;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.92rem;
  }

  th,
  td {
    padding: 0.5rem 0.6rem;
    border-bottom: 1px solid var(--border);
    text-align: left;
    vertical-align: top;
  }

  thead th {
    color: var(--muted);
    font-weight: 600;
    white-space: nowrap;
  }

  tbody th {
    font-weight: 600;
  }

  .number {
    text-align: right;
    white-space: nowrap;
  }

  td.file {
    max-width: 16rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .muted {
    color: var(--muted);
    font-weight: 400;
  }

  .coverage {
    display: inline-block;
    padding: 0 0.45rem;
    border-radius: 999px;
    font-size: 0.8rem;
    font-weight: 600;
    background: var(--badge-bg);
    color: var(--badge-text);
  }

  .coverage[data-mode='limited'] {
    background: var(--error-bg);
    color: var(--error-text);
  }

  .limitation {
    display: block;
    margin-top: 0.2rem;
    color: var(--muted);
    font-size: 0.82rem;
  }

  .footer {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: center;
    gap: 0.4rem;
    padding: 0.75rem;
    color: var(--muted);
    font-size: 0.85rem;
  }

  .offline-icon {
    width: 1rem;
    height: 1rem;
    fill: var(--ok);
  }
</style>
