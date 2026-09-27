//! The management report (PDF): summary, risks, priorities, recommendations, rule-of-thumb
//! effort, coverage, Windows components, method and sources – in the report language, with the
//! licensee's name and (MSP edition) logo. Only licensed editions may create it; the check is
//! here, not in the user interface.

pub mod layout;

use std::collections::BTreeMap;

use krilla::metadata::Metadata;
use vbs_core::model::FindingStatus;
use vbs_core::rules;

use self::layout::{
    ACCENT, Align, BODY, Cell, Column, Figure, Fonts, Layout, MARGIN_TOP, MARGIN_X, MUTED, Op, PAGE_HEIGHT, PAGE_WIDTH,
    Rgb, SHADE, SMALL, STRONG, Style, TEXT, Weight,
};
use super::text::Text;
use super::xlsx::effort_note;
use super::{ReportContext, ReportError};
use crate::assessment::{Assessment, Item, Risk};
use crate::coverage::{DEPRECATION_LOG, SYSMON_LOG};
use vbs_i18n::key;

/// Items listed in the priorities table.
const PRIORITY_ROWS: usize = 25;
/// Windows components listed by name.
const WINDOWS_ROWS: usize = 40;

const HIGH: Rgb = Rgb(0xB9, 0x1C, 0x1C);
const MEDIUM: Rgb = Rgb(0xB4, 0x53, 0x09);
const LOW: Rgb = Rgb(0x15, 0x80, 0x3D);
const INFO: Rgb = Rgb(0x6B, 0x72, 0x80);
const HIGH_FILL: Rgb = Rgb(0xFD, 0xE2, 0xE1);
const MEDIUM_FILL: Rgb = Rgb(0xFE, 0xF3, 0xC7);
const LOW_FILL: Rgb = Rgb(0xE0, 0xF2, 0xE9);

fn risk_color(risk: Risk) -> (Rgb, Rgb) {
    match risk {
        Risk::High => (HIGH, HIGH_FILL),
        Risk::Medium => (MEDIUM, MEDIUM_FILL),
        Risk::Low => (LOW, LOW_FILL),
        Risk::Info => (INFO, SHADE),
    }
}

/// VBScript objects and statements with their PowerShell counterparts (code names, not
/// translated) – a starting point for the migration, never an automatic conversion.
const COUNTERPARTS: &[(&str, &str)] = &[
    ("WScript.Echo", "Write-Output, Write-Host"),
    ("InputBox, MsgBox", "Read-Host, [System.Windows.MessageBox]::Show()"),
    (
        "Scripting.FileSystemObject",
        "Get-ChildItem, Get-Content, Set-Content, Copy-Item, Move-Item, Remove-Item, Test-Path",
    ),
    ("WScript.Shell: Run, Exec", "Start-Process, & (call operator)"),
    ("WScript.Shell: RegRead, RegWrite", "Get-ItemProperty, Set-ItemProperty, New-ItemProperty"),
    ("WScript.Shell: ExpandEnvironmentStrings", "$env:NAME, [Environment]::GetEnvironmentVariable()"),
    ("WScript.Network: MapNetworkDrive", "New-PSDrive -Persist, New-SmbMapping"),
    ("WScript.Network: AddWindowsPrinterConnection", "Add-Printer -ConnectionName"),
    ("WScript.Network: UserName, ComputerName", "$env:USERNAME, $env:COMPUTERNAME"),
    ("GetObject(\"winmgmts:…\"), ExecQuery", "Get-CimInstance, Invoke-CimMethod"),
    ("ADODB.Connection, ADODB.Recordset", "Invoke-Sqlcmd (SqlServer module), System.Data.SqlClient"),
    ("MSXML2.XMLHTTP, MSXML2.ServerXMLHTTP", "Invoke-RestMethod, Invoke-WebRequest"),
    ("MSXML2.DOMDocument", "[xml], Select-Xml"),
    ("VBScript.RegExp", "-match, -replace, Select-String, [regex]"),
    ("Scripting.Dictionary", "@{ } (hashtable), [ordered]@{ }"),
    ("WScript.Sleep", "Start-Sleep"),
    ("WScript.Arguments", "param( ), $args"),
    ("On Error Resume Next, Err.Number", "try { } catch { }, -ErrorAction, $Error"),
    ("GetObject(\"LDAP://…\")", "Get-ADUser, Get-ADGroup (ActiveDirectory module), [adsi]"),
];

