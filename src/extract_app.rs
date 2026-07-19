//! Interactive typeahead list for the visible-buffer extractor.
//!
//! Pure state machine: filter items by query, move the selection, copy or cancel.
//! No terminal I/O — fully unit-testable.

use crate::app::Outcome;
use crate::extract::ExtractItem;
use crate::theme::Theme;

const ESC: char = '\u{1b}';
const CTRL_C: char = '\u{3}';
const BACKSPACE_BS: char = '\u{8}';
const BACKSPACE_DEL: char = '\u{7f}';
const ENTER: char = '\n';
const UP: char = '\u{11}'; // DC1 — internal sentinel for Up
const DOWN: char = '\u{12}'; // DC2 — internal sentinel for Down

/// Inputs the extract TUI maps onto the pure state machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtractInput {
    Char(char),
    Backspace,
    Enter,
    Up,
    Down,
    Esc,
    CtrlC,
}

impl ExtractInput {
    /// Map a crossterm-style control/printable char used by `main` into an input.
    pub fn from_char(ch: char) -> Self {
        match ch {
            ESC => Self::Esc,
            CTRL_C => Self::CtrlC,
            BACKSPACE_BS | BACKSPACE_DEL => Self::Backspace,
            ENTER | '\r' => Self::Enter,
            UP => Self::Up,
            DOWN => Self::Down,
            other => Self::Char(other),
        }
    }

    pub fn up_sentinel() -> char {
        UP
    }

    pub fn down_sentinel() -> char {
        DOWN
    }

    pub fn enter_sentinel() -> char {
        ENTER
    }
}

/// Typeahead-filtered item list.
pub struct ExtractApp {
    items: Vec<ExtractItem>,
    /// Indices into `items` matching the current query (screen order).
    filtered: Vec<usize>,
    query: String,
    /// Index into `filtered`.
    selected: usize,
    message: Option<String>,
    theme: Theme,
}

impl ExtractApp {
    pub fn new(items: Vec<ExtractItem>, theme: Theme) -> Self {
        let mut app = Self {
            items,
            filtered: Vec::new(),
            query: String::new(),
            selected: 0,
            message: None,
            theme,
        };
        app.refilter();
        if app.filtered.is_empty() {
            app.message = Some("no matches".to_string());
        }
        app
    }

    pub fn from_visible_text(text: &str, theme: Theme) -> Self {
        Self::new(crate::extract::extract_items_from_visible_text(text), theme)
    }

    pub fn handle_input(&mut self, input: ExtractInput) -> Outcome {
        match input {
            ExtractInput::Esc | ExtractInput::CtrlC => Outcome::Cancel,
            ExtractInput::Enter => self.confirm(),
            ExtractInput::Backspace => {
                self.query.pop();
                self.refilter();
                self.message = None;
                Outcome::Continue
            }
            ExtractInput::Up => {
                self.move_sel(-1);
                Outcome::Continue
            }
            ExtractInput::Down => {
                self.move_sel(1);
                Outcome::Continue
            }
            ExtractInput::Char(ch) => {
                if ch.is_control() {
                    return Outcome::Continue;
                }
                self.query.push(ch);
                self.refilter();
                self.message = None;
                Outcome::Continue
            }
        }
    }

    /// Convenience for tests and simple char-only drivers.
    pub fn handle_char(&mut self, ch: char) -> Outcome {
        self.handle_input(ExtractInput::from_char(ch))
    }

    fn confirm(&self) -> Outcome {
        match self.selected_item() {
            Some(item) => Outcome::Copy(item.text.clone()),
            None => Outcome::Continue,
        }
    }

    fn move_sel(&mut self, delta: isize) {
        if self.filtered.is_empty() {
            self.selected = 0;
            return;
        }
        let len = self.filtered.len() as isize;
        let cur = self.selected as isize;
        let next = (cur + delta).rem_euclid(len);
        self.selected = next as usize;
    }

