//! Plain-text excerpts for lists, search results, and notifications.

use std::ops::Range;

/// One line: runs of whitespace become a single space.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The start of `text` on one line, cut to `max_chars`.
pub fn head(text: &str, max_chars: usize) -> String {
    let line = one_line(text);
    if line.chars().count() <= max_chars {
        return line;
    }
    let mut cut: String = line.chars().take(max_chars).collect();
    cut.push('…');
    cut
}

/// A line around the first case-insensitive match, and the match's byte range within that line.
pub fn excerpt(
    text: &str,
    query: &str,
    before: usize,
    max_chars: usize,
) -> (String, Option<Range<usize>>) {
    let line: Vec<char> = one_line(text).chars().collect();
    let needle: Vec<char> = one_line(query).chars().collect();
    let same = |a: char, b: char| a == b || a.to_lowercase().eq(b.to_lowercase());
    let found = (!needle.is_empty())
        .then(|| {
            line.windows(needle.len())
                .position(|window| window.iter().zip(&needle).all(|(a, b)| same(*a, *b)))
        })
        .flatten();
    let Some(at) = found else {
        return (head(text, max_chars), None);
    };
    let start = at.saturating_sub(before);
    let end = (start + max_chars).max(at + needle.len()).min(line.len());
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.extend(&line[start..at]);
    let match_start = out.len();
    out.extend(&line[at..at + needle.len()]);
    let match_end = out.len();
    out.extend(&line[at + needle.len()..end]);
    if end < line.len() {
        out.push('…');
    }
    (out, Some(match_start..match_end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_keeps_one_line_and_cuts() {
        assert_eq!(head("a\n\n  b", 10), "a b");
        assert_eq!(head("あいうえお", 3), "あいう…");
    }

    #[test]
    fn excerpt_marks_the_match() {
        let (line, range) = excerpt("先頭の文章\nログインを直す", "ログイン", 2, 20);
        assert_eq!(line, "…章 ログインを直す");
        assert_eq!(&line[range.unwrap()], "ログイン");
    }

    #[test]
    fn excerpt_ignores_case() {
        let (line, range) = excerpt("Fix the LoginForm", "loginform", 100, 100);
        assert_eq!(&line[range.unwrap()], "LoginForm");
    }

    #[test]
    fn excerpt_without_a_match_is_the_head() {
        assert_eq!(excerpt("abc", "z", 1, 10), ("abc".to_owned(), None));
    }

    #[test]
    fn excerpt_keeps_a_long_match() {
        let (line, range) = excerpt("xxabcdefg", "abcdefg", 0, 3);
        assert_eq!(&line[range.unwrap()], "abcdefg");
    }
}