/// Writes the management report. Refused in the free edition.
pub fn write(assessment: &Assessment, context: &ReportContext<'_>) -> Result<Vec<u8>, ReportError> {
    compose(assessment, context).map(|(_, bytes)| bytes)
}

/// Lays out and renders the report; also returns the text of every page (tests).
pub(crate) fn compose(
    assessment: &Assessment,
    context: &ReportContext<'_>,
) -> Result<(Vec<String>, Vec<u8>), ReportError> {
    if !context.edition.allows_pdf() {
        return Err(ReportError::NotLicensed);
    }
    let fonts = Fonts::load().map_err(ReportError::Pdf)?;
    let text = Text::new(context.lang);
    let mut layout = Layout::new(&fonts);

    title_page(&mut layout, assessment, context, &text);
    layout.new_page();
    summary(&mut layout, assessment, context, &text);
    risks(&mut layout, assessment, &text);
    priorities(&mut layout, assessment, context, &text);
    recommendations(&mut layout, assessment, &text);
    effort(&mut layout, assessment, &text);
    coverage(&mut layout, assessment, &text);
    windows(&mut layout, assessment, &text);
    method(&mut layout, &text);
    counterparts(&mut layout, &text);

    let pages = layout::page_texts(layout.pages());
    let product = vbs_config::product().name.clone();
    let title = text.t("report.title");
    let metadata = Metadata::new()
        .title(title.clone())
        .language(context.lang.tag().to_owned())
        .creator(format!("{product} {}", context.version))
        .producer(product.clone());
    let logo = context.logo().and_then(|logo| logo.image());
    let footer_left = match (context.customer(), context.licensee()) {
        (Some(customer), _) => format!("{product} · {title} · {customer}"),
        (None, Some(licensee)) => format!("{product} · {title} · {licensee}"),
        (None, None) => format!("{product} · {title}"),
    };
    let bytes = layout
        .render(metadata, logo.as_ref(), |layout, index, count| {
            let mut ops = Vec::new();
            if index == 0 {
                return ops;
            }
            let style = SMALL;
            let page = text.args("report.page", &[("page", &(index + 1)), ("pages", &count)]);
            let y = PAGE_HEIGHT - 34.0;
            let page_width = layout.fonts().width(&page, style.weight, style.size);
            let left_width = PAGE_WIDTH - 2.0 * MARGIN_X - page_width - 24.0;
            let left = layout.wrap(&footer_left, &style, left_width).into_iter().next().unwrap_or_default();
            ops.push(Op::Line {
                x1: MARGIN_X,
                y1: y - 12.0,
                x2: PAGE_WIDTH - MARGIN_X,
                y2: y - 12.0,
                width: 0.4,
                color: layout::RULE,
            });
            ops.push(Op::Text { x: MARGIN_X, y, text: left, style });
            ops.push(Op::Text { x: PAGE_WIDTH - MARGIN_X - page_width, y, text: page, style });
            ops
        })
        .map_err(ReportError::Pdf)?;
    Ok((pages, bytes))
}

fn catalog_date(text: &Text, date: time::Date) -> String {
    date.with_hms(0, 0, 0).map(|value| text.date(value.assume_utc())).unwrap_or_default()
}

