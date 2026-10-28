//! Line breaking and punctuation rules (Kinsoku Shori / 禁則処理).

/// Returns true if the character is prohibited from starting a new line (行頭禁則).
#[inline]
pub fn is_prohibited_line_start(c: char) -> bool {
    matches!(
        c,
        // English / ASCII punctuation & closing brackets
        ',' | '.' | '!' | '?' | ':' | ';' | '%' | ')' | ']' | '}'
        // CJK punctuation & symbols
        | '，' | '。' | '、' | '！' | '？' | '：' | '；' | '％'
        // CJK closing brackets & quotation marks
        | '）' | '】' | '〉' | '》' | '」' | '』' | '〕' | '｝' | '］'
        | '’' | '”'
        // Dashes, ellipsis, and prolong marks
        | '…' | '—' | '～' | '·'
    )
}

/// Returns true if the character is prohibited from ending a line (行末禁则).
#[inline]
pub fn is_prohibited_line_end(c: char) -> bool {
    matches!(
        c,
        // English / ASCII opening brackets
        '(' | '[' | '{'
        // CJK opening brackets & quotation marks
        | '（' | '【' | '〈' | '《' | '「' | '『' | '〔' | '｛' | '［'
        | '‘' | '“'
    )
}

/// Returns true if the character is a CJK ideograph, kana, hangul, or fullwidth form.
#[inline]
pub fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{2E80}'..='\u{2EFF}'   // CJK Radicals Supplement
        | '\u{3000}'..='\u{303F}' // CJK Symbols and Punctuation
        | '\u{3040}'..='\u{309F}' // Hiragana
        | '\u{30A0}'..='\u{30FF}' // Katakana
        | '\u{3400}'..='\u{4DBF}' // CJK Unified Ideographs Extension A
        | '\u{4E00}'..='\u{9FFF}' // CJK Unified Ideographs
        | '\u{AC00}'..='\u{D7AF}' // Hangul Syllables
        | '\u{F900}'..='\u{FAFF}' // CJK Compatibility Ideographs
        | '\u{FF00}'..='\u{FFEF}' // Halfwidth and Fullwidth Forms
        | '\u{20000}'..='\u{323AF}' // Supplementary Ideographic Planes (Ext B-H)
    )
}

/// Returns true if a permissible line break opportunity exists between `prev` and `curr`.
#[inline]
pub fn can_break_between(prev: char, curr: char) -> bool {
    if curr.is_whitespace() || is_prohibited_line_end(prev) || is_prohibited_line_start(curr) {
        return false;
    }
    prev.is_whitespace() || is_cjk(prev) || is_cjk(curr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prohibited_start_characters() {
        assert!(is_prohibited_line_start('。'));
        assert!(is_prohibited_line_start('，'));
        assert!(is_prohibited_line_start('！'));
        assert!(is_prohibited_line_start('？'));
        assert!(is_prohibited_line_start('）'));
        assert!(is_prohibited_line_start('》'));
        assert!(is_prohibited_line_start('…'));
        assert!(is_prohibited_line_start('.'));
        assert!(is_prohibited_line_start(')'));
        assert!(!is_prohibited_line_start('中'));
        assert!(!is_prohibited_line_start('a'));
    }

    #[test]
    fn prohibited_end_characters() {
        assert!(is_prohibited_line_end('（'));
        assert!(is_prohibited_line_end('《'));
        assert!(is_prohibited_line_end('「'));
        assert!(is_prohibited_line_end('“'));
        assert!(is_prohibited_line_end('('));
        assert!(!is_prohibited_line_end('。'));
        assert!(!is_prohibited_line_end('中'));
    }

    #[test]
    fn break_opportunities() {
        // Between two CJK characters: allowed
        assert!(can_break_between('你', '好'));
        // Between CJK character and closing punctuation: disallowed
        assert!(!can_break_between('好', '。'));
        assert!(!can_break_between('好', '！'));
        assert!(!can_break_between('好', '）'));
        // Between opening punctuation and CJK character: disallowed
        assert!(!can_break_between('（', '好'));
        assert!(!can_break_between('《', '好'));
        // Between Latin letters: disallowed (keep words together)
        assert!(!can_break_between('H', 'e'));
        assert!(!can_break_between('e', 'l'));
        // After space before Latin word: allowed
        assert!(can_break_between(' ', 'w'));
        // Before space: disallowed (space should not begin line)
        assert!(!can_break_between('o', ' '));
        // Between multiple spaces: disallowed
        assert!(!can_break_between(' ', ' '));
    }
}
