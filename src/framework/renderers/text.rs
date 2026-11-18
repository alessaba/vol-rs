//! Plain-text rendering: an aligned table, or a "quick" mode that streams rows
//! without buffering to measure column widths.
//!
//! Derived from Volatility 3, Copyright Volatility Foundation, licensed under
//! the Volatility Software License 1.0.

use std::io::Write;

use crate::error::Result;
use crate::framework::renderers::style::{Role, Style};
use crate::framework::renderers::{
    AbsentValue, Column, NumberFormat, Renderer, TreeGrid, Truncation, Value,
};

/// Renders a table with every column padded to its widest cell.
///
/// The tree is shown by a leading column of asterisks, and each column is right
/// aligned and separated by a pipe. A cell holding several lines is written
/// across as many rows, with the other columns left blank.
#[derive(Default)]
pub struct PrettyTextRenderer {
    pub options: crate::framework::renderers::filter::RenderOptions,
    pub style: Style,
    pub summary: bool,
}

impl PrettyTextRenderer {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Renderer for PrettyTextRenderer {
    fn render(&self, grid: &TreeGrid, output: &mut dyn Write) -> Result<()> {
        // Nothing is written for a plugin that stopped part way, since the
        // table cannot be laid out until every row is in.
        match grid.truncation() {
            Truncation::None => {}
            Truncation::Abrupt | Truncation::Discarded => return Ok(()),
            Truncation::Reported => {
                write!(output, "\n\n")?;
                return Ok(());
            }
        }
        let ignored = self.options.ignored(grid)?;
        let shown = |index: usize| !ignored.contains(&index);
        let columns = grid.columns();

        // Rows are gathered first, since a column is only as wide as its widest
        // cell and the tree column only as wide as the deepest row.
        let mut depth_width = 0;
        let mut widths: Vec<usize> = columns
            .iter()
            .map(|column| column.name.chars().count())
            .collect();
        let mut body: Vec<(usize, Vec<Vec<String>>, Vec<Role>)> = Vec::new();
        let mut unreadable = 0;
        let mut unparsable = 0;

        for row in grid.rows() {
            let cells: Vec<String> = row.values.iter().map(|value| value.to_string()).collect();
            if self.options.filtered(&cells) {
                continue;
            }
            depth_width = depth_width.max(row.depth + 1);

            let matched = self.options.highlights(&cells);
            let roles: Vec<Role> = row
                .values
                .iter()
                .enumerate()
                .filter(|(index, _)| shown(*index))
                .map(|(index, value)| {
                    if matched.get(index).copied().unwrap_or(false) {
                        Role::Match
                    } else {
                        role_of(value, &columns[index])
                    }
                })
                .collect();
            for value in row
                .values
                .iter()
                .enumerate()
                .filter(|(index, _)| shown(*index))
                .map(|(_, value)| value)
            {
                match value {
                    Value::Absent(AbsentValue::Unreadable | AbsentValue::NotAvailable) => {
                        unreadable += 1
                    }
                    Value::Absent(AbsentValue::Unparsable) => unparsable += 1,
                    _ => {}
                }
            }

            let mut lines: Vec<Vec<String>> = Vec::new();
            for (index, cell) in cells.iter().enumerate() {
                let parts: Vec<String> = cell.split('\n').map(tab_stop).collect();
                let widest = parts.iter().map(|part| part.chars().count()).max().unwrap_or(0);
                widths[index] = widths[index].max(widest);
                lines.push(parts);
            }
            body.push((row.depth, lines, roles));
        }

        let rows = body.len();
        // The header's own tree column is empty.
        let mut header: Vec<String> = vec![String::new()];
        header.extend(
            columns
                .iter()
                .enumerate()
                .filter(|(index, _)| shown(*index))
                .map(|(_, column)| column.name.clone()),
        );
        let header_roles = vec![Role::Header; header.len()];
        write_row(
            output,
            &header,
            &header_roles,
            depth_width,
            &widths,
            &ignored,
            &self.style,
        )?;

        for (depth, lines, roles) in body {
            let height = lines.iter().map(|parts| parts.len()).max().unwrap_or(1);
            for line in 0..height {
                // A row's depth is counted from one, so even a top level row
                // carries a single mark.
                let mut cells: Vec<String> = vec![if line == 0 {
                    "*".repeat(depth + 1)
                } else {
                    " ".repeat(depth + 1)
                }];
                cells.extend(
                    lines
                        .iter()
                        .enumerate()
                        .filter(|(index, _)| shown(*index))
                        .map(|(_, parts)| parts.get(line).cloned().unwrap_or_default()),
                );
                let mut line_roles = vec![Role::Faint];
                line_roles.extend(roles.iter().copied());
                write_row(
                    output,
                    &cells,
                    &line_roles,
                    depth_width,
                    &widths,
                    &ignored,
                    &self.style,
                )?;
            }
        }

        if self.summary {
            write!(
                output,
                "{}",
                summary(&self.style, rows, unreadable, unparsable)
            )?;
        }
        Ok(())
    }
}

/// Write one line of the pretty table: the tree column left aligned, then every
/// visible column right aligned to its own width.
///
/// Each cell is painted only after it has been padded, so the escape sequences
/// never take part in measuring a column.
fn write_row(
    output: &mut dyn Write,
    cells: &[String],
    roles: &[Role],
    depth_width: usize,
    widths: &[usize],
    ignored: &[usize],
    style: &Style,
) -> Result<()> {
    let tree = format!("{:<depth_width$}", cells[0]);
    let mut line = if tree.trim().is_empty() {
        tree
    } else {
        style.paint(Role::Faint, &tree).into_owned()
    };
    let mut cell = 1;
    for (index, width) in widths.iter().enumerate() {
        if ignored.contains(&index) {
            continue;
        }
        let text = cells.get(cell).cloned().unwrap_or_default();
        let padding = width.saturating_sub(text.chars().count());
        line.push_str(" | ");
        line.push_str(&" ".repeat(padding));
        let role = roles.get(cell).copied().unwrap_or(Role::Plain);
        line.push_str(&style.paint(role, &text));
        cell += 1;
    }
    writeln!(output, "{line}")?;
    Ok(())
}

/// The line a run may end with: how much was listed and how much of it could
/// not be read.
fn summary(style: &Style, rows: usize, unreadable: usize, unparsable: usize) -> String {
    let mut parts = vec![format!("{rows} rows")];
    if unreadable > 0 {
        parts.push(format!("{unreadable} unreadable"));
    }
    if unparsable > 0 {
        parts.push(format!("{unparsable} unparsable"));
    }
    let line = format!("\n{}\n", parts.join(", "));
    style.paint(Role::Faint, &line).into_owned()
}

/// The role a value plays in the output, taken from what it is rather than from
/// what it says.
fn role_of(value: &Value, column: &Column) -> Role {
    match value {
        Value::Absent(AbsentValue::NotApplicable) => Role::NotApplicable,
        Value::Absent(AbsentValue::Unreadable | AbsentValue::NotAvailable) => Role::Unreadable,
        Value::Absent(AbsentValue::Unparsable) => Role::Unparsable,
        Value::Bool(value) => Role::Bool(*value),
        Value::DateTime(_) | Value::NaiveDateTime(_) => Role::Time,
        Value::UInt(value, NumberFormat::Decimal) if is_pid(column) && *value != 0 => {
            Role::Pid((*value % PID_COLOURS) as u8)
        }
        Value::Int(value, NumberFormat::Decimal) if is_pid(column) && *value != 0 => {
            Role::Pid(value.rem_euclid(PID_COLOURS as i64) as u8)
        }
        Value::UInt(_, NumberFormat::Hex) | Value::Int(_, NumberFormat::Hex) => Role::Address,
        _ => Role::Plain,
    }
}

/// How many colours process IDs are spread over.
const PID_COLOURS: u64 = 6;

/// Whether a column holds a process ID.
fn is_pid(column: &Column) -> bool {
    column.name.eq_ignore_ascii_case("pid") || column.name.eq_ignore_ascii_case("ppid")
}

/// Expand tabs to the next eight column stop, as the reference implementation
/// does before measuring a cell.
fn tab_stop(line: &str) -> String {
    let mut text = line.to_string();
    while let Some(position) = text.find('\t') {
        let padding = " ".repeat(8 - (position % 8));
        text = text.replacen('\t', &padding, 1);
    }
    text
}

/// Streams rows as they arrive, without aligning columns.
///
/// Useful for long-running plugins where seeing partial output early matters
/// more than a tidy table.
pub struct QuickTextRenderer {
    pub separator: String,
    pub options: crate::framework::renderers::filter::RenderOptions,
    pub style: Style,
    pub summary: bool,
}

impl Default for QuickTextRenderer {
    fn default() -> Self {
        Self {
            separator: "\t".to_string(),
            options: Default::default(),
            style: Style::default(),
            summary: false,
        }
    }
}

impl Renderer for QuickTextRenderer {
    fn render(&self, grid: &TreeGrid, output: &mut dyn Write) -> Result<()> {
        // A plugin that gave up before it had a listing to render leaves
        // nothing at all behind it, not even a header.
        if grid.truncation() == Truncation::Discarded {
            return Ok(());
        }
        let ignored = self.options.ignored(grid)?;
        let headers: Vec<String> = grid
            .columns()
            .iter()
            .enumerate()
            .filter(|(index, _)| !ignored.contains(index))
            .map(|(_, column)| column.name.clone())
            .collect();
        // Upstream writes a newline, the header, then a newline before each
        // row. That leaves a blank line above and below the header, and is why
        // a listing cut short ends without its final newline, and why a run
        // that fails before rendering prints nothing but the banner.
        write!(output, "\n{}\n", headers.join(&self.separator))?;

        let columns = grid.columns();
        let mut rows = 0usize;
        let mut unreadable = 0;
        let mut unparsable = 0;
        for row in grid.rows() {
            let rendered: Vec<String> =
                row.values.iter().map(|value| value.to_string()).collect();
            if self.options.filtered(&rendered) {
                continue;
            }
            let matched = self.options.highlights(&rendered);
            rows += 1;
            for value in row
                .values
                .iter()
                .enumerate()
                .filter(|(index, _)| !ignored.contains(index))
                .map(|(_, value)| value)
            {
                match value {
                    Value::Absent(AbsentValue::Unreadable | AbsentValue::NotAvailable) => {
                        unreadable += 1
                    }
                    Value::Absent(AbsentValue::Unparsable) => unparsable += 1,
                    _ => {}
                }
            }
            let cells: Vec<String> = rendered
                .into_iter()
                .enumerate()
                .filter(|(index, _)| !ignored.contains(index))
                .map(|(index, value)| {
                    let text = if index == 0 && row.depth > 0 {
                        // One asterisk per level of nesting, separated from the
                        // value by a single space.
                        format!("{} {value}", "*".repeat(row.depth))
                    } else {
                        value
                    };
                    let role = if matched.get(index).copied().unwrap_or(false) {
                        Role::Match
                    } else {
                        role_of(&row.values[index], &columns[index])
                    };
                    self.style.paint(role, &text).into_owned()
                })
                .collect();
            write!(output, "\n{}", cells.join(&self.separator))?;
        }

        match grid.truncation() {
            // The run finished, so the last line is terminated.
            Truncation::None => writeln!(output)?,
            // The process died mid-write, leaving the last line unterminated.
            Truncation::Abrupt => {}
            // The error was reported, which leaves a blank line behind it.
            Truncation::Reported => write!(output, "\n\n")?,
            // Nothing was written at all, not even the header.
            Truncation::Discarded => {}
        }
        if self.summary && grid.truncation() == Truncation::None {
            write!(
                output,
                "{}",
                summary(&self.style, rows, unreadable, unparsable)
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework::renderers::{Column, TreeGrid, Value};

    fn sample_grid() -> TreeGrid {
        let mut grid = TreeGrid::new(vec![Column::uint("PID"), Column::string("ImageFileName")]);
        grid.push(0, vec![Value::uint(4), Value::string("System")])
            .unwrap();
        grid.push(1, vec![Value::uint(400), Value::string("smss.exe")])
            .unwrap();
        grid
    }

    #[test]
    fn aligns_columns_and_indents_children() {
        let mut buffer = Vec::new();
        PrettyTextRenderer::new()
            .render(&sample_grid(), &mut buffer)
            .unwrap();
        let text = String::from_utf8(buffer).unwrap();
        let lines: Vec<&str> = text.lines().collect();

        // The tree column comes first, then every column right aligned to its
        // widest cell.
        assert_eq!(lines[0], "   | PID | ImageFileName");
        assert_eq!(lines[1], "*  |   4 |        System");
        assert_eq!(lines[2], "** | 400 |      smss.exe");
    }

    #[test]
    fn styling_does_not_change_the_text() {
        let style = crate::framework::renderers::style::Style::new(
            crate::framework::renderers::style::When::Always,
            crate::framework::renderers::style::ThemeKind::Dark,
        );
        let mut plain = Vec::new();
        PrettyTextRenderer::new()
            .render(&sample_grid(), &mut plain)
            .unwrap();
        let mut painted = Vec::new();
        PrettyTextRenderer {
            style,
            ..Default::default()
        }
        .render(&sample_grid(), &mut painted)
        .unwrap();
        assert_eq!(
            strip_ansi(&String::from_utf8(painted).unwrap()),
            String::from_utf8(plain).unwrap()
        );
    }

    /// The text with every escape sequence removed.
    fn strip_ansi(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut characters = text.chars();
        while let Some(character) = characters.next() {
            if character != '\u{1b}' {
                out.push(character);
                continue;
            }
            // Skip the bracketed parameters and the letter that ends them.
            for character in characters.by_ref() {
                if character.is_ascii_alphabetic() {
                    break;
                }
            }
        }
        out
    }

    #[test]
    fn quick_mode_skips_alignment() {
        let mut buffer = Vec::new();
        QuickTextRenderer::default()
            .render(&sample_grid(), &mut buffer)
            .unwrap();
        let text = String::from_utf8(buffer).unwrap();
        assert!(text.contains("PID\tImageFileName"));
        // A blank line separates the header from the rows.
        assert!(text.contains("PID\tImageFileName\n\n"));
        // Upstream writes one asterisk per level and then a single space:
        // `"*" * (path_depth - 1) + " "`.
        assert!(text.contains("* 400\tsmss.exe"));
    }
}