fn title_page(layout: &mut Layout<'_>, assessment: &Assessment, context: &ReportContext<'_>, text: &Text) {
    if let Some(logo) = context.logo() {
        let (width, height) = logo.size();
        let scale = (180.0 / width as f32).min(70.0 / height as f32);
        layout.push(Op::Image {
            x: MARGIN_X,
            y: MARGIN_TOP,
            width: width as f32 * scale,
            height: height as f32 * scale,
        });
    }
    layout.push(Op::Rect { x: MARGIN_X, y: 250.0, width: 48.0, height: 4.0, color: ACCENT });
    layout.set_y(268.0);
    let title = Style::new(26.0, Weight::Bold, TEXT);
    let lines = layout.wrap(&text.t("report.title"), &title, layout::CONTENT_WIDTH);
    layout.lines_at(MARGIN_X, &lines, &title, Align::Left, layout::CONTENT_WIDTH);
    layout.space(4.0);
    layout.paragraph(&text.t("report.subtitle"), &Style::new(13.0, Weight::Regular, MUTED));
    layout.space(26.0);

    let large = Style::new(13.0, Weight::Bold, TEXT);
    let mut lines: Vec<(String, Style)> = Vec::new();
    match (context.edition, context.customer()) {
        (crate::edition::Edition::Msp { company, .. }, customer) => {
            if let Some(customer) = customer {
                lines.push((text.args("report.preparedFor", &[("name", &customer)]), large));
            }
            lines.push((text.args("report.preparedBy", &[("name", company)]), BODY));
        }
        (edition, customer) => {
            if let Some(licensee) = edition.licensee() {
                lines.push((licensee.to_owned(), large));
            }
            if let Some(customer) = customer {
                lines.push((text.args("report.environment", &[("name", &customer)]), BODY));
            }
        }
    }
    lines.push((text.args("report.createdOn", &[("date", &text.date(context.created))]), BODY));
    let first = assessment.machines.iter().map(|machine| machine.started_at).min();
    let last = assessment.machines.iter().map(|machine| machine.started_at).max();
    if let (Some(first), Some(last)) = (first, last) {
        lines.push((
            text.count(
                "report.scope",
                assessment.machines.len(),
                &[("from", &text.date(first)), ("to", &text.date(last))],
            ),
            BODY,
        ));
    }
    let as_of = catalog_date(text, rules::catalog().as_of);
    lines.push((
        text.args("report.generatedWith", &[("version", &context.version), ("date", &as_of)]),
        Style { color: MUTED, ..BODY },
    ));
    for (line, style) in lines {
        let wrapped = layout.wrap(&line, &style, layout::CONTENT_WIDTH);
        layout.lines_at(MARGIN_X, &wrapped, &style, Align::Left, layout::CONTENT_WIDTH);
        layout.space(4.0);
    }
    layout.set_y(PAGE_HEIGHT - 140.0);
    layout.note(&text.t("report.confidential"), &SMALL, SHADE);
}

fn summary(layout: &mut Layout<'_>, assessment: &Assessment, context: &ReportContext<'_>, text: &Text) {
    let summary = &assessment.summary;
    layout.heading(&text.t("report.section.summary"), 1);
    layout.paragraph(&text.count("report.summary.intro", summary.machines, &[]), &BODY);
    let mut figures = vec![
        Figure { value: text.count_of(summary.machines), label: text.t("report.figure.machines"), color: ACCENT },
        Figure { value: text.count_of(summary.items), label: text.t("report.figure.items"), color: TEXT },
        Figure { value: text.count_of(summary.high), label: text.t("report.figure.high"), color: HIGH },
        Figure { value: text.count_of(summary.medium), label: text.t("report.figure.medium"), color: MEDIUM },
        Figure { value: text.count_of(summary.low), label: text.t("report.figure.low"), color: LOW },
        Figure {
            value: text.count_of(summary.not_checkable),
            label: text.t("report.figure.notCheckable"),
            color: MUTED,
        },
        Figure { value: text.count_of(summary.windows_items), label: text.t("report.figure.windows"), color: INFO },
    ];
    if context.effort() {
        figures.push(Figure {
            value: text.hour_range(summary.effort_min, summary.effort_max),
            label: text.t("report.figure.effort"),
            color: ACCENT,
        });
    }
    layout.figures(&figures, 4);

    let mut statements = Vec::new();
    if summary.high > 0 {
        statements.push(text.count("report.summary.high", summary.high, &[]));
        let mut kinds: Vec<(String, usize)> = summary
            .by_kind
            .iter()
            .filter(|tally| tally.high > 0)
            .map(|tally| (text.kind(&tally.kind), tally.high))
            .collect();
        kinds.sort_by_key(|entry| std::cmp::Reverse(entry.1));
        let list: Vec<String> = kinds.iter().take(4).map(|(kind, count)| format!("{kind} ({count})")).collect();
        statements.push(text.args("report.summary.highKinds", &[("list", &list.join(", "))]));
    } else {
        statements.push(text.t("report.summary.noHigh"));
    }
    if summary.medium > 0 {
        statements.push(text.count("report.summary.medium", summary.medium, &[]));
    }
    if summary.not_checkable > 0 {
        statements.push(text.count("report.summary.notCheckable", summary.not_checkable, &[]));
    }
    if summary.credentials > 0 {
        statements.push(text.count("report.summary.credentials", summary.credentials, &[]));
    }
    if summary.windows_items > 0 {
        statements.push(text.count("report.summary.windows", summary.windows_items, &[]));
    }
    if context.effort() {
        statements.push(text.args(
            "report.summary.effort",
            &[
                ("min", &text.hours(summary.effort_min)),
                ("max", &text.hours(summary.effort_max)),
                ("minDays", &text.days(summary.effort_min)),
                ("maxDays", &text.days(summary.effort_max)),
            ],
        ));
    }
    statements.push(text.args(
        "report.summary.coverage",
        &[("full", &assessment.coverage.full), ("machines", &assessment.coverage.machines)],
    ));
    layout.bullets(&statements, &BODY);

    let catalog = rules::catalog();
    if let Some(source) = catalog.source("ms-vbscript-timeline") {
        let published = source.published.map(|date| catalog_date(text, date)).unwrap_or_default();
        layout.note(
            &text.args(
                "report.summary.timeline",
                &[
                    ("publisher", &source.publisher),
                    ("title", &source.title),
                    ("published", &published),
                    ("checked", &catalog_date(text, source.checked)),
                ],
            ),
            &BODY,
            SHADE,
        );
    }
}

