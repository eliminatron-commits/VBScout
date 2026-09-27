//! A small page layout engine for the management report: paragraphs with line breaks, headings
//! with bookmarks, lists, tables that continue on the next page, key-figure tiles and bars.
//!
//! Text is measured with the shaping engine krilla draws it with (rustybuzz), so lines never run
//! over the margin. The font is Liberation Sans (SIL Open Font License 1.1, `assets/fonts/`),
//! embedded as a subset – reports look the same on every machine and need nothing installed.
//! Layout happens first (pages of drawing operations), rendering afterwards, so every page can
//! say "page 3 of 12".

use krilla::Document;
use krilla::color::rgb;
use krilla::destination::XyzDestination;
use krilla::geom::{PathBuilder, Point, Rect, Size, Transform};
use krilla::image::Image;
use krilla::metadata::Metadata;
use krilla::num::NormalizedF32;
use krilla::outline::{Outline, OutlineNode};
use krilla::page::PageSettings;
use krilla::paint::{Fill, FillRule, Stroke};
use krilla::text::{Font, TextDirection};
use rustybuzz::{Face, UnicodeBuffer};

/// A4 in points.
pub const PAGE_WIDTH: f32 = 595.28;
pub const PAGE_HEIGHT: f32 = 841.89;
pub const MARGIN_X: f32 = 56.0;
pub const MARGIN_TOP: f32 = 64.0;
pub const MARGIN_BOTTOM: f32 = 62.0;
pub const CONTENT_WIDTH: f32 = PAGE_WIDTH - 2.0 * MARGIN_X;

/// Line height relative to the font size.
const LEADING: f32 = 1.32;
/// Most lines one table cell shows (a longer text ends with "…").
const MAX_CELL_LINES: usize = 14;

static REGULAR: &[u8] = include_bytes!("../../../assets/fonts/LiberationSans-Regular.ttf");
static BOLD: &[u8] = include_bytes!("../../../assets/fonts/LiberationSans-Bold.ttf");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

pub const TEXT: Rgb = Rgb(0x16, 0x18, 0x1D);
pub const MUTED: Rgb = Rgb(0x4F, 0x56, 0x64);
pub const ACCENT: Rgb = Rgb(0x1D, 0x4E, 0xD8);
pub const RULE: Rgb = Rgb(0xD5, 0xD9, 0xE0);
pub const SHADE: Rgb = Rgb(0xF3, 0xF5, 0xF8);
pub const WHITE: Rgb = Rgb(0xFF, 0xFF, 0xFF);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weight {
    Regular,
    Bold,
}

/// The embedded fonts, ready for drawing (krilla) and measuring (rustybuzz).
pub struct Fonts {
    regular: Font,
    bold: Font,
    regular_face: Face<'static>,
    bold_face: Face<'static>,
}

impl Fonts {
    pub fn load() -> Result<Fonts, String> {
        let font = |data: &'static [u8]| Font::new(data.to_vec().into(), 0).ok_or("the embedded font cannot be read");
        let face = |data: &'static [u8]| Face::from_slice(data, 0).ok_or("the embedded font cannot be read");
        Ok(Fonts { regular: font(REGULAR)?, bold: font(BOLD)?, regular_face: face(REGULAR)?, bold_face: face(BOLD)? })
    }

    fn font(&self, weight: Weight) -> &Font {
        match weight {
            Weight::Regular => &self.regular,
            Weight::Bold => &self.bold,
        }
    }

    /// Width of `text` in points.
    pub fn width(&self, text: &str, weight: Weight, size: f32) -> f32 {
        if text.is_empty() {
            return 0.0;
        }
        let face = match weight {
            Weight::Regular => &self.regular_face,
            Weight::Bold => &self.bold_face,
        };
        let mut buffer = UnicodeBuffer::new();
        buffer.push_str(text);
        buffer.guess_segment_properties();
        let output = rustybuzz::shape(face, &[], buffer);
        let units: i32 = output.glyph_positions().iter().map(|position| position.x_advance).sum();
        units as f32 * size / face.units_per_em() as f32
    }
}

/// How a piece of text looks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    pub size: f32,
    pub weight: Weight,
    pub color: Rgb,
}

impl Style {
    pub const fn new(size: f32, weight: Weight, color: Rgb) -> Self {
        Self { size, weight, color }
    }