    fn refilter(&mut self) {
        let selected_item = self.filtered.get(self.selected).copied();
        let q = self.query.to_ascii_lowercase();
        self.filtered = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                if q.is_empty() {
                    true
                } else {
                    item.text.to_ascii_lowercase().contains(&q)
                }
            })
            .map(|(i, _)| i)
            .collect();
        if self.filtered.is_empty() {
            self.selected = 0;
            if !self.query.is_empty() {
                self.message = Some("no matches".to_string());
            }
        } else {
            self.selected = selected_item
                .and_then(|item| {
                    self.filtered
                        .iter()
                        .position(|&candidate| candidate == item)
                })
                .unwrap_or(0);
        }
    }

    pub fn selected_item(&self) -> Option<&ExtractItem> {
        self.filtered
            .get(self.selected)
            .and_then(|&i| self.items.get(i))
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn filtered_count(&self) -> usize {
        self.filtered.len()
    }

    pub fn total_count(&self) -> usize {
        self.items.len()
    }

    /// Filtered items in display order: `(is_selected, text)`.
    pub fn visible_rows(&self) -> Vec<(bool, &str)> {
        self.filtered
            .iter()
            .enumerate()
            .filter_map(|(pos, &idx)| {
                self.items
                    .get(idx)
                    .map(|item| (pos == self.selected, item.text.as_str()))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::{ExtractItem, ItemKind};
    use pretty_assertions::assert_eq;

    fn items(texts: &[&str]) -> Vec<ExtractItem> {
        texts
            .iter()
            .map(|t| ExtractItem {
                text: (*t).to_string(),
                kind: ItemKind::Word,
            })
            .collect()
    }

    fn app(texts: &[&str]) -> ExtractApp {
        ExtractApp::new(items(texts), Theme::default())
    }

    #[test]
    fn empty_query_shows_all_seed_items() {
        let a = app(&["alpha-token", "beta-token", "gamma-token"]);
        assert_eq!(a.filtered_count(), 3);
        assert_eq!(a.total_count(), 3);
        let rows = a.visible_rows();
        assert_eq!(rows.len(), 3);
        assert!(rows[0].0, "first row selected by default");
        assert_eq!(rows[0].1, "alpha-token");
    }

    #[test]
    fn typeahead_filters_items() {
        let mut a = app(&[
            "https://example.com/a",
            "/tmp/path/here",
            "ordinary-long-word",
        ]);
        assert_eq!(a.handle_char('p'), Outcome::Continue);
        assert_eq!(a.handle_char('a'), Outcome::Continue);
        assert_eq!(a.handle_char('t'), Outcome::Continue);
        assert_eq!(a.query(), "pat");
        let rows = a.visible_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1, "/tmp/path/here");
    }

    #[test]
    fn enter_on_selection_copies_exact_item_string() {
        let mut a = app(&["first-item-here", "second-item-here"]);
        a.handle_input(ExtractInput::Down);
        let outcome = a.handle_input(ExtractInput::Enter);
        assert_eq!(outcome, Outcome::Copy("second-item-here".to_string()));
    }

    #[test]
    fn esc_cancels() {
        let mut a = app(&["only-item-value"]);
        assert_eq!(a.handle_char('\u{1b}'), Outcome::Cancel);
    }

    #[test]
    fn ctrl_c_cancels() {
        let mut a = app(&["only-item-value"]);
        assert_eq!(a.handle_char('\u{3}'), Outcome::Cancel);
    }

    #[test]
    fn enter_with_no_matches_stays() {
        let mut a = app(&["alpha-token"]);
        a.handle_char('z');
        a.handle_char('z');
        assert_eq!(a.filtered_count(), 0);
        assert_eq!(a.handle_input(ExtractInput::Enter), Outcome::Continue);
    }

    #[test]
    fn backspace_widens_filter() {
        let mut a = app(&["alpha-token", "alpine-trail"]);
        a.handle_char('a');
        a.handle_char('l');
        a.handle_char('p');
        a.handle_char('h');
        assert_eq!(a.filtered_count(), 1);
        a.handle_input(ExtractInput::Backspace);
        assert_eq!(a.query(), "alp");
        assert_eq!(a.filtered_count(), 2);
    }

    #[test]
    fn refilter_preserves_selected_item_identity() {
        let mut a = app(&["alpha-token", "beta-selected", "beta-later"]);
        a.handle_input(ExtractInput::Down);
        assert_eq!(a.selected_item().unwrap().text, "beta-selected");

        a.handle_char('b');

        assert_eq!(a.selected_index(), 0);
        assert_eq!(a.selected_item().unwrap().text, "beta-selected");
    }

    #[test]
    fn refilter_selects_first_result_when_selection_disappears() {
        let mut a = app(&["alpha-selected", "beta-first", "beta-second"]);
        assert_eq!(a.selected_item().unwrap().text, "alpha-selected");

        a.handle_char('b');

        assert_eq!(a.selected_index(), 0);
        assert_eq!(a.selected_item().unwrap().text, "beta-first");
    }
}
