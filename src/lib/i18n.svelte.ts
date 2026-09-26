// UI translations. The catalogs in /i18n are shared with the Rust crates
// (crates/vbs-i18n); English is the source. All catalogs are bundled – the app
// never loads anything at runtime.
import en from '../../i18n/en.json';
import languages from '../../i18n/languages.json';
import { product } from './product';

export const LANGUAGES = ['en', 'de', 'fr', 'es', 'it', 'nl', 'pl', 'pt-BR'] as const;
export type Lang = (typeof LANGUAGES)[number];

/** Every key of the English source catalog – typos fail type checking. */
export type MessageKey = keyof typeof en;
/** Base names of plural messages (`machine.count` for `machine.count_one` …). */
export type PluralKey = { [K in MessageKey]: K extends `${infer Base}_other` ? Base : never }[MessageKey];

type Catalog = Record<string, string>;

const modules = import.meta.glob<Catalog>(['../../i18n/*.json', '!../../i18n/languages.json'], {
  eager: true,
  import: 'default',
});
const catalogs = Object.fromEntries(
  Object.entries(modules).map(([path, catalog]) => [path.replace(/^.*\/(.+)\.json$/, '$1'), catalog]),
) as Record<Lang, Catalog>;

let current = $state<Lang>('en');

export function isLang(tag: string): tag is Lang {
  return (LANGUAGES as readonly string[]).includes(tag);
}

export function language(): Lang {
  return current;
}

export function setLanguage(tag: string): void {
  current = isLang(tag) ? tag : 'en';
  document.documentElement.lang = current;
}

/** Reviewed by a native speaker (English and German); others show a correction hint. */
export function isReviewed(lang: Lang): boolean {
  return (languages.languages as Record<string, { reviewed: boolean }>)[lang]?.reviewed ?? false;
}

/** Marks a key used outside `t()` so scripts/check-i18n.mjs can verify it. */
export function key<K extends MessageKey>(k: K): K {
  return k;
}

type Args = Record<string, string | number>;

function format(message: string, args: Args): string {
  return message.replace(/\{([A-Za-z0-9_]+)\}/g, (placeholder, name: string) => {
    if (name in args) return String(args[name]);
    if (name === 'product') return product.name;
    return placeholder;
  });
}

function lookup(lang: Lang, k: string): string | undefined {
  return catalogs[lang]?.[k];
}

export function t(k: MessageKey, args: Args = {}): string {
  return format(lookup(current, k) ?? lookup('en', k) ?? k, args);
}

/** Translates a key built at runtime (e.g. `limitation.<code>`); falls back to `fallback`. */
export function tDynamic(k: string, fallback: string, args: Args = {}): string {
  const message = lookup(current, k) ?? lookup('en', k);
  return message === undefined ? fallback : format(message, args);
}

export function tc(base: PluralKey, count: number, args: Args = {}): string {
  const category = new Intl.PluralRules(current).select(count);
  const message =
    lookup(current, `${base}_${category}`) ??
    lookup(current, `${base}_other`) ??
    lookup('en', `${base}_${new Intl.PluralRules('en').select(count)}`) ??
    base;
  return format(message, { count, ...args });
}

/** Name of a language in that language, e.g. "Deutsch". */
export function nativeName(lang: Lang): string {
  return lookup(lang, key('language.name')) ?? lang;
}