    pub fn line_height(&self) -> f32 {
        self.size * LEADING
    }
}

const BAR_LABEL: Style = Style::new(8.5, Weight::Regular, TEXT);
const BAR_LABEL_SHARE: f32 = 0.34;
const BAR_HEIGHT: f32 = 11.0;

pub const BODY: Style = Style::new(9.5, Weight::Regular, TEXT);
pub const SMALL: Style = Style::new(8.0, Weight::Regular, MUTED);
pub const STRONG: Style = Style::new(9.5, Weight::Bold, TEXT);

/// One drawing operation; `y` of text is its baseline, everything else is top-left based.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Text { x: f32, y: f32, text: String, style: Style },
    Rect { x: f32, y: f32, width: f32, height: f32, color: Rgb },
    Line { x1: f32, y1: f32, x2: f32, y2: f32, width: f32, color: Rgb },
    Image { x: f32, y: f32, width: f32, height: f32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
}

/// A table column: title, share of the content width and alignment.
#[derive(Debug, Clone)]
pub struct Column {
    pub title: String,
    pub share: f32,
    pub align: Align,
}

impl Column {
    pub fn new(title: impl Into<String>, share: f32, align: Align) -> Self {
        Self { title: title.into(), share, align }
    }
}

/// A table cell: main text, an optional smaller second text below it, style and background.
#[derive(Debug, Clone, Default)]
pub struct Cell {
    pub text: String,
    pub detail: Option<String>,
    pub style: Option<Style>,
    pub fill: Option<Rgb>,
}

impl Cell {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into(), ..Cell::default() }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        let detail = detail.into();
        if !detail.is_empty() {
            self.detail = Some(detail);
        }
        self
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = Some(style);
        self
    }

    pub fn fill(mut self, fill: Rgb) -> Self {
        self.fill = Some(fill);
        self
    }
}

/// A key figure tile.
#[derive(Debug, Clone)]
pub struct Figure {
    pub value: String,
    pub label: String,
    pub color: Rgb,
}

/// A bookmark in the document outline.
#[derive(Debug, Clone)]
struct Mark {
    title: String,
    page: usize,
    y: f32,
}

/// Lays out content onto pages.
pub struct Layout<'f> {
    fonts: &'f Fonts,
    pages: Vec<Vec<Op>>,
    y: f32,
    marks: Vec<Mark>,
}

impl<'f> Layout<'f> {
    pub fn new(fonts: &'f Fonts) -> Self {
        Self { fonts, pages: vec![Vec::new()], y: MARGIN_TOP, marks: Vec::new() }
    }

    pub fn fonts(&self) -> &Fonts {
        self.fonts
    }

    fn bottom() -> f32 {
        PAGE_HEIGHT - MARGIN_BOTTOM
    }

    pub fn y(&self) -> f32 {
        self.y
    }

