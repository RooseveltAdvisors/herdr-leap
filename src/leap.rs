//! Pure leap logic: the wrapped-buffer coordinate model, smartcase character search, and the
//! load-bearing region extraction from an anchor position to an extent position.
//!
//! No socket, TTY, or terminal state is touched here so every behavior is unit-testable.

/// A position in the wrapped visible buffer: a visual row and a character column within that row.
///
/// Ordering is row-major then column (derived from field order), which is exactly the order a
/// region is normalized against.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Pos {
    pub row: usize,
    pub col: usize,
}

impl Pos {
    pub fn new(row: usize, col: usize) -> Self {
        Self { row, col }
    }
}

/// The visible pane content modeled as wrapped visual rows.
///
/// `rows[i]` is one visual row. `continues_prev[i]` is `true` when `rows[i]` is a soft-wrap
/// continuation of `rows[i - 1]` (i.e. they came from the same logical line, split only because the
/// logical line was wider than the pane). It is `false` when `rows[i]` began a new logical line
/// (a hard `\n` break). `continues_prev[0]` is always `false`.
#[derive(Clone, Debug)]
pub struct WrappedBuffer {
    rows: Vec<String>,
    continues_prev: Vec<bool>,
}

impl WrappedBuffer {
    /// Build the wrapped buffer from raw visible pane text.
    ///
    /// - The text is split into logical lines on `\n`. Empty input yields a single empty row.
    /// - When `wrap_width` is `Some(w)` with `w > 0`, each logical line longer than `w` characters
    ///   is split into consecutive visual rows of at most `w` characters. The first visual row of a
    ///   logical line has `continues_prev = false`; every wrapped continuation row has
    ///   `continues_prev = true`.
    /// - `wrap_width` of `None` (or `Some(0)`) yields exactly one visual row per logical line.
    pub fn from_text(text: &str, wrap_width: Option<usize>) -> Self {
        let logical: Vec<&str> = if text.is_empty() {
            vec![""]
        } else {
            let collected: Vec<&str> = text.lines().collect();
            if collected.is_empty() {
                vec![""]
            } else {
                collected
            }
        };

        let width = wrap_width.filter(|w| *w > 0);
        let mut rows = Vec::new();
        let mut continues_prev = Vec::new();

        for line in logical {
            match width {
                Some(w) => {
                    let chars: Vec<char> = line.chars().collect();
                    if chars.is_empty() {
                        rows.push(String::new());
                        continues_prev.push(false);
                        continue;
                    }
                    let mut first = true;
                    for chunk in chars.chunks(w) {
                        rows.push(chunk.iter().collect());
                        continues_prev.push(!first);
                        first = false;
                    }
                }
                None => {
                    rows.push(line.to_string());
                    continues_prev.push(false);
                }
            }
        }

        Self {
            rows,
            continues_prev,
        }
    }

