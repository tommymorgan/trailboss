//! Token delimiters shared by text-row and rendered-cell word selection.

pub fn is_word_separator(ch: char) -> bool {
    ch.is_whitespace()
        || matches!(
            ch,
            '|' | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | ','
                | ';'
                | '!'
                | '（'
                | '）'
                | '：'
                | '、'
                | '。'
                | '，'
        )
}

pub fn is_leading_token_wrapper(ch: char) -> bool {
    matches!(ch, '(' | '[' | '{' | '<' | '"' | '\'' | '`')
}

pub fn is_trailing_token_wrapper(ch: char) -> bool {
    matches!(
        ch,
        ')' | ']' | '}' | '>' | '"' | '\'' | '`' | '.' | ',' | ';' | ':' | '!' | '?'
    )
}
