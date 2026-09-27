//! The technical Excel list: every item with its locations, the machines, the coverage of every
//! source and the rules with their sources – a working list with filters and frozen headers.
//! Effort columns are labelled as rules of thumb; the free edition gets no hint and no effort
//! columns at all.

use rust_xlsxwriter::{Color, DocProperties, ExcelDateTime, Format, FormatAlign, FormatBorder, Workbook, Worksheet};
use time::OffsetDateTime;
use vbs_core::rules::{self, EffortBasis};

use super::text::Text;
use super::{ReportContext, ReportError};
use crate::assessment::{Assessment, Item, Risk};
use crate::effort::Effort;
use vbs_i18n::key;

/// Longest text written into a cell (Excel's limit is 32,767 characters).
const MAX_CELL_CHARS: usize = 32_000;
/// Data rows per sheet (Excel allows 1,048,576 rows including the header).
const MAX_ROWS: usize = 1_048_000;
/// Machine names listed in one cell of the finding list.
const MAX_MACHINE_NAMES: usize = 50;

type Result<T> = std::result::Result<T, ReportError>;

struct Styles {
    title: Format,
    header: Format,
    label: Format,
    wrap: Format,
    date: Format,
    hours: Format,
    note: Format,
    risk: [Format; 4],
}

impl Styles {
    fn new() -> Self {
        let risk = |background: u32, text: u32| {
            Format::new()
                .set_background_color(Color::RGB(background))
                .set_font_color(Color::RGB(text))
                .set_bold()
                .set_align(FormatAlign::Top)
        };
        Self {
            title: Format::new().set_bold().set_font_size(16),
            header: Format::new()
                .set_bold()
                .set_text_wrap()
                .set_align(FormatAlign::Top)
                .set_background_color(Color::RGB(0x1E3A8A))
                .set_font_color(Color::White)
                .set_border_bottom(FormatBorder::Thin),
            label: Format::new().set_bold().set_align(FormatAlign::Top),
            wrap: Format::new().set_text_wrap().set_align(FormatAlign::Top),
            date: Format::new().set_num_format("yyyy-mm-dd hh:mm").set_align(FormatAlign::Top),
            hours: Format::new().set_num_format("0.0#").set_align(FormatAlign::Top),
            note: Format::new().set_italic().set_text_wrap().set_align(FormatAlign::Top),
            risk: [
                risk(0xFDE2E1, 0x8A1C1C),
                risk(0xFEF3C7, 0x7C4A03),
                risk(0xE0F2E9, 0x14532D),
                risk(0xEEF0F3, 0x3F4652),
            ],
        }
    }

    fn risk(&self, risk: Risk) -> &Format {
        &self.risk[Risk::ALL.iter().position(|value| *value == risk).unwrap_or(3)]
    }
}

/// A sheet with a header row, written row by row.
struct Sheet<'a> {
    sheet: &'a mut Worksheet,
    row: u32,
    top: Format,
}

impl<'a> Sheet<'a> {
    fn new(sheet: &'a mut Worksheet, name: &str, columns: &[(String, f64)], styles: &Styles) -> Result<Self> {
        sheet.set_name(sheet_name(name))?;
        for (column, (title, width)) in columns.iter().enumerate() {
            let column = column as u16;
            sheet.write_string_with_format(0, column, cell(title), &styles.header)?;
            sheet.set_column_width(column, *width)?;
        }
        if !columns.is_empty() {
            sheet.set_freeze_panes(1, 0)?;
            sheet.autofilter(0, 0, 0, columns.len() as u16 - 1)?;
        }
        Ok(Self { sheet, row: 0, top: Format::new().set_align(FormatAlign::Top) })
    }

    /// Starts the next row; `false` once Excel's row limit is reached.
    fn next(&mut self) -> bool {
        if self.row as usize >= MAX_ROWS {
            return false;
        }
        self.row += 1;
        true
    }

    fn text(&mut self, column: u16, value: &str) -> Result<()> {
        if !value.is_empty() {
            self.sheet.write_string_with_format(self.row, column, cell(value), &self.top)?;
        }
        Ok(())
    }

    fn styled(&mut self, column: u16, value: &str, format: &Format) -> Result<()> {
        if !value.is_empty() {
            self.sheet.write_string_with_format(self.row, column, cell(value), format)?;
        }
        Ok(())
    }

    fn number(&mut self, column: u16, value: f64) -> Result<()> {
        self.sheet.write_number_with_format(self.row, column, value, &self.top)?;
        Ok(())
    }

