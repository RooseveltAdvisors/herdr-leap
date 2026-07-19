//! Visible-buffer token extraction (extrakto-parity subset).
//!
//! Pure logic: bounded URL / path / quote / word extraction from visible text with
//! reverse + ordered dedupe. No socket or TTY.

use regex::Regex;
use std::collections::HashSet;
use std::sync::OnceLock;

/// Kind of extracted item (for tests and future filter cycling).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Url,
    Path,
    Quote,
    SQuote,
    Word,
}

/// One copy-eligible token from the visible buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtractItem {
    pub text: String,
    pub kind: ItemKind,
}

const MIN_LENGTH: usize = 5;

/// Extract the v1 item set from already-visible pane text.
///
/// Default list = path ∪ url ∪ quote ∪ s-quote ∪ word (min length 5), reversed so
/// lower/more-recent screen content appears first, then deduped preserving order.
pub fn extract_items_from_visible_text(text: &str) -> Vec<ExtractItem> {
    extract_items_from_flat(text)
}

fn extract_items_from_flat(text: &str) -> Vec<ExtractItem> {
    let mut raw: Vec<ExtractItem> = Vec::new();
    raw.extend(filter_urls(text));
    raw.extend(filter_paths(text));
    raw.extend(filter_quotes(text));
    raw.extend(filter_s_quotes(text));
    raw.extend(filter_words(text));

    raw.reverse();

    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in raw {
        if seen.insert(item.text.clone()) {
            out.push(item);
        }
    }
    out
}

fn filter_urls(text: &str) -> Vec<ExtractItem> {
    // Extrakto: (https?://|git@|git://|ssh://|s*ftp://|file:///)(body)
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(
            r"(?i)(https?://|git@|git://|ssh://|s?ftp://|file:///)([a-zA-Z0-9?=%/_.:,;~@!#$&()*+-]*)",
        )
        .expect("url regex")
    });
    collect_joined_groups(re, text, ItemKind::Url, Some(r#"",):"#))
}

fn filter_paths(text: &str) -> Vec<ExtractItem> {
    // Extrakto-parity path: lead-in + path body containing at least one `/`.
    // Haystack is prefixed with newline so column-0 paths still match.
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(concat!(
            r#"(?i)(?:[\t\n "'(\[<':]|^)"#,
            r#"((?:~|/)?[-~A-Za-z0-9_+,.]+/[^ \t\n\r|:"'$%&)>\]]*)"#,
        ))
        .expect("path regex")
    });
    static EXCLUDE: OnceLock<Regex> = OnceLock::new();
    let exclude =
        EXCLUDE.get_or_init(|| Regex::new(r"(?i)[kmg]/s$|^\d+/\d+$").expect("path exclude"));

    let mut out = Vec::new();
    let haystack = format!("\n{text}");
    for caps in re.captures_iter(&haystack) {
        let Some(m) = caps.get(1) else {
            continue;
        };
        let item = m
            .as_str()
            .trim_end_matches(['"', ',', ')', ':'])
            .to_string();
        if item.chars().count() < MIN_LENGTH {
            continue;
        }
        if exclude.is_match(&item) {
            continue;
        }
        out.push(ExtractItem {
            text: item,
            kind: ItemKind::Path,
        });
    }
    out
}

fn filter_quotes(text: &str) -> Vec<ExtractItem> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r#""([^"\n\r]+)""#).expect("quote regex"));
    collect_full_match(re, text, ItemKind::Quote)
}

fn filter_s_quotes(text: &str) -> Vec<ExtractItem> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"'([^'\n\r]+)'").expect("s-quote regex"));
    collect_full_match(re, text, ItemKind::SQuote)
}

fn filter_words(text: &str) -> Vec<ExtractItem> {
    // Extrakto word charset: anything but [](){}=$ box-drawing private-use whitespace.
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"[^\]\[(){}=$\u{2500}-\u{27BF}\u{E000}-\u{F8FF}⋅↴│ \t\n\r]+")
            .expect("word regex")
    });
    let lstrip: &[char] = &[
        ',', ':', ';', '(', ')', '[', ']', '{', '}', '<', '>', '\'', '"', '|',
    ];
    let rstrip: &[char] = &[
        ',', ':', ';', '(', ')', '[', ']', '{', '}', '<', '>', '\'', '"', '|', '.',
    ];
    let mut out = Vec::new();
    for m in re.find_iter(text) {
        let item = m
            .as_str()
            .trim_start_matches(lstrip)
            .trim_end_matches(rstrip);
        if item.chars().count() < MIN_LENGTH {
            continue;
        }
        out.push(ExtractItem {
            text: item.to_string(),
            kind: ItemKind::Word,
        });
    }
    out
}

fn collect_joined_groups(
    re: &Regex,
    text: &str,
    kind: ItemKind,
    rstrip: Option<&str>,
) -> Vec<ExtractItem> {
    let mut out = Vec::new();
    for caps in re.captures_iter(text) {
        let mut item = String::new();
        for i in 1..caps.len() {
            if let Some(g) = caps.get(i) {
                item.push_str(g.as_str());
            }
        }
        if let Some(chars) = rstrip {
            while item.chars().last().is_some_and(|c| chars.contains(c)) {
                item.pop();
            }
        }
        if item.chars().count() < MIN_LENGTH {
            continue;
        }
        out.push(ExtractItem { text: item, kind });
    }
    out
}

