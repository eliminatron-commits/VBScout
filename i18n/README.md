# Translations

The user interface, the collector's console output, the reports and the migration hints use the catalogs in this
folder. English (`en.json`) is the source; every other file must contain exactly the same keys.

| File | Language | Status |
|---|---|---|
| `en.json` | English (source) | reviewed |
| `de.json` | German | reviewed |
| `fr.json` | French | machine-assisted – corrections welcome |
| `es.json` | Spanish | machine-assisted – corrections welcome |
| `it.json` | Italian | machine-assisted – corrections welcome |
| `nl.json` | Dutch | machine-assisted – corrections welcome |
| `pl.json` | Polish | machine-assisted – corrections welcome |
| `pt-BR.json` | Portuguese (Brazil) | machine-assisted – corrections welcome |

`languages.json` records which languages have been reviewed by a native speaker. The app shows a short note with a
link to this folder whenever an unreviewed language is selected.

## How to contribute a fix

1. Edit the message in your language's file – keep the key unchanged.
2. Keep every `{placeholder}` exactly as in English; you may move it within the sentence. `{product}` is replaced by
   the product name – never write the name itself.
3. Use your language's quotation marks and typography (French uses a non-breaking space before `:` – ` ` in
   JSON).
4. Rule texts (`rule.<id>.title`, `rule.<id>.rationale`) explain why something breaks. Keep their meaning exact:
   dates and versions are only ever "expected" – never turn "voraussichtlich"/"expected" into a promise.
5. Plural messages have one key per plural category, e.g. `machine.count_one` / `machine.count_other`. Polish needs
   `_one`, `_few`, `_many` and `_other`; all other languages `_one` and `_other`.
6. `collector.help` is the console help text: keep the option names (`--out`, `--path`, …) and the layout (lines of
   at most 80 characters).
7. Run `npm run check:i18n` before opening a pull request. It checks that all keys exist, placeholders match, plural
   forms are complete, and no message is empty.

When a native speaker has reviewed a whole catalog, set `"reviewed": true` for that language in `languages.json`.

## Adding a language

Adding a language needs code changes (`LANGUAGES` in `src/lib/i18n.svelte.ts`, `Lang` in `crates/vbs-i18n`, the
checker in `scripts/check-i18n.mjs`, `languages.json`). Please open an issue first.