    pub fn rows(&self) -> &[String] {
        &self.rows
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// Every position of `needle`, ordered top-to-bottom then left-to-right.
    ///
    /// Smartcase: a lowercase (or caseless) `needle` matches both cases; an uppercase `needle`
    /// matches only uppercase.
    pub fn find_char(&self, needle: char) -> Vec<Pos> {
        let case_sensitive = needle.is_uppercase();
        let mut hits = Vec::new();
        for (row, line) in self.rows.iter().enumerate() {
            for (col, ch) in line.chars().enumerate() {
                if smartcase_matches(ch, needle, case_sensitive) {
                    hits.push(Pos::new(row, col));
                }
            }
        }
        hits
    }

    /// The inclusive region from `a` to `b`, normalized so the smaller position is the start.
    ///
    /// - Same row: the characters in the inclusive column span `[start.col ..= end.col]`, clamped
    ///   to the row's length.
    /// - Multiple rows: the anchor row's tail from `start.col`, the full intervening rows, and the
    ///   extent row's head through `end.col`. Adjacent rows are joined with `""` when the later row
    ///   is a soft-wrap continuation (reconstructing the real logical line) or with `"\n"` at a hard
    ///   line break.
    pub fn extract_region(&self, a: Pos, b: Pos) -> String {
        if self.rows.is_empty() {
            return String::new();
        }
        let max_row = self.rows.len() - 1;
        let (start, end) = if a <= b { (a, b) } else { (b, a) };
        let start_row = start.row.min(max_row);
        let end_row = end.row.min(max_row);

        let mut out = String::new();
        for row in start_row..=end_row {
            let chars: Vec<char> = self.rows[row].chars().collect();
            let len = chars.len();
            let seg_start = if row == start_row {
                start.col.min(len)
            } else {
                0
            };
            let seg_end = if row == end_row {
                end.col.saturating_add(1).min(len)
            } else {
                len
            };
            if row != start_row {
                // continues_prev is index-aligned with rows; a soft wrap joins with no separator.
                if !self.continues_prev[row] {
                    out.push('\n');
                }
            }
            if seg_start < seg_end {
                out.extend(&chars[seg_start..seg_end]);
            }
        }
        out
    }
}

fn smartcase_matches(ch: char, needle: char, case_sensitive: bool) -> bool {
    if case_sensitive {
        ch == needle
    } else {
        ch == needle || ch.to_lowercase().eq(needle.to_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn buf(text: &str) -> WrappedBuffer {
        WrappedBuffer::from_text(text, None)
    }

    #[test]
    fn from_text_splits_hard_line_breaks_into_rows() {
        let b = buf("alpha\nbeta");
        assert_eq!(b.rows(), &["alpha".to_string(), "beta".to_string()]);
        assert_eq!(b.row_count(), 2);
    }

    #[test]
    fn from_text_empty_yields_single_empty_row() {
        let b = buf("");
        assert_eq!(b.rows(), &[String::new()]);
    }

    #[test]
    fn from_text_wraps_long_logical_line_and_marks_continuations() {
        let b = WrappedBuffer::from_text("abcdefg", Some(3));
        assert_eq!(
            b.rows(),
            &["abc".to_string(), "def".to_string(), "g".to_string()]
        );
        // First row starts a logical line; the next two are soft-wrap continuations.
        assert_eq!(b.continues_prev, vec![false, true, true]);
    }

    #[test]
    fn find_char_finds_every_position_in_reading_order() {
        let b = buf("aba\nxa");
        assert_eq!(
            b.find_char('a'),
            vec![Pos::new(0, 0), Pos::new(0, 2), Pos::new(1, 1),]
        );
    }

    #[test]
    fn find_char_lowercase_is_case_insensitive() {
        let b = buf("aAbA");
        assert_eq!(
            b.find_char('a'),
            vec![Pos::new(0, 0), Pos::new(0, 1), Pos::new(0, 3)]
        );
    }

    #[test]
    fn find_char_uppercase_is_case_sensitive() {
        let b = buf("aAbA");
        assert_eq!(b.find_char('A'), vec![Pos::new(0, 1), Pos::new(0, 3)]);
    }

    #[test]
    fn extract_single_char_region() {
        let b = buf("hello");
        assert_eq!(b.extract_region(Pos::new(0, 1), Pos::new(0, 1)), "e");
    }

    #[test]
    fn extract_inclusive_column_span_on_one_row() {
        let b = buf("hello world");
        // columns 0..=4 inclusive -> "hello"
        assert_eq!(b.extract_region(Pos::new(0, 0), Pos::new(0, 4)), "hello");
        // columns 6..=10 inclusive -> "world"
        assert_eq!(b.extract_region(Pos::new(0, 6), Pos::new(0, 10)), "world");
    }

    #[test]
    fn extract_reversed_order_normalizes() {
        let b = buf("hello world");
        let forward = b.extract_region(Pos::new(0, 0), Pos::new(0, 4));
        let reversed = b.extract_region(Pos::new(0, 4), Pos::new(0, 0));
        assert_eq!(forward, reversed);
        assert_eq!(reversed, "hello");
    }

    #[test]
    fn extract_multiline_hard_break_joins_with_newline() {
        let b = buf("hello\nworld\nagain");
        // from (0,2) through (2,2): "llo" + "\n" + "world" + "\n" + "aga"
        assert_eq!(
            b.extract_region(Pos::new(0, 2), Pos::new(2, 2)),
            "llo\nworld\naga"
        );
    }

    #[test]
    fn extract_multiline_reversed_hard_break_normalizes() {
        let b = buf("hello\nworld");
        assert_eq!(b.extract_region(Pos::new(1, 2), Pos::new(0, 2)), "llo\nwor");
    }

    #[test]
    fn extract_across_soft_wrap_reconstructs_the_logical_line() {
        // One logical line "abcdefghij" wrapped at width 4 -> rows ["abcd","efgh","ij"].
        let b = WrappedBuffer::from_text("abcdefghij", Some(4));
        assert_eq!(b.rows(), &["abcd", "efgh", "ij"]);
        // Region from (0,1) through (2,0): soft wraps join with NO separator -> "bcdefghi".
        assert_eq!(b.extract_region(Pos::new(0, 1), Pos::new(2, 0)), "bcdefghi");
    }

    #[test]
    fn extract_mixes_soft_wrap_and_hard_break_correctly() {
        // Logical line 1 "abcdef" wraps at 3 -> "abc","def" (soft). Then hard break to "XY".
        let b = WrappedBuffer::from_text("abcdef\nXY", Some(3));
        assert_eq!(b.rows(), &["abc", "def", "XY"]);
        assert_eq!(b.continues_prev, vec![false, true, false]);
        // (0,0)..(2,1): "abc" + "" + "def" + "\n" + "XY" == "abcdef\nXY"
        assert_eq!(
            b.extract_region(Pos::new(0, 0), Pos::new(2, 1)),
            "abcdef\nXY"
        );
    }

    #[test]
    fn extract_clamps_out_of_range_extent_column() {
        let b = buf("hi");
        // extent col 99 clamps to the row length; region is the whole row.
        assert_eq!(b.extract_region(Pos::new(0, 0), Pos::new(0, 99)), "hi");
    }

    #[test]
    fn extract_handles_multibyte_on_char_boundaries() {
        let b = buf("café");
        // 'é' is one char at col 3; slicing by char must not panic.
        assert_eq!(b.extract_region(Pos::new(0, 3), Pos::new(0, 3)), "é");
        assert_eq!(b.extract_region(Pos::new(0, 0), Pos::new(0, 3)), "café");
    }

    #[test]
    fn extract_handles_wide_multibyte_multiline() {
        let b = buf("あいう\nえお");
        assert_eq!(
            b.extract_region(Pos::new(0, 1), Pos::new(1, 1)),
            "いう\nえお"
        );
    }

    #[test]
    fn find_char_multibyte_does_not_panic() {
        let b = buf("あいあ");
        assert_eq!(b.find_char('あ'), vec![Pos::new(0, 0), Pos::new(0, 2)]);
    }
}
