//! Abstract syntax tree for M-code.

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Num(f64),
    /// Imaginary literal `2i` — the pure-imaginary scalar `v·i`.
    CNum(f64),
    Str(String),
    Ident(String),
    /// `:` — full extent inside indexing (`A(:, 2)`).
    Colon,
    /// `end` — last index inside indexing (`A(end)`).
    End,
    /// `a:b` or `a:step:b`.
    Range(Box<Expr>, Option<Box<Expr>>, Box<Expr>),
    /// `[a b; c d]` — rows of element lists.
    Matrix(Vec<Vec<Expr>>),
    /// `{a b; c d}` — cell-array literal (same row/col rules as `Matrix`).
    CellMat(Vec<Vec<Expr>>),
    /// `c{i, j}` — cell-content indexing; `c(...)` stays `CallOrIndex`.
    BraceIndex(String, Vec<Expr>),
    /// `expr.name` — struct field access (`s.a`, `s.a.b`).
    Field(Box<Expr>, String),
    /// `expr.(name_expr)` — dynamic field access; name must eval to a char.
    DynField(Box<Expr>, Box<Expr>),
    /// `expr(...)` — chained paren indexing on a non-name expression
    /// (`c{1}(2)`, `s.f(i)` reads).
    Index(Box<Expr>, Vec<Expr>),
    /// `expr{...}` — chained brace indexing (`c{1}{2}`).
    BraceIdx(Box<Expr>, Vec<Expr>),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// `A'` (conjugate) or `A.'` (plain) — `conj` selects `'` vs `.'`.
    Transpose(Box<Expr>, bool),
    /// `f(...)` — a function call or `A(...)` indexing; resolved at eval time
    /// by whether the name is bound in the workspace.
    CallOrIndex(String, Vec<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnOp {
    Neg,
    Pos,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Backslash,
    Pow,
    ElemMul,
    ElemDiv,
    ElemPow,
    Eq,
    Neq,
    Lt,
    Le,
    Gt,
    Ge,
    /// `&` element-wise logical and.
    And,
    /// `|` element-wise logical or.
    Or,
    /// `&&` short-circuit scalar and.
    ScAnd,
    /// `||` short-circuit scalar or.
    ScOr,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    /// `x = expr`, `A(i, j) = expr` or `c{i, j} = expr` (`braces`).
    /// `;` suppresses the echo.
    Assign {
        name: String,
        index: Option<Vec<Expr>>,
        /// `true` when the index used `{}` — cell content assignment.
        braces: bool,
        value: Expr,
        suppress: bool,
    },
    /// Bare expression statement (echoes `ans =` unless suppressed).
    Expr(Expr, bool),
    /// `if/elseif/else/end` — `None` condition marks the `else` arm.
    If {
        arms: Vec<(Option<Expr>, Vec<Stmt>)>,
    },
    /// `for v = expr ... end` — iterates the columns of `expr`.
    For {
        var: String,
        iter: Expr,
        body: Vec<Stmt>,
    },
    While {
        cond: Expr,
        body: Vec<Stmt>,
    },
    /// `switch expr ... case v ... otherwise ... end` — `None` is `otherwise`.
    Switch {
        expr: Expr,
        cases: Vec<(Option<Expr>, Vec<Stmt>)>,
    },
    Break,
    Continue,
    /// `return` — exits the script or the enclosing function.
    Return,
    /// `function [a, b] = name(p, q) ... end` — `outputs` may be empty,
    /// `params` may be empty, `end` optional at file scope.
    Function {
        name: String,
        params: Vec<String>,
        outputs: Vec<String>,
        body: Vec<Stmt>,
    },
    /// `[a, b] = f(x)` — multiple left-hand targets bound from a
    /// multi-output function call.
    AssignMany {
        names: Vec<String>,
        value: Expr,
        suppress: bool,
    },
    /// `s.f = v`, `s.a.b = v`, `s.f(i) = v`, `s.c{i} = v`,
    /// `s.c{1}.f = v`, `c{1}{2} = v` — a chain of field and subscript
    /// steps applied to a root variable, in left-to-right order.
    /// Field steps (`Name`/`Dyn`) and subscript steps (`Paren`/`Brace`)
    /// interleave freely.
    FieldAssign {
        name: String,
        path: Vec<PathStep>,
        value: Expr,
        suppress: bool,
    },
    /// `try ... catch [e] ... end` — `catch_var` receives a struct with
    /// `message`/`identifier` fields when present.
    TryCatch {
        try_body: Vec<Stmt>,
        catch_var: Option<String>,
        catch_body: Vec<Stmt>,
    },
}

/// One step of an assignment path: `.name`, `.(expr)` (evaluated to a
/// char at assign time), a `(…)` subscript, or a `{…}` cell-content
/// subscript. Steps interleave — `s.c{1}.f = v` is
/// `[Name(c), Brace(1), Name(f)]`.
#[derive(Debug, Clone, PartialEq)]
pub enum PathStep {
    Name(String),
    Dyn(Expr),
    Paren(Vec<Expr>),
    Brace(Vec<Expr>),
}
