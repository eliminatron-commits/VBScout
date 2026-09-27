<script lang="ts">
  import type { EditionInfo, Overview, RiskLevel } from '../lib/api';
  import { formatDate, formatDays, formatNumber, formatRange } from '../lib/format';
  import { t, tc, tDynamic } from '../lib/i18n.svelte';

  let {
    overview,
    edition,
    onshow,
  }: { overview: Overview; edition: EditionInfo; onshow: (risk: RiskLevel | null, origin: 'own' | 'windows') => void } =
    $props();

  const days = (value: number | null) => (value === null ? '–' : formatNumber(value));
</script>

<section class="panel" aria-labelledby="overview-figures">
  <h2 id="overview-figures" class="visually-hidden">{t('report.section.summary')}</h2>
  <div class="figures">
    <div class="figure" data-tone="accent">
      <span class="value">{formatNumber(overview.machines)}</span>
      <span class="label">{t('report.figure.machines')}</span>
    </div>
    <button type="button" class="figure" data-tone="text" onclick={() => onshow(null, 'own')}>
      <span class="value">{formatNumber(overview.items)}</span>
      <span class="label">{t('report.figure.items')}</span>
    </button>
    <button type="button" class="figure" data-tone="high" onclick={() => onshow('high', 'own')}>
      <span class="value">{formatNumber(overview.high)}</span>
      <span class="label">{t('report.figure.high')}</span>
    </button>
    <button type="button" class="figure" data-tone="medium" onclick={() => onshow('medium', 'own')}>
      <span class="value">{formatNumber(overview.medium)}</span>
      <span class="label">{t('report.figure.medium')}</span>
    </button>
    <button type="button" class="figure" data-tone="low" onclick={() => onshow('low', 'own')}>
      <span class="value">{formatNumber(overview.low)}</span>
      <span class="label">{t('report.figure.low')}</span>
    </button>
    <div class="figure" data-tone="muted">
      <span class="value">{formatNumber(overview.notCheckable)}</span>
      <span class="label">{t('report.figure.notCheckable')}</span>
    </div>
    <button type="button" class="figure" data-tone="info" onclick={() => onshow(null, 'windows')}>
      <span class="value">{formatNumber(overview.windowsItems)}</span>
      <span class="label">{t('report.figure.windows')}</span>
    </button>
    <div class="figure" data-tone="accent">
      {#if overview.effort}
        <span class="value">{formatRange(overview.effort)}</span>
        <span class="label">
          {t('report.figure.effort')} · {t('report.personDays', {
            min: formatDays(overview.effort.min),
            max: formatDays(overview.effort.max),
          })}
        </span>
      {:else}
        <span class="value locked" aria-hidden="true">–</span>
        <span class="label">{t('report.figure.effort')} · {t('detail.locked')}</span>
      {/if}
    </div>
  </div>
  {#if overview.credentials}
    <p class="note">{tc('report.summary.credentials', overview.credentials)}</p>
  {/if}
  {#if overview.effort}
    <p class="hint">{t('report.effort.intro')}</p>
  {/if}
</section>

<section class="panel" aria-labelledby="overview-risks">
  <h2 id="overview-risks">{t('report.section.risks')}</h2>
  {#if overview.byKind.length}
    <div class="table-wrap">
      <table>
        <thead>
          <tr>
            <th scope="col">{t('report.column.kind')}</th>
            <th scope="col" class="number">{t('report.column.items')}</th>
            <th scope="col" class="number">{t('risk.high')}</th>
            <th scope="col" class="number">{t('risk.medium')}</th>
            <th scope="col" class="number">{t('risk.low')}</th>
            <th scope="col" class="number">{t('report.column.notCheckable')}</th>
            <th scope="col" class="number">{t('report.column.machines')}</th>
            {#if edition.effort}<th scope="col" class="number">{t('report.column.effort')}</th>{/if}
          </tr>
        </thead>
        <tbody>
          {#each overview.byKind as row (row.kind)}
            <tr>
              <th scope="row">{tDynamic(`kind.${row.kind}`, row.kind)}</th>
              <td class="number">{formatNumber(row.items)}</td>
              <td class="number">{formatNumber(row.high)}</td>
              <td class="number">{formatNumber(row.medium)}</td>
              <td class="number">{formatNumber(row.low)}</td>
              <td class="number">{formatNumber(row.notCheckable)}</td>
              <td class="number">{formatNumber(row.machines)}</td>
              {#if edition.effort}<td class="number">{row.effort ? formatRange(row.effort) : '–'}</td>{/if}
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
    <p class="hint">{t('report.risks.intro')}</p>
  {:else}
    <p>{t('report.nothingFound')}</p>
  {/if}
</section>

<section class="panel" aria-labelledby="overview-coverage">
  <h2 id="overview-coverage">{t('report.section.coverage')}</h2>
  <ul class="facts">
    <li>
      {t('report.coverage.machines', {
        full: overview.coverage.full,
        limited: overview.coverage.limited,
        machines: overview.machines,
      })}
    </li>
    {#each overview.coverage.limitations as [code, count] (code)}
      <li>{tDynamic(`limitation.${code}`, code)} ({tc('machine.count', count)})</li>
    {/each}
    {#if overview.coverage.deprecation.read}
      <li>
        {t('report.coverage.deprecation', {
          read: overview.coverage.deprecation.read,
          reported: overview.coverage.deprecation.reported,
          minDays: days(overview.coverage.deprecation.minDays),
          maxDays: days(overview.coverage.deprecation.maxDays),
          earliest: overview.coverage.deprecation.earliest ? formatDate(overview.coverage.deprecation.earliest, false) : '–',
        })}
      </li>
    {:else}
      <li>{t('report.coverage.deprecationNone')}</li>
    {/if}
    <li>
      {t('report.coverage.sysmon', {
        read: overview.coverage.sysmon.read,
        reported: overview.coverage.sysmon.reported,
      })}
    </li>
    <li>
      {t('report.coverage.files', {
        entries: formatNumber(overview.coverage.fileEntries),
        errors: formatNumber(overview.coverage.fileErrors),
        skipped: formatNumber(overview.coverage.fileSkipped),
      })}
    </li>
    {#each overview.coverage.notCheckable as [reason, count] (reason)}
      <li>{t('findingStatus.notCheckable')} – {tDynamic(`reason.${reason}`, reason)}: {formatNumber(count)}</li>
    {/each}
    {#if overview.setAside.length}
      <li>{tc('report.coverage.setAside', overview.setAside.length)}</li>
    {/if}
  </ul>
  <p class="hint">{t('report.coverage.intro')}</p>
  {#if overview.setAside.length}
    <details>
      <summary>{t('report.sheet.setAside')}</summary>
      <ul class="set-aside">
        {#each overview.setAside as entry (entry.file)}
          <li>
            <strong>{entry.hostname}</strong> · {formatDate(entry.scannedAt)} · {tDynamic(`setAside.${entry.why}`, entry.why)}
            <span class="file">{entry.file}</span>
          </li>
        {/each}
      </ul>
    </details>
  {/if}
</section>

<style>
  .figures {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(11rem, 1fr));
    gap: 0.75rem;
  }

  .figure {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    padding: 0.75rem 0.9rem;
    border: 1px solid var(--border);
    border-top: 4px solid var(--tone, var(--accent));
    border-radius: var(--radius);
    background: var(--surface);
    color: var(--text);
    text-align: left;
    font: inherit;
  }

  button.figure {
    cursor: pointer;
  }

  button.figure:hover {
    background: var(--badge-bg);
  }

  .figure[data-tone='accent'] {
    --tone: var(--accent);
  }
  .figure[data-tone='text'] {
    --tone: var(--text);
  }
  .figure[data-tone='high'] {
    --tone: var(--risk-high);
  }
  .figure[data-tone='medium'] {
    --tone: var(--risk-medium);
  }
  .figure[data-tone='low'] {
    --tone: var(--risk-low);
  }
  .figure[data-tone='muted'],
  .figure[data-tone='info'] {
    --tone: var(--muted);
  }

  .value {
    font-size: 1.6rem;
    font-weight: 700;
    line-height: 1.2;
    color: var(--tone);
  }

  .value.locked {
    color: var(--muted);
  }

  .label {
    color: var(--muted);
    font-size: 0.85rem;
  }

  .facts {
    margin: 0 0 0.75rem;
    padding-left: 1.2rem;
  }

  .facts li {
    margin-bottom: 0.25rem;
  }

  .note {
    margin: 0.75rem 0 0;
    font-weight: 600;
  }

  .set-aside {
    font-size: 0.9rem;
  }

  .set-aside .file {
    display: block;
    color: var(--muted);
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
    overflow-wrap: anywhere;
  }
</style>
