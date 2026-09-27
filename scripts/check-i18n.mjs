#!/usr/bin/env node
// Translation check (Definition of Done #6: all languages complete, no missing keys).
// Taken over from Stepwright and extended by the rule catalog and languages.json.
//
//  1. Every catalog in i18n/ has exactly the keys of en.json; plural messages
//     have exactly the CLDR categories the language needs (+ "other").
//  2. Placeholders match the English source ({count} may be spelled out).
//  3. No empty or padded messages; the product name only via {product}.
//  4. Every key used in the code exists: t("…"), tc("…"), key("…") in
//     TypeScript/Svelte and t(lang, "…"), t_args(…), t_count(…), key("…"),
//     the collector console's info/error/text("…") and the report texts'
//     .t("…"), .args("…") and .count("…") in Rust.
//  5. Every rule of rules/catalog.json has a title and a rationale.
//  6. i18n/languages.json lists exactly the supported languages; en and de are
//     the reviewed ones.
//
// Unused keys are reported as warnings. Exit code 1 on any error.

import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const i18nDir = join(root, 'i18n');
const SOURCE = 'en';
const LANGUAGES = ['en', 'de', 'fr', 'es', 'it', 'nl', 'pl', 'pt-BR'];
const REVIEWED = ['en', 'de'];
const PLURALS = { pl: ['few', 'many', 'one', 'other'] };
const DEFAULT_PLURALS = ['one', 'other'];
const PLURAL_SUFFIX = /^(.*)_(one|few|many|other)$/;
// Keys built at runtime from enumeration values (checked by vbs-core's tests for completeness).
const DYNAMIC_PREFIXES = [
  'kind.', 'activation.', 'reason.', 'limitation.', 'classification.', 'findingStatus.', 'rule.', 'hint.', 'risk.',
  'origin.', 'source.', 'sourceStatus.', 'sourceReason.', 'setAside.', 'locationKind.', 'productType.', 'coverage.',
  'edition.',
];

const errors = [];
const warnings = [];
const product = JSON.parse(readFileSync(join(root, 'product.json'), 'utf8'));

const placeholders = (message) => [...new Set([...message.matchAll(/\{([A-Za-z0-9_]+)\}/g)].map((m) => m[1]))].sort();
const same = (a, b) => a.length === b.length && a.every((value, i) => value === b[i]);

function load(lang) {
  const path = join(i18nDir, `${lang}.json`);
  let catalog;
  try {
    catalog = JSON.parse(readFileSync(path, 'utf8'));
  } catch (error) {
    errors.push(`${lang}.json: cannot be read (${error.message})`);
    return null;
  }
  for (const [key, value] of Object.entries(catalog)) {
    if (typeof value !== 'string') errors.push(`${lang}.json: "${key}" is not a string`);
  }
  return catalog;
}

/** Plain keys and plural groups (base → sorted categories). */
function shape(catalog) {
  const plain = new Set();
  const plural = new Map();
  for (const key of Object.keys(catalog)) {
    const match = key.match(PLURAL_SUFFIX);
    if (match) plural.set(match[1], [...(plural.get(match[1]) ?? []), match[2]].sort());
    else plain.add(key);
  }
  return { plain, plural };
}

// --- 6: language metadata --------------------------------------------------
const meta = JSON.parse(readFileSync(join(i18nDir, 'languages.json'), 'utf8'));
if (meta.source !== SOURCE) errors.push(`languages.json: source must be "${SOURCE}"`);
if (!same(Object.keys(meta.languages ?? {}).sort(), [...LANGUAGES].sort())) {
  errors.push(`languages.json: languages must be exactly ${LANGUAGES.join(', ')}`);
}
const reviewed = Object.entries(meta.languages ?? {}).filter(([, value]) => value.reviewed).map(([lang]) => lang);
if (!same(reviewed.sort(), [...REVIEWED].sort())) errors.push(`languages.json: reviewed must be ${REVIEWED}`);

// --- 1–3: catalogs ---------------------------------------------------------
const files = readdirSync(i18nDir).filter((f) => f.endsWith('.json') && f !== 'languages.json').map((f) => f.slice(0, -5));
for (const file of files) if (!LANGUAGES.includes(file)) errors.push(`i18n/${file}.json: unsupported language`);
for (const lang of LANGUAGES) if (!files.includes(lang)) errors.push(`i18n/${lang}.json is missing`);

const catalogs = Object.fromEntries(LANGUAGES.map((lang) => [lang, load(lang)]));
const source = catalogs[SOURCE];
if (!source) {
  console.error(errors.join('\n'));
  process.exit(1);
}
const sourceShape = shape(source);