fn risks(layout: &mut Layout<'_>, assessment: &Assessment, text: &Text) {
    let summary = &assessment.summary;
    let rows: Vec<(String, Vec<(f32, Rgb)>)> = summary
        .by_kind
        .iter()
        .map(|tally| {
            (
                text.kind(&tally.kind),
                vec![(tally.high as f32, HIGH), (tally.medium as f32, MEDIUM), (tally.low as f32, LOW)],
            )
        })
        .collect();
    // Heading, introduction and chart on one page.
    let intro = text.t("report.risks.intro");
    layout.ensure(40.0 + layout.paragraph_height(&intro, &BODY) + layout.bars_height(&rows));
    layout.heading(&text.t("report.section.risks"), 1);
    layout.paragraph(&intro, &BODY);
    if summary.by_kind.is_empty() {
        layout.paragraph(&text.t("report.nothingFound"), &STRONG);
        return;
    }
    let legend: Vec<(String, Rgb)> =
        vec![(text.risk(Risk::High), HIGH), (text.risk(Risk::Medium), MEDIUM), (text.risk(Risk::Low), LOW)];
    layout.bars(&rows, &legend);

    let columns = vec![
        Column::new(text.t("report.column.kind"), 3.2, Align::Left),
        Column::new(text.t("report.column.items"), 1.0, Align::Right),
        Column::new(text.risk(Risk::High), 1.0, Align::Right),
        Column::new(text.risk(Risk::Medium), 1.0, Align::Right),
        Column::new(text.risk(Risk::Low), 1.0, Align::Right),
        Column::new(text.t("report.column.notCheckable"), 1.3, Align::Right),
        Column::new(text.t("report.column.machines"), 1.1, Align::Right),
    ];
    let mut rows: Vec<Vec<Cell>> = summary
        .by_kind
        .iter()
        .map(|tally| {
            vec![
                Cell::new(text.kind(&tally.kind)),
                Cell::new(text.count_of(tally.items)),
                Cell::new(text.count_of(tally.high)),
                Cell::new(text.count_of(tally.medium)),
                Cell::new(text.count_of(tally.low)),
                Cell::new(text.count_of(tally.not_checkable)),
                Cell::new(text.count_of(tally.machines)),
            ]
        })
        .collect();
    rows.push(vec![
        Cell::new(text.t("report.effort.total")).style(STRONG),
        Cell::new(text.count_of(summary.items)).style(STRONG),
        Cell::new(text.count_of(summary.high)).style(STRONG),
        Cell::new(text.count_of(summary.medium)).style(STRONG),
        Cell::new(text.count_of(summary.low)).style(STRONG),
        Cell::new(text.count_of(summary.not_checkable)).style(STRONG),
        Cell::new(text.count_of(summary.machines)).style(STRONG),
    ]);
    layout.table(&columns, &rows, 8.5);
}

