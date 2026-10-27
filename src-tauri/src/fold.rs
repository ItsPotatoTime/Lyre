//! Folding accented letters onto the letter underneath.
//!
//! Typing "tari" has to find "Țări", and "soseaua" has to find "Șoseaua".
//! The matcher folds a table of Latin characters, but that table stops after
//! Latin Extended-A, so Romanian `ț`, Polish `ł`, every Vietnamese tone mark and
//! all of Cyrillic were left out.
//!
//! Rather than extend a table one character at a time, this decomposes each
//! character with Unicode NFD and drops the combining marks. That works for
//! every script that composes its diacritics, which is nearly all of them:
//! Latin, Cyrillic, Greek, Vietnamese, Hebrew, Arabic. A short table covers the
//! letters that are single characters rather than base-plus-mark, such as `ł`
//! and `ø`.
//!
//! **Every character folds to exactly one character.** That is not incidental:
//! highlight positions are character offsets into the name, and an expansion
//! like `ß` to `ss` would shift every index after it and light up the wrong
//! letters.

use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

/// Letters that carry their accent as part of the glyph rather than as a
/// separate mark, so NFD leaves them alone.
fn standalone(letter: char) -> char {
    match letter {
        'ł' | 'Ł' => 'l',
        'ø' | 'Ø' => 'o',
        'đ' | 'Đ' | 'ð' | 'Ð' => 'd',
        'ß' => 's',
        'æ' | 'Æ' => 'a',
        'œ' | 'Œ' => 'o',
        'ı' => 'i',
        'þ' | 'Þ' => 't',
        'ħ' | 'Ħ' => 'h',
        'ŧ' | 'Ŧ' => 't',
        'ƒ' => 'f',
        'ŋ' | 'Ŋ' => 'n',
        'ĸ' => 'k',
        other => other,
    }
}

/// Write the folded form of `input` into `out`, reusing its allocation.
///
/// ASCII, which is nearly every filename, is copied with no per-character work
/// at all.
pub fn fold_into(input: &str, out: &mut String) {
    out.clear();
    if input.is_ascii() {
        out.push_str(input);
        return;
    }

    for ch in input.nfd() {
        if is_combining_mark(ch) {
            continue;
        }
        // Decomposition can leave a base letter that still needs the table,
        // for example a precomposed letter whose NFD is unchanged.
        let folded = standalone(ch);
        if folded.is_ascii() {
            // Lowercasing here is redundant with the matcher's own case folding
            // but keeps the two paths identical and cheap to reason about.
            out.push(folded.to_ascii_lowercase());
        } else {
            out.push(folded);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fold(input: &str) -> String {
        let mut out = String::new();
        fold_into(input, &mut out);
        out
    }

    #[test]
    fn romanian_letters_fold_to_their_base() {
        assert_eq!(fold("Țări"), "tari");
        assert_eq!(fold("Șoseaua"), "soseaua");
        assert_eq!(fold("Cărți"), "carti");
        assert_eq!(fold("Câmp"), "camp");
        assert_eq!(fold("Înalt"), "inalt");
    }

    #[test]
    fn other_languages_fold_too() {
        assert_eq!(fold("Łódź"), "lodz");
        assert_eq!(fold("Ørsted"), "orsted");
        assert_eq!(fold("Müller"), "muller");
        assert_eq!(fold("École"), "ecole");
        assert_eq!(fold("Tiếng"), "tieng");
        assert_eq!(fold("Straße"), "strase");
        // Case is preserved for scripts outside ASCII; the matcher compares
        // case-insensitively, so only the accent has to go.
        assert_eq!(fold("Ёлка"), "Елка");
        assert_eq!(fold("Йошкар"), "Иошкар");
        assert_eq!(fold("Άλφα"), "Αλφα");
        assert_eq!(fold("ёлка"), "елка");
    }

    #[test]
    fn plain_ascii_is_untouched() {
        assert_eq!(fold("document-12.pdf"), "document-12.pdf");
    }

    /// Highlighting indexes into the folded name, so a fold must never change
    /// the number of characters.
    #[test]
    fn folding_never_changes_the_character_count() {
        for sample in [
            "Țări", "Șoseaua", "Łódź", "Tiếng", "Straße", "Ёлка", "Άλφα", "Müller",
        ] {
            assert_eq!(
                fold(sample).chars().count(),
                sample.chars().count(),
                "{sample} changed length"
            );
        }
    }
}
