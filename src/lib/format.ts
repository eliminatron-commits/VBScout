// Numbers, dates and rule-of-thumb hours in the UI language.
import type { Range } from './api';
import { language, t } from './i18n.svelte';

export function formatNumber(value: number, digits = 0): string {
  return new Intl.NumberFormat(language(), { maximumFractionDigits: digits }).format(value);
}

/** Hours without false precision: two decimals below 1 h, one below 10 h, whole hours above. */
export function formatHours(value: number): string {
  const digits = value >= 10 ? 0 : value >= 1 ? 1 : 2;
  return formatNumber(digits === 0 ? Math.round(value) : value, digits);
}

/** "0.5–2 h" – always a rule of thumb. */
export function formatRange(range: Range): string {
  if (Math.abs(range.max - range.min) < 1e-9) return t('report.hours', { value: formatHours(range.min) });
  return t('report.hourRange', { min: formatHours(range.min), max: formatHours(range.max) });
}

/** Person-days of 8 hours. */
export function formatDays(hours: number): string {
  const days = hours / 8;
  return formatNumber(days >= 10 ? Math.round(days) : days, days >= 10 ? 0 : 1);
}

export function formatDate(iso: string, withTime = true): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  const options: Intl.DateTimeFormatOptions = withTime ? { dateStyle: 'medium', timeStyle: 'short' } : { dateStyle: 'medium' };
  return new Intl.DateTimeFormat(language(), options).format(date);
}

/** Title of a rule; rules from newer versions show their ID. */
export function ruleKey(rule: string, part: 'title' | 'rationale'): string {
  return `rule.${rule.toLowerCase().replace('-', '')}.${part}`;
}