fn collect_full_match(re: &Regex, text: &str, kind: ItemKind) -> Vec<ExtractItem> {
    let mut out = Vec::new();
    for m in re.find_iter(text) {
        let item = m.as_str();
        if item.chars().count() < MIN_LENGTH {
            continue;
        }
        out.push(ExtractItem {
            text: item.to_string(),
            kind,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn texts(items: &[ExtractItem]) -> Vec<&str> {
        items.iter().map(|i| i.text.as_str()).collect()
    }

    fn fixture_visible() -> &'static str {
        "\
# on-screen decoys
DECOY_LINE_076 https://decoy-76.invalid/x /decoy/path/76.txt
DECOY_LINE_077 https://decoy-77.invalid/x /decoy/path/77.txt
Visit https://example.com/docs/api?v=1 for docs.
Config at ~/projects/herdr-leap/config.toml and /var/log/herdr/server.log
Run: cargo test --release --locked
double: \"hello world value\"
single: 'single-quoted-token'
clone git@github.com:RooseveltAdvisors/herdr-leap.git
see path/with/relative/file.rs
short hi ordinary-long-word-here
curl https://cdn.example.org/v2/asset.tar.gz
# END_FIXTURE"
    }

    #[test]
    fn extracts_urls_paths_quotes_words_from_fixture() {
        let items = extract_items_from_visible_text(fixture_visible());
        let t = texts(&items);
        for expected in [
            "https://example.com/docs/api?v=1",
            "https://cdn.example.org/v2/asset.tar.gz",
            "git@github.com:RooseveltAdvisors/herdr-leap.git",
            "~/projects/herdr-leap/config.toml",
            "path/with/relative/file.rs",
            "\"hello world value\"",
            "'single-quoted-token'",
            "ordinary-long-word-here",
        ] {
            assert!(t.contains(&expected), "missing {expected:?} in {t:?}");
        }
        assert!(
            !t.iter().any(|s| s.contains("decoy-0")),
            "off-screen decoy-0 must not appear: {t:?}"
        );
    }

    #[test]
    fn exact_width_rows_are_not_joined_without_wrap_metadata() {
        let items = extract_items_from_visible_text("abcde\nfghij");
        let t = texts(&items);
        assert!(t.contains(&"abcde"), "got {t:?}");
        assert!(t.contains(&"fghij"), "got {t:?}");
        assert!(!t.contains(&"abcdefghij"), "got {t:?}");
    }

    #[test]
    fn paths_capture_complete_extrakto_tail_tokens() {
        let paths = filter_paths("/tmp/foo=bar/baz /tmp/über/file /tmp/@scope/package");
        let paths: Vec<_> = paths.iter().map(|item| item.text.as_str()).collect();
        for expected in ["/tmp/foo=bar/baz", "/tmp/über/file", "/tmp/@scope/package"] {
            assert!(
                paths.contains(&expected),
                "missing {expected:?} in {paths:?}"
            );
        }
        assert!(!paths.contains(&"/tmp/foo"), "truncated path in {paths:?}");
        assert!(!paths.contains(&"/tmp/"), "truncated path in {paths:?}");
    }

    #[test]
    fn min_length_5_drops_short_words() {
        let items = extract_items_from_visible_text("short hi ordinary-long-word-here");
        let t = texts(&items);
        assert!(!t.contains(&"hi"));
        assert!(t.contains(&"ordinary-long-word-here"));
        // "short" is exactly 5 and should survive as a word.
        assert!(t.contains(&"short"));
    }

    #[test]
    fn dedupes_preserving_order_after_reverse() {
        let text =
            "see /tmp/alpha/file.txt once\nand /tmp/alpha/file.txt twice\nzz-bottom-unique-token";
        let items = extract_items_from_visible_text(text);
        let paths: Vec<_> = items
            .iter()
            .filter(|i| i.text.contains("/tmp/alpha/file.txt"))
            .collect();
        assert_eq!(paths.len(), 1, "expected one deduped path, got {paths:?}");
        // Bottom content should rank earlier after reverse+dedupe.
        let t = texts(&items);
        let bottom = t
            .iter()
            .position(|s| *s == "zz-bottom-unique-token")
            .expect("bottom token");
        let path_pos = t
            .iter()
            .position(|s| s.contains("/tmp/alpha/file.txt"))
            .expect("path");
        assert!(bottom < path_pos, "bottom-first order broken: {t:?}");
    }

    #[test]
    fn word_uses_distinct_leading_and_trailing_strip_sets() {
        let items = extract_items_from_visible_text("edit .gitignore with plugin.");
        let t = texts(&items);
        assert!(t.contains(&".gitignore"), "got {t:?}");
        assert!(!t.contains(&"gitignore"), "got {t:?}");
        assert!(t.contains(&"plugin"), "got {t:?}");
        assert!(!t.iter().any(|s| s.ends_with('.')), "got {t:?}");
    }

    #[test]
    fn empty_visible_text_extracts_nothing() {
        assert!(extract_items_from_visible_text("").is_empty());
    }
}
