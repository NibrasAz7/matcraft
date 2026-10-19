//! Recursive-descent parser: tokens -> `Stmt`/`Expr` tree.
//!
//! Grammar sketch (low to high precedence):
//!   statement   := assign | expr | if | for | while
//!   expr        := range
//!   range       := compare (':' compare (':' compare)?)?
//!   compare     := add (('=='|'~='|'<'|'<='|'>'|'>=') add)?
//!   add         := mul (('+'|'-') mul)*
//!   mul         := power (('*'|'/'|'\'|'.*'|'./') power)*
//!   power       := postfix (('^'|'.^') unary)?           (right assoc)
//!   postfix     := unary ("'" | ".'")*
//!   unary       := ('-'|'+'|'~') unary | primary
//!   primary     := num | str | ident ['(' args ')'] | '(' expr ')'
//!                | '[' rows ']' | ':' | 'end'

use crate::ast::{BinOp, Expr, MultiTarget, PathStep, Stmt, UnOp};
use crate::lexer::{Lexed, Tok};
use crate::LangError;

const MAX_DEPTH: usize = 256;

/// Octave-dialect block closers. Each is accepted wherever `end` closes a
/// block, and a stray one at statement level parses as a no-op.
const END_VARIANTS: &[&str] = &[
    "endfunction",
    "endif",
    "endfor",
    "endwhile",
    "endswitch",
    "end_try_catch",
    "endproperties",
    "endmethods",
    "endevents",
    "endenumeration",
    "endclassdef",
];

pub fn parse(source: &str) -> Result<Vec<Stmt>, LangError> {
    Ok(parse_lines(source)?.into_iter().map(|(_, s)| s).collect())
}

/// Like [`parse`] but pairs each top-level statement with its 1-based source
/// line — the interpreter stamps it onto runtime errors so `Error: line 3`
/// messages can link back into the editor.
pub fn parse_lines(source: &str) -> Result<Vec<(usize, Stmt)>, LangError> {
    let toks = crate::lexer::lex(source)?;
    let mut p = Parser { toks, pos: 0, elem_break: false };
    let mut out = Vec::new();
    p.skip_separators();
    while p.peek() != &Tok::Eof {
        let line = p.line();
        out.push((line, p.statement(0)?));
        p.skip_separators();
    }
    Ok(out)
}

struct Parser {
    toks: Vec<Lexed>,
    pos: usize,
    /// True while parsing an element directly inside a `[a b; c d]` row.
    /// Lets `add()` honour M-code's whitespace rule: `a -b` breaks the
    /// element (the `-` starts a unary-signed element), while `a - b` and
    /// `a-b` stay subtraction. The same rule ends an element before a
    /// whitespace-separated `(`/`{`: `{x {y}}` is two cells, `x{y}`
    /// attached is indexing.
    elem_break: bool,
}

impl Parser {
    fn peek(&self) -> &Tok {
        self.toks.get(self.pos).map(|l| &l.tok).unwrap_or(&Tok::Eof)
    }

    fn peek2(&self) -> &Tok {
        self.toks.get(self.pos + 1).map(|l| &l.tok).unwrap_or(&Tok::Eof)
    }

    fn peek3(&self) -> &Tok {
        self.toks.get(self.pos + 2).map(|l| &l.tok).unwrap_or(&Tok::Eof)
    }

    /// Whitespace preceded the token at `pos`?
    fn ws_before(&self, pos: usize) -> bool {
        self.toks.get(pos).map(|l| l.ws_before).unwrap_or(false)
    }

    fn line(&self) -> usize {
        self.toks.get(self.pos).map(|l| l.line).unwrap_or(0)
    }