fn where_text(assessment: &Assessment, item: &Item, text: &Text) -> String {
    let first = item.first();
    let mut parts = vec![first.location.path.clone()];
    if let Some(sub) = &first.location.item {
        parts.push(sub.clone());
    }
    let mut detail = parts.join(" · ");
    if let Some(target) = &first.target {
        detail.push_str(&format!(" → {target}"));
    }
    let machine = &assessment.machines[first.machine].machine.hostname;
    if item.machines > 1 {
        format!("{detail}\n{}", text.count("report.onMachines", item.machines, &[]))
    } else {
        format!("{detail}\n{machine}")
    }
}

fn priorities(layout: &mut Layout<'_>, assessment: &Assessment, context: &ReportContext<'_>, text: &Text) {
    layout.heading(&text.t("report.section.priorities"), 1);
    let own: Vec<&Item> = assessment.own_items().collect();
    if own.is_empty() {
        layout.paragraph(&text.t("report.nothingFound"), &STRONG);
        return;
    }
    let shown = own.len().min(PRIORITY_ROWS);
    layout.paragraph(&text.count("report.priorities.intro", shown, &[]), &BODY);
    let mut columns = vec![
        Column::new(text.t("report.column.number"), 0.55, Align::Right),
        Column::new(text.t("report.column.risk"), 1.0, Align::Left),
        Column::new(text.t("report.column.finding"), 5.2, Align::Left),
        Column::new(text.t("report.column.activation"), 1.6, Align::Left),
    ];
    if context.effort() {
        columns.push(Column::new(text.t("report.column.effort"), 1.35, Align::Right));
    }
    let rows: Vec<Vec<Cell>> = own
        .iter()
        .take(shown)
        .map(|item| {
            let (color, fill) = risk_color(item.risk);
            let mut title = text.rule_title(&item.rule);
            if item.status == FindingStatus::NotCheckable
                && let Some(reason) = &item.reason
            {
                title = format!("{title} ({})", text.reason(reason));
            }
            let mut row = vec![
                Cell::new(format!("#{}", item.number)),
                Cell::new(text.risk(item.risk)).style(Style::new(8.0, Weight::Bold, color)).fill(fill),
                Cell::new(title).detail(where_text(assessment, item, text)),
                Cell::new(text.activation(&item.activation)),
            ];
            if context.effort() {
                let effort = item.effort.hours().map(|(min, max)| text.hour_range(min, max)).unwrap_or_default();
                row.push(Cell::new(effort).detail(effort_note(item, text, false)));
            }
            row
        })
        .collect();
    layout.table(&columns, &rows, 8.0);
    if own.len() > shown {
        layout.paragraph(&text.count("report.priorities.more", own.len() - shown, &[]), &SMALL);
    }
}

fn numbers(numbers: &[usize], limit: usize) -> String {
    let mut shown: Vec<String> = numbers.iter().take(limit).map(|number| format!("#{number}")).collect();
    if numbers.len() > limit {
        shown.push("…".into());
    }
    shown.join(", ")
}

fn recommendations(layout: &mut Layout<'_>, assessment: &Assessment, text: &Text) {
    // One recommendation per distinct hint, in the order of the most urgent item it applies to.
    let mut groups: Vec<(String, Vec<String>, Vec<usize>)> = Vec::new();
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    for item in assessment.own_items() {
        let hint = text.hint(item);
        let title = match &item.reason {
            Some(reason) => format!("{} ({})", text.rule_title(&item.rule), text.reason(reason)),
            None => text.rule_title(&item.rule),
        };
        let group = *index.entry(hint.clone()).or_insert_with(|| {
            groups.push((hint, Vec::new(), Vec::new()));
            groups.len() - 1
        });
        let (_, titles, items) = &mut groups[group];
        if !titles.contains(&title) {
            titles.push(title);
        }
        items.push(item.number);
    }
    if groups.is_empty() {
        return;
    }
    layout.heading(&text.t("report.section.recommendations"), 1);
    layout.paragraph(&text.t("report.recommendations.intro"), &BODY);
    for (hint, titles, items) in groups {
        let heading = format!(
            "{} – {}",
            titles.join("; "),
            text.count("report.recommendations.items", items.len(), &[("numbers", &numbers(&items, 12))])
        );
        layout.ensure(STRONG.line_height() * 2.0 + BODY.line_height() * 3.0);
        layout.paragraph(&heading, &STRONG);
        layout.space(-3.0);
        layout.paragraph(&hint, &BODY);
    }
}