    pub fn set_y(&mut self, y: f32) {
        self.y = y;
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn new_page(&mut self) {
        self.pages.push(Vec::new());
        self.y = MARGIN_TOP;
    }

    /// Starts a new page unless `height` still fits on this one.
    pub fn ensure(&mut self, height: f32) {
        if self.y + height > Self::bottom() && self.y > MARGIN_TOP {
            self.new_page();
        }
    }

    pub fn space(&mut self, height: f32) {
        self.y += height;
    }

    pub fn push(&mut self, op: Op) {
        if let Some(page) = self.pages.last_mut() {
            page.push(op);
        }
    }

    /// Breaks `text` into lines no wider than `width`; `\n` starts a new line. Words that are
    /// too long (paths) break after `\`, `/`, `.`, `-` or `_`, or anywhere if they must.
    pub fn wrap(&self, text: &str, style: &Style, width: f32) -> Vec<String> {
        let space = self.fonts.width(" ", style.weight, style.size);
        let mut lines = Vec::new();
        for paragraph in text.split('\n') {
            let mut line = String::new();
            let mut line_width = 0.0;
            for word in paragraph.split_whitespace() {
                let word_width = self.fonts.width(word, style.weight, style.size);
                if line.is_empty() && word_width <= width {
                    line.push_str(word);
                    line_width = word_width;
                } else if !line.is_empty() && line_width + space + word_width <= width {
                    line.push(' ');
                    line.push_str(word);
                    line_width += space + word_width;
                } else if word_width <= width {
                    lines.push(std::mem::take(&mut line));
                    line.push_str(word);
                    line_width = word_width;
                } else {
                    if !line.is_empty() {
                        lines.push(std::mem::take(&mut line));
                    }
                    let mut pieces = self.break_word(word, style, width);
                    let last = pieces.pop().unwrap_or_default();
                    lines.extend(pieces);
                    line_width = self.fonts.width(&last, style.weight, style.size);
                    line = last;
                }
            }
            lines.push(line);
        }
        lines
    }

    fn break_word(&self, word: &str, style: &Style, width: f32) -> Vec<String> {
        let mut pieces = Vec::new();
        let mut rest: &str = word;
        while !rest.is_empty() {
            if self.fonts.width(rest, style.weight, style.size) <= width {
                pieces.push(rest.to_owned());
                break;
            }
            // The longest prefix that fits, preferably ending after a separator.
            let mut fit = 0;
            let mut separator = 0;
            for (index, character) in rest.char_indices() {
                let end = index + character.len_utf8();
                if self.fonts.width(&rest[..end], style.weight, style.size) > width {
                    break;
                }
                fit = end;
                if matches!(character, '\\' | '/' | '.' | '-' | '_' | ',' | ';') {
                    separator = end;
                }
            }
            let cut = if separator > 0 { separator } else { fit.max(rest.chars().next().map_or(1, char::len_utf8)) };
            pieces.push(rest[..cut].to_owned());
            rest = &rest[cut..];
        }
        pieces
    }

    /// Draws lines of text at `x` from the current position; long texts continue on the next page.
    pub fn lines_at(&mut self, x: f32, lines: &[String], style: &Style, align: Align, width: f32) {
        for line in lines {
            self.ensure(style.line_height());
            let offset = match align {
                Align::Left => 0.0,
                Align::Right => width - self.fonts.width(line, style.weight, style.size),
            };
            if !line.is_empty() {
                let baseline = self.y + style.size;
                self.push(Op::Text { x: x + offset, y: baseline, text: line.clone(), style: *style });
            }
            self.y += style.line_height();
        }
    }

    pub fn paragraph(&mut self, text: &str, style: &Style) {
        let lines = self.wrap(text, style, CONTENT_WIDTH);
        self.lines_at(MARGIN_X, &lines, style, Align::Left, CONTENT_WIDTH);
        self.y += style.size * 0.55;
    }

    /// A section heading with a bookmark; kept together with at least a few lines of what follows.
    pub fn heading(&mut self, text: &str, level: u8) {
        let style = match level {
            1 => Style::new(16.0, Weight::Bold, ACCENT),
            _ => Style::new(11.5, Weight::Bold, TEXT),
        };
        self.ensure(style.line_height() + 8.0 * BODY.line_height());
        if self.y > MARGIN_TOP {
            self.y += if level == 1 { 14.0 } else { 8.0 };
        }
        if level == 1 {
            self.marks.push(Mark { title: text.to_owned(), page: self.pages.len() - 1, y: self.y });
        }
        let lines = self.wrap(text, &style, CONTENT_WIDTH);
        self.lines_at(MARGIN_X, &lines, &style, Align::Left, CONTENT_WIDTH);
        if level == 1 {
            self.push(Op::Line {
                x1: MARGIN_X,
                y1: self.y + 1.0,
                x2: MARGIN_X + CONTENT_WIDTH,
                y2: self.y + 1.0,
                width: 0.8,
                color: RULE,
            });
            self.y += 8.0;
        } else {
            self.y += 2.0;
        }
    }

    pub fn bullets(&mut self, items: &[String], style: &Style) {
        let indent = 12.0;
        for item in items {
            let lines = self.wrap(item, style, CONTENT_WIDTH - indent);
            self.ensure(style.line_height());
            let baseline = self.y + style.size;
            self.push(Op::Text { x: MARGIN_X + 2.0, y: baseline, text: "•".into(), style: *style });
            self.lines_at(MARGIN_X + indent, &lines, style, Align::Left, CONTENT_WIDTH - indent);
            self.y += style.size * 0.35;
        }
        self.y += style.size * 0.3;
    }

    /// A shaded box with text inside (notes such as the timeline statement).
    pub fn note(&mut self, text: &str, style: &Style, background: Rgb) {
        let padding = 8.0;
        let lines = self.wrap(text, style, CONTENT_WIDTH - 2.0 * padding);
        let height = lines.len() as f32 * style.line_height() + 2.0 * padding;
        self.ensure(height.min(Self::bottom() - MARGIN_TOP));
        self.push(Op::Rect { x: MARGIN_X, y: self.y, width: CONTENT_WIDTH, height, color: background });
        self.push(Op::Rect { x: MARGIN_X, y: self.y, width: 3.0, height, color: ACCENT });
        self.y += padding;
        self.lines_at(MARGIN_X + padding + 2.0, &lines, style, Align::Left, CONTENT_WIDTH - 2.0 * padding);
        self.y += padding + 6.0;
    }

    /// Key figure tiles, `per_row` in a row.
    pub fn figures(&mut self, figures: &[Figure], per_row: usize) {
        let gap = 8.0;
        let per_row = per_row.max(1);
        let width = (CONTENT_WIDTH - gap * (per_row as f32 - 1.0)) / per_row as f32;
        let value_style = Style::new(19.0, Weight::Bold, TEXT);
        let label_style = Style::new(8.0, Weight::Regular, MUTED);
        for row in figures.chunks(per_row) {
            let labels: Vec<Vec<String>> =
                row.iter().map(|figure| self.wrap(&figure.label, &label_style, width - 16.0)).collect();
            let label_lines = labels.iter().map(Vec::len).max().unwrap_or(1);
            let height = 14.0 + value_style.line_height() + label_lines as f32 * label_style.line_height() + 8.0;
            self.ensure(height);
            let top = self.y;
            for (index, (figure, label)) in row.iter().zip(&labels).enumerate() {
                let x = MARGIN_X + index as f32 * (width + gap);
                self.push(Op::Rect { x, y: top, width, height, color: SHADE });
                self.push(Op::Rect { x, y: top, width, height: 3.0, color: figure.color });
                let value = self.fit(&figure.value, &value_style, width - 16.0);
                self.push(Op::Text {
                    x: x + 8.0,
                    y: top + 10.0 + value.size,
                    text: figure.value.clone(),
                    style: Style { color: figure.color, ..value },
                });
                let mut y = top + 12.0 + value_style.line_height();
                for line in label {
                    self.push(Op::Text { x: x + 8.0, y: y + label_style.size, text: line.clone(), style: label_style });
                    y += label_style.line_height();
                }
            }
            self.y = top + height + gap;
        }
        self.y += 4.0;
    }

    /// A smaller font size if `text` does not fit into `width` on one line.
    fn fit(&self, text: &str, style: &Style, width: f32) -> Style {
        let mut fitted = *style;
        while fitted.size > 7.0 && self.fonts.width(text, fitted.weight, fitted.size) > width {
            fitted.size -= 1.0;
        }
        fitted
    }

    /// Height of a paragraph of `text`.
    pub fn paragraph_height(&self, text: &str, style: &Style) -> f32 {
        self.wrap(text, style, CONTENT_WIDTH).len() as f32 * style.line_height() + style.size * 0.55
    }

    /// Height of a bar chart with its legend.
    pub fn bars_height(&self, rows: &[(String, Vec<(f32, Rgb)>)]) -> f32 {
        let label_width = CONTENT_WIDTH * BAR_LABEL_SHARE;
        let rows: f32 = rows
            .iter()
            .map(|(label, _)| {
                (self.wrap(label, &BAR_LABEL, label_width).len() as f32 * BAR_LABEL.line_height()).max(BAR_HEIGHT) + 5.0
            })
            .sum();
        rows + BAR_LABEL.line_height() + 12.0
    }

    /// Horizontal stacked bars, one per row, scaled to the largest total; with a legend.
    pub fn bars(&mut self, rows: &[(String, Vec<(f32, Rgb)>)], legend: &[(String, Rgb)]) {
        let label_width = CONTENT_WIDTH * BAR_LABEL_SHARE;
        let bar_x = MARGIN_X + label_width + 8.0;
        let bar_width = CONTENT_WIDTH - label_width - 48.0;
        let bar_height = BAR_HEIGHT;
        let max = rows.iter().map(|(_, parts)| parts.iter().map(|(value, _)| value).sum::<f32>()).fold(0.0, f32::max);
        let style = BAR_LABEL;
        // The chart stays on one page if it fits on one.
        let total = self.bars_height(rows);
        if total <= Self::bottom() - MARGIN_TOP {
            self.ensure(total);
        }
        for (label, parts) in rows {
            let lines = self.wrap(label, &style, label_width);
            let height = (lines.len() as f32 * style.line_height()).max(bar_height) + 5.0;
            self.ensure(height);
            let top = self.y;
            let mut y = top;
            for line in &lines {
                self.push(Op::Text { x: MARGIN_X, y: y + style.size, text: line.clone(), style });
                y += style.line_height();
            }
            let mut x = bar_x;
            let total: f32 = parts.iter().map(|(value, _)| value).sum();
            for (value, color) in parts {
                if *value <= 0.0 || max <= 0.0 {
                    continue;
                }
                let width = bar_width * value / max;
                self.push(Op::Rect { x, y: top + 1.0, width, height: bar_height, color: *color });
                x += width;
            }
            self.push(Op::Text {
                x: x + 4.0,
                y: top + 1.0 + style.size,
                text: format!("{}", total.round() as u64),
                style: Style { color: MUTED, ..style },
            });
            self.y = top + height;
        }
        self.y += 4.0;
        let mut x = MARGIN_X;
        self.ensure(style.line_height() + 4.0);
        for (label, color) in legend {
            self.push(Op::Rect { x, y: self.y + 1.5, width: 8.0, height: 8.0, color: *color });
            self.push(Op::Text { x: x + 12.0, y: self.y + style.size, text: label.clone(), style });
            x += 12.0 + self.fonts.width(label, style.weight, style.size) + 16.0;
        }
        self.y += style.line_height() + 8.0;
    }

    /// A table with a header row that repeats on every page it continues on.
    pub fn table(&mut self, columns: &[Column], rows: &[Vec<Cell>], font_size: f32) {
        let total: f32 = columns.iter().map(|column| column.share).sum();
        let widths: Vec<f32> = columns.iter().map(|column| CONTENT_WIDTH * column.share / total).collect();
        let padding = 4.0;
        let body = Style::new(font_size, Weight::Regular, TEXT);
        let detail = Style::new(font_size - 1.0, Weight::Regular, MUTED);
        let header = Style::new(font_size, Weight::Bold, WHITE);

        let header_lines: Vec<Vec<String>> = columns
            .iter()
            .zip(&widths)
            .map(|(column, width)| self.wrap(&column.title, &header, width - 2.0 * padding))
            .collect();
        let header_height =
            header_lines.iter().map(Vec::len).max().unwrap_or(1) as f32 * header.line_height() + 2.0 * padding;
        let draw_header = |layout: &mut Layout<'_>| {
            let top = layout.y;
            layout.push(Op::Rect { x: MARGIN_X, y: top, width: CONTENT_WIDTH, height: header_height, color: ACCENT });
            let mut x = MARGIN_X;
            for ((column, lines), width) in columns.iter().zip(&header_lines).zip(&widths) {
                let mut y = top + padding;
                for line in lines {
                    let offset = match column.align {
                        Align::Left => 0.0,
                        Align::Right => width - 2.0 * padding - layout.fonts.width(line, header.weight, header.size),
                    };
                    layout.push(Op::Text {
                        x: x + padding + offset,
                        y: y + header.size,
                        text: line.clone(),
                        style: header,
                    });
                    y += header.line_height();
                }
                x += width;
            }
            layout.y = top + header_height;
        };

        // The header never stands alone: it starts where the first rows fit, too.
        let first_rows: f32 =
            rows.iter().take(3).map(|row| self.row_height(row, &widths, &body, &detail, padding)).sum();
        self.ensure(header_height + first_rows);
        draw_header(self);
        for (index, row) in rows.iter().enumerate() {
            let cells: Vec<(Vec<String>, Vec<String>, Style)> = row
                .iter()
                .zip(&widths)
                .map(|(cell, width)| {
                    let style = cell.style.unwrap_or(body);
                    let mut lines = self.wrap(&cell.text, &style, width - 2.0 * padding);
                    let mut details = cell
                        .detail
                        .as_deref()
                        .map(|text| self.wrap(text, &detail, width - 2.0 * padding))
                        .unwrap_or_default();
                    truncate_lines(&mut lines, MAX_CELL_LINES);
                    truncate_lines(&mut details, MAX_CELL_LINES / 2);
                    (lines, details, style)
                })
                .collect();
            let height = cells
                .iter()
                .map(|(lines, details, style)| {
                    lines.len() as f32 * style.line_height() + details.len() as f32 * detail.line_height()
                })
                .fold(0.0, f32::max)
                + 2.0 * padding;
            if self.y + height > Self::bottom() {
                self.new_page();
                draw_header(self);
            }
            let top = self.y;
            if index % 2 == 1 {
                self.push(Op::Rect { x: MARGIN_X, y: top, width: CONTENT_WIDTH, height, color: SHADE });
            }
            let mut x = MARGIN_X;
            for (((lines, details, style), cell), (column, width)) in
                cells.iter().zip(row).zip(columns.iter().zip(&widths))
            {
                if let Some(fill) = cell.fill {
                    self.push(Op::Rect { x, y: top, width: *width, height, color: fill });
                }
                let mut y = top + padding;
                for line in lines {
                    let offset = match column.align {
                        Align::Left => 0.0,
                        Align::Right => width - 2.0 * padding - self.fonts.width(line, style.weight, style.size),
                    };
                    if !line.is_empty() {
                        self.push(Op::Text {
                            x: x + padding + offset,
                            y: y + style.size,
                            text: line.clone(),
                            style: *style,
                        });
                    }
                    y += style.line_height();
                }
                for line in details {
                    if !line.is_empty() {
                        self.push(Op::Text { x: x + padding, y: y + detail.size, text: line.clone(), style: detail });
                    }
                    y += detail.line_height();
                }
                x += width;
            }
            self.push(Op::Line {
                x1: MARGIN_X,
                y1: top + height,
                x2: MARGIN_X + CONTENT_WIDTH,
                y2: top + height,
                width: 0.4,
                color: RULE,
            });
            self.y = top + height;
        }
        self.y += 10.0;
    }

