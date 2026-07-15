#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TextPoint {
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyCellKind {
    Normal,
    Match,
    ActiveMatch,
    Selection,
    Cursor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionKind {
    Characters,
    Lines,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Selection {
    anchor: TextPoint,
    kind: SelectionKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MatchRange {
    start: TextPoint,
    end: TextPoint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyModeRow {
    pub line: usize,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct CopyModeView {
    pub pane: usize,
    pub rows: Vec<CopyModeRow>,
    pub total_lines: usize,
    pub top_line: usize,
    pub left_column: usize,
    pub cursor: TextPoint,
    pub query: String,
    pub searching: bool,
    pub active_match: Option<usize>,
    pub match_count: usize,
    pub selection_label: Option<&'static str>,
    selection: Option<Selection>,
    matches: Vec<MatchRange>,
}

impl CopyModeView {
    pub fn cell_kind(&self, point: TextPoint) -> CopyCellKind {
        if point == self.cursor {
            return CopyCellKind::Cursor;
        }
        if self
            .selection
            .is_some_and(|selection| selection_contains(selection, self.cursor, point))
        {
            return CopyCellKind::Selection;
        }
        if self
            .active_match
            .and_then(|index| self.matches.get(index))
            .is_some_and(|range| match_contains(*range, point))
        {
            return CopyCellKind::ActiveMatch;
        }
        if self
            .matches
            .iter()
            .any(|range| match_contains(*range, point))
        {
            return CopyCellKind::Match;
        }
        CopyCellKind::Normal
    }
}

#[derive(Debug, Clone)]
pub struct CopyMode {
    pane: usize,
    lines: Vec<String>,
    cursor: TextPoint,
    selection: Option<Selection>,
    query: String,
    searching: bool,
    matches: Vec<MatchRange>,
    active_match: Option<usize>,
    top_line: usize,
    left_column: usize,
}

impl CopyMode {
    pub fn new(pane: usize, mut lines: Vec<String>, width: usize, height: usize) -> Self {
        if lines.is_empty() {
            lines.push(String::new());
        }
        let line = lines.len().saturating_sub(1);
        let column = line_char_len(&lines[line]).saturating_sub(1);
        let mut mode = Self {
            pane,
            lines,
            cursor: TextPoint { line, column },
            selection: None,
            query: String::new(),
            searching: false,
            matches: Vec::new(),
            active_match: None,
            top_line: 0,
            left_column: 0,
        };
        mode.ensure_visible(width, height);
        mode
    }

    pub fn pane(&self) -> usize {
        self.pane
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn searching(&self) -> bool {
        self.searching
    }

    pub fn status_label(&self) -> String {
        if self.searching {
            return format!(
                "copy search: /{} ({} matches)",
                self.query,
                self.matches.len()
            );
        }
        if !self.query.is_empty() {
            return format!(
                "copy mode: match {}/{} for /{}",
                self.active_match.map_or(0, |index| index + 1),
                self.matches.len(),
                self.query
            );
        }
        format!(
            "copy mode: line {}/{}",
            self.cursor.line + 1,
            self.lines.len()
        )
    }

    pub fn begin_search(&mut self, width: usize, height: usize) {
        self.searching = true;
        self.query.clear();
        self.matches.clear();
        self.active_match = None;
        self.ensure_visible(width, height);
    }

    pub fn finish_search(&mut self) {
        self.searching = false;
    }

    pub fn insert_search_char(&mut self, ch: char, width: usize, height: usize) {
        self.query.push(ch);
        self.rebuild_matches(width, height);
    }

    pub fn backspace_search(&mut self, width: usize, height: usize) {
        self.query.pop();
        self.rebuild_matches(width, height);
    }

    pub fn clear_search(&mut self, width: usize, height: usize) {
        self.query.clear();
        self.rebuild_matches(width, height);
    }

    pub fn next_match(&mut self, forward: bool, width: usize, height: usize) {
        if self.matches.is_empty() {
            self.active_match = None;
            return;
        }
        let current = self.active_match.unwrap_or(0);
        let next = if forward {
            (current + 1) % self.matches.len()
        } else {
            current
                .checked_sub(1)
                .unwrap_or_else(|| self.matches.len() - 1)
        };
        self.activate_match(next, width, height);
    }

    pub fn move_vertical(&mut self, delta: isize, width: usize, height: usize) {
        self.cursor.line = offset_index(self.cursor.line, delta, self.lines.len());
        self.clamp_cursor_column();
        self.ensure_visible(width, height);
    }

    pub fn move_horizontal(&mut self, delta: isize, width: usize, height: usize) {
        if delta < 0 {
            if self.cursor.column > 0 {
                self.cursor.column -= 1;
            } else if self.cursor.line > 0 {
                self.cursor.line -= 1;
                self.cursor.column = self.line_max_column(self.cursor.line);
            }
        } else if delta > 0 {
            let max_column = self.line_max_column(self.cursor.line);
            if self.cursor.column < max_column {
                self.cursor.column += 1;
            } else if self.cursor.line + 1 < self.lines.len() {
                self.cursor.line += 1;
                self.cursor.column = 0;
            }
        }
        self.ensure_visible(width, height);
    }

    pub fn move_page(&mut self, pages: isize, width: usize, height: usize) {
        let rows = height.max(1).min(isize::MAX as usize) as isize;
        self.move_vertical(pages.saturating_mul(rows), width, height);
    }

    pub fn move_line_boundary(&mut self, end: bool, width: usize, height: usize) {
        self.cursor.column = if end {
            self.line_max_column(self.cursor.line)
        } else {
            0
        };
        self.ensure_visible(width, height);
    }

    pub fn move_document_boundary(&mut self, end: bool, width: usize, height: usize) {
        self.cursor.line = if end {
            self.lines.len().saturating_sub(1)
        } else {
            0
        };
        self.cursor.column = if end {
            self.line_max_column(self.cursor.line)
        } else {
            0
        };
        self.ensure_visible(width, height);
    }

    pub fn toggle_character_selection(&mut self) {
        self.selection = match self.selection {
            Some(Selection {
                kind: SelectionKind::Characters,
                ..
            }) => None,
            _ => Some(Selection {
                anchor: self.cursor,
                kind: SelectionKind::Characters,
            }),
        };
    }

    pub fn toggle_line_selection(&mut self) {
        self.selection = match self.selection {
            Some(Selection {
                kind: SelectionKind::Lines,
                ..
            }) => None,
            _ => Some(Selection {
                anchor: self.cursor,
                kind: SelectionKind::Lines,
            }),
        };
    }

    pub fn copy_text(&self) -> String {
        let Some(selection) = self.selection else {
            return self.lines[self.cursor.line].clone();
        };

        match selection.kind {
            SelectionKind::Lines => {
                let start = selection.anchor.line.min(self.cursor.line);
                let end = selection.anchor.line.max(self.cursor.line);
                self.lines[start..=end].join("\n")
            }
            SelectionKind::Characters => {
                let (start, end) = normalize_points(selection.anchor, self.cursor);
                let mut selected = Vec::new();
                for line in start.line..=end.line {
                    let from = if line == start.line { start.column } else { 0 };
                    let to = if line == end.line {
                        end.column.saturating_add(1)
                    } else {
                        line_char_len(&self.lines[line])
                    };
                    selected.push(slice_chars(&self.lines[line], from, to));
                }
                selected.join("\n")
            }
        }
    }

    pub fn view(&self, width: usize, height: usize) -> CopyModeView {
        let width = width.max(1);
        let height = height.max(1);
        let max_top = self.lines.len().saturating_sub(height);
        let top_line = self.top_line.min(max_top);
        let rows = self
            .lines
            .iter()
            .enumerate()
            .skip(top_line)
            .take(height)
            .map(|(line, text)| {
                let mut text = slice_chars(
                    text,
                    self.left_column,
                    self.left_column.saturating_add(width),
                );
                if text.is_empty() && line == self.cursor.line {
                    text.push(' ');
                }
                CopyModeRow { line, text }
            })
            .collect();

        CopyModeView {
            pane: self.pane,
            rows,
            total_lines: self.lines.len(),
            top_line,
            left_column: self.left_column,
            cursor: self.cursor,
            query: self.query.clone(),
            searching: self.searching,
            active_match: self.active_match,
            match_count: self.matches.len(),
            selection_label: self.selection.map(|selection| match selection.kind {
                SelectionKind::Characters => "CHAR",
                SelectionKind::Lines => "LINE",
            }),
            selection: self.selection,
            matches: self.matches.clone(),
        }
    }

    fn rebuild_matches(&mut self, width: usize, height: usize) {
        self.matches.clear();
        self.active_match = None;
        if self.query.is_empty() {
            self.ensure_visible(width, height);
            return;
        }

        let query_chars = self.query.chars().count();
        for (line, text) in self.lines.iter().enumerate() {
            for (byte_start, _) in text.match_indices(&self.query) {
                let start_column = text[..byte_start].chars().count();
                self.matches.push(MatchRange {
                    start: TextPoint {
                        line,
                        column: start_column,
                    },
                    end: TextPoint {
                        line,
                        column: start_column + query_chars,
                    },
                });
            }
        }

        let next = self
            .matches
            .iter()
            .position(|range| range.start >= self.cursor)
            .unwrap_or(0);
        self.activate_match(next, width, height);
    }

    fn activate_match(&mut self, index: usize, width: usize, height: usize) {
        let Some(found) = self.matches.get(index).copied() else {
            self.active_match = None;
            return;
        };
        self.active_match = Some(index);
        self.cursor = found.start;
        self.ensure_visible(width, height);
    }

    fn line_max_column(&self, line: usize) -> usize {
        line_char_len(&self.lines[line]).saturating_sub(1)
    }

    fn clamp_cursor_column(&mut self) {
        self.cursor.column = self
            .cursor
            .column
            .min(self.line_max_column(self.cursor.line));
    }

    fn ensure_visible(&mut self, width: usize, height: usize) {
        let width = width.max(1);
        let height = height.max(1);
        if self.cursor.line < self.top_line {
            self.top_line = self.cursor.line;
        } else if self.cursor.line >= self.top_line.saturating_add(height) {
            self.top_line = self.cursor.line + 1 - height;
        }
        self.top_line = self.top_line.min(self.lines.len().saturating_sub(height));

        if self.cursor.column < self.left_column {
            self.left_column = self.cursor.column;
        } else if self.cursor.column >= self.left_column.saturating_add(width) {
            self.left_column = self.cursor.column + 1 - width;
        }
    }
}

fn selection_contains(selection: Selection, cursor: TextPoint, point: TextPoint) -> bool {
    match selection.kind {
        SelectionKind::Lines => {
            let start = selection.anchor.line.min(cursor.line);
            let end = selection.anchor.line.max(cursor.line);
            (start..=end).contains(&point.line)
        }
        SelectionKind::Characters => {
            let (start, end) = normalize_points(selection.anchor, cursor);
            (start..=end).contains(&point)
        }
    }
}

fn match_contains(range: MatchRange, point: TextPoint) -> bool {
    point.line == range.start.line
        && point.column >= range.start.column
        && point.column < range.end.column
}

fn normalize_points(first: TextPoint, second: TextPoint) -> (TextPoint, TextPoint) {
    if first <= second {
        (first, second)
    } else {
        (second, first)
    }
}

fn offset_index(index: usize, delta: isize, len: usize) -> usize {
    if delta.is_negative() {
        index.saturating_sub(delta.unsigned_abs())
    } else {
        index
            .saturating_add(delta as usize)
            .min(len.saturating_sub(1))
    }
}

fn line_char_len(line: &str) -> usize {
    line.chars().count()
}

fn slice_chars(line: &str, start: usize, end: usize) -> String {
    line.chars()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(lines: &[&str]) -> CopyMode {
        CopyMode::new(
            0,
            lines.iter().map(|line| (*line).to_string()).collect(),
            20,
            4,
        )
    }

    #[test]
    fn incremental_search_and_match_navigation_are_unicode_safe() {
        let mut mode = mode(&["alpha caf\u{e9}", "beta", "another caf\u{e9}"]);
        mode.move_document_boundary(false, 20, 4);
        mode.begin_search(20, 4);
        for ch in "caf\u{e9}".chars() {
            mode.insert_search_char(ch, 20, 4);
        }

        let first = mode.view(20, 4);
        assert_eq!(first.match_count, 2);
        assert_eq!(first.cursor, TextPoint { line: 0, column: 6 });

        mode.next_match(true, 20, 4);
        assert_eq!(mode.view(20, 4).cursor.line, 2);
        mode.next_match(true, 20, 4);
        assert_eq!(mode.view(20, 4).cursor.line, 0);
        mode.next_match(false, 20, 4);
        assert_eq!(mode.view(20, 4).cursor.line, 2);
    }

    #[test]
    fn character_and_line_selections_extract_expected_text() {
        let mut mode = mode(&["alpha", "bravo", "charlie"]);
        mode.move_document_boundary(false, 20, 4);
        mode.move_horizontal(1, 20, 4);
        mode.toggle_character_selection();
        mode.move_vertical(1, 20, 4);
        assert_eq!(mode.copy_text(), "lpha\nbr");

        mode.toggle_line_selection();
        mode.move_vertical(1, 20, 4);
        assert_eq!(mode.copy_text(), "bravo\ncharlie");
    }

    #[test]
    fn empty_history_and_narrow_views_remain_renderable() {
        let mode = CopyMode::new(2, Vec::new(), 0, 0);
        let view = mode.view(1, 1);

        assert_eq!(view.pane, 2);
        assert_eq!(view.total_lines, 1);
        assert_eq!(view.rows.len(), 1);
        assert_eq!(view.rows[0].text, " ");
        assert_eq!(mode.copy_text(), "");
    }

    #[test]
    fn selection_and_active_search_styles_have_stable_precedence() {
        let mut mode = mode(&["find this"]);
        mode.move_document_boundary(false, 20, 4);
        mode.begin_search(20, 4);
        for ch in "find".chars() {
            mode.insert_search_char(ch, 20, 4);
        }
        mode.finish_search();
        mode.toggle_character_selection();
        mode.move_horizontal(1, 20, 4);
        let view = mode.view(20, 4);

        assert_eq!(
            view.cell_kind(TextPoint { line: 0, column: 0 }),
            CopyCellKind::Selection
        );
        assert_eq!(
            view.cell_kind(TextPoint { line: 0, column: 1 }),
            CopyCellKind::Cursor
        );
        assert_eq!(
            view.cell_kind(TextPoint { line: 0, column: 2 }),
            CopyCellKind::ActiveMatch
        );
    }
}