fn effort(layout: &mut Layout<'_>, assessment: &Assessment, text: &Text) {
    let summary = &assessment.summary;
    layout.heading(&text.t("report.section.effort"), 1);
    layout.note(&text.t("report.effort.intro"), &BODY, SHADE);
    let columns = vec![
        Column::new(text.t("report.column.kind"), 3.4, Align::Left),
        Column::new(text.t("report.column.counted"), 1.3, Align::Right),
        Column::new(text.t("report.column.effort"), 2.0, Align::Right),
    ];
    let mut counted: BTreeMap<String, usize> = BTreeMap::new();
    for item in assessment.own_items().filter(|item| item.effort.hours().is_some()) {
        *counted.entry(item.kind.as_str().to_owned()).or_default() += 1;
    }
    let mut rows: Vec<Vec<Cell>> = summary
        .by_kind
        .iter()
        .map(|tally| {
            vec![
                Cell::new(text.kind(&tally.kind)),
                Cell::new(text.count_of(counted.get(tally.kind.as_str()).copied().unwrap_or(0))),
                Cell::new(text.hour_range(tally.effort_min, tally.effort_max)),
            ]
        })
        .collect();
    rows.push(vec![
        Cell::new(text.t("report.effort.total")).style(STRONG),
        Cell::new(text.count_of(counted.values().sum())).style(STRONG),
        Cell::new(text.hour_range(summary.effort_min, summary.effort_max)).style(STRONG).detail(text.args(
            "report.personDays",
            &[("min", &text.days(summary.effort_min)), ("max", &text.days(summary.effort_max))],
        )),
    ]);
    layout.table(&columns, &rows, 8.5);
    layout.heading(&text.t("report.effort.assumptions"), 2);
    let assumptions: Vec<String> = [
        key("report.effort.assumption.perRule"),
        key("report.effort.assumption.central"),
        key("report.effort.assumption.copies"),
        key("report.effort.assumption.windows"),
        key("report.effort.assumption.excluded"),
    ]
    .iter()
    .map(|name| text.t(name))
    .collect();
    layout.bullets(&assumptions, &BODY);
}

fn coverage(layout: &mut Layout<'_>, assessment: &Assessment, text: &Text) {
    let coverage = &assessment.coverage;
    layout.heading(&text.t("report.section.coverage"), 1);
    layout.note(&text.t("report.coverage.intro"), &BODY, SHADE);
    let mut lines = vec![text.args(
        "report.coverage.machines",
        &[("full", &coverage.full), ("limited", &coverage.limited), ("machines", &coverage.machines)],
    )];
    for (code, count) in &coverage.limitations {
        lines.push(format!("{} ({})", text.limitation(code), text.count("machine.count", *count, &[])));
    }
    let deprecation = coverage.log_span(DEPRECATION_LOG);
    if deprecation.read > 0 {
        let days = |value: Option<f64>| value.map(|days| text.decimal(days.round(), 0)).unwrap_or_else(|| "–".into());
        lines.push(text.args(
            "report.coverage.deprecation",
            &[
                ("read", &deprecation.read),
                ("reported", &deprecation.reported),
                ("minDays", &days(deprecation.min_days)),
                ("maxDays", &days(deprecation.max_days)),
                ("earliest", &deprecation.earliest.map(|value| text.date(value)).unwrap_or_else(|| "–".into())),
            ],
        ));
    } else {
        lines.push(text.t("report.coverage.deprecationNone"));
    }
    let sysmon = coverage.log_span(SYSMON_LOG);
    lines.push(text.args("report.coverage.sysmon", &[("read", &sysmon.read), ("reported", &sysmon.reported)]));
    let files = coverage.files;
    lines.push(text.args(
        "report.coverage.files",
        &[
            ("entries", &text.integer(files.entries)),
            ("errors", &text.integer(files.errors)),
            ("skipped", &text.integer(files.skipped)),
        ],
    ));
    if !assessment.set_aside.is_empty() {
        lines.push(text.count("report.coverage.setAside", assessment.set_aside.len(), &[]));
    }
    layout.bullets(&lines, &BODY);
    if !coverage.not_checkable.is_empty() {
        layout.heading(&text.t("report.coverage.notCheckable"), 2);
        let reasons: Vec<String> = coverage
            .not_checkable
            .iter()
            .map(|(reason, count)| format!("{}: {}", text.value("reason", reason), text.count_of(*count)))
            .collect();
        layout.bullets(&reasons, &BODY);
    }
    let limited: Vec<String> = assessment
        .machines
        .iter()
        .filter(|machine| !machine.coverage.limitations.is_empty())
        .take(20)
        .map(|machine| {
            let codes: Vec<String> = machine
                .coverage
                .limitations
                .iter()
                .map(|limitation| text.limitation(limitation.code.as_str()))
                .collect();
            format!("{}: {}", machine.machine.hostname, codes.join(" "))
        })
        .collect();
    if !limited.is_empty() {
        layout.heading(&text.t("report.coverage.limitedMachines"), 2);
        layout.bullets(&limited, &SMALL);
    }
}