    fn formatted_number(&mut self, column: u16, value: f64, format: &Format) -> Result<()> {
        self.sheet.write_number_with_format(self.row, column, value, format)?;
        Ok(())
    }

    fn date(&mut self, column: u16, value: Option<OffsetDateTime>, format: &Format) -> Result<()> {
        if let Some(value) = value.and_then(excel_date) {
            self.sheet.write_datetime_with_format(self.row, column, &value, format)?;
        }
        Ok(())
    }

    /// A note under the table when rows were left out.
    fn truncated(&mut self, left_out: usize, text: &Text, styles: &Styles) -> Result<()> {
        if left_out > 0 {
            let note = text.count("report.rowsLeftOut", left_out, &[]);
            self.sheet.write_string_with_format(self.row + 1, 0, cell(&note), &styles.note)?;
        }
        Ok(())
    }
}

/// Excel sheet names: at most 31 characters, none of `[]:*?/\`.
fn sheet_name(name: &str) -> String {
    let cleaned: String = name.chars().filter(|c| !matches!(c, '[' | ']' | ':' | '*' | '?' | '/' | '\\')).collect();
    cleaned.chars().take(31).collect()
}

fn cell(value: &str) -> String {
    if value.len() <= MAX_CELL_CHARS {
        return value.to_owned();
    }
    let mut end = MAX_CELL_CHARS;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &value[..end])
}

fn excel_date(value: OffsetDateTime) -> Option<ExcelDateTime> {
    ExcelDateTime::from_timestamp(value.unix_timestamp()).ok()
}

/// Writes the technical list as an `.xlsx` file in memory.
pub fn write(assessment: &Assessment, context: &ReportContext<'_>) -> Result<Vec<u8>> {
    let text = Text::new(context.lang);
    let styles = Styles::new();
    let product = &vbs_config::product().name;
    let mut workbook = Workbook::new();
    let mut properties =
        DocProperties::new().set_title(text.t("report.technicalTitle")).set_author(product).set_comment(
            text.args("report.generatedWith", &[("version", &context.version), ("date", &catalog_date(&text))]),
        );
    if let Some(licensee) = context.licensee() {
        properties = properties.set_company(licensee);
    }
    workbook.set_properties(&properties);

    summary(workbook.add_worksheet(), assessment, context, &text, &styles)?;
    findings(workbook.add_worksheet(), assessment, context, &text, &styles)?;
    locations(workbook.add_worksheet(), assessment, &text, &styles)?;
    machines(workbook.add_worksheet(), assessment, &text, &styles)?;
    coverage(workbook.add_worksheet(), assessment, &text, &styles)?;
    rule_sheet(workbook.add_worksheet(), context, &text, &styles)?;
    if !assessment.set_aside.is_empty() {
        set_aside(workbook.add_worksheet(), assessment, &text, &styles)?;
    }
    Ok(workbook.save_to_buffer()?)
}

fn catalog_date(text: &Text) -> String {
    let as_of = rules::catalog().as_of;
    let midnight = as_of.with_hms(0, 0, 0).map(|value| value.assume_utc());
    midnight.map(|value| text.date(value)).unwrap_or_else(|_| rules::catalog().as_of_text())
}

