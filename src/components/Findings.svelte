<script lang="ts">
  import { api, type EditionInfo, type FindingDetail as Detail, type FindingsPage, type RiskLevel } from '../lib/api';
  import { formatNumber, formatRange, ruleKey } from '../lib/format';
  import { t, tDynamic } from '../lib/i18n.svelte';
  import FindingDetail from './FindingDetail.svelte';

  let {
    edition,
    kinds,
    risk = $bindable(null),
    origin = $bindable('own'),
    revision,
  }: {
    edition: EditionInfo;
    kinds: string[];
    risk: RiskLevel | null;
    origin: 'own' | 'windows' | 'all';
    revision: number;
  } = $props();

  const PAGE = 100;
  let kind = $state('');
  let search = $state('');
  let offset = $state(0);
  let page = $state<FindingsPage>({ total: 0, offset: 0, rows: [] });
  let detail = $state<Detail | null>(null);
  let error = $state<string | null>(null);
  let request = 0;

  $effect(() => {
    // Reload whenever a filter, the page or the loaded results change.
    const query = { risk, kind: kind || null, origin, search: search.trim() || null, offset, limit: PAGE };
    void revision;
    const ticket = ++request;
    api
      .findings(query)
      .then((result) => {
        if (ticket === request) page = result;
      })
      .catch((e) => (error = String(e)));
  });

  function resetPaging() {
    offset = 0;
  }

  async function open(number: number) {
    try {
      detail = await api.finding(number);
    } catch (e) {
      error = String(e);
    }
  }

  const riskOptions: (RiskLevel | null)[] = [null, 'high', 'medium', 'low'];
</script>

<div class="layout" class:with-detail={detail !== null}>
  <section class="panel list" aria-labelledby="findings-title">
    <h2 id="findings-title" class="visually-hidden">{t('nav.findings')}</h2>
    <div class="filters">
      <label>
        <span>{t('findings.filter.risk')}</span>
        <select bind:value={risk} onchange={resetPaging}>
          {#each riskOptions as option (option ?? 'all')}
            <option value={option}>{option ? tDynamic(`risk.${option}`, option) : t('findings.filter.all')}</option>
          {/each}
        </select>
      </label>
      <label>
        <span>{t('findings.filter.kind')}</span>
        <select bind:value={kind} onchange={resetPaging}>
          <option value="">{t('findings.filter.all')}</option>
          {#each kinds as value (value)}
            <option {value}>{tDynamic(`kind.${value}`, value)}</option>
          {/each}
        </select>
      </label>
      <label>
        <span>{t('findings.filter.origin')}</span>
        <select bind:value={origin} onchange={resetPaging}>
          <option value="own">{t('origin.own')}</option>
          <option value="windows">{t('origin.windows')}</option>
          <option value="all">{t('findings.filter.all')}</option>
        </select>
      </label>
      <label class="search">
        <span>{t('findings.search')}</span>
        <input type="search" bind:value={search} oninput={resetPaging} />
      </label>
    </div>

    {#if error}
      <p class="error" role="alert">{t('error.generic', { message: error })}</p>
    {/if}

    {#if page.rows.length}
      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th scope="col" class="number">#</th>
              <th scope="col">{t('report.column.risk')}</th>
              <th scope="col">{t('report.column.finding')}</th>
              <th scope="col">{t('report.column.activation')}</th>
              <th scope="col" class="number">{t('report.column.machines')}</th>
              {#if edition.effort}<th scope="col" class="number">{t('report.column.effort')}</th>{/if}
            </tr>
          </thead>
          <tbody>
            {#each page.rows as row (row.number)}
              <tr class:selected={detail?.number === row.number}>
                <td class="number">{row.number}</td>
                <td><span class="risk" data-risk={row.risk}>{tDynamic(`risk.${row.risk}`, row.risk)}</span></td>
                <td>
                  <button
                    type="button"
                    class="link"
                    aria-label={t('findings.open', { number: row.number })}
                    onclick={() => open(row.number)}
                  >
                    {tDynamic(ruleKey(row.rule, 'title'), row.rule)}
                    {#if row.reason}<span class="muted"> ({tDynamic(`reason.${row.reason}`, row.reason)})</span>{/if}
                  </button>
                  <span class="path">{row.location}{row.item ? ` · ${row.item}` : ''}</span>
                  {#if row.target}<span class="path">→ {row.target}</span>{/if}
                </td>
                <td>{tDynamic(`activation.${row.activation}`, row.activation)}</td>
                <td class="number">
                  {#if row.machines === 1}{row.machine}{:else}{formatNumber(row.machines)}{/if}
                </td>
                {#if edition.effort}<td class="number">{row.effort ? formatRange(row.effort) : '–'}</td>{/if}
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      <div class="paging">
        <span>
          {t('findings.range', {
            from: formatNumber(page.offset + 1),
            to: formatNumber(page.offset + page.rows.length),
            total: formatNumber(page.total),
          })}
        </span>
        <button type="button" disabled={offset === 0} onclick={() => (offset = Math.max(0, offset - PAGE))}>
          {t('findings.previous')}
        </button>
        <button type="button" disabled={offset + PAGE >= page.total} onclick={() => (offset += PAGE)}>
          {t('findings.next')}
        </button>
      </div>
    {:else}
      <p>{t('findings.empty')}</p>
    {/if}
  </section>

  {#if detail}
    <FindingDetail {detail} {edition} onclose={() => (detail = null)} onselect={open} />
  {/if}
</div>

<style>
  .layout {
    display: grid;
    gap: 1.25rem;
  }

  @media (min-width: 1180px) {
    .layout.with-detail {
      grid-template-columns: minmax(0, 5fr) minmax(0, 3fr);
      align-items: start;
    }
  }

  .filters {
    display: flex;
    flex-wrap: wrap;
    gap: 0.75rem;
    margin-bottom: 1rem;
  }

  .filters label {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    font-size: 0.85rem;
    color: var(--muted);
  }

  .filters .search {
    flex: 1 1 14rem;
  }

  select,
  input {
    padding: 0.35rem 0.5rem;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface);
    color: var(--text);
  }

  button.link {
    padding: 0;
    border: none;
    background: none;
    color: var(--accent);
    font-weight: 600;
    text-align: left;
    cursor: pointer;
  }

  button.link:hover {
    text-decoration: underline;
  }

  .path {
    display: block;
    color: var(--muted);
    font-size: 0.82rem;
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
    overflow-wrap: anywhere;
  }

  tr.selected td {
    background: var(--badge-bg);
  }

  .paging {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin-top: 0.75rem;
    color: var(--muted);
    font-size: 0.9rem;
  }

  .paging button {
    padding: 0.35rem 0.8rem;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface);
    color: var(--text);
    cursor: pointer;
  }

  .paging button:disabled {
    cursor: not-allowed;
    opacity: 0.5;
  }
</style>
