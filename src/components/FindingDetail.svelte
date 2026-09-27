<script lang="ts">
  import type { EditionInfo, FindingDetail } from '../lib/api';
  import { formatNumber, formatRange, ruleKey } from '../lib/format';
  import { t, tc, tDynamic } from '../lib/i18n.svelte';

  let {
    detail,
    edition,
    onclose,
    onselect,
  }: { detail: FindingDetail; edition: EditionInfo; onclose: () => void; onselect: (number: number) => void } =
    $props();

  const effortNotes = $derived.by(() => {
    const note = detail.effortNote;
    if (!note) return [];
    const notes: string[] = [];
    if (note.sameAs !== null) notes.push(t('effort.sameAs', { number: note.sameAs }));
    if (note.windows) notes.push(t('effort.windows'));
    if (note.sizeFactor > 1) notes.push(t('effort.sizeFactor', { factor: note.sizeFactor }));
    if (note.typicalScript) notes.push(t('effort.typicalScript'));
    if (note.countedOnce) notes.push(t('effort.countedOnce'));
    return notes;
  });
</script>

<aside class="panel detail" aria-labelledby="detail-title">
  <div class="head">
    <h2 id="detail-title">#{detail.number} · {tDynamic(ruleKey(detail.rule, 'title'), detail.rule)}</h2>
    <button type="button" class="close" onclick={onclose}>{t('detail.close')}</button>
  </div>
  <p class="badges">
    <span class="risk" data-risk={detail.risk}>{tDynamic(`risk.${detail.risk}`, detail.risk)}</span>
    <span class="badge">{tDynamic(`classification.${detail.classification}`, detail.classification)}</span>
    {#if detail.status === 'notCheckable'}
      <span class="badge warn">
        {t('findingStatus.notCheckable')}{detail.reason ? ` – ${tDynamic(`reason.${detail.reason}`, detail.reason)}` : ''}
      </span>
    {/if}
    <span class="badge">{tDynamic(`kind.${detail.kind}`, detail.kind)}</span>
    <span class="badge">{detail.rule}</span>
  </p>

  <h3>{t('detail.rationale')}</h3>
  <p>{tDynamic(ruleKey(detail.rule, 'rationale'), '–')}</p>

  <h3>{t('detail.runs')}</h3>
  <p>
    {tDynamic(`activation.${detail.activation}`, detail.activation)}
    {#if detail.reportedActivation !== detail.activation}
      <span class="muted">
        ({t('detail.reported', {
          activation: tDynamic(`activation.${detail.reportedActivation}`, detail.reportedActivation),
        })})
      </span>
    {/if}
  </p>
  {#if detail.startedBy.length}
    <p class="links">
      {t('detail.startedBy')}:
      {#each detail.startedBy as number (number)}
        <button type="button" class="link" onclick={() => onselect(number)}>#{number}</button>
      {/each}
    </p>
  {/if}
  {#if detail.starts.length}
    <p class="links">
      {t('detail.starts')}:
      {#each detail.starts as number (number)}
        <button type="button" class="link" onclick={() => onselect(number)}>#{number}</button>
      {/each}
    </p>
  {/if}

  <h3>{t('detail.locations')} ({formatNumber(detail.occurrences.length + detail.moreOccurrences)})</h3>
  <ul class="locations">
    {#each detail.occurrences as occurrence, index (index)}
      <li>
        <strong>{occurrence.machine}</strong>
        <span class="path">{occurrence.path}{occurrence.item ? ` · ${occurrence.item}` : ''}</span>
        {#if occurrence.target}<span class="path">→ {occurrence.target}</span>{/if}
      </li>
    {/each}
  </ul>
  {#if detail.moreOccurrences}
    <p class="muted">{tc('detail.moreLocations', detail.moreOccurrences)}</p>
  {/if}

  {#if detail.evidence.length}
    <h3>{t('detail.evidence')}</h3>
    <pre class="evidence">{#each detail.evidence as line, index (index)}{line.line !== null ? `${line.line}: ` : ''}{line.text}{line.masked ? `  (${t('detail.masked')})` : ''}{'\n'}{/each}</pre>
  {/if}

  {#if detail.fileSize !== null || detail.sha256}
    <h3>{t('detail.file')}</h3>
    <p class="path">
      {#if detail.fileSize !== null}{t('report.column.size')}: {formatNumber(detail.fileSize)}{/if}
      {#if detail.sha256}<br />SHA-256: {detail.sha256}{/if}
    </p>
  {/if}
  {#if detail.sameContentAs !== null}
    <p class="links">
      {t('report.column.sameContent')}:
      <button type="button" class="link" onclick={() => onselect(detail.sameContentAs ?? 0)}>#{detail.sameContentAs}</button>
    </p>
  {/if}

  <h3>{t('detail.hint')}</h3>
  {#if detail.hint}
    <p>{tDynamic(detail.hint, '–')}</p>
  {:else}
    <p class="locked">{t('detail.locked')}</p>
  {/if}

  <h3>{t('report.column.effort')}</h3>
  {#if edition.effort}
    <p>
      {detail.effort ? formatRange(detail.effort) : '–'}
      {#each effortNotes as note (note)}<span class="muted"> {note}</span>{/each}
    </p>
  {:else}
    <p class="locked">{t('detail.locked')}</p>
  {/if}

  {#if detail.sources.length}
    <h3>{t('report.column.sources')}</h3>
    <ul class="sources">
      {#each detail.sources as source (source.url)}
        <li>
          {source.publisher}: {source.title}
          <span class="path">{source.url} ({t('report.checked', { date: source.checked })})</span>
        </li>
      {/each}
    </ul>
  {/if}
</aside>

<style>
  .detail {
    position: sticky;
    top: 1rem;
    max-height: calc(100vh - 2rem);
    overflow: auto;
  }

  .head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 1rem;
  }

  h2 {
    margin: 0 0 0.5rem;
    font-size: 1.05rem;
  }

  h3 {
    margin: 1rem 0 0.3rem;
    font-size: 0.9rem;
    color: var(--muted);
  }

  p {
    margin: 0 0 0.4rem;
  }

  .close {
    padding: 0.3rem 0.7rem;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface);
    color: var(--text);
    cursor: pointer;
  }

  .badges {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
  }

  .badge {
    padding: 0 0.5rem;
    border-radius: 999px;
    font-size: 0.8rem;
    background: var(--badge-bg);
    color: var(--badge-text);
  }

  .badge.warn {
    background: var(--error-bg);
    color: var(--error-text);
  }

  .links {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    align-items: baseline;
  }

  button.link {
    padding: 0;
    border: none;
    background: none;
    color: var(--accent);
    font-weight: 600;
    cursor: pointer;
  }

  .locations,
  .sources {
    margin: 0;
    padding-left: 1.1rem;
    font-size: 0.9rem;
  }

  .path {
    display: block;
    color: var(--muted);
    font-size: 0.82rem;
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
    overflow-wrap: anywhere;
  }

  .evidence {
    margin: 0;
    padding: 0.6rem;
    border-radius: 6px;
    background: var(--bg);
    font-size: 0.82rem;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .locked {
    color: var(--muted);
    font-style: italic;
  }
</style>
