//! Ratatui rendering for Leap's search and pick phases.
//!
//! The visible pane content is drawn dimmed; hint labels are overlaid bright at each reachable
//! match. A bottom status line reports the phase, mode, search char, pending input, and any message,
//! width-clamped so it never overflows a narrow pane.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{App, Phase};
use crate::leap::Pos;
use crate::theme::Theme;

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    // Paint status over the last row so source viewport row indices remain unchanged.
    let body_rows = usize::from(area.height);
    let lines = render_body(app, body_rows);
    frame.render_widget(Paragraph::new(lines), area);
    draw_status(frame, app, area);
}

fn base_style() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

/// Collect the label placements for a given row: `(col, hint)` for every label whose target is on
/// this row, still reachable given the current input, and within the row's character bounds.
fn row_placements(app: &App, row: usize, row_len: usize) -> Vec<(usize, String)> {
    if matches!(app.phase(), Phase::AwaitSearch) {
        return Vec::new();
    }
    let input = app.input();
    let mut placements: Vec<(usize, String)> = app
        .labels()
        .iter()
        .filter(|target| target.target.row == row && target.target.col < row_len)
        .filter(|target| input.is_empty() || target.hint.starts_with(input))
        .map(|target| (target.target.col, target.hint.clone()))
        .collect();
    placements.sort_by_key(|(col, _)| *col);
    // Drop placements that would overlap a previous label's span (defensive; matches are single
    // chars so this is rare).
    let mut deduped: Vec<(usize, String)> = Vec::new();
    let mut next_free = 0usize;
    for (col, hint) in placements {
        if col >= next_free {
            next_free = col + hint.chars().count().max(1);
            deduped.push((col, hint));
        }
    }
    deduped
}

fn render_body(app: &App, max_rows: usize) -> Vec<Line<'static>> {
    let theme = app.theme();
    let anchor = if matches!(app.phase(), Phase::PickEnd) {
        app.anchor()
    } else {
        None
    };
    let rows = app.buffer().rows();
    let start = rows.len().saturating_sub(max_rows);
    rows.iter()
        .enumerate()
        .skip(start)
        .take(max_rows)
        .map(|(row, line)| {
            render_row(
                row,
                line,
                &row_placements(app, row, line.chars().count()),
                anchor,
                theme,
            )
        })
        .collect()
}

fn render_row(
    row: usize,
    line: &str,
    placements: &[(usize, String)],
    anchor: Option<Pos>,
    theme: &Theme,
) -> Line<'static> {
    let chars: Vec<char> = line.chars().collect();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut col = 0usize;
    let mut iter = placements.iter().peekable();

    while col < chars.len() {
        if let Some((placement_col, hint)) = iter.peek() {
            if *placement_col == col {
                spans.push(Span::styled(hint.clone(), theme.hint_style(false)));
                col += hint.chars().count().max(1);
                iter.next();
                continue;
            }
        }
        let next_col = iter
            .peek()
            .map(|(c, _)| *c)
            .unwrap_or(chars.len())
            .min(chars.len());
        if next_col <= col {
            // The next placement is inside a span we already emitted; skip it to avoid a stall.
            iter.next();
            continue;
        }
        for (offset, ch) in chars[col..next_col].iter().enumerate() {
            let absolute = col + offset;
            let style = if anchor == Some(Pos::new(row, absolute)) {
                theme.match_style(true)
            } else {
                base_style()
            };
            spans.push(Span::styled(ch.to_string(), style));
        }
        col = next_col;
    }

    if spans.is_empty() {
        // Preserve an empty row so line indices stay aligned with the buffer.
        spans.push(Span::raw(String::new()));
    }
    Line::from(spans)
}

fn draw_status(frame: &mut Frame<'_>, app: &App, area: Rect) {
    if area.height == 0 {
        return;
    }
    let status_area = Rect {
        x: area.x,
        y: area.y + area.height - 1,
        width: area.width,
        height: 1,
    };
    let search = app
        .search()
        .map(|c| c.to_string())
        .unwrap_or_else(|| "-".to_string());
    let input = if app.input().is_empty() {
        "-".to_string()
    } else {
        app.input().to_string()
    };
    let text = status_text(
        usize::from(status_area.width),
        phase_label(app.phase()),
        app.mode().label(),
        &search,
        &input,
        app.message().unwrap_or(""),
    );
    frame.render_widget(
        Paragraph::new(text).style(app.theme().status_style()),
        status_area,
    );
}

fn phase_label(phase: Phase) -> &'static str {
    match phase {
        Phase::AwaitSearch => "search",
        Phase::PickStart => "start",
        Phase::PickEnd => "end",
    }
}

/// Build a status line that fits within `width`, dropping detail as the width tightens.
pub fn status_text(
    width: usize,
    phase: &str,
    mode: &str,
    search: &str,
    input: &str,
    message: &str,
) -> String {
    let variants = [
        format!(
            " leap  {phase}  {mode}  search:{search}  input:{input}  {message}  esc:cancel  bksp:back "
        ),
        format!(" leap  {phase}  {mode}  search:{search}  input:{input}  {message}"),
        format!(" leap  {phase}  search:{search}  in:{input}  {message}"),
        format!(" leap {phase} s:{search} i:{input}"),
        format!(" {phase}"),
    ];
    let text = variants
        .into_iter()
        .find(|candidate| candidate.chars().count() <= width)
        .unwrap_or_else(|| " leap ".to_string());
    text.chars().take(width).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn status_text_keeps_help_when_width_allows() {
        let text = status_text(100, "start", "select", "t", "-", "");
        assert!(text.contains("esc:cancel"));
        assert!(text.contains("bksp:back"));
        assert!(text.chars().count() <= 100);
    }

    #[test]
    fn status_text_drops_help_when_width_is_tight() {
        let text = status_text(40, "end", "select", "t", "a", "no hint starts with z");
        assert!(!text.contains("esc:cancel"));
        assert!(text.chars().count() <= 40);
        assert!(text.contains("end"));
    }

    #[test]
    fn status_text_fits_very_narrow_width() {
        let text = status_text(6, "start", "select", "t", "-", "");
        assert!(text.chars().count() <= 6);
    }

    #[test]
    fn render_row_overlays_hint_without_widening_single_char_labels() {
        let theme = Theme::default();
        let line = render_row(0, "hello", &[(0, "a".to_string())], None, &theme);
        let rendered: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(rendered, "aello");
        assert_eq!(rendered.chars().count(), "hello".chars().count());
    }

    #[test]
    fn render_body_keeps_the_source_viewports_bottom_rows() {
        let app = App::new(
            crate::leap::WrappedBuffer::from_text("old\ntop\nmiddle\nbottom", None),
            Theme::default(),
            crate::app::Mode::Jump,
        );
        let rendered: Vec<String> = render_body(&app, 3)
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect()
            })
            .collect();
        assert_eq!(rendered, ["top", "middle", "bottom"]);
    }
}