fn summary(
    sheet: &mut Worksheet,
    assessment: &Assessment,
    context: &ReportContext<'_>,
    text: &Text,
    styles: &Styles,
) -> Result<()> {
    sheet.set_name(sheet_name(&text.t("report.sheet.summary")))?;
    sheet.set_column_width(0, 42)?;
    sheet.set_column_width(1, 90)?;
    let summary = &assessment.summary;
    let product = &vbs_config::product().name;
    let mut rows: Vec<(String, String)> = Vec::new();
    if let Some(licensee) = context.licensee() {
        rows.push((text.t("report.label.licensee"), licensee.to_owned()));
    }
    if let Some(customer) = context.customer() {
        rows.push((text.t("report.label.customer"), customer.to_owned()));
    }
    rows.push((text.t("report.label.created"), text.date_time(context.created)));
    rows.push((
        text.t("report.label.createdWith"),
        text.args("report.generatedWith", &[("version", &context.version), ("date", &catalog_date(text))]),
    ));
    rows.push((text.t("report.label.edition"), text.t(&format!("edition.{}", context.edition.kind()))));
    rows.push((String::new(), String::new()));
    rows.push((text.t("report.figure.machines"), text.count_of(summary.machines)));
    rows.push((text.t("report.figure.items"), text.count_of(summary.items)));
    rows.push((text.t("report.figure.high"), text.count_of(summary.high)));
    rows.push((text.t("report.figure.medium"), text.count_of(summary.medium)));
    rows.push((text.t("report.figure.low"), text.count_of(summary.low)));
    rows.push((text.t("report.figure.notCheckable"), text.count_of(summary.not_checkable)));
    rows.push((text.t("report.figure.credentials"), text.count_of(summary.credentials)));
    rows.push((text.t("report.figure.windows"), text.count_of(summary.windows_items)));
    if context.effort() {
        rows.push((
            text.t("report.figure.effort"),
            format!(
                "{} ({})",
                text.hour_range(summary.effort_min, summary.effort_max),
                text.args(
                    "report.personDays",
                    &[("min", &text.days(summary.effort_min)), ("max", &text.days(summary.effort_max))]
                )
            ),
        ));
    }
    if !assessment.set_aside.is_empty() {
        rows.push((text.t("report.figure.setAside"), text.count_of(assessment.set_aside.len())));
    }

    sheet.write_string_with_format(0, 0, cell(&text.t("report.technicalTitle")), &styles.title)?;
    sheet.write_string_with_format(1, 0, cell(product), &styles.label)?;
    let mut row = 3;
    for (label, value) in rows {
        if !label.is_empty() {
            sheet.write_string_with_format(row, 0, cell(&label), &styles.label)?;
            sheet.write_string_with_format(row, 1, cell(&value), &styles.wrap)?;
        }
        row += 1;
    }
    row += 1;
    let mut notes = vec![text.t("report.coverage.intro"), text.t("report.method.risk")];
    if context.effort() {
        notes.push(text.t("report.effort.intro"));
    }
    if context.edition.is_free() {
        let max = context.edition.machine_limit().unwrap_or_default();
        notes.push(text.args("report.freeNotice", &[("max", &max)]));
    }
    for note in notes {
        sheet.merge_range(row, 0, row, 1, &cell(&note), &styles.note)?;
        sheet.set_row_height(row, 48)?;
        row += 1;
    }
    sheet.set_active(true);
    Ok(())
}

fn machine_names(assessment: &Assessment, item: &Item, text: &Text) -> String {
    let mut names: Vec<&str> = Vec::new();
    for occurrence in &item.occurrences {
        let name = assessment.machines[occurrence.machine].machine.hostname.as_str();
        if !names.contains(&name) {
            names.push(name);
        }
    }
    let shown = names.len().min(MAX_MACHINE_NAMES);
    let mut joined = names[..shown].join(", ");
    if names.len() > shown {
        joined.push_str(&format!(" {}", text.count("report.moreMachines", names.len() - shown, &[])));
    }
    joined
}

fn numbers(numbers: &[usize]) -> String {
    numbers.iter().map(|number| format!("#{number}")).collect::<Vec<_>>().join(", ")
}