    fn next(&mut self) -> Tok {
        let t = self.peek().clone();
        if self.pos < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    fn err<T>(&self, msg: impl Into<String>) -> Result<T, LangError> {
        Err(LangError::new(msg, self.line()))
    }

    /// Consume every token through the end of the line and return the raw
    /// text — for `!cmd` shell escapes. Whitespace between tokens is kept
    /// as single spaces; `;`/`,` are literal text here, not separators.
    fn line_text(&mut self) -> String {
        let mut out = String::new();
        loop {
            match self.peek() {
                Tok::Newline | Tok::Eof => break,
                t => {
                    let t = t.clone();
                    if !out.is_empty() && self.ws_before(self.pos) {
                        out.push(' ');
                    }
                    self.next();
                    out.push_str(&tok_text(&t).unwrap_or_default());
                }
            }
        }
        out
    }

    fn expect(&mut self, t: &Tok) -> Result<(), LangError> {
        if self.peek() == t {
            self.next();
            Ok(())
        } else {
            self.err(format!("expected {:?}, found {:?}", t, self.peek()))
        }
    }

    fn at_ident(&self, kw: &str) -> bool {
        matches!(self.peek(), Tok::Ident(n) if n == kw)
    }

    fn skip_separators(&mut self) {
        while matches!(self.peek(), Tok::Newline | Tok::Semicolon | Tok::Comma) {
            self.next();
        }
    }

    /// Consume the separator ending a statement; `true` if it was `;`
    /// (suppress echo). A following block keyword also ends the statement.
    fn stmt_end(&mut self) -> Result<bool, LangError> {
        match self.peek() {
            Tok::Semicolon => {
                self.next();
                Ok(true)
            }
            Tok::Newline | Tok::Comma => {
                self.next();
                Ok(false)
            }
            Tok::Eof => Ok(false),
            Tok::Ident(k) if matches!(k.as_str(), "end" | "else" | "elseif" | "case" | "otherwise") => Ok(false),
            t => self.err(format!("unexpected {:?} after statement", t)),
        }
    }

    fn statement(&mut self, depth: usize) -> Result<Stmt, LangError> {
        if depth > MAX_DEPTH {
            return self.err("nesting too deep");
        }
        if self.at_ident("if") {
            return self.parse_if(depth);
        }
        if self.at_ident("for") || self.at_ident("parfor") {
            // `parfor` parses as `for` — a sequential interpreter runs the
            // same iterations, just without the parallel workers.
            return self.parse_for(depth);
        }
        if self.peek() == &Tok::Bang {
            // `!cmd …` — shell escape: the rest of the line is the command
            // string, equivalent to `system('cmd …')` with the echo
            // suppressed.
            self.next();
            let cmd = self.line_text();
            let _ = self.stmt_end()?;
            // Shell output is the visible result; there is no `ans`.
            return Ok(Stmt::Expr(Expr::CallOrIndex("system".into(), vec![Expr::Str(cmd)]), true));
        }
        if self.at_ident("while") {
            return self.parse_while(depth);
        }
        if self.at_ident("switch") {
            return self.parse_switch(depth);
        }
        for (kw, stmt) in [("break", Stmt::Break), ("continue", Stmt::Continue), ("return", Stmt::Return)] {
            if self.at_ident(kw) {
                self.next();
                let _ = self.stmt_end()?;
                return Ok(stmt);
            }
        }
        // A stray Octave-dialect closer (`endfunction`, `endif`, …) where a
        // statement was expected — inside a block it's consumed as `end`
        // before we get here; at top level it parses as a no-op.
        if let Tok::Ident(k) = self.peek() {
            if END_VARIANTS.contains(&k.as_str()) {
                self.next();
                let _ = self.stmt_end()?;
                return Ok(Stmt::Noop);
            }
        }
        if self.at_ident("function") {
            return self.parse_function(depth);
        }
        if self.at_ident("try") {
            self.next();
            self.skip_inline_separators();
            let try_body = self.block(&["catch", "end"], depth)?;
            let (catch_var, catch_body) = if self.at_ident("catch") {
                self.next();
                // `catch e` binds the exception; bare `catch` doesn't.
                // An identifier directly after `catch` is the variable.
                let var = match self.peek() {
                    Tok::Ident(n) => {
                        let n = n.clone();
                        self.next();
                        Some(n)
                    }
                    _ => None,
                };
                self.skip_inline_separators();
                (var, self.block(&["end"], depth)?)
            } else {
                (None, Vec::new())
            };
            self.close_block()?;
            return Ok(Stmt::TryCatch { try_body, catch_var, catch_body });
        }
        // Command syntax: `name arg arg ...` means `name('arg','arg',...)`.
        // Triggered when a statement starts with Ident followed by a word-ish
        // token (Ident/Num/Str), e.g. `format long`, `clear x`, `disp hello`.
        if let Tok::Ident(name) = self.peek().clone() {
            // `cd ..` / `cd ../x`: a whitespace-separated `..`/`./`/`..\` is a
            // command argument, not field access (glued `x.y` has no ws_before).
            let dot_path = matches!(self.peek2(), Tok::Dot)
                && self.ws_before(self.pos + 1)
                && matches!(self.peek3(), Tok::Dot | Tok::Slash | Tok::Backslash | Tok::DotSlash);
            if matches!(self.peek2(), Tok::Ident(_) | Tok::Num(_) | Tok::Str(_)) || dot_path {
                self.next();
                let mut args = Vec::new();
                loop {
                    // Command args are whitespace-separated *words*, so a
                    // glued run like `f.txt`, `a-b`, `x+1` or `-ascii` is
                    // one string (a bare `-` is the literal arg "-").
                    let mut word = match self.peek() {
                        Tok::Ident(s) | Tok::Str(s) => {
                            let s = s.clone();
                            self.next();
                            s
                        }
                        Tok::Num(v) => {
                            let v = *v;
                            self.next();
                            crate::interp::fmt_num(v, false).to_string()
                        }
                        Tok::Minus => {
                            self.next();
                            "-".to_string()
                        }
                        // Leading `.` of a `..`/`./` path arg; the glued-continuation
                        // loop below absorbs the rest (`..`, `../x`, `..\x`).
                        Tok::Dot => {
                            self.next();
                            ".".to_string()
                        }
                        // A bracketed word (`im subplot [1 2 3]` passes the
                        // literal text `[1`, `2`, `3]`).
                        Tok::LBracket | Tok::LParen => {
                            let t = self.next();
                            tok_text(&t).unwrap_or_default()
                        }
                        _ => break,
                    };
                    // Absorb glued continuations (no whitespace between
                    // tokens): `save b5.txt -ascii`, `load a-b.txt`,
                    // `ft_warning once FieldTrip:fileio:id`, `cd ../..`.
                    while !self.ws_before(self.pos) {
                        match self.peek() {
                            Tok::Ident(s) | Tok::Str(s) => {
                                let s = s.clone();
                                self.next();
                                word.push_str(&s);
                            }
                            Tok::Num(v) => {
                                let v = *v;
                                self.next();
                                word.push_str(&crate::interp::fmt_num(v, false).to_string());
                            }
                            Tok::Comma | Tok::Semicolon | Tok::Newline | Tok::Eof => break,
                            t => {
                                let t = t.clone();
                                match tok_text(&t) {
                                    Some(p) => {
                                        self.next();
                                        word.push_str(&p);
                                    }
                                    None => break,
                                }
                            }
                        }
                    }
                    args.push(Expr::Str(word));
                }
                let _ = self.stmt_end()?;
                return Ok(Stmt::Expr(Expr::CallOrIndex(name, args), true));
            }
        }
        // General statement: expression, optionally `lhs = rhs`.
        let lhs = self.expr(depth)?;
        if self.peek() == &Tok::Eq {
            self.next();
            let value = self.expr(depth)?;
            let suppress = self.stmt_end()?;
            return match lhs {
                Expr::Ident(name) => Ok(Stmt::Assign { name, index: None, braces: false, value, suppress }),
                Expr::CallOrIndex(name, index) => Ok(Stmt::Assign { name, index: Some(index), braces: false, value, suppress }),
                // `c{i} = v` — cell content assignment.
                Expr::BraceIndex(name, index) => Ok(Stmt::Assign { name, index: Some(index), braces: true, value, suppress }),
                // `s.f = v`, `s.a.b = v`, `s.f(i) = v`, `s.c{i} = v` — field
                // assignment.
                e @ (Expr::Field(..) | Expr::DynField(..) | Expr::Index(..) | Expr::BraceIdx(..)) => match field_lhs(e) {
                    Some((name, path)) => Ok(Stmt::FieldAssign { name, path, value, suppress }),
                    None => self.err("left side of '=' must be a variable or indexed variable"),
                },
                // `[a, b] = f(x)` — a bracketed list of targets. Each slot
                // is a plain name or an lvalue path (`s.f`, `v(i)`,
                // `c{1}.g`) — M-code assigns through those directly.
                Expr::Matrix(rows) if rows.len() == 1 && rows[0].iter().all(|c| matches!(c, Expr::Ident(_)) || field_lhs(c.clone()).is_some()) => {
                    let targets = rows[0]
                        .iter()
                        .map(|c| match c {
                            Expr::Ident(n) => MultiTarget::Name(n.clone()),
                            e => match field_lhs(e.clone()) {
                                Some((n, path)) if !path.is_empty() => MultiTarget::Path(n, path),
                                Some((n, _)) => MultiTarget::Name(n),
                                None => MultiTarget::Name(String::new()),
                            },
                        })
                        .collect();
                    Ok(Stmt::AssignMany { targets, value, suppress })
                }
                _ => self.err("left side of '=' must be a variable or indexed variable"),
            };
        }
        let suppress = self.stmt_end()?;
        Ok(Stmt::Expr(lhs, suppress))
    }

    /// Does `k` terminate a block expecting one of `keywords`? `end` and
    /// the Octave-dialect `end<kind>` variants are interchangeable.
    fn closes_block(k: &str, keywords: &[&str]) -> bool {
        keywords.contains(&k) || (keywords.contains(&"end") && END_VARIANTS.contains(&k))
    }

    /// Parse a statement block until one of `keywords` (not consumed).
    fn block(&mut self, keywords: &[&str], depth: usize) -> Result<Vec<Stmt>, LangError> {
        let mut out = Vec::new();
        loop {
            self.skip_separators();
            match self.peek() {
                Tok::Ident(k) if Self::closes_block(k, keywords) => return Ok(out),
                Tok::Eof => return self.err(format!("missing 'end' (expected one of {keywords:?})")),
                _ => out.push(self.statement(depth + 1)?),
            }
        }
    }

    fn close_block(&mut self) -> Result<(), LangError> {
        match self.peek() {
            Tok::Ident(k) if k == "end" || END_VARIANTS.contains(&k.as_str()) => {
                self.next();
                Ok(())
            }
            _ => self.err("expected 'end'"),
        }
    }

    fn parse_if(&mut self, depth: usize) -> Result<Stmt, LangError> {
        self.next(); // 'if'
        let cond = self.expr(depth)?;
        self.skip_inline_separators();
        let mut arms = vec![(Some(cond), self.block(&["elseif", "else", "end"], depth)?)];
        while self.at_ident("elseif") {
            self.next();
            let c = self.expr(depth)?;
            self.skip_inline_separators();
            let body = self.block(&["elseif", "else", "end"], depth)?;
            arms.push((Some(c), body));
        }
        if self.at_ident("else") {
            self.next();
            self.skip_inline_separators();
            let body = self.block(&["end"], depth)?;
            arms.push((None, body));
        }
        self.close_block()?;
        Ok(Stmt::If { arms })
    }

    fn parse_for(&mut self, depth: usize) -> Result<Stmt, LangError> {
        self.next(); // 'for'
                     // `for(i=1:n)` — MATLAB accepts parens around the loop header.
        let paren = matches!(self.peek(), Tok::LParen);
        if paren {
            self.next();
        }
        let var = match self.next() {
            Tok::Ident(n) => n,
            t => return self.err(format!("expected loop variable, found {t:?}")),
        };
        self.expect(&Tok::Eq)?;
        let iter = self.expr(depth)?;
        if paren {
            self.expect(&Tok::RParen)?;
        }
        self.skip_inline_separators();
        let body = self.block(&["end"], depth)?;
        self.close_block()?;
        Ok(Stmt::For { var, iter, body })
    }

    fn parse_while(&mut self, depth: usize) -> Result<Stmt, LangError> {
        self.next(); // 'while'
        let cond = self.expr(depth)?;
        self.skip_inline_separators();
        let body = self.block(&["end"], depth)?;
        self.close_block()?;
        Ok(Stmt::While { cond, body })
    }

    fn parse_switch(&mut self, depth: usize) -> Result<Stmt, LangError> {
        self.next(); // 'switch'
        let expr = self.expr(depth)?;
        self.skip_inline_separators();
        let mut cases: Vec<(Option<Expr>, Vec<Stmt>)> = Vec::new();
        loop {
            self.skip_separators();
            if self.at_ident("case") {
                self.next();
                let c = self.expr(depth)?;
                self.skip_inline_separators();
                let body = self.block(&["case", "otherwise", "end"], depth)?;
                cases.push((Some(c), body));
            } else if self.at_ident("otherwise") {
                self.next();
                self.skip_inline_separators();
                let body = self.block(&["end"], depth)?;
                cases.push((None, body));
            } else {
                break;
            }
        }
        self.close_block()?;
        Ok(Stmt::Switch { expr, cases })
    }

    /// `function [a b] = name(p, q)` / `function y = name(p)` / `function name`
    /// — `end` is optional at file scope (next `function` or EOF closes it).
    fn parse_function(&mut self, depth: usize) -> Result<Stmt, LangError> {
        self.next(); // 'function'
        let mut outputs = Vec::new();
        if self.peek() == &Tok::LBracket {
            self.next();
            loop {
                match self.next() {
                    Tok::Ident(n) => outputs.push(n),
                    // `[~, b]` — ignored output slot.
                    Tok::Tilde => outputs.push("~".into()),
                    Tok::RBracket => break,
                    Tok::Comma | Tok::Newline => {}
                    t => return self.err(format!("expected output name or ']', found {t:?}")),
                }
            }
            self.expect(&Tok::Eq)?;
        } else if matches!(self.peek(), Tok::Ident(_)) && self.peek2() == &Tok::Eq {
            if let Tok::Ident(n) = self.next() {
                outputs.push(n);
            }
            self.next(); // '='
        }
        let name = match self.next() {
            Tok::Ident(n) => n,
            t => return self.err(format!("expected function name, found {t:?}")),
        };
        let mut params = Vec::new();
        if self.peek() == &Tok::LParen {
            self.next();
            loop {
                match self.next() {
                    Tok::Ident(n) => params.push(n),
                    // `f(~, b)` — ignored input slot.
                    Tok::Tilde => params.push("~".into()),
                    Tok::RParen => break,
                    Tok::Comma => {}
                    t => return self.err(format!("expected parameter or ')', found {t:?}")),
                }
            }
        }
        self.skip_inline_separators();
        // `end` optional: the body also terminates at 'function' or EOF.
        let body = self.block_or_eof(&["end", "function"], depth)?;
        if self.at_ident("end") {
            self.next();
        }
        Ok(Stmt::Function { name, params, outputs, body })
    }

    /// Like `block`, but EOF also terminates (for top-level function bodies).
    fn block_or_eof(&mut self, keywords: &[&str], depth: usize) -> Result<Vec<Stmt>, LangError> {
        let mut out = Vec::new();
        loop {
            self.skip_separators();
            match self.peek() {
                Tok::Ident(k) if Self::closes_block(k, keywords) => return Ok(out),
                Tok::Eof => return Ok(out),
                _ => out.push(self.statement(depth + 1)?),
            }
        }
    }

    /// `,`/`;` may sit between a control header and its body newline.
    fn skip_inline_separators(&mut self) {
        while matches!(self.peek(), Tok::Comma | Tok::Semicolon) {
            self.next();
        }
    }

    fn expr(&mut self, depth: usize) -> Result<Expr, LangError> {
        self.range(depth)
    }

    fn range(&mut self, depth: usize) -> Result<Expr, LangError> {
        let first = self.logical_or(depth)?;
        if self.peek() != &Tok::Colon {
            return Ok(first);
        }
        self.next();
        let second = self.compare(depth)?;
        if self.peek() == &Tok::Colon {
            self.next();
            let last = self.compare(depth)?;
            Ok(Expr::Range(Box::new(first), Some(Box::new(second)), Box::new(last)))
        } else {
            Ok(Expr::Range(Box::new(first), None, Box::new(second)))
        }
    }

    /// `||` — lowest precedence inside an expression.
    fn logical_or(&mut self, depth: usize) -> Result<Expr, LangError> {
        let mut left = self.logical_and(depth)?;
        while self.peek() == &Tok::PipePipe {
            self.next();
            let right = self.logical_and(depth)?;
            left = Expr::Binary(BinOp::ScOr, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    /// `&&`
    fn logical_and(&mut self, depth: usize) -> Result<Expr, LangError> {
        let mut left = self.elwise_or(depth)?;
        while self.peek() == &Tok::AmpAmp {
            self.next();
            let right = self.elwise_or(depth)?;
            left = Expr::Binary(BinOp::ScAnd, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    /// `|` element-wise or.
    fn elwise_or(&mut self, depth: usize) -> Result<Expr, LangError> {
        let mut left = self.elwise_and(depth)?;
        while self.peek() == &Tok::Pipe {
            self.next();
            let right = self.elwise_and(depth)?;
            left = Expr::Binary(BinOp::Or, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    /// `&` element-wise and.
    fn elwise_and(&mut self, depth: usize) -> Result<Expr, LangError> {
        let mut left = self.compare(depth)?;
        while self.peek() == &Tok::Amp {
            self.next();
            let right = self.compare(depth)?;
            left = Expr::Binary(BinOp::And, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn compare(&mut self, depth: usize) -> Result<Expr, LangError> {
        let left = self.add(depth)?;
        let op = match self.peek() {
            Tok::EqEq => BinOp::Eq,
            Tok::Neq => BinOp::Neq,
            Tok::Lt => BinOp::Lt,
            Tok::Le => BinOp::Le,
            Tok::Gt => BinOp::Gt,
            Tok::Ge => BinOp::Ge,
            _ => return Ok(left),
        };
        self.next();
        let right = self.add(depth)?;
        Ok(Expr::Binary(op, Box::new(left), Box::new(right)))
    }

    fn add(&mut self, depth: usize) -> Result<Expr, LangError> {
        let mut left = self.mul(depth)?;
        loop {
            let op = match self.peek() {
                Tok::Plus => BinOp::Add,
                Tok::Minus => BinOp::Sub,
                _ => return Ok(left),
            };
            // `[a -b]`: space before the sign, none after → new element.
            if self.elem_break && self.ws_before(self.pos) && !self.ws_before(self.pos + 1) {
                return Ok(left);
            }
            self.next();
            let right = self.mul(depth)?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
    }

    fn mul(&mut self, depth: usize) -> Result<Expr, LangError> {
        let mut left = self.power(depth)?;
        loop {
            let op = match self.peek() {
                Tok::Star => BinOp::Mul,
                Tok::Slash => BinOp::Div,
                Tok::Backslash => BinOp::Backslash,
                Tok::DotStar => BinOp::ElemMul,
                Tok::DotSlash => BinOp::ElemDiv,
                _ => return Ok(left),
            };
            self.next();
            let right = self.power(depth)?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
    }

    fn power(&mut self, depth: usize) -> Result<Expr, LangError> {
        let base = self.postfix(depth)?;
        let op = match self.peek() {
            Tok::Caret => BinOp::Pow,
            Tok::DotCaret => BinOp::ElemPow,
            _ => return Ok(base),
        };
        self.next();
        let exp = self.power_rhs(depth)?;
        Ok(Expr::Binary(op, Box::new(base), Box::new(exp)))
    }

    /// Exponent of `^`/`.^`: a leading sign binds inside (`x^-2` =
    /// `x^(-2)`), postfix ops apply (`x.^s.d`, `(x.^2)'`), and the tail is
    /// right-associative (`a^b^c` = `a^(b^c)`).
    fn power_rhs(&mut self, depth: usize) -> Result<Expr, LangError> {
        match self.peek() {
            Tok::Minus => {
                self.next();
                Ok(Expr::Unary(UnOp::Neg, Box::new(self.power_rhs(depth + 1)?)))
            }
            Tok::Plus => {
                self.next();
                Ok(Expr::Unary(UnOp::Pos, Box::new(self.power_rhs(depth + 1)?)))
            }
            Tok::Tilde => {
                self.next();
                Ok(Expr::Unary(UnOp::Not, Box::new(self.power_rhs(depth + 1)?)))
            }
            _ => self.power(depth + 1),
        }
    }

    /// `(a, b, …)` or `{a, b, …}` argument/index list — the open token has
    /// been peeked, not consumed.
    fn arg_list(&mut self, depth: usize, close: &Tok) -> Result<Vec<Expr>, LangError> {
        self.next(); // opening bracket
        let mut args = Vec::new();
        let saved = self.elem_break;
        self.elem_break = false;
        let res = (|| -> Result<Vec<Expr>, LangError> {
            self.skip_newlines();
            if self.peek() != close {
                loop {
                    args.push(self.expr(depth + 1)?);
                    self.skip_newlines();
                    let t = self.next();
                    match t {
                        Tok::Comma => self.skip_newlines(),
                        _ if &t == close => break,
                        _ => return self.err(format!("expected ',' or {close:?}, found {t:?}")),
                    }
                }
            } else {
                self.next();
            }
            Ok(args)
        })();
        self.elem_break = saved;
        res
    }

    fn postfix(&mut self, depth: usize) -> Result<Expr, LangError> {
        let mut e = self.unary(depth)?;
        loop {
            // Inside a `[…]`/`{…}` literal, a `(`/`{` preceded by
            // whitespace opens the next element — subscripts must be
            // attached to their target: `{c{1} {2}}` is `{c{1}, {2}}`.
            if self.elem_break && matches!(self.peek(), Tok::LParen | Tok::LBrace) && self.ws_before(self.pos) {
                break;
            }
            match self.peek() {
                Tok::Apostrophe | Tok::DotApos => {
                    // `'` conjugates, `.'` doesn't — matters once complex.
                    let conj = self.peek() == &Tok::Apostrophe;
                    self.next();
                    e = Expr::Transpose(Box::new(e), conj);
                }
                Tok::Dot => {
                    self.next();
                    match self.next() {
                        Tok::Ident(f) => e = Expr::Field(Box::new(e), f),
                        // `s.(expr)` — dynamic field name.
                        Tok::LParen => {
                            let name = self.expr(depth + 1)?;
                            match self.next() {
                                Tok::RParen => e = Expr::DynField(Box::new(e), Box::new(name)),
                                t => return self.err(format!("expected ')' in dynamic field, found {t:?}")),
                            }
                        }
                        t => return self.err(format!("expected field name after '.', found {t:?}")),
                    }
                }
                // `expr(...)`/`expr{...}` — chained subscripts on a non-name
                // expression (`c{1}(2)`, `s.f(i)` reads). `name(...)` is
                // already consumed by `primary` as CallOrIndex/BraceIndex.
                Tok::LParen => {
                    let args = self.arg_list(depth, &Tok::RParen)?;
                    e = Expr::Index(Box::new(e), args);
                }
                Tok::LBrace => {
                    let args = self.arg_list(depth, &Tok::RBrace)?;
                    e = Expr::BraceIdx(Box::new(e), args);
                }
                _ => break,
            }
        }
        Ok(e)
    }

    fn unary(&mut self, depth: usize) -> Result<Expr, LangError> {
        match self.peek() {
            Tok::Minus => {
                self.next();
                Ok(Expr::Unary(UnOp::Neg, Box::new(self.unary(depth)?)))
            }
            Tok::Plus => {
                self.next();
                Ok(Expr::Unary(UnOp::Pos, Box::new(self.unary(depth)?)))
            }
            Tok::Tilde => {
                // Bare `~` before a separator is the ignored-output
                // placeholder (`[~, b] = f()`), not logical not.
                if matches!(self.peek2(), Tok::Comma | Tok::RBracket | Tok::Semicolon | Tok::Newline | Tok::Eof) {
                    self.next();
                    return Ok(Expr::Ident("~".into()));
                }
                self.next();
                Ok(Expr::Unary(UnOp::Not, Box::new(self.unary(depth)?)))
            }
            _ => self.primary(depth),
        }
    }

    fn primary(&mut self, depth: usize) -> Result<Expr, LangError> {
        if depth > MAX_DEPTH {
            return self.err("expression too deep");
        }
        match self.next() {
            Tok::Num(v) => Ok(Expr::Num(v)),
            Tok::CNum(v) => Ok(Expr::CNum(v)),
            Tok::Str(s) => Ok(Expr::Str(s)),
            Tok::Colon => Ok(Expr::Colon),
            Tok::Ident(name) => {
                if name == "end" {
                    return Ok(Expr::End);
                }
                // `name(`/`name{` indexes only when the bracket is
                // attached — inside a literal, `c {9}` leaves `c` as a
                // bare element and the `{` starts a new cell.
                let detached = self.elem_break && self.ws_before(self.pos);
                if self.peek() == &Tok::LParen && !detached {
                    let args = self.arg_list(depth, &Tok::RParen)?;
                    Ok(Expr::CallOrIndex(name, args))
                } else if self.peek() == &Tok::LBrace && !detached {
                    // `c{i, j}` — cell content indexing.
                    let args = self.arg_list(depth, &Tok::RBrace)?;
                    Ok(Expr::BraceIndex(name, args))
                } else {
                    Ok(Expr::Ident(name))
                }
            }
            Tok::LParen => {
                self.skip_newlines();
                let saved = self.elem_break;
                self.elem_break = false;
                let e = self.expr(depth + 1);
                self.elem_break = saved;
                let e = e?;
                self.skip_newlines();
                self.expect(&Tok::RParen)?;
                Ok(e)
            }
            Tok::LBracket => Ok(Expr::Matrix(self.table_literal(depth, &Tok::RBracket)?)),
            Tok::LBrace => Ok(Expr::CellMat(self.table_literal(depth, &Tok::RBrace)?)),
            Tok::At => match self.next() {
                // `@name` — handle to a named function or builtin.
                Tok::Ident(name) => Ok(Expr::FuncRef(name)),
                // `@(a, b) expr` — anonymous function. The body grabs the
                // longest valid expression (M-code rule); `,`/`;`/`)` end it.
                Tok::LParen => {
                    let mut params = Vec::new();
                    loop {
                        match self.next() {
                            Tok::Ident(n) => params.push(n),
                            Tok::Comma | Tok::Newline => {}
                            Tok::RParen => break,
                            t => return self.err(format!("expected parameter or ')' in anonymous function, found {t:?}")),
                        }
                    }
                    let body = self.expr(depth + 1)?;
                    Ok(Expr::AnonFunc(params, Box::new(body)))
                }
                t => self.err(format!("expected a name or '(args)' after '@', found {t:?}")),
            },
            t => self.err(format!("unexpected {t:?}")),
        }
    }

    fn skip_newlines(&mut self) {
        while self.peek() == &Tok::Newline {
            self.next();
        }
    }

    /// `[a b c; d e f]` / `{a b; c d}` — columns split on `,` or
    /// whitespace, rows on `;` or newline. `close` is `]` or `}`.
    fn table_literal(&mut self, depth: usize, close: &Tok) -> Result<Vec<Vec<Expr>>, LangError> {
        let mut rows: Vec<Vec<Expr>> = Vec::new();
        let mut row: Vec<Expr> = Vec::new();
        self.skip_newlines();
        loop {
            match self.peek() {
                t if t == close => {
                    self.next();
                    if !row.is_empty() {
                        rows.push(row);
                    }
                    return Ok(rows);
                }
                Tok::Semicolon | Tok::Newline => {
                    self.next();
                    if !row.is_empty() {
                        rows.push(std::mem::take(&mut row));
                    }
                    self.skip_newlines();
                }
                Tok::Comma => {
                    self.next();
                }
                Tok::Eof => return self.err(format!("unterminated '{close:?}'")),
                _ => {
                    let saved = self.elem_break;
                    self.elem_break = true;
                    let e = self.expr(depth + 1);
                    self.elem_break = saved;
                    row.push(e?);
                }
            }
        }
    }
}

/// Convert a parsed LHS expression to `(name, field-path, index-ops)` for
/// `s.f = v` / `s.a.b = v` / `s.f(i) = v` / `s.c{i} = v` and chained
/// subscripts like `s.c{1}(i) = v` / `c{1}{2} = v`; `None` when it isn't
/// a field/subscript lvalue (e.g. `f(x).g` — call results can't be
/// assigned through).
///
/// `(root name, step path)` for a field- or chain-assignment LHS.
/// Field steps and subscript steps interleave, in left-to-right order:
/// `s.c{1}.f` → `[Name(c), Brace(1), Name(f)]`, `c{1}(2)` →
/// `[Brace(1), Paren(2)]` rooted at `c`.
type FieldLhs = (String, Vec<PathStep>);

/// Surface text for a token — used when raw source is needed back for a
/// command-syntax word or a `!` shell-escape line. `None` for tokens that
/// have no sensible spelling in that context.
fn tok_text(t: &Tok) -> Option<String> {
    Some(match t {
        Tok::Num(v) => crate::interp::fmt_num(*v, false).to_string(),
        Tok::CNum(v) => format!("{}i", crate::interp::fmt_num(*v, false)),
        Tok::Ident(s) | Tok::Str(s) => s.clone(),
        Tok::Plus => "+".into(),
        Tok::Minus => "-".into(),
        Tok::Star => "*".into(),
        Tok::Slash => "/".into(),
        Tok::Backslash => "\\".into(),
        Tok::Caret => "^".into(),
        Tok::DotStar => ".*".into(),
        Tok::DotSlash => "./".into(),
        Tok::DotCaret => ".^".into(),
        Tok::Dot => ".".into(),
        Tok::DotApos => ".'".into(),
        Tok::Apostrophe => "'".into(),
        Tok::Tilde => "~".into(),
        Tok::LParen => "(".into(),
        Tok::RParen => ")".into(),
        Tok::LBracket => "[".into(),
        Tok::RBracket => "]".into(),
        Tok::LBrace => "{".into(),
        Tok::RBrace => "}".into(),
        Tok::Comma => ",".into(),
        Tok::Semicolon => ";".into(),
        Tok::Colon => ":".into(),
        Tok::Eq => "=".into(),
        Tok::EqEq => "==".into(),
        Tok::Neq => "~=".into(),
        Tok::Lt => "<".into(),
        Tok::Le => "<=".into(),
        Tok::Gt => ">".into(),
        Tok::Ge => ">=".into(),
        Tok::Amp => "&".into(),
        Tok::Pipe => "|".into(),
        Tok::At => "@".into(),
        Tok::AmpAmp => "&&".into(),
        Tok::PipePipe => "||".into(),
        Tok::Bang => "!".into(),
        Tok::Newline | Tok::Eof => return None,
    })
}

fn field_lhs(e: Expr) -> Option<FieldLhs> {
    // Walk the expr outward→inward collecting steps, then reverse so the
    // list reads left-to-right as written.
    let mut steps: Vec<PathStep> = Vec::new();
    let mut cur = e;
    loop {
        cur = match cur {
            Expr::Field(inner, f) => {
                steps.push(PathStep::Name(f));
                *inner
            }
            Expr::DynField(inner, f) => {
                steps.push(PathStep::Dyn(*f));
                *inner
            }
            Expr::Index(inner, ix) => {
                steps.push(PathStep::Paren(ix));
                *inner
            }
            Expr::BraceIdx(inner, ix) => {
                steps.push(PathStep::Brace(ix));
                *inner
            }
            Expr::CallOrIndex(n, ix) => {
                steps.push(PathStep::Paren(ix));
                steps.reverse();
                return Some((n, steps));
            }
            Expr::BraceIndex(n, ix) => {
                steps.push(PathStep::Brace(ix));
                steps.reverse();
                return Some((n, steps));
            }
            Expr::Ident(n) => {
                steps.reverse();
                return Some((n, steps));
            }
            _ => return None,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assignment_and_echo() {
        let s = parse("x = 1 + 2 * 3\n").unwrap();
        match &s[0] {
            Stmt::Assign { name, value: Expr::Binary(BinOp::Add, _, _), .. } => assert_eq!(name, "x"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn matrix_literal_rows() {
        let s = parse("A = [1 2; 3 4]").unwrap();
        match &s[0] {
            Stmt::Assign { value: Expr::Matrix(rows), .. } => {
                assert_eq!(rows.len(), 2);
                assert_eq!(rows[0].len(), 2);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn control_flow_blocks() {
        let s = parse("if x > 0\n y = 1;\nelse\n y = 2;\nend\n").unwrap();
        assert!(matches!(&s[0], Stmt::If { arms } if arms.len() == 2));
    }

    #[test]
    fn indexed_assignment() {
        let s = parse("A(2,3) = 9").unwrap();
        match &s[0] {
            Stmt::Assign { index: Some(ix), .. } => assert_eq!(ix.len(), 2),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn for_range() {
        let s = parse("for k = 1:3\ndisp(k)\nend").unwrap();
        assert!(matches!(&s[0], Stmt::For { .. }));
    }

    /// A space before `{` inside a `[…]`/`{…}` literal ends the current
    /// element — `{a {b}}` is two cells, not `a{b}` indexing.
    #[test]
    fn cell_literal_space_separated_cells() {
        let s = parse("{{1} {2}}").unwrap();
        match &s[0] {
            Stmt::Expr(Expr::CellMat(rows), _) => {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].len(), 2);
                assert!(matches!(rows[0][0], Expr::CellMat(_)));
                assert!(matches!(rows[0][1], Expr::CellMat(_)));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn cell_literal_space_separated_rows_of_cells() {
        let s = parse("{{1 2} {3 4}}").unwrap();
        match &s[0] {
            Stmt::Expr(Expr::CellMat(rows), _) => {
                assert_eq!(rows.len(), 1);
                assert!(matches!(&rows[0][0], Expr::CellMat(r) if r[0].len() == 2));
                assert!(matches!(&rows[0][1], Expr::CellMat(r) if r[0].len() == 2));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn cell_literal_space_separated_after_scalar() {
        let s = parse("{1 {2}}").unwrap();
        match &s[0] {
            Stmt::Expr(Expr::CellMat(rows), _) => {
                assert_eq!(rows[0].len(), 2);
                assert!(matches!(rows[0][0], Expr::Num(1.0)));
                assert!(matches!(rows[0][1], Expr::CellMat(_)));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn matrix_literal_space_before_brace_is_new_element() {
        let s = parse("[c {9}]").unwrap();
        match &s[0] {
            Stmt::Expr(Expr::Matrix(rows), _) => {
                assert_eq!(rows[0].len(), 2);
                assert!(matches!(rows[0][0], Expr::Ident(_)));
                assert!(matches!(rows[0][1], Expr::CellMat(_)));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    /// Attached `{` still indexes even inside a literal: `{c{1} {2}}` is
    /// `{c{1}, {2}}`, and `c{1}` outside a literal is unchanged.
    #[test]
    fn attached_brace_still_indexes() {
        let s = parse("{c{1} {2}}").unwrap();
        match &s[0] {
            Stmt::Expr(Expr::CellMat(rows), _) => {
                assert_eq!(rows[0].len(), 2);
                assert!(matches!(rows[0][0], Expr::BraceIndex(_, _)));
                assert!(matches!(rows[0][1], Expr::CellMat(_)));
            }
            other => panic!("unexpected {other:?}"),
        }
        let s = parse("x = c{1}").unwrap();
        match &s[0] {
            Stmt::Assign { value: Expr::BraceIndex(n, ix), .. } => {
                assert_eq!(n, "c");
                assert_eq!(ix.len(), 1);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    /// Chained subscripts on the LHS become a `FieldAssign` whose `path`
    /// interleaves field and subscript steps in left-to-right order.
    #[test]
    fn chained_subscript_lhs() {
        let s = parse("c{1}{2} = 9").unwrap();
        match &s[0] {
            Stmt::FieldAssign { name, path, .. } => {
                assert_eq!(name, "c");
                assert_eq!(path.len(), 2);
                assert!(matches!(path[0], PathStep::Brace(_)) && matches!(path[1], PathStep::Brace(_)));
            }
            other => panic!("unexpected {other:?}"),
        }
        let s = parse("s.c{1}(2) = 9").unwrap();
        match &s[0] {
            Stmt::FieldAssign { name, path, .. } => {
                assert_eq!(name, "s");
                assert_eq!(path.len(), 3);
                assert!(matches!(path[0], PathStep::Name(_)));
                assert!(matches!(path[1], PathStep::Brace(_)) && matches!(path[2], PathStep::Paren(_)));
            }
            other => panic!("unexpected {other:?}"),
        }
        // A field step *after* a subscript (`s.c{1}.f`) is representable.
        let s = parse("s.c{1}.f = 9").unwrap();
        match &s[0] {
            Stmt::FieldAssign { path, .. } => {
                assert_eq!(path.len(), 3);
                assert!(matches!(path[0], PathStep::Name(_)) && matches!(path[1], PathStep::Brace(_)) && matches!(path[2], PathStep::Name(_)));
            }
            other => panic!("unexpected {other:?}"),
        }
        // Single-op field indexing → [Name, Paren].
        let s = parse("s.f(2) = 9").unwrap();
        match &s[0] {
            Stmt::FieldAssign { path, .. } => {
                assert_eq!(path.len(), 2);
                assert!(matches!(path[0], PathStep::Name(_)) && matches!(path[1], PathStep::Paren(_)));
            }
            other => panic!("unexpected {other:?}"),
        }
        // `f(1).g` parses (illegal on scalar values at runtime, but the
        // path is what struct-array assignment `s(2).f` will need).
        match &parse("f(1).g = 9").unwrap()[0] {
            Stmt::FieldAssign { path, .. } => {
                assert!(matches!(path[0], PathStep::Paren(_)) && matches!(path[1], PathStep::Name(_)));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parfor_parses_as_for() {
        match &parse("parfor i = 1:3\nx = i\nend").unwrap()[0] {
            Stmt::For { var, .. } => assert_eq!(var, "i"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn bang_shell_escape() {
        match &parse("!del tmp*.smf").unwrap()[0] {
            Stmt::Expr(Expr::CallOrIndex(name, args), _) => {
                assert_eq!(name, "system");
                assert_eq!(args[0], Expr::Str("del tmp*.smf".into()));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn power_rhs_field_and_right_assoc() {
        // `x.^s.d` reads the field as the exponent.
        assert!(parse("y = x.^s.d").is_ok());
        // `a^b^c` is right-associative: a^(b^c).
        match &parse("y = a^b^c").unwrap()[0] {
            Stmt::Assign { value: Expr::Binary(BinOp::Pow, _, e2), .. } => {
                assert!(matches!(**e2, Expr::Binary(BinOp::Pow, _, _)));
            }
            other => panic!("unexpected {other:?}"),
        }
        // `(-1).^(0:5)'` — transpose inside the exponent.
        assert!(parse("sw = (-1).^(0:5)';").is_ok());
        // `x^-2` puts the sign inside the exponent.
        match &parse("y = x^-2").unwrap()[0] {
            Stmt::Assign { value: Expr::Binary(BinOp::Pow, _, e2), .. } => {
                assert!(matches!(**e2, Expr::Unary(UnOp::Neg, _)));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn command_args_wide_words() {
        // `:`/`[`/`]`/`=`/`/` glue into command words: error identifiers
        // and MATLAB-ish arg lists pass through as literal strings.
        match &parse("ft_warning once FieldTrip:fileio:UnknownResolution").unwrap()[0] {
            Stmt::Expr(Expr::CallOrIndex(_, args), _) => {
                assert_eq!(args[1], Expr::Str("FieldTrip:fileio:UnknownResolution".into()));
            }
            other => panic!("unexpected {other:?}"),
        }
        match &parse("im subplot [1 2 3]").unwrap()[0] {
            Stmt::Expr(Expr::CallOrIndex(_, args), _) => {
                assert_eq!(args[1], Expr::Str("[1".into()));
                assert_eq!(args[3], Expr::Str("3]".into()));
            }
            other => panic!("unexpected {other:?}"),
        }
        // `cd ../..` — a parent-of-parent path arg.
        match &parse("cd ../..").unwrap()[0] {
            Stmt::Expr(Expr::CallOrIndex(_, args), _) => {
                assert_eq!(args[0], Expr::Str("../..".into()));
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}
