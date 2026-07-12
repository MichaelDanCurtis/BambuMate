//! Small string helpers used across the backend.

/// Return a prefix of `s` that is at most `max_bytes` bytes long, cut on a
/// UTF-8 char boundary. Never panics for any `&str` input.
///
/// Plain slicing (`&s[..N]`) panics if `N` falls in the middle of a
/// multi-byte character — a real hazard for LLM outputs, HTML pages, and
/// GitHub release notes that may contain accented characters or emoji.
pub fn safe_prefix(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    // Walk back from max_bytes until we find a char boundary.
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Format a truncated preview of `s`, appending `ellipsis` if truncation
/// occurred. The returned string is always well-formed UTF-8.
pub fn truncate_with_ellipsis(s: &str, max_bytes: usize, ellipsis: &str) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    format!("{}{}", safe_prefix(s, max_bytes), ellipsis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_prefix_ascii_shorter_than_max() {
        assert_eq!(safe_prefix("hello", 100), "hello");
    }

    #[test]
    fn safe_prefix_ascii_exact_boundary() {
        assert_eq!(safe_prefix("hello world", 5), "hello");
    }

    #[test]
    fn safe_prefix_does_not_split_multibyte() {
        // "héllo" — 'é' is 2 bytes at index 1..=2
        let s = "héllo";
        // Naive slice at 2 would panic; safe_prefix returns "h".
        let out = safe_prefix(s, 2);
        assert_eq!(out, "h");
        assert!(std::str::from_utf8(out.as_bytes()).is_ok());
    }

    #[test]
    fn safe_prefix_multibyte_boundary_ok() {
        // Cut exactly after 'é' (byte index 3).
        assert_eq!(safe_prefix("héllo", 3), "hé");
    }

    #[test]
    fn safe_prefix_emoji_boundary() {
        // 🎉 is 4 bytes.
        let s = "abc🎉def";
        // Ask for 5 bytes — should return "abc" (not split the emoji).
        assert_eq!(safe_prefix(s, 5), "abc");
        assert_eq!(safe_prefix(s, 7), "abc🎉");
    }

    #[test]
    fn safe_prefix_empty_input() {
        assert_eq!(safe_prefix("", 10), "");
    }

    #[test]
    fn safe_prefix_zero_max() {
        assert_eq!(safe_prefix("héllo", 0), "");
    }

    #[test]
    fn truncate_with_ellipsis_no_truncation() {
        assert_eq!(truncate_with_ellipsis("hi", 100, "..."), "hi");
    }

    #[test]
    fn truncate_with_ellipsis_truncates_and_appends() {
        assert_eq!(truncate_with_ellipsis("hello world", 5, "..."), "hello...");
    }

    #[test]
    fn truncate_with_ellipsis_never_splits_char() {
        // Ensure no panic on any adversarial length across a mixed string.
        let s = "英文a👋テスト🎉";
        for n in 0..=s.len() + 4 {
            let out = truncate_with_ellipsis(s, n, "…");
            // The prefix (without ellipsis) must be valid UTF-8, which is
            // guaranteed by construction, but re-verify.
            assert!(std::str::from_utf8(out.as_bytes()).is_ok());
        }
    }
}