fn evidence(item: &Item) -> String {
    item.evidence
        .iter()
        .map(|line| match line.line {
            Some(number) => format!("{number}: {}", line.text),
            None => line.text.clone(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The effort note of an item: what the rule-of-thumb value includes or why it is not counted.
/// `all` adds that items on several machines are counted once (the PDF says so once, in its
/// assumptions).
pub(crate) fn effort_note(item: &Item, text: &Text, all: bool) -> String {
    match item.effort {
        Effort::SameAs(number) => text.args("effort.sameAs", &[("number", &number)]),
        Effort::Windows => text.t("effort.windows"),
        Effort::Hours(estimate) => {
            let mut notes = Vec::new();
            if estimate.size_factor > 1 {
                notes.push(text.args("effort.sizeFactor", &[("factor", &estimate.size_factor)]));
            }
            if estimate.typical_script {
                notes.push(text.t("effort.typicalScript"));
            }
            if all && item.machines > 1 {
                notes.push(text.t("effort.countedOnce"));
            }
            notes.join(" ")
        }
    }
}

fn findings(
    sheet: &mut Worksheet,
    assessment: &Assessment,
    context: &ReportContext<'_>,
    text: &Text,
    styles: &Styles,
) -> Result<()> {
    let mut columns: Vec<(String, f64)> = [
        (key("report.column.number"), 6.0),
        (key("report.column.risk"), 12.0),
        (key("report.column.classification"), 12.0),
        (key("report.column.status"), 18.0),
        (key("report.column.rule"), 10.0),
        (key("report.column.finding"), 40.0),
        (key("report.column.kind"), 22.0),
        (key("report.column.activation"), 22.0),
        (key("report.column.origin"), 16.0),
        (key("report.column.machines"), 10.0),
        (key("report.column.machineNames"), 28.0),
        (key("report.column.location"), 60.0),
        (key("report.column.item"), 24.0),
        (key("report.column.target"), 40.0),
        (key("report.column.startedBy"), 14.0),
        (key("report.column.starts"), 14.0),
        (key("report.column.evidence"), 60.0),
        (key("report.column.size"), 12.0),
        (key("report.column.modified"), 17.0),
        (key("report.column.sha256"), 20.0),
        (key("report.column.sameContent"), 12.0),
    ]
    .iter()
    .map(|(name, width)| (text.t(name), *width))
    .collect();
    if context.hints() {
        columns.push((text.t("report.column.hint"), 70.0));
    }
    if context.effort() {
        columns.push((text.t("report.column.effortMin"), 14.0));
        columns.push((text.t("report.column.effortMax"), 14.0));
        columns.push((text.t("report.column.effortNote"), 40.0));
    }
    let mut table = Sheet::new(sheet, &text.t("report.sheet.findings"), &columns, styles)?;
    let mut written = 0;
    for item in &assessment.items {
        if !table.next() {
            break;
        }
        written += 1;
        let first = item.first();
        table.number(0, item.number as f64)?;
        table.styled(1, &text.risk(item.risk), styles.risk(item.risk))?;
        table.text(2, &text.classification(&item.classification))?;
        table.text(3, &text.status(&item.status, item.reason.as_ref()))?;
        table.text(4, &item.rule)?;
        table.text(5, &text.rule_title(&item.rule))?;
        table.text(6, &text.kind(&item.kind))?;
        table.text(7, &text.activation(&item.activation))?;
        table.text(8, &text.origin(item.origin))?;
        table.number(9, item.machines as f64)?;
        table.text(10, &machine_names(assessment, item, text))?;
        table.text(11, &first.location.path)?;
        table.text(12, first.location.item.as_deref().unwrap_or_default())?;
        table.text(13, first.target.as_deref().unwrap_or_default())?;
        table.text(14, &numbers(&item.started_by))?;
        table.text(15, &numbers(&item.starts))?;
        table.styled(16, &evidence(item), &styles.wrap)?;
        if let Some(file) = &first.file {
            table.number(17, file.size as f64)?;
            table.date(18, file.modified_at, &styles.date)?;
            table.text(19, file.sha256.as_deref().unwrap_or_default())?;
        }
        if let Some(number) = item.same_content_as {
            table.text(20, &format!("#{number}"))?;
        }
        let mut column = 21;
        if context.hints() {
            table.styled(column, &text.hint(item), &styles.wrap)?;
            column += 1;
        }
        if context.effort() {
            if let Some((min, max)) = item.effort.hours() {
                table.formatted_number(column, min, &styles.hours)?;
                table.formatted_number(column + 1, max, &styles.hours)?;
            }
            table.text(column + 2, &effort_note(item, text, true))?;
        }
    }
    table.truncated(assessment.items.len() - written, text, styles)
}

fn locations(sheet: &mut Worksheet, assessment: &Assessment, text: &Text, styles: &Styles) -> Result<()> {
    let columns: Vec<(String, f64)> = [
        (key("report.column.number"), 6.0),
        (key("report.column.machine"), 20.0),
        (key("report.column.locationType"), 16.0),
        (key("report.column.location"), 70.0),
        (key("report.column.item"), 24.0),
        (key("report.column.target"), 40.0),
        (key("report.column.activationReported"), 22.0),
        (key("report.column.size"), 12.0),
        (key("report.column.modified"), 17.0),
        (key("report.column.sha256"), 20.0),
        (key("report.column.findingId"), 10.0),
    ]
    .iter()
    .map(|(name, width)| (text.t(name), *width))
    .collect();
    let mut table = Sheet::new(sheet, &text.t("report.sheet.locations"), &columns, styles)?;
    let total: usize = assessment.items.iter().map(|item| item.occurrences.len()).sum();
    let mut written = 0;
    'items: for item in &assessment.items {
        for occurrence in &item.occurrences {
            if !table.next() {
                break 'items;
            }
            written += 1;
            table.number(0, item.number as f64)?;
            table.text(1, &assessment.machines[occurrence.machine].machine.hostname)?;
            table.text(2, &text.value("locationKind", occurrence.location.kind.as_str()))?;
            table.text(3, &occurrence.location.path)?;
            table.text(4, occurrence.location.item.as_deref().unwrap_or_default())?;
            table.text(5, occurrence.target.as_deref().unwrap_or_default())?;
            table.text(6, &text.activation(&occurrence.activation))?;
            if let Some(file) = &occurrence.file {
                table.number(7, file.size as f64)?;
                table.date(8, file.modified_at, &styles.date)?;
                table.text(9, file.sha256.as_deref().unwrap_or_default())?;
            }
            table.text(10, &occurrence.finding)?;
        }
    }
    table.truncated(total - written, text, styles)
}

fn machines(sheet: &mut Worksheet, assessment: &Assessment, text: &Text, styles: &Styles) -> Result<()> {
    let columns: Vec<(String, f64)> = [
        (key("report.column.machine"), 20.0),
        (key("report.column.domain"), 22.0),
        (key("report.column.os"), 30.0),
        (key("report.column.osVersion"), 18.0),
        (key("report.column.role"), 16.0),
        (key("report.column.architecture"), 10.0),
        (key("report.column.scanned"), 17.0),
        (key("report.column.duration"), 12.0),
        (key("report.column.collector"), 12.0),
        (key("report.column.rulesAsOf"), 12.0),
        (key("report.column.coverage"), 12.0),
        (key("report.column.elevated"), 14.0),
        (key("report.column.limitations"), 50.0),
        (key("report.column.findings"), 10.0),
        (key("report.column.notCheckable"), 12.0),
        (key("report.column.resultFile"), 50.0),
    ]
    .iter()
    .map(|(name, width)| (text.t(name), *width))
    .collect();
    let mut table = Sheet::new(sheet, &text.t("report.sheet.machines"), &columns, styles)?;
    for info in &assessment.machines {
        if !table.next() {
            break;
        }
        let machine = &info.machine;
        let os = &machine.os;
        table.text(0, &machine.hostname)?;
        table.text(1, machine.domain.as_deref().unwrap_or_default())?;
        let name = match (&os.name, &os.display_version) {
            (Some(name), Some(version)) => format!("{name} {version}"),
            (Some(name), None) => name.clone(),
            (None, _) => os.family.clone(),
        };
        table.text(2, &name)?;
        table.text(3, os.version.as_deref().unwrap_or_default())?;
        if let Some(role) = &os.product_type {
            table.text(4, &text.value("productType", role.as_str()))?;
        }
        table.text(5, os.architecture.as_deref().unwrap_or_default())?;
        table.date(6, Some(info.started_at), &styles.date)?;
        let minutes = (info.finished_at - info.started_at).whole_seconds().max(0) as f64 / 60.0;
        table.formatted_number(7, (minutes * 10.0).round() / 10.0, &styles.hours)?;
        table.text(8, &info.generator.version)?;
        table.text(9, &info.generator.rules_as_of)?;
        table.text(10, &text.value("coverage", info.coverage.mode.as_str()))?;
        table.text(11, &text.yes_no(info.coverage.elevated))?;
        let limitations: Vec<String> =
            info.coverage.limitations.iter().map(|limitation| text.limitation(limitation.code.as_str())).collect();
        table.styled(12, &limitations.join("\n"), &styles.wrap)?;
        table.number(13, info.detected as f64)?;
        table.number(14, info.not_checkable as f64)?;
        table.text(15, &info.file.display().to_string())?;
    }
    Ok(())
}

fn coverage(sheet: &mut Worksheet, assessment: &Assessment, text: &Text, styles: &Styles) -> Result<()> {
    let columns: Vec<(String, f64)> = [
        (key("report.column.machine"), 20.0),
        (key("report.column.source"), 28.0),
        (key("report.column.sourceStatus"), 16.0),
        (key("report.column.sourceReason"), 30.0),
        (key("report.column.roots"), 30.0),
        (key("report.column.entries"), 12.0),
        (key("report.column.inspected"), 12.0),
        (key("report.column.errors"), 12.0),
        (key("report.column.skipped"), 12.0),
        (key("report.column.from"), 17.0),
        (key("report.column.to"), 17.0),
        (key("report.column.days"), 8.0),
        (key("report.column.errorSamples"), 60.0),
    ]
    .iter()
    .map(|(name, width)| (text.t(name), *width))
    .collect();
    let mut table = Sheet::new(sheet, &text.t("report.sheet.coverage"), &columns, styles)?;
    for info in &assessment.machines {
        for source in &info.coverage.sources {
            if !table.next() {
                return Ok(());
            }
            table.text(0, &info.machine.hostname)?;
            table.text(1, &text.source(&source.source))?;
            table.text(2, &text.source_status(&source.status))?;
            if let Some(reason) = &source.reason {
                table.text(3, &text.source_reason(reason))?;
            }
            table.styled(4, &source.roots.join("\n"), &styles.wrap)?;
            table.number(5, source.entries as f64)?;
            table.number(6, source.inspected as f64)?;
            table.number(7, source.errors as f64)?;
            table.number(8, source.skipped as f64)?;
            if let Some(range) = source.time_range {
                table.date(9, Some(range.from), &styles.date)?;
                table.date(10, Some(range.to), &styles.date)?;
                let days = (range.to - range.from).whole_seconds().max(0) as f64 / 86_400.0;
                table.formatted_number(11, (days * 10.0).round() / 10.0, &styles.hours)?;
            }
            table.styled(12, &source.error_samples.join("\n"), &styles.wrap)?;
        }
    }
    Ok(())
}

fn rule_sheet(sheet: &mut Worksheet, context: &ReportContext<'_>, text: &Text, styles: &Styles) -> Result<()> {
    let mut columns: Vec<(String, f64)> = [
        (key("report.column.rule"), 10.0),
        (key("report.column.finding"), 40.0),
        (key("report.column.kind"), 22.0),
        (key("report.column.classification"), 12.0),
        (key("report.column.rationale"), 80.0),
        (key("report.column.sources"), 80.0),
    ]
    .iter()
    .map(|(name, width)| (text.t(name), *width))
    .collect();
    if context.hints() {
        columns.push((text.t("report.column.hint"), 70.0));
    }
    if context.effort() {
        columns.push((text.t("report.column.effortRule"), 18.0));
        columns.push((text.t("report.column.effortBasis"), 40.0));
    }
    let mut table = Sheet::new(sheet, &text.t("report.sheet.rules"), &columns, styles)?;
    let catalog = rules::catalog();
    for rule in &catalog.rules {
        table.next();
        table.text(0, &rule.id)?;
        table.text(1, &text.rule_title(&rule.id))?;
        table.text(2, &text.kind(&rule.kind))?;
        table.text(3, &text.classification(&rule.classification))?;
        table.styled(4, &text.rule_rationale(&rule.id).unwrap_or_default(), &styles.wrap)?;
        let sources: Vec<String> = rule
            .sources
            .iter()
            .filter_map(|id| catalog.source(id))
            .map(|source| {
                let checked =
                    source.checked.with_hms(0, 0, 0).map(|value| text.date(value.assume_utc())).unwrap_or_default();
                format!(
                    "{} – {} – {} ({})",
                    source.publisher,
                    source.title,
                    source.url,
                    text.args("report.checked", &[("date", &checked)])
                )
            })
            .collect();
        table.styled(5, &sources.join("\n"), &styles.wrap)?;
        let mut column = 6;
        if context.hints() {
            table.styled(column, &text.t(&rule.hint_key()), &styles.wrap)?;
            column += 1;
        }
        if context.effort() {
            table.text(column, &text.hour_range(rule.effort.min_hours, rule.effort.max_hours))?;
            let basis = match rule.effort.basis {
                EffortBasis::Fixed => key("effort.basis.fixed"),
                EffortBasis::ScriptSize => key("effort.basis.scriptSize"),
                EffortBasis::Entry => key("effort.basis.entry"),
            };
            table.styled(column + 1, &text.t(basis), &styles.wrap)?;
        }
    }
    Ok(())
}

fn set_aside(sheet: &mut Worksheet, assessment: &Assessment, text: &Text, styles: &Styles) -> Result<()> {
    let columns: Vec<(String, f64)> = [
        (key("report.column.machine"), 20.0),
        (key("report.column.scanned"), 17.0),
        (key("report.column.setAsideReason"), 50.0),
        (key("report.column.resultFile"), 70.0),
    ]
    .iter()
    .map(|(name, width)| (text.t(name), *width))
    .collect();
    let mut table = Sheet::new(sheet, &text.t("report.sheet.setAside"), &columns, styles)?;
    for entry in &assessment.set_aside {
        table.next();
        table.text(0, &entry.hostname)?;
        table.date(1, Some(entry.started_at), &styles.date)?;
        table.text(2, &text.t(&format!("setAside.{}", entry.why.as_str())))?;
        table.text(3, &entry.file.display().to_string())?;
    }
    Ok(())
}
