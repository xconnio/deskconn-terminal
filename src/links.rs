//! Finding URLs in the terminal's text, for a click to open.

use std::ops::Range;

/// What ends a word that could hold a URL, besides whitespace.
pub const BOUNDARIES: &[char] = &[' ', '\t', '"', '\'', '`', '<', '>', '│'];

const SCHEMES: &[&str] = &["https://", "http://", "file://", "ftp://", "mailto:"];

/// Whether url has a scheme a click may open: links of other schemes can launch any app that
/// handles them, from a link whose text shows something else.
pub fn opens(url: &str) -> bool {
    SCHEMES.iter().any(|scheme| {
        url.get(..scheme.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(scheme))
    })
}

/// Where in word, a run of text without whitespace, there's a URL.
///
/// Punctuation after a URL ends the sentence rather than the URL, and a closing bracket ends it
/// unless the URL opened one, as in `(see https://en.wikipedia.org/wiki/Rust_(language)).`
pub fn find(word: &str) -> Option<Range<usize>> {
    let (start, scheme) = SCHEMES
        .iter()
        .filter_map(|scheme| word.find(scheme).map(|start| (start, scheme)))
        .min()?;
    let mut end = word.len();
    while let Some(last) = word[start..end].chars().last() {
        let url = &word[start..end];
        let unbalanced = |open, close| url.matches(open).count() < url.matches(close).count();
        let trailing = match last {
            '.' | ',' | ':' | ';' | '!' | '?' => true,
            ')' => unbalanced('(', ')'),
            ']' => unbalanced('[', ']'),
            '}' => unbalanced('{', '}'),
            _ => false,
        };
        if !trailing {
            break;
        }
        end -= last.len_utf8();
    }
    (end > start + scheme.len()).then_some(start..end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(word: &str) -> Option<&str> {
        find(word).map(|range| &word[range])
    }

    #[test]
    fn urls() {
        assert_eq!(url("https://xconn.io/docs"), Some("https://xconn.io/docs"));
        assert_eq!(url("see:https://xconn.io."), Some("https://xconn.io"));
        assert_eq!(url("(https://xconn.io)"), Some("https://xconn.io"));
        assert_eq!(
            url("https://en.wikipedia.org/wiki/Rust_(language))."),
            Some("https://en.wikipedia.org/wiki/Rust_(language)")
        );
        assert_eq!(url("mailto:hello@xconn.io"), Some("mailto:hello@xconn.io"));
        assert_eq!(url("https://"), None);
        assert_eq!(url("xconn.io"), None);
    }

    #[test]
    fn schemes() {
        assert!(opens("https://xconn.io"));
        assert!(opens("HTTPS://xconn.io"));
        assert!(opens("file:///home/me/notes.txt"));
        assert!(!opens("vscode://file/home/me/notes.txt"));
        assert!(!opens("ht"));
    }
}