for (const lang of LANGUAGES) {
  const catalog = catalogs[lang];
  if (!catalog) continue;
  const { plain, plural } = shape(catalog);
  for (const key of sourceShape.plain) if (!plain.has(key)) errors.push(`${lang}: missing key "${key}"`);
  for (const key of plain) if (!sourceShape.plain.has(key)) errors.push(`${lang}: unknown key "${key}"`);
  const required = PLURALS[lang] ?? DEFAULT_PLURALS;
  for (const base of sourceShape.plural.keys()) {
    const found = plural.get(base) ?? [];
    if (!same(found, required)) errors.push(`${lang}: plural "${base}" needs [${required}], has [${found}]`);
  }
  for (const base of plural.keys()) {
    if (!sourceShape.plural.has(base)) errors.push(`${lang}: unknown plural "${base}"`);
  }

  for (const [key, message] of Object.entries(catalog)) {
    if (typeof message !== 'string') continue;
    if (!message.trim()) errors.push(`${lang}: "${key}" is empty`);
    else if (message.trim() !== message) errors.push(`${lang}: "${key}" has surrounding whitespace`);
    if (message.includes(product.name)) errors.push(`${lang}: "${key}" hard-codes the product name – use {product}`);

    const plural = key.match(PLURAL_SUFFIX);
    const sourceKey = plural ? `${plural[1]}_other` : key;
    if (typeof source[sourceKey] !== 'string') continue;
    let expected = placeholders(source[sourceKey]);
    let actual = placeholders(message);
    if (plural) {
      expected = expected.filter((p) => p !== 'count');
      actual = actual.filter((p) => p !== 'count');
    }
    if (!same(expected, actual)) {
      errors.push(`${lang}: "${key}" placeholders {${actual}} ≠ source {${expected}}`);
    }
  }
}

// --- 4: keys used in code ---------------------------------------------------
const SCAN = [
  { dir: 'src', ext: ['.ts', '.svelte'] },
  { dir: 'src-tauri/src', ext: ['.rs'] },
  { dir: 'crates', ext: ['.rs'] },
];
const PATTERNS = {
  '.ts': [/\bt\(\s*['"]([^'"\\]+)['"]/g, /\bkey\(\s*['"]([^'"\\]+)['"]\s*\)/g],
  '.svelte': [/\bt\(\s*['"]([^'"\\]+)['"]/g, /\bkey\(\s*['"]([^'"\\]+)['"]\s*\)/g],
  '.rs': [
    /\bt(?:_args)?\(\s*[^,()]+?,\s*"([^"\\]+)"/g,
    /\bkey\(\s*"([^"\\]+)"\s*\)/g,
    /\bconsole\.(?:info|error|text)\(\s*"([^"\\]+)"/g,
    /\.(?:t|args)\(\s*"([^"\\]+)"/g,
  ],
};
const PLURAL_PATTERNS = {
  '.ts': [/\btc\(\s*['"]([^'"\\]+)['"]/g],
  '.svelte': [/\btc\(\s*['"]([^'"\\]+)['"]/g],
  '.rs': [/\bt_count\(\s*[^,()]+?,\s*"([^"\\]+)"/g, /\.count\(\s*"([^"\\]+)"/g],
};

function* walk(dir) {
  for (const entry of readdirSync(dir)) {
    if (entry === 'node_modules' || entry === 'target') continue;
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) yield* walk(path);
    else yield path;
  }
}

const used = new Set();
for (const { dir, ext } of SCAN) {
  for (const path of walk(join(root, dir))) {
    const extension = ext.find((e) => path.endsWith(e));
    if (!extension) continue;
    const text = readFileSync(path, 'utf8');
    const where = relative(root, path).replaceAll('\\', '/');
    for (const pattern of PATTERNS[extension]) {
      for (const [, key] of text.matchAll(pattern)) {
        used.add(key);
        if (!(key in source)) errors.push(`${where}: unknown message key "${key}"`);
      }
    }
    for (const pattern of PLURAL_PATTERNS[extension]) {
      for (const [, base] of text.matchAll(pattern)) {
        for (const category of DEFAULT_PLURALS) used.add(`${base}_${category}`);
        if (!sourceShape.plural.has(base)) errors.push(`${where}: unknown plural message "${base}"`);
      }
    }
  }
}

// --- 5: rule texts ------------------------------------------------------------
const catalog = JSON.parse(readFileSync(join(root, 'rules', 'catalog.json'), 'utf8'));
for (const rule of catalog.rules) {
  const stem = rule.id.toLowerCase().replace('-', '');
  for (const part of ['title', 'rationale']) {
    const key = `rule.${stem}.${part}`;
    used.add(key);
    if (!(key in source)) errors.push(`rules/catalog.json: ${rule.id} lacks the text "${key}"`);
  }
}

for (const key of Object.keys(source)) {
  const plural = key.match(PLURAL_SUFFIX);
  const probe = plural ? `${plural[1]}_other` : key;
  if (!used.has(probe) && !DYNAMIC_PREFIXES.some((prefix) => key.startsWith(prefix))) {
    warnings.push(`unused key "${key}"`);
  }
}

// --- report -----------------------------------------------------------------
for (const warning of warnings) console.warn(`⚠ ${warning}`);
if (errors.length) {
  for (const error of errors) console.error(`✗ ${error}`);
  console.error(`\n${errors.length} i18n error(s).`);
  process.exit(1);
}
console.log(
  `✓ ${LANGUAGES.length} languages complete (${Object.keys(source).length} keys, reviewed: ${REVIEWED.join(', ')}), ` +
    `all used keys and rule texts exist`,
);
