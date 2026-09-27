#!/usr/bin/env python3
"""Checks reports written by the evaluation with tools that did not write them.

    python3 scripts/reports/check.py <folder> [--free <folder>]

<folder> holds report-<lang>.pdf and findings-<lang>.xlsx of a licensed edition, --free the
findings-<lang>.xlsx of the free edition. The PDF text is extracted with pdftotext (poppler), the
Excel list is read with the Python standard library (zipfile, XML). Checked per language:

* every section heading of the PDF and every sheet name and column header of the Excel list
  appears in that language (from i18n/<lang>.json) – no raw message keys, no open placeholders;
* effort is labelled as a rule of thumb (the translated section and column titles say so);
* the free edition's list has no migration hint and no effort columns.

Exit code 1 on any problem.
"""

import json
import re
import subprocess
import sys
import zipfile
from pathlib import Path
from xml.etree import ElementTree

ROOT = Path(__file__).resolve().parents[2]
NS = {"s": "http://schemas.openxmlformats.org/spreadsheetml/2006/main"}
SECTIONS = ["summary", "risks", "priorities", "recommendations", "effort", "coverage", "method", "powershell"]
SHEETS = ["summary", "findings", "locations", "machines", "coverage", "rules"]
LICENSED_COLUMNS = ["hint", "effortMin", "effortMax", "effortNote"]
problems = []


def catalog(lang):
    return json.loads((ROOT / "i18n" / f"{lang}.json").read_text(encoding="utf-8"))


def squeeze(text):
    return re.sub(r"\s+", "", text)


def raw_keys(text, messages):
    return [key for key in messages if "." in key and key in text]


def placeholders(text):
    return re.findall(r"\{[A-Za-z0-9_]+\}", text)


def check_pdf(path, lang, messages):
    text = subprocess.run(["pdftotext", "-enc", "UTF-8", str(path), "-"], capture_output=True, text=True, check=True).stdout
    flat = squeeze(text)
    for section in SECTIONS:
        title = messages[f"report.section.{section}"]
        if squeeze(title) not in flat:
            problems.append(f"{path.name}: section title missing: {title!r}")
    for key in raw_keys(text, messages):
        problems.append(f"{path.name}: raw message key {key}")
    for found in placeholders(text):
        problems.append(f"{path.name}: open placeholder {found}")
    if len(text) < 2000:
        problems.append(f"{path.name}: only {len(text)} characters of text")
    return len(text)


def xlsx_texts(path):
    with zipfile.ZipFile(path) as archive:
        workbook = ElementTree.fromstring(archive.read("xl/workbook.xml"))
        sheets = [sheet.get("name") for sheet in workbook.find("s:sheets", NS)]
        shared = ElementTree.fromstring(archive.read("xl/sharedStrings.xml"))
        strings = ["".join(node.itertext()) for node in shared.findall("s:si", NS)]
    return sheets, strings


def check_xlsx(path, lang, messages, licensed):
    sheets, strings = xlsx_texts(path)
    expected = [messages[f"report.sheet.{sheet}"] for sheet in SHEETS]
    if sheets[: len(expected)] != expected:
        problems.append(f"{path.name}: sheets {sheets} ≠ {expected}")
    present = set(strings)
    for column in ["finding", "risk", "location", "machines"]:
        if messages[f"report.column.{column}"] not in present:
            problems.append(f"{path.name}: column {column} missing")
    for column in LICENSED_COLUMNS:
        title = messages[f"report.column.{column}"]
        if licensed and title not in present:
            problems.append(f"{path.name}: licensed column {column} missing")
        if not licensed and title in present:
            problems.append(f"{path.name}: the free edition must not have the column {column}")
    joined = "\n".join(strings)
    for key in raw_keys(joined, messages):
        problems.append(f"{path.name}: raw message key {key}")
    for found in placeholders(joined):
        problems.append(f"{path.name}: open placeholder {found}")
    return len(strings)


def main(argv):
    if not argv:
        print(__doc__)
        return 2
    folder = Path(argv[0])
    free = Path(argv[argv.index("--free") + 1]) if "--free" in argv else None
    languages = json.loads((ROOT / "i18n" / "languages.json").read_text(encoding="utf-8"))["languages"]
    for lang in languages:
        messages = catalog(lang)
        pdf = folder / f"report-{lang}.pdf"
        xlsx = folder / f"findings-{lang}.xlsx"
        if not pdf.exists() or not xlsx.exists():
            problems.append(f"{lang}: report-{lang}.pdf or findings-{lang}.xlsx missing in {folder}")
            continue
        characters = check_pdf(pdf, lang, messages)
        strings = check_xlsx(xlsx, lang, messages, licensed=True)
        summary = f"{lang}: PDF {characters} characters, Excel {strings} texts"
        if free:
            free_xlsx = free / f"findings-{lang}.xlsx"
            if free_xlsx.exists():
                check_xlsx(free_xlsx, lang, messages, licensed=False)
                summary += ", free edition list checked"
            else:
                problems.append(f"{lang}: {free_xlsx} missing")
        print(summary)
    for problem in problems:
        print(f"✗ {problem}", file=sys.stderr)
    if problems:
        return 1
    print("✓ reports complete in every language (read with pdftotext and zipfile/XML)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