fn windows(layout: &mut Layout<'_>, assessment: &Assessment, text: &Text) {
    let items: Vec<&Item> = assessment.windows_items().collect();
    if items.is_empty() {
        return;
    }
    layout.heading(&text.t("report.section.windows"), 1);
    layout.paragraph(&text.t("report.windows.intro"), &BODY);
    let mut sorted = items.clone();
    sorted.sort_by(|a, b| b.occurrences.len().cmp(&a.occurrences.len()).then(a.number.cmp(&b.number)));
    let columns = vec![
        Column::new(text.t("report.column.file"), 2.6, Align::Left),
        Column::new(text.t("report.column.finding"), 3.6, Align::Left),
        Column::new(text.t("report.column.locations"), 1.2, Align::Right),
        Column::new(text.t("report.column.machines"), 1.2, Align::Right),
    ];
    let rows: Vec<Vec<Cell>> = sorted
        .iter()
        .take(WINDOWS_ROWS)
        .map(|item| {
            let first = item.first();
            let name = first.location.path.rsplit(['\\', '/']).next().unwrap_or(&first.location.path).to_owned();
            let mut finding = text.rule_title(&item.rule);
            if let Some(reason) = &item.reason {
                finding = format!("{finding} ({})", text.reason(reason));
            }
            vec![
                Cell::new(name),
                Cell::new(finding),
                Cell::new(text.count_of(item.occurrences.len())),
                Cell::new(text.count_of(item.machines)),
            ]
        })
        .collect();
    layout.table(&columns, &rows, 8.0);
    if sorted.len() > WINDOWS_ROWS {
        layout.paragraph(&text.count("report.windows.more", sorted.len() - WINDOWS_ROWS, &[]), &SMALL);
    }
}

fn method(layout: &mut Layout<'_>, text: &Text) {
    layout.heading(&text.t("report.section.method"), 1);
    for name in [
        key("report.method.classification"),
        key("report.method.risk"),
        key("report.method.dedup"),
        key("report.method.windows"),
    ] {
        layout.paragraph(&text.t(name), &BODY);
    }
    let catalog = rules::catalog();
    layout.heading(&text.args("report.method.sources", &[("date", &catalog_date(text, catalog.as_of))]), 2);
    let sources: Vec<String> = catalog
        .sources
        .iter()
        .map(|source| {
            let mut dates = Vec::new();
            if let Some(published) = source.published {
                dates.push(text.args("report.published", &[("date", &catalog_date(text, published))]));
            }
            dates.push(text.args("report.checked", &[("date", &catalog_date(text, source.checked))]));
            format!("{}: {}. {} ({})", source.publisher, source.title, source.url, dates.join(", "))
        })
        .collect();
    layout.bullets(&sources, &SMALL);
}

fn counterparts(layout: &mut Layout<'_>, text: &Text) {
    layout.new_page();
    layout.heading(&text.t("report.section.powershell"), 1);
    layout.paragraph(&text.t("report.powershell.intro"), &BODY);
    let columns = vec![
        Column::new(text.t("report.column.vbscript"), 1.0, Align::Left),
        Column::new(text.t("report.column.powershell"), 1.3, Align::Left),
    ];
    let rows: Vec<Vec<Cell>> =
        COUNTERPARTS.iter().map(|(vbscript, powershell)| vec![Cell::new(*vbscript), Cell::new(*powershell)]).collect();
    layout.table(&columns, &rows, 8.5);
}
