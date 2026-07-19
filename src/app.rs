//! The interactive leap state machine.
//!
//! Three phases drive the flow:
//! 1. `AwaitSearch` — the user types one search character.
//! 2. `PickStart` — every match of that character is labeled; the user picks the anchor.
//! 3. `PickEnd` — the matches are re-labeled; the user picks the extent, which copies the region.
//!
//! All logic here is pure (no terminal I/O), so `handle_char` is fully unit-testable.

use crate::hints::{assign_hints, HintTarget};
use crate::leap::{Pos, WrappedBuffer};
use crate::theme::Theme;

/// What a keypress produced.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// Stay in the overlay and keep reading input.
    Continue,
    /// Copy this text and exit.
    Copy(String),
    /// Jump the source pane's copy-mode cursor to this visible buffer position and exit.
    Jump(Pos),
    /// Abort with no copy.
    Cancel,
}

/// Interaction mode.
///
/// - `Select` — two-point EasyMotion region copy (anchor then extent).
/// - `Jump` — tmux-jump style single target: word-start labels, one pick yields `Outcome::Jump`.
///   The entry point places the source pane's copy-mode cursor via Herdr's `pane.copy_mode_jump`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Select,
    Jump,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Select => "select",
            Mode::Jump => "jump",
        }
    }
}

/// The current phase of the state machine.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    AwaitSearch,
    PickStart,
    PickEnd,
}

const ESC: char = '\u{1b}';
const CTRL_C: char = '\u{3}';
const BACKSPACE_BS: char = '\u{8}';
const BACKSPACE_DEL: char = '\u{7f}';

pub struct App {
    buffer: WrappedBuffer,
    phase: Phase,
    search: Option<char>,
    matches: Vec<Pos>,
    labels: Vec<HintTarget<Pos>>,
    input: String,
    anchor: Option<Pos>,
    message: Option<String>,
    theme: Theme,
    mode: Mode,
}

impl App {
    pub fn new(buffer: WrappedBuffer, theme: Theme, mode: Mode) -> Self {
        Self {
            buffer,
            phase: Phase::AwaitSearch,
            search: None,
            matches: Vec::new(),
            labels: Vec::new(),
            input: String::new(),
            anchor: None,
            message: None,
            theme,
            mode,
        }
    }

    /// Advance the state machine by one input character. `Esc`/`Ctrl-C` cancel in every phase.
    pub fn handle_char(&mut self, ch: char) -> Outcome {
        if ch == ESC || ch == CTRL_C {
            return Outcome::Cancel;
        }
        let is_backspace = ch == BACKSPACE_BS || ch == BACKSPACE_DEL;

        match self.phase {
            Phase::AwaitSearch => {
                if is_backspace || ch.is_control() {
                    return Outcome::Continue;
                }
                self.begin_search(ch);
                Outcome::Continue
            }
            Phase::PickStart => {
                if is_backspace {
                    self.input.pop();
                    self.message = None;
                    return Outcome::Continue;
                }
                self.pick_label(ch, false)
            }
            Phase::PickEnd => {
                if is_backspace {
                    self.back_to_pick_start();
                    return Outcome::Continue;
                }
                self.pick_label(ch, true)
            }
        }
    }

    fn begin_search(&mut self, ch: char) {
        let matches = match self.mode {
            Mode::Jump => self.buffer.find_word_start_char(ch),
            Mode::Select => self.buffer.find_char(ch),
        };
        if matches.is_empty() {
            self.search = None;
            self.message = Some(format!("no match for '{ch}'"));
            return;
        }
        self.search = Some(ch);
        self.matches = matches;
        self.relabel();
        self.input.clear();
        self.message = None;
        self.phase = Phase::PickStart;
    }

    fn pick_label(&mut self, ch: char, is_end: bool) -> Outcome {
        // Labels are drawn from a lowercase alphabet; ignore anything that cannot be part of one.
        if !ch.is_ascii_alphabetic() {
            return Outcome::Continue;
        }
        self.input.push(ch.to_ascii_lowercase());

        let exact = self
            .labels
            .iter()
            .find(|target| target.hint == self.input)
            .map(|target| target.target);
        let has_longer = self
            .labels
            .iter()
            .any(|target| target.hint.starts_with(&self.input) && target.hint != self.input);

        if let Some(pos) = exact {
            if !has_longer {
                return self.resolve(pos, is_end);
            }
        }

        if self
            .labels
            .iter()
            .any(|target| target.hint.starts_with(&self.input))
        {
            self.message = None;
        } else {
            self.message = Some(format!("no hint starts with {}", self.input));
            self.input.clear();
        }
        Outcome::Continue
    }

