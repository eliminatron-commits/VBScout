<script lang="ts">
  import type { MachineSummary } from '../lib/api';
  import { formatDate } from '../lib/format';
  import { t, tc, tDynamic } from '../lib/i18n.svelte';

  let { machines }: { machines: MachineSummary[] } = $props();
</script>

<section class="panel" aria-labelledby="machines-title">
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

<style>
  td.file {
    max-width: 16rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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
</style>
