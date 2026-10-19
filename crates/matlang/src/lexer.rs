//! Tokenizer for M-code.
//!
//! M-code's `'` does double duty (string quote vs. transpose), so the lexer
//! tracks whether the previous token could end an operand: `'` after an
//! operand is transpose, otherwise it opens a string literal.

use crate::LangError;

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Num(f64),
    /// Imaginary literal `2i`/`3j` — a pure-imaginary scalar `v·i`.
    CNum(f64),
    Ident(String),
    Str(String),
    Plus,
    Minus,
    Star,
    Slash,
    Backslash,
    Caret,
    DotStar,
    DotSlash,
    DotCaret,
    /// `.` — struct field access (`s.field`).
    Dot,
    /// `.'` non-conjugate transpose.
    DotApos,
    /// `'` transpose.
    Apostrophe,
    Tilde,
    LParen,
    RParen,
    LBracket,
    RBracket,
    /// `{` — opens a cell literal or cell-content indexing.
    LBrace,
    /// `}` — closes them.
    RBrace,
    Comma,
    Semicolon,
    Colon,
    Newline,
    Eq,
    EqEq,
    Neq,
    Lt,
    Le,
    Gt,
    Ge,
    /// `&` element-wise and.
    Amp,
    /// `|` element-wise or.
    Pipe,
    /// `@` — function handle (`@sin`) or anonymous function (`@(x) expr`).
    At,
    /// `&&` short-circuit and.
    AmpAmp,
    /// `||` short-circuit or.
    PipePipe,
    /// `!` — shell escape at statement start (`!dir`, `!del f.txt`).
    Bang,
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Lexed {
    pub tok: Tok,
    pub line: usize,
    /// Whitespace (or a line start) preceded this token. The parser uses it
    /// for M-code's matrix-literal rule: `[a -b]` is two elements while
    /// `[a - b]` / `[a-b]` is subtraction.
    pub ws_before: bool,
}

/// Does this token allow a following `'` to mean transpose?
fn ends_operand(t: &Tok) -> bool {
    matches!(t, Tok::Num(_) | Tok::CNum(_) | Tok::Ident(_) | Tok::Str(_) | Tok::RParen | Tok::RBracket | Tok::RBrace | Tok::Apostrophe | Tok::DotApos)
}