    fn resolve(&mut self, pos: Pos, is_end: bool) -> Outcome {
        if matches!(self.mode, Mode::Jump) {
            // Jump never enters PickEnd; the first unique label is the destination.
            return Outcome::Jump(pos);
        }
        if is_end {
            let anchor = self
                .anchor
                .expect("PickEnd requires an anchor from PickStart");
            return Outcome::Copy(self.buffer.extract_region(anchor, pos));
        }
        self.anchor = Some(pos);
        self.relabel();
        self.input.clear();
        self.message = None;
        self.phase = Phase::PickEnd;
        Outcome::Continue
    }

    fn back_to_pick_start(&mut self) {
        self.anchor = None;
        self.input.clear();
        self.message = None;
        self.relabel();
        self.phase = Phase::PickStart;
    }

    fn relabel(&mut self) {
        self.labels = assign_hints(self.matches.clone());
    }

    // --- read-only accessors for the UI layer ---

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn search(&self) -> Option<char> {
        self.search
    }

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    pub fn labels(&self) -> &[HintTarget<Pos>] {
        &self.labels
    }

    pub fn anchor(&self) -> Option<Pos> {
        self.anchor
    }

    pub fn buffer(&self) -> &WrappedBuffer {
        &self.buffer
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// Number of labels still reachable given the current pending input (for the status line).
    pub fn visible_label_count(&self) -> usize {
        if self.input.is_empty() {
            return self.labels.len();
        }
        self.labels
            .iter()
            .filter(|target| target.hint.starts_with(&self.input))
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn app(text: &str) -> App {
        let buffer = WrappedBuffer::from_text(text, None);
        App::new(buffer, Theme::default(), Mode::Select)
    }

    fn jump_app(text: &str) -> App {
        let buffer = WrappedBuffer::from_text(text, None);
        App::new(buffer, Theme::default(), Mode::Jump)
    }

    /// Drive the app to a resolved anchor by typing the label the app actually assigned to `pos`.
    fn label_for(app: &App, pos: Pos) -> String {
        app.labels()
            .iter()
            .find(|target| target.target == pos)
            .expect("expected a label for the position")
            .hint
            .clone()
    }

    fn feed(app: &mut App, hint: &str) -> Outcome {
        let mut outcome = Outcome::Continue;
        for ch in hint.chars() {
            outcome = app.handle_char(ch);
        }
        outcome
    }

    #[test]
    fn esc_cancels_in_await_search() {
        let mut a = app("hello");
        assert_eq!(a.phase(), Phase::AwaitSearch);
        assert_eq!(a.handle_char('\u{1b}'), Outcome::Cancel);
    }

    #[test]
    fn ctrl_c_cancels_in_pick_start() {
        let mut a = app("hello");
        assert_eq!(a.handle_char('l'), Outcome::Continue);
        assert_eq!(a.phase(), Phase::PickStart);
        assert_eq!(a.handle_char('\u{3}'), Outcome::Cancel);
    }

    #[test]
    fn esc_cancels_in_pick_end() {
        let mut a = app("hello");
        a.handle_char('l'); // search -> PickStart (two matches: (0,2),(0,3))
        let anchor = Pos::new(0, 2);
        let hint = label_for(&a, anchor);
        feed(&mut a, &hint); // resolve anchor -> PickEnd
        assert_eq!(a.phase(), Phase::PickEnd);
        assert_eq!(a.handle_char('\u{1b}'), Outcome::Cancel);
    }

    #[test]
    fn search_char_with_matches_enters_pick_start_with_labels() {
        let mut a = app("banana");
        assert_eq!(a.handle_char('a'), Outcome::Continue);
        assert_eq!(a.phase(), Phase::PickStart);
        assert_eq!(a.search(), Some('a'));
        assert_eq!(a.labels().len(), 3); // three 'a's
    }

    #[test]
    fn search_char_without_match_stays_await_with_message() {
        let mut a = app("banana");
        assert_eq!(a.handle_char('z'), Outcome::Continue);
        assert_eq!(a.phase(), Phase::AwaitSearch);
        assert_eq!(a.search(), None);
        assert_eq!(a.message(), Some("no match for 'z'"));
    }

    #[test]
    fn full_flow_copies_region_between_two_chosen_matches() {
        // "one two three" — search 't': matches at "two" (col 4) and "three" (col 8).
        let mut a = app("one two three");
        a.handle_char('t');
        assert_eq!(a.phase(), Phase::PickStart);

        let anchor = Pos::new(0, 4); // the 't' of "two"
        let anchor_hint = label_for(&a, anchor);
        assert_eq!(feed(&mut a, &anchor_hint), Outcome::Continue);
        assert_eq!(a.phase(), Phase::PickEnd);
        assert_eq!(a.anchor(), Some(anchor));

        let extent = Pos::new(0, 8); // the 't' of "three"
        let extent_hint = label_for(&a, extent);
        let outcome = feed(&mut a, &extent_hint);
        assert_eq!(outcome, Outcome::Copy("two t".to_string()));
    }

    #[test]
    fn full_flow_reversed_selection_normalizes() {
        let mut a = app("one two three");
        a.handle_char('t');
        // Pick the LATER match as the anchor, the EARLIER as the extent — region must normalize.
        let anchor_hint = label_for(&a, Pos::new(0, 8));
        feed(&mut a, &anchor_hint);
        let extent_hint = label_for(&a, Pos::new(0, 4));
        let outcome = feed(&mut a, &extent_hint);
        assert_eq!(outcome, Outcome::Copy("two t".to_string()));
    }

    #[test]
    fn backspace_in_pick_end_returns_to_pick_start() {
        let mut a = app("hello");
        a.handle_char('l');
        let anchor_hint = label_for(&a, Pos::new(0, 2));
        feed(&mut a, &anchor_hint);
        assert_eq!(a.phase(), Phase::PickEnd);
        assert_eq!(a.handle_char('\u{7f}'), Outcome::Continue);
        assert_eq!(a.phase(), Phase::PickStart);
        assert_eq!(a.anchor(), None);
    }

    #[test]
    fn backspace_in_pick_start_pops_pending_input() {
        // Force multi-char labels so there is pending input to pop.
        let mut a = app("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"); // 30 'a's -> two-char labels
        a.handle_char('a');
        assert!(a.labels().iter().all(|t| t.hint.chars().count() == 2));
        assert_eq!(a.handle_char('a'), Outcome::Continue);
        assert_eq!(a.input(), "a");
        assert_eq!(a.handle_char('\u{7f}'), Outcome::Continue);
        assert_eq!(a.input(), "");
    }

    #[test]
    fn unknown_hint_prefix_resets_input_with_message() {
        let mut a = app("banana");
        a.handle_char('a'); // PickStart, single-char labels from the "asdf..." alphabet
                            // 'g' is late in the alphabet; with 3 targets no label starts with 'g'.
        assert_eq!(a.handle_char('g'), Outcome::Continue);
        assert_eq!(a.input(), "");
        assert_eq!(a.message(), Some("no hint starts with g"));
    }

    #[test]
    fn jump_mode_uses_word_starts_and_returns_jump_on_first_label() {
        let mut a = jump_app("two three button test");
        assert_eq!(a.handle_char('t'), Outcome::Continue);
        assert_eq!(a.phase(), Phase::PickStart);
        // word starts only: two, three, test — not the mid-word t in button
        assert_eq!(a.labels().len(), 3);

        let target = Pos::new(0, 4); // 't' of "three"
        let hint = label_for(&a, target);
        assert_eq!(feed(&mut a, &hint), Outcome::Jump(target));
        assert_ne!(a.phase(), Phase::PickEnd);
    }

    #[test]
    fn jump_mode_esc_cancels_from_pick_start() {
        let mut a = jump_app("target");
        a.handle_char('t');
        assert_eq!(a.phase(), Phase::PickStart);
        assert_eq!(a.handle_char('\u{1b}'), Outcome::Cancel);
    }

    #[test]
    fn select_mode_still_two_point_copies_after_jump_mode_exists() {
        let mut a = app("one two three");
        a.handle_char('t');
        let anchor_hint = label_for(&a, Pos::new(0, 4));
        assert_eq!(feed(&mut a, &anchor_hint), Outcome::Continue);
        assert_eq!(a.phase(), Phase::PickEnd);
        let extent_hint = label_for(&a, Pos::new(0, 8));
        assert_eq!(
            feed(&mut a, &extent_hint),
            Outcome::Copy("two t".to_string())
        );
    }
}