    fn row_height(&self, row: &[Cell], widths: &[f32], body: &Style, detail: &Style, padding: f32) -> f32 {
        row.iter()
            .zip(widths)
            .map(|(cell, width)| {
                let style = cell.style.unwrap_or(*body);
                let lines = self.wrap(&cell.text, &style, width - 2.0 * padding).len().min(MAX_CELL_LINES);
                let details = cell
                    .detail
                    .as_deref()
                    .map_or(0, |text| self.wrap(text, detail, width - 2.0 * padding).len().min(MAX_CELL_LINES / 2));
                lines as f32 * style.line_height() + details as f32 * detail.line_height()
            })
            .fold(0.0, f32::max)
            + 2.0 * padding
    }

    /// The laid-out pages (for tests and rendering).
    pub fn pages(&self) -> &[Vec<Op>] {
        &self.pages
    }

    /// Renders all pages; `decorate` adds header and footer to page `index` of `count`.
    pub fn render(
        self,
        metadata: Metadata,
        logo: Option<&Image>,
        decorate: impl Fn(&Layout<'_>, usize, usize) -> Vec<Op>,
    ) -> Result<Vec<u8>, String> {
        let mut document = Document::new();
        let count = self.pages.len();
        for (index, ops) in self.pages.iter().enumerate() {
            let settings = PageSettings::from_wh(PAGE_WIDTH, PAGE_HEIGHT).ok_or("page size")?;
            let mut page = document.start_page_with(settings);
            let mut surface = page.surface();
            for op in ops.iter().chain(decorate(&self, index, count).iter()) {
                match op {
                    Op::Text { x, y, text, style } => {
                        surface.set_stroke(None);
                        surface.set_fill(Some(fill(style.color)));
                        surface.draw_text(
                            Point::from_xy(*x, *y),
                            self.fonts.font(style.weight).clone(),
                            style.size,
                            text,
                            false,
                            TextDirection::Auto,
                        );
                    }
                    Op::Rect { x, y, width, height, color } => {
                        let Some(rect) = Rect::from_xywh(*x, *y, width.max(0.01), height.max(0.01)) else { continue };
                        let mut builder = PathBuilder::new();
                        builder.push_rect(rect);
                        if let Some(path) = builder.finish() {
                            surface.set_stroke(None);
                            surface.set_fill(Some(fill(*color)));
                            surface.draw_path(&path);
                        }
                    }
                    Op::Line { x1, y1, x2, y2, width, color } => {
                        let mut builder = PathBuilder::new();
                        builder.move_to(*x1, *y1);
                        builder.line_to(*x2, *y2);
                        if let Some(path) = builder.finish() {
                            surface.set_fill(None);
                            surface.set_stroke(Some(Stroke {
                                paint: rgb::Color::new(color.0, color.1, color.2).into(),
                                width: *width,
                                ..Stroke::default()
                            }));
                            surface.draw_path(&path);
                        }
                    }
                    Op::Image { x, y, width, height } => {
                        if let (Some(image), Some(size)) = (logo, Size::from_wh(*width, *height)) {
                            surface.push_transform(&Transform::from_translate(*x, *y));
                            surface.draw_image(image.clone(), size);
                            surface.pop();
                        }
                    }
                }
            }
            surface.finish();
            page.finish();
        }
        let mut outline = Outline::new();
        for mark in &self.marks {
            outline.push_child(OutlineNode::new(
                mark.title.clone(),
                XyzDestination::new(mark.page, Point::from_xy(MARGIN_X, (mark.y - 8.0).max(0.0))),
            ));
        }
        document.set_outline(outline);
        document.set_metadata(metadata);
        document.finish().map_err(|error| format!("{error:?}"))
    }
}

fn fill(color: Rgb) -> Fill {
    Fill {
        paint: rgb::Color::new(color.0, color.1, color.2).into(),
        opacity: NormalizedF32::ONE,
        rule: FillRule::NonZero,
    }
}

fn truncate_lines(lines: &mut Vec<String>, max: usize) {
    if lines.len() > max {
        lines.truncate(max);
        if let Some(last) = lines.last_mut() {
            last.push_str(" …");
        }
    }
}

/// All text of the laid-out pages, page by page (tests check completeness and language).
pub fn page_texts(pages: &[Vec<Op>]) -> Vec<String> {
    pages
        .iter()
        .map(|ops| {
            ops.iter()
                .filter_map(|op| match op {
                    Op::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_never_exceed_the_width() {
        let fonts = Fonts::load().unwrap();
        let layout = Layout::new(&fonts);
        let text = "Rewrite the script in PowerShell, the supported successor on Windows. \
                    C:\\ProgramData\\Microsoft\\Windows\\Start Menu\\Programs\\StartUp\\very-long-folder-name\\another_long_folder\\script.vbs \
                    Zażółć gęślą jaźń – Ärger über Öl.";
        for width in [60.0, 150.0, 300.0] {
            let lines = layout.wrap(text, &BODY, width);
            assert!(lines.len() > 1);
            for line in &lines {
                assert!(fonts.width(line, BODY.weight, BODY.size) <= width + 0.01, "{line:?} wider than {width}");
            }
            let joined: String = lines.concat().chars().filter(|c| !c.is_whitespace()).collect();
            let original: String = text.chars().filter(|c| !c.is_whitespace()).collect();
            assert_eq!(joined, original, "nothing is lost");
        }
        assert_eq!(layout.wrap("a\n\nb", &BODY, 100.0), ["a", "", "b"]);
    }

    #[test]
    fn tables_continue_on_the_next_page_with_their_header() {
        let fonts = Fonts::load().unwrap();
        let mut layout = Layout::new(&fonts);
        let columns = vec![Column::new("Number", 1.0, Align::Right), Column::new("Text", 4.0, Align::Left)];
        let rows: Vec<Vec<Cell>> =
            (1..=120).map(|n| vec![Cell::new(n.to_string()), Cell::new(format!("Row {n}")).detail("detail")]).collect();
        layout.table(&columns, &rows, 8.5);
        assert!(layout.page_count() >= 3);
        let texts = page_texts(layout.pages());
        assert!(texts.iter().all(|page| page.starts_with("Number\nText")), "header on every page");
        assert!(texts.last().unwrap().contains("Row 120"));
        let pdf = layout.render(Metadata::new(), None, |_, _, _| Vec::new()).unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
    }
}