pub fn lex(source: &str) -> Result<Vec<Lexed>, LangError> {
    let chars: Vec<char> = source.chars().collect();
    let mut toks: Vec<Lexed> = Vec::new();
    let mut i = 0usize;
    let mut line = 1usize;
    let mut prev_ends_operand = false;
    // Track whether whitespace ran before the next token.
    let mut pending_ws = false;
    // Nesting depth of `[`/`{` literals: inside them, a `'` preceded by
    // whitespace opens a string (`[1 'a']`), not a transpose.
    let mut literal_depth = 0usize;

    macro_rules! push {
        ($t:expr) => {{
            let t: Tok = $t;
            prev_ends_operand = ends_operand(&t);
            toks.push(Lexed { tok: t, line, ws_before: pending_ws });
            pending_ws = false;
        }};
    }

    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' | '\r' => {
                pending_ws = true;
                i += 1;
            }
            '\n' => {
                line += 1;
                i += 1;
                // Collapse runs of blank lines into one separator.
                if toks.last().map(|t| &t.tok) != Some(&Tok::Newline) {
                    prev_ends_operand = ends_operand(&Tok::Newline);
                    toks.push(Lexed { tok: Tok::Newline, line, ws_before: pending_ws });
                }
                // A newline counts as whitespace for the next token.
                pending_ws = true;
            }
            '%' => {
                // Comment to end of line.
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '.' if chars.get(i + 1) == Some(&'.') && chars.get(i + 2) == Some(&'.') => {
                // `...` line continuation: swallow the rest of the line
                // including the newline — no Newline token is emitted.
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                if chars.get(i) == Some(&'\n') {
                    i += 1;
                    line += 1;
                }
                // A continuation is whitespace at the join.
                pending_ws = true;
            }
            '.' => {
                let next = chars.get(i + 1).copied();
                match next {
                    Some('*') => {
                        i += 2;
                        push!(Tok::DotStar);
                    }
                    Some('/') => {
                        i += 2;
                        push!(Tok::DotSlash);
                    }
                    Some('^') => {
                        i += 2;
                        push!(Tok::DotCaret);
                    }
                    Some('\'') => {
                        i += 2;
                        push!(Tok::DotApos);
                    }
                    Some(d) if d.is_ascii_digit() => {
                        let (tok, ni) = lex_number(&chars, i, line)?;
                        i = ni;
                        push!(tok);
                    }
                    // Field access `s.field`; anything else (`1..2`, `a. `)
                    // reaches the parser as Dot and errors there.
                    _ => {
                        i += 1;
                        push!(Tok::Dot);
                    }
                }
            }
            '0'..='9' => {
                let (tok, ni) = lex_number(&chars, i, line)?;
                i = ni;
                push!(tok);
            }
            'a'..='z' | 'A'..='Z' | '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let name: String = chars[start..i].iter().collect();
                push!(Tok::Ident(name));
            }
            '\'' => {
                // `'` after an operand is transpose — unless a space
                // separates it from a command-syntax position: `save 'f.m'`,
                // `cmd arg 'x'` (any word of a statement that began with a
                // bare identifier), or a second quoted argument
                // `save 'a' 'b'` opens a string instead.
                // Command-argument position: scan back over the run of
                // word-like tokens (ident/str/num); if its head is an
                // identifier at a statement start, this `'` is a quoted
                // argument, not a transpose.
                let command_arg = {
                    let mut j = toks.len();
                    while j > 0 && matches!(toks[j - 1].tok, Tok::Ident(_) | Tok::Str(_) | Tok::Num(_)) {
                        j -= 1;
                    }
                    j < toks.len()
                        && matches!(toks[j].tok, Tok::Ident(_))
                        && (j == 0 || matches!(toks[j - 1].tok, Tok::Newline | Tok::Semicolon | Tok::Comma))
                };
                let open_string = !prev_ends_operand
                    || (pending_ws && matches!(toks.last().map(|l| &l.tok), Some(Tok::Str(_))))
                    || (pending_ws && command_arg)
                    || (pending_ws && literal_depth > 0);
                if !open_string {
                    i += 1;
                    push!(Tok::Apostrophe);
                } else {
                    // String literal; '' inside is an escaped quote.
                    i += 1;
                    let mut s = String::new();
                    loop {
                        match chars.get(i) {
                            Some('\'') if chars.get(i + 1) == Some(&'\'') => {
                                s.push('\'');
                                i += 2;
                            }
                            Some('\'') => {
                                i += 1;
                                break;
                            }
                            Some(&ch) => {
                                s.push(ch);
                                i += 1;
                            }
                            None => {
                                return Err(LangError::new("unterminated string", line));
                            }
                        }
                    }
                    push!(Tok::Str(s));
                }
            }
            '"' => {
                // `"…"` — the R2016b+ string form. Always a quote (never an
                // operator); `""` inside is an escaped quote. MatCraft
                // yields a char vector rather than a `string` object —
                // a documented deviation.
                i += 1;
                let mut s = String::new();
                loop {
                    match chars.get(i) {
                        Some('"') if chars.get(i + 1) == Some(&'"') => {
                            s.push('"');
                            i += 2;
                        }
                        Some('"') => {
                            i += 1;
                            break;
                        }
                        Some(&ch) => {
                            s.push(ch);
                            i += 1;
                        }
                        None => {
                            return Err(LangError::new("unterminated string", line));
                        }
                    }
                }
                push!(Tok::Str(s));
            }
            '#' if i == 0 && chars.get(1) == Some(&'!') => {
                // `#!…` shebang line (Octave-style script headers): a
                // comment through end of line when it opens the file.
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '+' => {
                i += 1;
                push!(Tok::Plus);
            }
            '-' => {
                i += 1;
                push!(Tok::Minus);
            }
            '*' => {
                i += 1;
                push!(Tok::Star);
            }
            '/' => {
                i += 1;
                push!(Tok::Slash);
            }
            '\\' => {
                i += 1;
                push!(Tok::Backslash);
            }
            '^' => {
                i += 1;
                push!(Tok::Caret);
            }
            '~' => {
                if chars.get(i + 1) == Some(&'=') {
                    i += 2;
                    push!(Tok::Neq);
                } else {
                    i += 1;
                    push!(Tok::Tilde);
                }
            }
            '(' => {
                i += 1;
                push!(Tok::LParen);
            }
            ')' => {
                i += 1;
                push!(Tok::RParen);
            }
            '[' => {
                i += 1;
                literal_depth += 1;
                push!(Tok::LBracket);
            }
            ']' => {
                i += 1;
                literal_depth = literal_depth.saturating_sub(1);
                push!(Tok::RBracket);
            }
            '{' => {
                i += 1;
                literal_depth += 1;
                push!(Tok::LBrace);
            }
            '}' => {
                i += 1;
                literal_depth = literal_depth.saturating_sub(1);
                push!(Tok::RBrace);
            }
            ',' => {
                i += 1;
                push!(Tok::Comma);
            }
            ';' => {
                i += 1;
                push!(Tok::Semicolon);
            }
            ':' => {
                i += 1;
                push!(Tok::Colon);
            }
            '=' => {
                if chars.get(i + 1) == Some(&'=') {
                    i += 2;
                    push!(Tok::EqEq);
                } else {
                    i += 1;
                    push!(Tok::Eq);
                }
            }
            '<' => {
                if chars.get(i + 1) == Some(&'=') {
                    i += 2;
                    push!(Tok::Le);
                } else {
                    i += 1;
                    push!(Tok::Lt);
                }
            }
            '>' => {
                if chars.get(i + 1) == Some(&'=') {
                    i += 2;
                    push!(Tok::Ge);
                } else {
                    i += 1;
                    push!(Tok::Gt);
                }
            }
            '&' => {
                if chars.get(i + 1) == Some(&'&') {
                    i += 2;
                    push!(Tok::AmpAmp);
                } else {
                    i += 1;
                    push!(Tok::Amp);
                }
            }
            '|' => {
                if chars.get(i + 1) == Some(&'|') {
                    i += 2;
                    push!(Tok::PipePipe);
                } else {
                    i += 1;
                    push!(Tok::Pipe);
                }
            }
            '@' => {
                i += 1;
                push!(Tok::At);
            }
            '!' => {
                i += 1;
                push!(Tok::Bang);
            }
            _ => {
                return Err(LangError::new(format!("unexpected character '{c}'"), line));
            }
        }
    }
    toks.push(Lexed { tok: Tok::Eof, line, ws_before: pending_ws });
    Ok(toks)
}

