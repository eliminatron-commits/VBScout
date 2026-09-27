<script lang="ts">
  import { onMount } from 'svelte';
  import { getCurrentWebview } from '@tauri-apps/api/webview';
  import {
    api,
    type AppInfo,
    type EditionKind,
    type ImportSummary,
    type LicenseView,
    type MachineSummary,
    type Overview as OverviewData,
    type RiskLevel,
  } from './lib/api';
  import {
    LANGUAGES,
    isLang,
    key,
    language,
    nativeName,
    setLanguage,
    t,
    tc,
    type MessageKey,
  } from './lib/i18n.svelte';
  import { formatDay } from './lib/format';
  import { product } from './lib/product';
  import Logo from './lib/Logo.svelte';
  import Findings from './components/Findings.svelte';
  import License from './components/License.svelte';
  import FindingsOverview from './components/Overview.svelte';
  import Machines from './components/Machines.svelte';
  import Reports from './components/Reports.svelte';

  type Tab = 'overview' | 'findings' | 'machines' | 'reports';

  let info = $state<AppInfo | null>(null);
  let error = $state<string | null>(null);
  let languageChoice = $state('system');
  let reviewed = $state(true);
  let machines = $state<MachineSummary[]>([]);
  let overview = $state<OverviewData | null>(null);
  let lastImport = $state<ImportSummary | null>(null);
  let busy = $state(false);
  let dropActive = $state(false);
  let tab = $state<Tab>('overview');
  /** The license page replaces the main content (reachable before any import). */
  let licenseOpen = $state(false);
  let revision = $state(0);
  let findingsRisk = $state<RiskLevel | null>(null);
  let findingsOrigin = $state<'own' | 'windows' | 'all'>('own');

  const editionLabels: Record<EditionKind, MessageKey> = {
    free: key('edition.free'),
    organization: key('edition.organization'),
    msp: key('edition.msp'),
  };

  const tabs: { id: Tab; label: MessageKey }[] = [
    { id: 'overview', label: key('nav.overview') },
    { id: 'findings', label: key('nav.findings') },
    { id: 'machines', label: key('nav.machines') },
    { id: 'reports', label: key('nav.reports') },
  ];

  const errorKeys: Record<ImportSummary['errors'][number]['code'], MessageKey> = {
    notResultFile: key('import.error.notResultFile'),
    wrongFormat: key('import.error.wrongFormat'),
    newerSchema: key('import.error.newerSchema'),
    invalid: key('import.error.invalid'),
    tooLarge: key('import.error.tooLarge'),
    io: key('import.error.io'),
  };

  const kinds = $derived(overview ? overview.byKind.map((row) => row.kind) : []);

  function importError(entry: ImportSummary['errors'][number]): string {
    return t(errorKeys[entry.code], {
      format: entry.message ?? '',
      message: entry.message ?? '',
      found: entry.found ?? '?',
      supported: entry.supported ?? '?',
    });
  }

  async function refresh() {
    overview = await api.overview();
    revision += 1;
  }

  async function runImport(action: () => Promise<ImportSummary>) {
    busy = true;
    error = null;
    try {
      const summary = await action();
      machines = summary.machines;
      if (!summary.cancelled) lastImport = summary;
      await refresh();
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
    overview = null;
    tab = 'overview';
    revision += 1;
  }

  /** A license was activated or removed: the edition, the machine limit and the views change. */
  async function licenseChanged(view: LicenseView) {
    try {
      info = await api.appInfo();
      if (info) info.license = view;
      machines = await api.loadedMachines();
      await refresh();
    } catch (e) {
      error = String(e);
    }
  }

  function showFindings(risk: RiskLevel | null, origin: 'own' | 'windows') {
    findingsRisk = risk;
    findingsOrigin = origin;
    tab = 'findings';
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
        if (machines.length) await refresh();
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

  function tabKey(event: KeyboardEvent, index: number) {
    const step = event.key === 'ArrowRight' ? 1 : event.key === 'ArrowLeft' ? -1 : 0;
    if (!step) return;
    event.preventDefault();
    const next = tabs[(index + step + tabs.length) % tabs.length];
    if (next) {
      tab = next.id;
      document.getElementById(`tab-${next.id}`)?.focus();
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
        {#if info.edition.licensee}
          <span class="licensee">{t('edition.licensedTo', { name: info.edition.licensee })}</span>
        {/if}
        <button type="button" class="quiet license-link" aria-pressed={licenseOpen} onclick={() => (licenseOpen = !licenseOpen)}>
          {t('nav.license')}
        </button>
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

  {#if info?.license.status === 'expired' && info.license.expires && !licenseOpen}
    <p class="notice" role="alert">
      {t('license.expiredNotice', { date: formatDay(info.license.expires) })}
      <button type="button" class="quiet license-link" onclick={() => (licenseOpen = true)}>{t('license.enter')}</button>
    </p>
  {/if}

  {#if info && !reviewed && isLang(language())}
    <p class="notice" role="note">{t('language.unreviewed', { url: info.translationsUrl })}</p>
  {/if}

  <main class="content">
    {#if licenseOpen && info}
      <div class="actions">
        <button type="button" class="quiet" onclick={() => (licenseOpen = false)}>← {t('license.back')}</button>
      </div>
      <License license={info.license} onchange={licenseChanged} />
    {:else}
    {#if machines.length === 0}
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
        </div>
        <p class="hint">{busy ? t('import.reading') : t('home.dropHint')}</p>
      </section>
    {:else}
      <section class="toolbar" aria-label={t('home.loaded')}>
        <strong>{tc('machine.count', machines.length)}</strong>
        <div class="actions">
          <button type="button" disabled={busy} onclick={() => runImport(api.openResultFiles)}>{t('home.openFiles')}</button>
          <button type="button" disabled={busy} onclick={() => runImport(api.openResultFolder)}>
            {t('home.openFolder')}
          </button>
          <button type="button" class="quiet" disabled={busy} onclick={clearAll}>{t('home.clear')}</button>
        </div>
        <span class="hint">{busy ? t('import.reading') : t('home.dropHint')}</span>
      </section>
    {/if}

    {#if error}
      <p class="error" role="alert">{t('error.generic', { message: error })}</p>
    {/if}

    {#if lastImport}
      <section class="panel import-status" aria-live="polite">
        <p>{tc('import.loaded', lastImport.loaded)}</p>
        {#if lastImport.duplicates}
          <p>{tc('import.duplicates', lastImport.duplicates)}</p>
        {/if}
        {#if lastImport.overLimit}
          <p class="failed">{tc('import.overLimit', lastImport.overLimit, { max: lastImport.machineLimit ?? 0 })}</p>
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

    {#if machines.length && overview && info}
      <div class="tabs" role="tablist" aria-label={t('nav.label')}>
        {#each tabs as item, index (item.id)}
          <button
            type="button"
            role="tab"
            id={`tab-${item.id}`}
            aria-selected={tab === item.id}
            aria-controls={`panel-${item.id}`}
            tabindex={tab === item.id ? 0 : -1}
            onclick={() => (tab = item.id)}
            onkeydown={(event) => tabKey(event, index)}
          >
            {t(item.label)}
          </button>
        {/each}
      </div>
      <div class="tab-panel" role="tabpanel" id={`panel-${tab}`} aria-labelledby={`tab-${tab}`}>
        {#if tab === 'overview'}
          <FindingsOverview {overview} edition={info.edition} onshow={showFindings} />
        {:else if tab === 'findings'}
          <Findings
            edition={info.edition}
            {kinds}
            {revision}
            bind:risk={findingsRisk}
            bind:origin={findingsOrigin}
          />
        {:else if tab === 'machines'}
          <Machines {machines} />
        {:else}
          <Reports edition={info.edition} />
        {/if}
      </div>
    {/if}

    {#if info && info.edition.kind === 'free' && info.edition.maxMachines !== null}
      <section class="panel edition" aria-label={t('about.edition')}>
        <p>
          <strong>{t('edition.free')}</strong> · {t('edition.freeLimit', { max: info.edition.maxMachines })}
        </p>
        <p class="hint">{t('edition.freeExcludes')}</p>
        <div class="actions">
          <button type="button" onclick={() => (licenseOpen = true)}>{t('license.enter')}</button>
        </div>
      </section>
    {/if}
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

  .licensee {
    color: var(--muted);
    font-size: 0.85rem;
  }

  .license-link {
    padding: 0.3rem 0.7rem;
    font-size: 0.85rem;
  }

  .license-link[aria-pressed='true'] {
    border-color: var(--accent);
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
    width: min(1280px, 100% - 3rem);
    margin: 0 auto;
    padding: 2rem 0;
    display: flex;
    flex-direction: column;
    gap: 1.25rem;
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

  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.75rem 1.25rem;
  }

  .toolbar .hint {
    margin: 0;
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

  .tabs {
    display: flex;
    gap: 0.25rem;
    border-bottom: 1px solid var(--border);
  }

  .tabs button {
    padding: 0.55rem 1rem;
    border: none;
    border-bottom: 3px solid transparent;
    border-radius: 0;
    background: transparent;
    color: var(--muted);
    font-weight: 600;
  }

  .tabs button[aria-selected='true'] {
    border-bottom-color: var(--accent);
    color: var(--text);
  }

  .tab-panel {
    display: flex;
    flex-direction: column;
    gap: 1.25rem;
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
