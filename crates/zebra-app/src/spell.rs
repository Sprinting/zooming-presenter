//! Spell checking for text being edited on the canvas: the Mac's own
//! checker (NSSpellChecker), so it follows the person's languages and the
//! words they taught it. Elsewhere, and on non-main threads (tests), nothing
//! is ever marked.

use std::ops::Range;

/// Last result for one text, so the checker runs only when the text changes.
#[derive(Debug, Default, Clone)]
pub struct Cache {
    key: Option<u64>,
    bad: Vec<Range<usize>>,
}

fn hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Misspelled words in `text`, as char ranges.
pub fn misspelled<'a>(text: &str, cache: &'a mut Cache) -> &'a [Range<usize>] {
    let k = hash(text);
    if cache.key != Some(k) {
        cache.key = Some(k);
        cache.bad = imp::check(text);
    }
    &cache.bad
}

/// Forget the cached result (after Learn or Ignore).
pub fn invalidate(cache: &mut Cache) {
    cache.key = None;
}

/// Suggested spellings for the word at `range` (chars) in `text`.
pub fn guesses(text: &str, range: Range<usize>) -> Vec<String> {
    imp::guesses(text, range)
}

/// Add `word` to the person's dictionary (shared with every Mac app).
pub fn learn(word: &str) {
    imp::learn(word)
}

/// Stop marking `word` while Zebra runs.
pub fn ignore(word: &str) {
    imp::ignore(word)
}

/// UTF-16 offset → char index in `text`.
pub fn char_index(text: &str, utf16: usize) -> usize {
    let mut u = 0;
    for (i, c) in text.chars().enumerate() {
        if u >= utf16 {
            return i;
        }
        u += c.len_utf16();
    }
    text.chars().count()
}

/// Char index → UTF-16 offset in `text`.
pub fn utf16_index(text: &str, chars: usize) -> usize {
    text.chars().take(chars).map(|c| c.len_utf16()).sum()
}

#[cfg(target_os = "macos")]
mod imp {
    use std::ops::Range;
    use std::sync::OnceLock;

    use objc2::MainThreadMarker;
    use objc2_app_kit::NSSpellChecker;
    use objc2_foundation::{NSRange, NSString};

    use super::{char_index, utf16_index};

    /// One spell "document" for Zebra, so Ignore lasts for the session.
    fn tag() -> isize {
        static TAG: OnceLock<isize> = OnceLock::new();
        *TAG.get_or_init(NSSpellChecker::uniqueSpellDocumentTag)
    }

    fn checker() -> Option<objc2::rc::Retained<NSSpellChecker>> {
        MainThreadMarker::new()?;
        Some(NSSpellChecker::sharedSpellChecker())
    }

    pub fn check(text: &str) -> Vec<Range<usize>> {
        let Some(sc) = checker() else { return vec![] };
        let ns = NSString::from_str(text);
        let len = ns.length();
        let mut out = vec![];
        let mut start = 0usize;
        while start < len {
            // SAFETY: word_count may be null.
            let r = unsafe { sc.checkSpellingOfString_startingAt_language_wrap_inSpellDocumentWithTag_wordCount(&ns, start as isize, None, false, tag(), std::ptr::null_mut()) };
            if r.length == 0 || r.location >= isize::MAX as usize || r.location < start {
                break;
            }
            out.push(char_index(text, r.location)..char_index(text, r.location + r.length));
            start = r.location + r.length;
        }
        out
    }

    pub fn guesses(text: &str, range: Range<usize>) -> Vec<String> {
        let Some(sc) = checker() else { return vec![] };
        let ns = NSString::from_str(text);
        let (a, b) = (utf16_index(text, range.start), utf16_index(text, range.end));
        let r = NSRange { location: a, length: b - a };
        sc.guessesForWordRange_inString_language_inSpellDocumentWithTag(r, &ns, None, tag())
            .map(|v| v.iter().map(|s| s.to_string()).take(6).collect())
            .unwrap_or_default()
    }

    pub fn learn(word: &str) {
        if let Some(sc) = checker() {
            sc.learnWord(&NSString::from_str(word));
        }
    }

    pub fn ignore(word: &str) {
        if let Some(sc) = checker() {
            sc.ignoreWord_inSpellDocumentWithTag(&NSString::from_str(word), tag());
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use std::ops::Range;
    pub fn check(_: &str) -> Vec<Range<usize>> {
        vec![]
    }
    pub fn guesses(_: &str, _: Range<usize>) -> Vec<String> {
        vec![]
    }
    pub fn learn(_: &str) {}
    pub fn ignore(_: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_offsets_map_to_chars() {
        let s = "a😀b é";
        assert_eq!(utf16_index(s, 2), 3);
        assert_eq!(char_index(s, 3), 2);
        assert_eq!(char_index(s, utf16_index(s, 5)), 5);
    }
}