fn lex_number(chars: &[char], start: usize, line: usize) -> Result<(Tok, usize), LangError> {
    let mut i = start;
    let mut text = String::new();
    while i < chars.len() && chars[i].is_ascii_digit() {
        text.push(chars[i]);
        i += 1;
    }
    // Fractional part: consume '.' unless a second '.' follows (that would
    // be the start of a `...` continuation).
    if chars.get(i) == Some(&'.') && chars.get(i + 1) != Some(&'.') {
        text.push('.');
        i += 1;
        while i < chars.len() && chars[i].is_ascii_digit() {
            text.push(chars[i]);
            i += 1;
        }
    }
    // Exponent.
    if matches!(chars.get(i), Some('e') | Some('E')) {
        let mut j = i + 1;
        if matches!(chars.get(j), Some('+') | Some('-')) {
            j += 1;
        }
        if chars.get(j).map(|c| c.is_ascii_digit()) == Some(true) {
            text.push(chars[i]);
            i += 1;
            if matches!(chars.get(i), Some('+') | Some('-')) {
                text.push(chars[i]);
                i += 1;
            }
            while i < chars.len() && chars[i].is_ascii_digit() {
                text.push(chars[i]);
                i += 1;
            }
        }
    }
    match text.parse::<f64>() {
        Ok(v) => {
            // Glued `i`/`j` suffix: `2i` → pure imaginary 2·i. The suffix
            // must not run into an identifier (`2ix` is a syntax error,
            // not `2i * x`).
            if matches!(chars.get(i), Some('i') | Some('j')) && !chars.get(i + 1).map(|c| c.is_ascii_alphanumeric() || *c == '_').unwrap_or(false) {
                return Ok((Tok::CNum(v), i + 1));
            }
            Ok((Tok::Num(v), i))
        }
        Err(_) => Err(LangError::new(format!("bad number '{text}'"), line)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<Tok> {
        lex(src).unwrap().into_iter().map(|l| l.tok).collect()
    }

    #[test]
    fn transpose_vs_string() {
        let t = kinds("a' + 'hi'");
        assert!(t.contains(&Tok::Apostrophe));
        assert!(t.contains(&Tok::Str("hi".into())));
    }

    #[test]
    fn ranges_and_dot_ops() {
        let t = kinds("1:5 .* 2 .^ x");
        assert!(t.contains(&Tok::Colon));
        assert!(t.contains(&Tok::DotStar));
        assert!(t.contains(&Tok::DotCaret));
    }

    #[test]
    fn continuation_skips_newline() {
        let t = kinds("a = 1 + ...\n2");
        assert_eq!(t.iter().filter(|k| **k == Tok::Newline).count(), 0);
    }
}
