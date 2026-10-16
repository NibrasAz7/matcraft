//! Tree-walk interpreter over the parsed AST.
//!
//! Dynamic semantics: a `f(...)` call resolves to indexing when `f` is bound
//! in the workspace, otherwise to a builtin function — the same rule M-code
//! uses. Matrices grow on out-of-range assignment.

use std::collections::HashMap;
use std::fmt;
// `CNum` arithmetic uses the operator traits — method syntax needs them
// in scope (`z.mul(w)`, `z.neg()`).
use std::ops::{Add, Div, Mul, Neg, Sub};

use matcore::{CMatrix, CNum, MatError, Matrix};

use crate::ast::{BinOp, Expr, PathStep, Stmt, UnOp};
use crate::LangError;

/// Hard caps so hostile or buggy scripts terminate.
const MAX_LOOP_ITERS: usize = 1_000_000;
const MAX_RANGE_LEN: usize = 1_000_000;
/// Cap on elements in any user-driven array allocation (~800 MB of f64).
const MAX_ELEMS: usize = 100_000_000;
/// Cells hold whole `Value`s — a much heavier element, so a tighter cap.
const MAX_CELL_ELEMS: usize = 10_000_000;
/// `figure(n)` may not open an unbounded number of figures.
const MAX_FIGURES: usize = 1_000;
/// Nested function/handle calls are bounded — a runaway recursive script
/// must error, not overflow the host stack.
const MAX_CALL_DEPTH: usize = 256;

/// Control-flow signal threading through statement execution.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Flow {
    Normal,
    Break,
    Continue,
    Return,
}

/// A parsed `function` definition.
#[derive(Debug, Clone)]
struct FuncDef {
    params: Vec<String>,
    outputs: Vec<String>,
    body: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Num(f64),
    Mat(Matrix),
    /// Complex matrix — produced the moment an imaginary part exists
    /// (`A(1) = 3+4i` promotes `A`), never stored when fully real.
    Cplx(CMatrix),
    Str(String),
    /// Cell array — `{a, b; c, d}`, contents via `c{i}`.
    Cell(CellArray),
    /// Sparse double matrix — `sparse(...)`/`speye(...)`; mixed sparse+full
    /// ops return full (MATLAB rule), sparse+sparse stays sparse.
    Sparse(matcore::SparseMat),
    /// Struct array — `s.field`, `s(2).f`. Elements share the same field
    /// set; `StructArray::fields` stores each field's values per element
    /// (column-major element order), like MATLAB's field-oriented storage.
    Struct(StructArray),
    /// Function handle — `@name` or an anonymous function `@(args) expr`.
    Func(FuncHandle),
}

/// What a `Value::Func` refers to.
#[derive(Debug, Clone, PartialEq)]
pub enum FuncHandle {
    /// `@name` — resolved at call time: user function, then builtin.
    Named(String),
    /// `@(a, b) expr` — `env` is a snapshot of the workspace taken at
    /// definition time (MATLAB anonymous-function semantics).
    Closure { params: Vec<String>, body: Box<Expr>, env: HashMap<String, Value> },
}

/// A MATLAB struct array: `rows x cols` elements, each holding the same
/// named fields. `fields[i]` keeps insertion order (for `orderfields`);
/// every `Vec<Value>` has `rows*cols` entries.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StructArray {
    rows: usize,
    cols: usize,
    fields: Vec<(String, Vec<Value>)>,
}

impl StructArray {
    /// Empty 1x1 struct with no fields.
    pub fn new() -> Self {
        StructArray { rows: 1, cols: 1, fields: Vec::new() }
    }

    /// Struct array with the given dims and no fields.
    pub fn with_dims(rows: usize, cols: usize) -> Self {
        StructArray { rows, cols, fields: Vec::new() }
    }

    /// 1x1 struct from a field map (e.g. `S = load(...)`).
    pub fn from_map(m: std::collections::BTreeMap<String, Value>) -> Self {
        StructArray { rows: 1, cols: 1, fields: m.into_iter().map(|(k, v)| (k, vec![v])).collect() }
    }

    /// Scalar (1x1) map view — used by sites that only handle scalar structs.
    pub fn scalar_map(&self) -> std::collections::BTreeMap<String, Value> {
        self.fields.iter().map(|(k, v)| (k.clone(), v.first().cloned().unwrap_or(Value::Mat(Matrix::zeros(0, 0))))).collect()
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn numel(&self) -> usize {
        self.rows * self.cols
    }

    pub fn is_scalar(&self) -> bool {
        self.numel() == 1
    }

    /// Field names in insertion order.
    pub fn field_names(&self) -> Vec<String> {
        self.fields.iter().map(|(k, _)| k.clone()).collect()
    }

    pub fn has_field(&self, name: &str) -> bool {
        self.fields.iter().any(|(k, _)| k == name)
    }

    /// `s(i).f` — field value at linear element index `i` (0-based).
    pub fn field_at(&self, i: usize, name: &str) -> Option<&Value> {
        self.fields.iter().find(|(k, _)| k == name).and_then(|(_, v)| v.get(i))
    }

    /// `s.f` — all elements' values for `f` (a comma-separated list when
    /// the array isn't scalar).
    pub fn field_values(&self, name: &str) -> Option<Vec<Value>> {
        self.fields.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
    }

    /// `s(i).f = v` — write one element's field, growing the array (and
    /// back-filling `[]` for the other elements' new field) like MATLAB.
    pub fn set_field(&mut self, i: usize, name: &str, v: Value) -> Result<(), LangError> {
        if i >= self.numel() {
            self.grow_linear(i + 1)?;
        }
        match self.fields.iter_mut().find(|(k, _)| k == name) {
            Some((_, vals)) => {
                vals[i] = v;
            }
            None => {
                let mut vals = vec![Value::Mat(Matrix::zeros(0, 0)); self.numel()];
                vals[i] = v;
                self.fields.push((name.to_string(), vals));
            }
        }
        Ok(())
    }

    /// New field on every element, initialized to `[]`.
    pub fn add_field(&mut self, name: &str) {
        if !self.has_field(name) {
            self.fields.push((name.to_string(), vec![Value::Mat(Matrix::zeros(0, 0)); self.numel()]));
        }
    }

    pub fn remove_field(&mut self, name: &str) -> bool {
        let before = self.fields.len();
        self.fields.retain(|(k, _)| k != name);
        self.fields.len() != before
    }

    /// `orderfields` — reorder to `names` (must be exactly the field set,
    /// any order). `perm` support lives in the builtin.
    pub fn reorder(&mut self, names: &[String]) -> Result<(), LangError> {
        if names.len() != self.fields.len() || names.iter().any(|n| !self.has_field(n)) {
            return Err(LangError::new("orderfields needs the same set of field names", 0));
        }
        let mut new_fields = Vec::with_capacity(self.fields.len());
        for n in names {
            if let Some(pos) = self.fields.iter().position(|(k, _)| k == n) {
                new_fields.push(self.fields[pos].clone());
            }
        }
        self.fields = new_fields;
        Ok(())
    }

    /// Grow the element vector to `n` entries — a row vector extends its
    /// columns, a column its rows; true 2-D arrays can't grow linearly.
    fn grow_linear(&mut self, n: usize) -> Result<(), LangError> {
        if self.numel() >= n {
            return Ok(());
        }
        let grown = n - self.numel();
        if self.rows <= 1 {
            self.cols = n;
        } else if self.cols <= 1 {
            self.rows = n;
        } else {
            return Err(LangError::new("linear index out of range for struct array growth", 0));
        }
        for (_, vals) in self.fields.iter_mut() {
            vals.resize(vals.len() + grown, Value::Mat(Matrix::zeros(0, 0)));
        }
        Ok(())
    }

    /// `s(idx)` — pick elements by linear index into a sub-array that
    /// keeps `s`'s field order. `idx` are 0-based linear indices.
    pub fn pick(&self, idx: &[usize], rows: usize, cols: usize) -> Result<StructArray, LangError> {
        for &i in idx {
            if i >= self.numel() {
                return Err(LangError::new("index exceeds the number of struct array elements", 0));
            }
        }
        let mut out = StructArray::with_dims(rows, cols);
        for (name, vals) in &self.fields {
            out.fields.push((name.clone(), idx.iter().map(|&i| vals.get(i).cloned().unwrap_or(Value::Mat(Matrix::zeros(0, 0)))).collect()));
        }
        Ok(out)
    }

    /// `s(i) = t` / `s(idx) = t` — write element(s) of `s` from a struct
    /// with the same field set; grows `s` when the index is past the end.
    pub fn assign_elements(&mut self, idx: &[usize], src: &StructArray) -> Result<(), LangError> {
        if src.numel() != idx.len() && !(src.is_scalar() && !idx.is_empty()) {
            return Err(LangError::new("subscripted assignment dimension mismatch on a struct array", 0));
        }
        for name in src.field_names() {
            if !self.has_field(&name) {
                self.add_field(&name);
            }
        }
        let max_i = idx.iter().copied().max().unwrap_or(0);
        if max_i >= self.numel() {
            self.grow_linear(max_i + 1)?;
        }
        for (j, &i) in idx.iter().enumerate() {
            for (name, vals) in src.fields.iter() {
                let v = if src.is_scalar() { vals.first() } else { vals.get(j) }.cloned().unwrap_or(Value::Mat(Matrix::zeros(0, 0)));
                self.set_field(i, name, v)?;
            }
        }
        Ok(())
    }

    /// `[s t]` — horizontal concat; fields must be the same set.
    pub fn hcat(a: &StructArray, b: &StructArray) -> Result<StructArray, LangError> {
        if a.rows != b.rows {
            return Err(LangError::new("dimensions of arrays being concatenated are not consistent", 0));
        }
        let mut names = a.field_names();
        for n in b.field_names() {
            if !names.contains(&n) {
                names.push(n);
            }
        }
        let mut out = StructArray::with_dims(a.rows, a.cols + b.cols);
        for name in &names {
            let mut vals = Vec::with_capacity(out.numel());
            for c in 0..out.cols {
                let src = if c < a.cols { a } else { b };
                let sc = if c < a.cols { c } else { c - a.cols };
                for r in 0..out.rows {
                    let i = r + sc * src.rows;
                    vals.push(src.field_at(i, name).cloned().unwrap_or(Value::Mat(Matrix::zeros(0, 0))));
                }
            }
            out.fields.push((name.clone(), vals));
        }
        Ok(out)
    }

    /// `[s; t]` — vertical concat; fields must be the same set.
    pub fn vcat(a: &StructArray, b: &StructArray) -> Result<StructArray, LangError> {
        if a.cols != b.cols {
            return Err(LangError::new("dimensions of arrays being concatenated are not consistent", 0));
        }
        let mut names = a.field_names();
        for n in b.field_names() {
            if !names.contains(&n) {
                names.push(n);
            }
        }
        let mut out = StructArray::with_dims(a.rows + b.rows, a.cols);
        for name in &names {
            let mut vals = Vec::with_capacity(out.numel());
            for c in 0..out.cols {
                for r in 0..out.rows {
                    let (src, sr) = if r < a.rows { (a, r) } else { (b, r - a.rows) };
                    let i = sr + c * src.rows;
                    vals.push(src.field_at(i, name).cloned().unwrap_or(Value::Mat(Matrix::zeros(0, 0))));
                }
            }
            out.fields.push((name.clone(), vals));
        }
        Ok(out)
    }

    /// `s(idx) = []` — delete elements (keeps field set, shrinks the array).
    pub fn delete_elements(&mut self, keep_mask: &[bool], rows: usize, cols: usize) {
        for (_, vals) in self.fields.iter_mut() {
            let mut kept = Vec::with_capacity(vals.len());
            for (i, v) in vals.iter().enumerate() {
                if keep_mask.get(i).copied().unwrap_or(true) {
                    kept.push(v.clone());
                }
            }
            *vals = kept;
        }
        self.rows = rows;
        self.cols = cols;
    }
}

/// A MATLAB cell array: an `rows x cols` grid of arbitrary `Value`s,
/// stored column-major like `Matrix` so `c{i}` linear indexing matches.
#[derive(Debug, Clone, PartialEq)]
pub struct CellArray {
    rows: usize,
    cols: usize,
    items: Vec<Value>,
}

impl CellArray {
    /// The value an unwritten cell holds: `[]`.
    fn empty() -> Value {
        Value::Mat(Matrix::zeros(0, 0))
    }

    /// `rows x cols` cell filled with `[]`.
    pub fn new(rows: usize, cols: usize) -> Self {
        CellArray { rows, cols, items: vec![Self::empty(); rows.saturating_mul(cols)] }
    }

    /// 1×n cell from contents (for `{a, b, c}` rows and `varargin`).
    pub fn row(items: Vec<Value>) -> Self {
        let cols = items.len();
        CellArray { rows: 1, cols, items }
    }

    /// n×1 cell from contents (`c(:)` columnizes).
    pub fn col(items: Vec<Value>) -> Self {
        let rows = items.len();
        CellArray { rows, cols: 1, items }
    }

    /// `rows x cols` cell from a column-major item list — used by the
    /// .mat reader (items beyond `rows*cols` are dropped; short lists
    /// pad with `[]`).
    pub(crate) fn from_col_major(rows: usize, cols: usize, items: Vec<Value>) -> Self {
        let mut ca = CellArray::new(rows, cols);
        for (i, v) in items.into_iter().enumerate().take(rows.saturating_mul(cols)) {
            ca.set_linear(i, v);
        }
        ca
    }

    pub fn shape(&self) -> (usize, usize) {
        (self.rows, self.cols)
    }

    pub fn numel(&self) -> usize {
        self.items.len()
    }

    /// 0-based element access.
    pub fn get(&self, r: usize, c: usize) -> Option<&Value> {
        if r < self.rows && c < self.cols {
            self.items.get(c * self.rows + r)
        } else {
            None
        }
    }

    /// 0-based column-major linear access.
    pub fn get_linear(&self, i: usize) -> Option<&Value> {
        self.items.get(i)
    }

    fn set(&mut self, r: usize, c: usize, v: Value) {
        if let Some(slot) = self.items.get_mut(c * self.rows + r) {
            *slot = v;
        }
    }

    fn set_linear(&mut self, i: usize, v: Value) {
        if let Some(slot) = self.items.get_mut(i) {
            *slot = v;
        }
    }

    /// Grow to at least `rows x cols`, padding new cells with `[]`.
    fn grow_to(&mut self, rows: usize, cols: usize) {
        if rows <= self.rows && cols <= self.cols {
            return;
        }
        // Growing one dim must not shrink the other — the copy loop below
        // would index past the new buffer otherwise.
        let (rows, cols) = (rows.max(self.rows), cols.max(self.cols));
        let mut items = vec![Self::empty(); rows.saturating_mul(cols)];
        for c in 0..self.cols {
            for r in 0..self.rows {
                if let Some(v) = self.items.get(c * self.rows + r) {
                    items[c * rows + r] = v.clone();
                }
            }
        }
        self.rows = rows;
        self.cols = cols;
        self.items = items;
    }

    /// `grow_to` guarded by the cell-element cap (`c{1e9} = x` must error,
    /// not attempt a multi-GB allocation).
    fn grow_capped(&mut self, rows: usize, cols: usize) -> Result<(), LangError> {
        let n = rows.max(self.rows).saturating_mul(cols.max(self.cols));
        if n > MAX_CELL_ELEMS {
            return Err(LangError::new("allocation exceeds limit", 0));
        }
        self.grow_to(rows, cols);
        Ok(())
    }

    /// Column-major reshape (`reshape(c, r, c)`); element order preserved.
    fn reshape(&self, rows: usize, cols: usize) -> Result<CellArray, LangError> {
        if rows.saturating_mul(cols) != self.numel() {
            return Err(LangError::new("to RESHAPE the number of elements must not change", 0));
        }
        if self.numel() > MAX_CELL_ELEMS {
            return Err(LangError::new("allocation exceeds limit", 0));
        }
        let mut out = CellArray::new(rows, cols);
        for i in 0..self.numel() {
            if let Some(v) = self.get_linear(i) {
                out.set_linear(i, v.clone());
            }
        }
        Ok(out)
    }

    /// `fliplr`/`flipud` on cells — reverse one dimension.
    fn flip(&self, left_right: bool) -> CellArray {
        let mut out = CellArray::new(self.rows, self.cols);
        for c in 0..self.cols {
            for r in 0..self.rows {
                let (rr, cc) = if left_right { (r, self.cols - 1 - c) } else { (self.rows - 1 - r, c) };
                if let Some(v) = self.get(r, c) {
                    out.set(rr, cc, v.clone());
                }
            }
        }
        out
    }

    /// `repmat` on cells — tile the whole array `rr`×`cc` times.
    fn repmat(&self, rr: usize, cc: usize) -> Result<CellArray, LangError> {
        let (tr, tc) = (self.rows.saturating_mul(rr), self.cols.saturating_mul(cc));
        if tr.saturating_mul(tc) > MAX_CELL_ELEMS {
            return Err(LangError::new("allocation exceeds limit", 0));
        }
        let mut out = CellArray::new(tr, tc);
        for bc in 0..cc {
            for br in 0..rr {
                for c in 0..self.cols {
                    for r in 0..self.rows {
                        if let Some(v) = self.get(r, c) {
                            out.set(br * self.rows + r, bc * self.cols + c, v.clone());
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    /// `c(:,j) = []` — remove whole columns.
    fn remove_cols(&mut self, cols: &[usize]) {
        let keep: Vec<usize> = (1..=self.cols).filter(|c| !cols.contains(c)).collect();
        let mut out = CellArray::new(self.rows, keep.len());
        for (nc, &oc) in keep.iter().enumerate() {
            for r in 0..self.rows {
                if let Some(v) = self.get(r, oc - 1) {
                    out.set(r, nc, v.clone());
                }
            }
        }
        *self = out;
    }

    /// `c(i,:) = []` — remove whole rows.
    fn remove_rows(&mut self, rows: &[usize]) {
        let keep: Vec<usize> = (1..=self.rows).filter(|r| !rows.contains(r)).collect();
        let mut out = CellArray::new(keep.len(), self.cols);
        for c in 0..self.cols {
            for (nr, &or) in keep.iter().enumerate() {
                if let Some(v) = self.get(or - 1, c) {
                    out.set(nr, c, v.clone());
                }
            }
        }
        *self = out;
    }

    /// `c(idx) = []` — remove linear indices; the result stays a vector.
    fn remove_linear(&mut self, idxs: &[usize]) -> Result<(), LangError> {
        if self.rows > 1 && self.cols > 1 {
            return Err(LangError::new("c(i) = [] on a 2-d cell array needs a colon index: c(i,:) or c(:,j)", 0));
        }
        let mut kill = vec![false; self.items.len()];
        for &i in idxs {
            if let Some(k) = kill.get_mut(i.wrapping_sub(1)) {
                *k = true;
            }
        }
        self.items = self.items.iter().zip(&kill).filter(|(_, &k)| !k).map(|(v, _)| v.clone()).collect();
        if self.rows == 1 {
            self.cols = self.items.len();
        } else {
            self.rows = self.items.len();
        }
        Ok(())
    }

    /// Horizontal concatenation of cells (`[c1 c2]`); empty dims are flexible.
    fn hcat(a: &CellArray, b: &CellArray) -> Result<CellArray, LangError> {
        let rows = a.rows.max(b.rows);
        if (a.rows != rows && a.numel() > 0) || (b.rows != rows && b.numel() > 0) {
            return Err(LangError::new("cell concatenation needs matching row counts", 0));
        }
        if rows.saturating_mul(a.cols.saturating_add(b.cols)) > MAX_CELL_ELEMS {
            return Err(LangError::new("allocation exceeds limit", 0));
        }
        let mut out = CellArray::new(rows, a.cols + b.cols);
        for (src, off) in [(a, 0usize), (b, a.cols)] {
            for c in 0..src.cols {
                for r in 0..src.rows {
                    if let Some(v) = src.get(r, c) {
                        out.set(r, c + off, v.clone());
                    }
                }
            }
        }
        Ok(out)
    }

    /// Vertical concatenation of cells (`[c1; c2]`).
    fn vcat(a: &CellArray, b: &CellArray) -> Result<CellArray, LangError> {
        let cols = a.cols.max(b.cols);
        if (a.cols != cols && a.numel() > 0) || (b.cols != cols && b.numel() > 0) {
            return Err(LangError::new("cell concatenation needs matching column counts", 0));
        }
        if a.rows.saturating_add(b.rows).saturating_mul(cols) > MAX_CELL_ELEMS {
            return Err(LangError::new("allocation exceeds limit", 0));
        }
        let mut out = CellArray::new(a.rows + b.rows, cols);
        for (src, off) in [(a, 0usize), (b, a.rows)] {
            for c in 0..src.cols {
                for r in 0..src.rows {
                    if let Some(v) = src.get(r, c) {
                        out.set(r + off, c, v.clone());
                    }
                }
            }
        }
        Ok(out)
    }
}

impl Value {
    fn to_matrix(&self) -> Result<Matrix, LangError> {
        match self {
            Value::Num(v) => Ok(Matrix::scalar(*v)),
            Value::Mat(m) => Ok(m.clone()),
            // Real-only contexts (`det`, `lu`, real indexing…) take the
            // imaginary part's word for it: all-zero imag drops to real,
            // genuinely complex errors.
            Value::Cplx(m) => m.real_if_real().ok_or_else(|| LangError::new("complex value used where a real number was expected", 0)),
            Value::Sparse(s) => Ok(s.full()),
            Value::Str(_) => Err(LangError::new("string used where a number was expected", 0)),
            Value::Cell(_) => Err(LangError::new("cell array used where a number was expected", 0)),
            Value::Struct(_) => Err(LangError::new("struct used where a number was expected", 0)),
            Value::Func(_) => Err(LangError::new("function handle used where a number was expected", 0)),
        }
    }

    /// Numeric-context conversion: chars promote to their code points as a
    /// 1×n double row (`'a' + 1` → 98, `'a' == 'a'` → 1). Everything else
    /// behaves like `to_matrix`.
    fn to_numeric(&self) -> Result<Matrix, LangError> {
        match self {
            Value::Str(s) => Ok(Matrix::row_vector(s.chars().map(|c| c as u32 as f64).collect())),
            _ => self.to_matrix(),
        }
    }

    /// Complex-context conversion: reals/chars promote with a zero imaginary
    /// part; `Cplx` is cloned. Cells/structs still error.
    fn to_cmatrix(&self) -> Result<CMatrix, LangError> {
        match self {
            Value::Cplx(m) => Ok(m.clone()),
            _ => Ok(CMatrix::from_real(&self.to_numeric()?)),
        }
    }

    /// `(rows, cols)` of any value — for `size`/`numel`/`length`/`isempty`.
    fn dims(&self) -> (usize, usize) {
        match self {
            Value::Num(_) => (1, 1),
            Value::Mat(m) => m.shape(),
            Value::Cplx(m) => m.shape(),
            Value::Str(s) => {
                if s.is_empty() {
                    (0, 0)
                } else {
                    (1, s.chars().count())
                }
            }
            Value::Cell(c) => c.shape(),
            Value::Struct(s) => (s.rows(), s.cols()),
            Value::Sparse(s) => s.shape(),
            Value::Func(_) => (1, 1),
        }
    }

    fn truthy(&self) -> Result<bool, LangError> {
        // Complex truthiness: all elements nonzero (re or im).
        if let Value::Cplx(m) = self {
            return Ok(m.numel() > 0 && m.data().iter().all(|z| z.re != 0.0 || z.im != 0.0));
        }
        let m = self.to_numeric()?;
        // Empty is false (M-code semantics); otherwise all elements nonzero.
        Ok(!m.data().is_empty() && m.data().iter().all(|&v| v != 0.0))
    }
}

/// A live workspace: variables, RNG state, pending printed output.
/// `Clone` so the engine's panic guard can snapshot/restore a session.
#[derive(Clone)]
pub struct Session {
    vars: HashMap<String, Value>,
    rng: u64,
    /// Output from print builtins (disp/who) accumulated during a statement.
    print_buf: String,
    /// `format long` prints 14 digits; `format short` (default) prints 4.
    fmt_long: bool,
    /// `function` definitions (hoisted from scripts + loaded `.m` files).
    funcs: HashMap<String, FuncDef>,
    /// Directories searched for `<name>.m` function files.
    paths: Vec<std::path::PathBuf>,
    /// Argument/result counts of the innermost active call (`nargin`/`nargout`).
    call_ctx: Option<(usize, usize)>,
    /// Nested call depth — bounded so runaway recursion errors out instead
    /// of overflowing the host stack.
    call_depth: usize,
    /// Open figures; index is the figure number minus one.
    figs: Vec<matplot::Figure>,
    /// Current figure number (1-based); 0 = none.
    cur_fig: usize,
    /// Most recent error (caught or not) — backs `lasterror` and `catch e`.
    last_err: Option<LangError>,
    /// Stopwatch for `tic`/`toc`. `Instant` doesn't exist on wasm.
    #[cfg(not(target_arch = "wasm32"))]
    tic: Option<std::time::Instant>,
}

impl Session {
    pub fn new() -> Self {
        let paths = std::env::current_dir().map(|d| vec![d]).unwrap_or_default();
        Session {
            vars: HashMap::new(),
            rng: 0x9E3779B97F4A7C15,
            print_buf: String::new(),
            fmt_long: false,
            funcs: HashMap::new(),
            paths,
            call_ctx: None,
            call_depth: 0,
            last_err: None,
            figs: Vec::new(),
            cur_fig: 0,
            #[cfg(not(target_arch = "wasm32"))]
            tic: None,
        }
    }

    /// Deterministic session (for tests and reproducible scripts).
    pub fn with_seed(seed: u64) -> Self {
        let mut s = Session::new();
        s.rng = seed | 1;
        s
    }

    /// Add a directory to the function-file search path.
    pub fn add_path(&mut self, dir: impl Into<std::path::PathBuf>) {
        self.paths.push(dir.into());
    }

    pub fn get(&self, name: &str) -> Option<&Value> {
        self.vars.get(name)
    }

    pub fn set(&mut self, name: impl Into<String>, v: Value) {
        self.vars.insert(name.into(), v);
    }

    /// Variable names in the workspace, sorted (for `workspace.list`).
    pub fn var_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.vars.keys().cloned().collect();
        names.sort();
        names
    }

    /// Defined `function` names, sorted (for command-window completion).
    pub fn func_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.funcs.keys().cloned().collect();
        names.sort();
        names
    }

    /// Remove `names` from the workspace; empty slice clears everything.
    pub fn clear_vars(&mut self, names: &[String]) {
        if names.is_empty() {
            self.vars.clear();
        } else {
            for n in names {
                self.vars.remove(n);
            }
        }
    }

    /// Directories on the function-file search path.
    pub fn paths(&self) -> &[std::path::PathBuf] {
        &self.paths
    }

    /// Open figures; index is figure number minus one (empty slots are
    /// not kept, so numbers can shift after `close`).
    pub fn figures(&self) -> &[matplot::Figure] {
        &self.figs
    }

    /// Current figure number (1-based); 0 = none.
    pub fn current_figure(&self) -> usize {
        self.cur_fig
    }

    /// Render figure `n` (1-based) to SVG.
    pub fn figure_svg(&self, n: usize) -> Option<String> {
        self.figs.get(n.wrapping_sub(1)).map(matplot::to_svg)
    }

    /// Parse and run `source`; returns everything the script echoed.
    pub fn run(&mut self, source: &str) -> Result<String, LangError> {
        let stmts = crate::parser::parse_lines(source)?;
        let bare: Vec<Stmt> = stmts.iter().map(|(_, s)| s.clone()).collect();
        self.hoist_functions(&bare);
        let mut out = String::new();
        for (line, s) in &stmts {
            let flow = match self.exec_stmt(s, &mut out) {
                Ok(f) => f,
                Err(mut e) => {
                    // Errors raised without position get the statement's line
                    // so `Error: line N` messages link back to the source.
                    if e.line == 0 {
                        e.line = *line;
                    }
                    self.last_err = Some(e.clone());
                    out.push_str(&self.print_buf);
                    self.print_buf.clear();
                    return Err(e);
                }
            };
            out.push_str(&self.print_buf);
            self.print_buf.clear();
            match flow {
                Flow::Normal => {}
                Flow::Return => break,
                Flow::Break | Flow::Continue => return Err(LangError::new("break/continue outside a loop", 0)),
            }
        }
        Ok(out)
    }

    /// Register every top-level `function` before running — script-local
    /// functions are callable throughout the file, like M-code.
    fn hoist_functions(&mut self, stmts: &[Stmt]) {
        for s in stmts {
            if let Stmt::Function { name, params, outputs, body } = s {
                self.funcs.insert(name.clone(), FuncDef { params: params.clone(), outputs: outputs.clone(), body: body.clone() });
            }
        }
    }

    /// Run a block of statements; stops early on non-Normal flow.
    fn exec_block(&mut self, body: &[Stmt], out: &mut String) -> Result<Flow, LangError> {
        for st in body {
            let f = self.exec_stmt(st, out)?;
            if f != Flow::Normal {
                return Ok(f);
            }
        }
        Ok(Flow::Normal)
    }

    fn exec_stmt(&mut self, s: &Stmt, out: &mut String) -> Result<Flow, LangError> {
        match s {
            Stmt::Assign { name, index, value, suppress, braces } => {
                let v = self.eval(value)?;
                match index {
                    None => {
                        self.vars.insert(name.clone(), v.clone());
                    }
                    Some(ix) => {
                        self.indexed_assign(name, ix, v.clone(), *braces)?;
                    }
                }
                if !suppress {
                    let long = self.fmt_long;
                    out.push_str(&format!("{name} =\n\n{}\n\n", fmt_value(&v, long)));
                }
            }
            Stmt::FieldAssign { name, path, value, suppress } => {
                let v = self.eval(value)?;
                // Dynamic field names resolve while the root is still bound —
                // `s.(s.k) = v` reads the pre-assignment struct.
                let mut resolved = Vec::with_capacity(path.len());
                for step in path {
                    match step {
                        PathStep::Dyn(e) => {
                            let n = match self.eval(e)? {
                                Value::Str(s) if valid_field_name(&s) => s,
                                Value::Str(s) => return Err(LangError::new(format!("invalid field name '{s}'"), 0)),
                                _ => return Err(LangError::new("dynamic field name must be a char", 0)),
                            };
                            resolved.push(PathStep::Name(n));
                        }
                        other => resolved.push(other.clone()),
                    }
                }
                // Detach the root so `path_set` can call back into `self`
                // (index expressions may evaluate user code).
                let root = self.vars.remove(name);
                let cur = root.clone().unwrap_or_else(|| Value::Mat(Matrix::zeros(0, 0)));
                match self.path_set(cur, &resolved, v, name) {
                    Ok(new) => {
                        self.vars.insert(name.clone(), new);
                    }
                    Err(e) => {
                        // Restore the old binding on failure.
                        if let Some(orig) = root {
                            self.vars.insert(name.clone(), orig);
                        }
                        return Err(e);
                    }
                }
                if !suppress {
                    let long = self.fmt_long;
                    if let Some(v) = self.vars.get(name) {
                        out.push_str(&format!("{name} =\n\n{}\n\n", fmt_value(v, long)));
                    }
                }
            }
            Stmt::Expr(e, suppress) => {
                // A bare comma-separated list echoes each element through
                // `ans` in turn, like M-code (`c{1:2}` prints ans twice).
                for v in self.eval_multi(e)? {
                    self.vars.insert("ans".into(), v.clone());
                    if !suppress {
                        let long = self.fmt_long;
                        out.push_str(&format!("ans =\n\n{}\n\n", fmt_value(&v, long)));
                    }
                }
            }
            Stmt::TryCatch { try_body, catch_var, catch_body } => {
                match self.exec_block(try_body, out) {
                    Ok(flow) => {
                        if flow != Flow::Normal {
                            return Ok(flow);
                        }
                    }
                    Err(e) => {
                        // The exception object: `e.message`, `e.identifier`.
                        self.last_err = Some(e.clone());
                        if let Some(var) = catch_var {
                            let mut m = std::collections::BTreeMap::new();
                            m.insert("identifier".into(), Value::Str(e.id.clone()));
                            m.insert("message".into(), Value::Str(e.message.clone()));
                            self.vars.insert(var.clone(), Value::Struct(StructArray::from_map(m)));
                        }
                        let flow = self.exec_block(catch_body, out)?;
                        if flow != Flow::Normal {
                            return Ok(flow);
                        }
                    }
                }
            }
            Stmt::If { arms } => {
                for (cond, body) in arms {
                    let take = match cond {
                        Some(c) => self.eval(c)?.truthy()?,
                        None => true,
                    };
                    if take {
                        return self.exec_block(body, out);
                    }
                }
            }
            Stmt::For { var, iter, body } => {
                let it = self.eval(iter)?;
                if let Value::Cell(ca) = &it {
                    // `for x = c` iterates the cell array's columns; the loop
                    // variable is a rows×1 cell each time.
                    for c in 0..ca.cols {
                        let mut items = Vec::with_capacity(ca.rows);
                        for r in 0..ca.rows {
                            items.push(ca.get(r, c).cloned().unwrap_or_else(CellArray::empty));
                        }
                        self.vars.insert(var.clone(), Value::Cell(CellArray { rows: ca.rows, cols: 1, items }));
                        match self.exec_block(body, out)? {
                            Flow::Normal => {}
                            Flow::Break => break,
                            Flow::Continue => continue,
                            Flow::Return => return Ok(Flow::Return),
                        }
                    }
                    return Ok(Flow::Normal);
                }
                // Chars promote: `for c = 'abc'` iterates the code points
                // (MATLAB yields 1x1 chars — the loop var's class is a
                // documented deviation).
                // Complex iterates complex columns.
                if let Value::Cplx(cm) = &it {
                    for c in 0..cm.cols() {
                        let mut colv = Vec::with_capacity(cm.rows());
                        for r in 0..cm.rows() {
                            colv.push(cm.get(r, c).unwrap_or(CNum::real(0.0)));
                        }
                        let colm = CMatrix::from_col_major(cm.rows(), 1, colv).map_err(lang)?;
                        self.vars.insert(var.clone(), cmat_or_value(colm));
                        match self.exec_block(body, out)? {
                            Flow::Normal => {}
                            Flow::Break => break,
                            Flow::Continue => continue,
                            Flow::Return => return Ok(Flow::Return),
                        }
                    }
                    return Ok(Flow::Normal);
                }
                let m = it.to_numeric()?;
                // The loop variable takes each column of `iter` in turn;
                // for a row vector `1:n` each column is a scalar.
                for c in 0..m.cols() {
                    let mut colv = Vec::with_capacity(m.rows());
                    for r in 0..m.rows() {
                        colv.push(m.get(r, c).unwrap_or(0.0));
                    }
                    self.vars.insert(var.clone(), Value::Mat(Matrix::col_vector(colv)));
                    match self.exec_block(body, out)? {
                        Flow::Normal => {}
                        Flow::Break => break,
                        Flow::Continue => continue,
                        Flow::Return => return Ok(Flow::Return),
                    }
                }
            }
            Stmt::While { cond, body } => {
                let mut n = 0usize;
                while self.eval(cond)?.truthy()? {
                    n += 1;
                    if n > MAX_LOOP_ITERS {
                        return Err(LangError::new("loop iteration limit exceeded", 0));
                    }
                    match self.exec_block(body, out)? {
                        Flow::Normal => {}
                        Flow::Break => break,
                        Flow::Continue => continue,
                        Flow::Return => return Ok(Flow::Return),
                    }
                }
            }
            Stmt::Switch { expr, cases } => {
                let val = self.eval(expr)?;
                for (cond, body) in cases {
                    let take = match cond {
                        Some(c) => {
                            let cv = self.eval(c)?;
                            values_equal(&val, &cv)
                        }
                        None => true,
                    };
                    if take {
                        return self.exec_block(body, out);
                    }
                }
            }
            Stmt::Break => return Ok(Flow::Break),
            Stmt::Continue => return Ok(Flow::Continue),
            Stmt::Return => return Ok(Flow::Return),
            // Already registered by `hoist_functions`; executing the node
            // is a no-op.
            Stmt::Function { .. } => {}
            Stmt::AssignMany { names, value, suppress } => {
                let call = match value {
                    Expr::CallOrIndex(n, a) => Some((n, a)),
                    _ => None,
                };
                let Some((fname, fargs)) = call else {
                    // `[a, b] = c{1:2}` — a comma-separated list on the right
                    // fills the targets one-to-one.
                    let outs = self.eval_multi(value)?;
                    if outs.len() != names.len() {
                        return Err(LangError::new(
                            format!("expected {} outputs from the expression, but it produced {}", names.len(), outs.len()),
                            0,
                        ));
                    }
                    for (n, v) in names.iter().zip(outs) {
                        // `~` consumes an output slot but binds nothing.
                        if n != "~" {
                            self.vars.insert(n.clone(), v);
                        }
                    }
                    if !suppress {
                        for n in names {
                            if let Some(v) = self.vars.get(n) {
                                let long = self.fmt_long;
                                out.push_str(&format!("{n} =\n\n{}\n\n", fmt_value(v, long)));
                            }
                        }
                    }
                    return Ok(Flow::Normal);
                };
                // `[a,b] = f(x)` where `f` is a function handle variable.
                if let Some(Value::Func(h)) = self.vars.get(fname).cloned() {
                    let argv = self.eval_args(fargs)?;
                    let outs = self.call_handle(&h, argv, names.len())?;
                    for (i, n) in names.iter().enumerate() {
                        let v = outs.get(i).cloned().ok_or_else(|| LangError::new(format!("function '{fname}' returned too few outputs"), 0))?;
                        if n != "~" {
                            self.vars.insert(n.clone(), v);
                        }
                    }
                    if !suppress {
                        for n in names {
                            if let Some(v) = self.vars.get(n) {
                                let long = self.fmt_long;
                                out.push_str(&format!("{n} =\n\n{}\n\n", fmt_value(v, long)));
                            }
                        }
                    }
                    return Ok(Flow::Normal);
                }
                if !self.is_function(fname)? {
                    // Multi-output builtins: [M,I]=max(A), [V,D]=eig(A), …
                    match self.builtin_multi(fname, fargs, names.len())? {
                        Some(outs) => {
                            for (i, n) in names.iter().enumerate() {
                                let v =
                                    outs.get(i).cloned().ok_or_else(|| LangError::new(format!("function '{fname}' returned too few outputs"), 0))?;
                                if n != "~" {
                                    self.vars.insert(n.clone(), v);
                                }
                            }
                            if !suppress {
                                for n in names {
                                    if let Some(v) = self.vars.get(n) {
                                        let long = self.fmt_long;
                                        out.push_str(&format!("{n} =\n\n{}\n\n", fmt_value(v, long)));
                                    }
                                }
                            }
                            return Ok(Flow::Normal);
                        }
                        None => return Err(LangError::new(format!("'{fname}' is not a function"), 0)),
                    }
                }
                let outs = self.call_function(fname, fargs, names.len())?;
                for (i, n) in names.iter().enumerate() {
                    let v = outs.get(i).cloned().ok_or_else(|| LangError::new(format!("function '{fname}' returned too few outputs"), 0))?;
                    if n != "~" {
                        self.vars.insert(n.clone(), v);
                    }
                }
                if !suppress {
                    for n in names {
                        if let Some(v) = self.vars.get(n) {
                            let long = self.fmt_long;
                            out.push_str(&format!("{n} =\n\n{}\n\n", fmt_value(v, long)));
                        }
                    }
                }
            }
        }
        Ok(Flow::Normal)
    }

    fn eval(&mut self, e: &Expr) -> Result<Value, LangError> {
        self.eval_inner(e)?.ok_or_else(|| LangError::new("expression produced no value", 0))
    }

    fn eval_inner(&mut self, e: &Expr) -> Result<Option<Value>, LangError> {
        match e {
            Expr::Num(v) => Ok(Some(Value::Num(*v))),
            Expr::CNum(v) => Ok(Some(Value::Cplx(CMatrix::scalar(CNum::new(0.0, *v))))),
            Expr::Str(s) => Ok(Some(Value::Str(s.clone()))),
            Expr::Ident(name) => match self.vars.get(name) {
                // Workspace bindings shadow constants — `pi = 3` is legal.
                Some(v) => Ok(Some(v.clone())),
                None => match self.constant(name) {
                    Some(v) => Ok(Some(v)),
                    // Unbound names are zero-arg calls: `x = rand`, `nargin`
                    // inside a function, bare `who`.
                    None => {
                        if self.is_function(name)? {
                            let outs = self.call_function(name, &[], 1)?;
                            Ok(outs.into_iter().next())
                        } else {
                            self.builtin(name, &[])
                        }
                    }
                },
            },
            Expr::Colon => Err(LangError::new("':' used outside indexing", 0)),
            Expr::End => Err(LangError::new("'end' used outside indexing", 0)),
            Expr::Range(a, step, b) => {
                let lo = self.eval(a)?.scalar()?;
                let hi = self.eval(b)?.scalar()?;
                let st = match step {
                    Some(s) => self.eval(s)?.scalar()?,
                    None => 1.0,
                };
                if st == 0.0 {
                    return Err(LangError::new("range step cannot be zero", 0));
                }
                let mut v = Vec::new();
                let mut x = lo;
                if st > 0.0 {
                    while x <= hi + st * 1e-9 {
                        v.push(x);
                        x = lo + st * (v.len() as f64);
                        if v.len() >= MAX_RANGE_LEN {
                            return Err(LangError::new("range too long", 0));
                        }
                    }
                } else {
                    while x >= hi + st * 1e-9 {
                        v.push(x);
                        x = lo + st * (v.len() as f64);
                        if v.len() >= MAX_RANGE_LEN {
                            return Err(LangError::new("range too long", 0));
                        }
                    }
                }
                Ok(Some(Value::Mat(Matrix::row_vector(v))))
            }
            Expr::Matrix(rows) => {
                // Evaluate elements first — if any is a cell, all must be
                // cells and `[c1 c2]` is cell concatenation.
                let mut grid: Vec<Vec<Value>> = Vec::with_capacity(rows.len());
                let mut any_cell = false;
                for row in rows {
                    let mut rv = Vec::with_capacity(row.len());
                    for cell in row {
                        // `[c{1:2}]` expands to the comma-separated list.
                        for v in self.eval_multi(cell)? {
                            any_cell |= matches!(v, Value::Cell(_));
                            rv.push(v);
                        }
                    }
                    grid.push(rv);
                }
                if any_cell {
                    let mut built: Option<CellArray> = None;
                    for row in &grid {
                        let mut rowmat: Option<CellArray> = None;
                        for v in row {
                            let c = match v {
                                Value::Cell(c) => c.clone(),
                                _ => return Err(LangError::new("cannot mix cells and non-cells in a [...] concatenation — use {…}", 0)),
                            };
                            rowmat = Some(match rowmat {
                                None => c,
                                Some(prev) => CellArray::hcat(&prev, &c)?,
                            });
                        }
                        if let Some(rm) = rowmat {
                            built = Some(match built {
                                None => rm,
                                Some(prev) => CellArray::vcat(&prev, &rm)?,
                            });
                        }
                    }
                    return Ok(Some(Value::Cell(built.unwrap_or_else(|| CellArray::new(0, 0)))));
                }
                // Sparse concat: `[S T]` / `[S; T]` stays sparse when all
                // elements are sparse.
                if grid.iter().flatten().all(|v| matches!(v, Value::Sparse(_))) && grid.iter().any(|r| !r.is_empty()) {
                    let mut built: Option<matcore::SparseMat> = None;
                    for row in &grid {
                        let mut rowmat: Option<matcore::SparseMat> = None;
                        for v in row {
                            let s = match v {
                                Value::Sparse(s) => s.clone(),
                                _ => continue,
                            };
                            rowmat = Some(match rowmat {
                                None => s,
                                Some(prev) => prev.hcat(&s).map_err(lang)?,
                            });
                        }
                        if let Some(rm) = rowmat {
                            built = Some(match built {
                                None => rm,
                                Some(prev) => prev.vcat(&rm).map_err(lang)?,
                            });
                        }
                    }
                    return Ok(Some(Value::Sparse(built.unwrap_or_default())));
                }
                // Struct concat: `[s1 s2]` / `[s1; s2]` when every element
                // is a struct (same field-set requirement lives in hcat/vcat).
                if grid.iter().flatten().all(|v| matches!(v, Value::Struct(_))) && grid.iter().any(|r| !r.is_empty()) {
                    let mut built: Option<StructArray> = None;
                    for row in &grid {
                        let mut rowmat: Option<StructArray> = None;
                        for v in row {
                            let s = match v {
                                Value::Struct(s) => s.clone(),
                                _ => continue,
                            };
                            rowmat = Some(match rowmat {
                                None => s,
                                Some(prev) => StructArray::hcat(&prev, &s)?,
                            });
                        }
                        if let Some(rm) = rowmat {
                            built = Some(match built {
                                None => rm,
                                Some(prev) => StructArray::vcat(&prev, &rm)?,
                            });
                        }
                    }
                    return Ok(Some(Value::Struct(built.unwrap_or_default())));
                }
                // Char context: any `Str` element pulls the whole literal
                // to char — numerics become code points, chars expand.
                // Single-row results come out as `Str`; multi-row char
                // arrays need a 2-D char type we don't have (error).
                let any_char = grid.iter().flatten().any(|v| matches!(v, Value::Str(_)));
                // Complex context: any `Cplx` element promotes the literal.
                if grid.iter().flatten().any(|v| matches!(v, Value::Cplx(_))) {
                    let mut built: Option<CMatrix> = None;
                    for row in &grid {
                        let mut rowmat: Option<CMatrix> = None;
                        for v in row {
                            let m = v.to_cmatrix()?;
                            rowmat = Some(match rowmat {
                                None => m,
                                Some(prev) => {
                                    check_elems(prev.numel().saturating_add(m.numel()))?;
                                    CMatrix::hcat(&prev, &m).map_err(lang)?
                                }
                            });
                        }
                        if let Some(rm) = rowmat {
                            built = Some(match built {
                                None => rm,
                                Some(prev) => {
                                    check_elems(prev.numel().saturating_add(rm.numel()))?;
                                    CMatrix::vcat(&prev, &rm).map_err(lang)?
                                }
                            });
                        }
                    }
                    return Ok(Some(cmat_or_value(built.unwrap_or_else(|| CMatrix::zeros(0, 0)))));
                }
                let mut built: Option<Matrix> = None;
                for row in &grid {
                    let mut rowmat: Option<Matrix> = None;
                    for v in row {
                        let m = if any_char { v.to_numeric()? } else { v.to_matrix()? };
                        rowmat = Some(match rowmat {
                            None => m,
                            Some(prev) => {
                                check_elems(prev.numel().saturating_add(m.numel()))?;
                                Matrix::hcat(&prev, &m).map_err(lang)?
                            }
                        });
                    }
                    if let Some(rm) = rowmat {
                        built = Some(match built {
                            None => rm,
                            Some(prev) => {
                                check_elems(prev.numel().saturating_add(rm.numel()))?;
                                Matrix::vcat(&prev, &rm).map_err(lang)?
                            }
                        });
                    }
                }
                let out = built.unwrap_or_else(|| Matrix::zeros(0, 0));
                if any_char {
                    if out.rows() > 1 {
                        return Err(LangError::new("multi-row char arrays are not supported (char is a row vector)", 0));
                    }
                    let s: String =
                        (0..out.cols()).filter_map(|i| out.get(0, i)).map(|x| char::from_u32(x.max(0.0) as u32).unwrap_or('\u{FFFD}')).collect();
                    return Ok(Some(Value::Str(s)));
                }
                Ok(Some(Value::Mat(out)))
            }
            Expr::CellMat(rows) => {
                // `{c{1:2}}` — comma-separated lists expand per element.
                let mut grid: Vec<Vec<Value>> = Vec::with_capacity(rows.len());
                for row in rows {
                    let mut rv = Vec::with_capacity(row.len());
                    for cell in row {
                        rv.extend(self.eval_multi(cell)?);
                    }
                    grid.push(rv);
                }
                let cols = grid.iter().map(|r| r.len()).max().unwrap_or(0);
                if grid.iter().any(|r| r.len() != cols) {
                    return Err(LangError::new("rows of a cell array must have the same number of elements", 0));
                }
                let mut ca = CellArray::new(grid.len(), cols);
                for (r, row) in grid.iter().enumerate() {
                    for (c, v) in row.iter().enumerate() {
                        ca.set(r, c, v.clone());
                    }
                }
                Ok(Some(Value::Cell(ca)))
            }
            Expr::BraceIndex(name, args) => {
                let ca = match self.vars.get(name) {
                    Some(Value::Cell(c)) => c.clone(),
                    Some(_) => return Err(LangError::new(format!("'{name}' is not a cell array (use parens for matrix indexing)"), 0)),
                    None => return Err(LangError::new(format!("undefined variable '{name}'"), 0)),
                };
                self.cell_content(&ca, args)
            }
            Expr::Field(base, field) => {
                let v = self.eval(base)?;
                match v {
                    Value::Struct(m) => match m.field_values(field) {
                        Some(mut vals) if vals.len() == 1 => Ok(vals.pop()),
                        Some(vals) => Err(LangError::new(
                            format!("expected one output from a curly brace or dot indexing expression, but there were {} results", vals.len()),
                            0,
                        )),
                        None => Err(LangError::new(format!("reference to non-existent field '{field}'"), 0)),
                    },
                    _ => Err(LangError::new("'.' field access needs a struct", 0)),
                }
            }
            Expr::DynField(base, name) => {
                // `s.(expr)` — the field name comes from an expression.
                let field = match self.eval(name)? {
                    Value::Str(s) => s,
                    _ => return Err(LangError::new("dynamic field name must be a char", 0)),
                };
                match self.eval(base)? {
                    Value::Struct(m) => match m.field_values(&field) {
                        Some(mut vals) if vals.len() == 1 => Ok(vals.pop()),
                        Some(vals) => Err(LangError::new(
                            format!("expected one output from a curly brace or dot indexing expression, but there were {} results", vals.len()),
                            0,
                        )),
                        None => Err(LangError::new(format!("reference to non-existent field '{field}'"), 0)),
                    },
                    _ => Err(LangError::new("'.' field access needs a struct", 0)),
                }
            }
            Expr::Index(base, args) => {
                // Chained subscript: `c{1}(2)`, `s.f(i)` — index the value
                // the base expression produced. A handle is called instead:
                // `c{1}(x)` when `c{1}` is a function handle calls it.
                let v = self.eval(base)?;
                if let Value::Func(h) = v {
                    let argv = self.eval_args(args)?;
                    return Ok(self.call_handle(&h, argv, 1)?.into_iter().next());
                }
                self.index_into(&v, args).map(Some)
            }
            Expr::BraceIdx(base, args) => {
                let v = self.eval(base)?;
                match v {
                    Value::Cell(ca) => self.cell_content(&ca, args),
                    _ => Err(LangError::new("brace indexing needs a cell array", 0)),
                }
            }
            Expr::Unary(op, e) => {
                let v = self.eval(e)?;
                if let Value::Sparse(s) = &v {
                    return match op {
                        UnOp::Neg => Ok(Some(Value::Sparse(s.map(|x| -x)))),
                        UnOp::Pos => Ok(Some(v)),
                        UnOp::Not => Ok(Some(mat_or_num(s.full().map(|x| if x == 0.0 { 1.0 } else { 0.0 })))),
                    };
                }
                if let Value::Cplx(m) = &v {
                    return match op {
                        UnOp::Neg => Ok(Some(cmat_or_value(m.map(|z| z.neg())))),
                        UnOp::Pos => Ok(Some(v)),
                        UnOp::Not => Err(LangError::new("logical NOT requires a real operand (complex input)", 0)),
                    };
                }
                let m = v.to_numeric()?;
                let out = match op {
                    UnOp::Neg => m.map(|x| -x),
                    UnOp::Pos => m,
                    UnOp::Not => m.map(|x| if x == 0.0 { 1.0 } else { 0.0 }),
                };
                Ok(Some(mat_or_num(out)))
            }
            Expr::Transpose(e, conj) => {
                let v = self.eval(e)?;
                // `x'` on a char array: MATLAB yields a char column; our
                // `Str` has no shape to flip, so it promotes to a double
                // column of code points. Class change is a documented
                // deviation until a char-matrix type exists.
                if let Value::Str(_) = &v {
                    let m = v.to_numeric()?;
                    return Ok(Some(mat_or_num(m.transpose())));
                }
                if let Value::Cell(ca) = &v {
                    let mut out = CellArray::new(ca.cols, ca.rows);
                    for r in 0..ca.rows {
                        for c in 0..ca.cols {
                            if let Some(x) = ca.get(r, c) {
                                out.set(c, r, x.clone());
                            }
                        }
                    }
                    return Ok(Some(Value::Cell(out)));
                }
                if let Value::Sparse(s) = &v {
                    // Real sparse: `'` and `.'` are the same transpose.
                    let _ = conj;
                    return Ok(Some(Value::Sparse(s.transpose())));
                }
                if let Value::Cplx(m) = &v {
                    // `'` is conjugate transpose; `.'` plain.
                    let out = if *conj { m.ctranspose() } else { m.transpose() };
                    return Ok(Some(cmat_or_value(out)));
                }
                let m = v.to_matrix()?;
                Ok(Some(mat_or_num(m.transpose())))
            }
            Expr::Binary(op, a, b) => {
                // `&&` / `||` short-circuit: the right side is not evaluated
                // when the left decides the result.
                match op {
                    BinOp::ScAnd => {
                        if !self.eval(a)?.truthy()? {
                            return Ok(Some(Value::Num(0.0)));
                        }
                        let r = self.eval(b)?.truthy()?;
                        return Ok(Some(Value::Num(if r { 1.0 } else { 0.0 })));
                    }
                    BinOp::ScOr => {
                        if self.eval(a)?.truthy()? {
                            return Ok(Some(Value::Num(1.0)));
                        }
                        let r = self.eval(b)?.truthy()?;
                        return Ok(Some(Value::Num(if r { 1.0 } else { 0.0 })));
                    }
                    _ => {}
                }
                let va = self.eval(a)?;
                let vb = self.eval(b)?;
                self.binary(*op, va, vb).map(Some)
            }
            Expr::CallOrIndex(name, args) => {
                if self.vars.contains_key(name) {
                    let target = self.vars.get(name).cloned();
                    match target {
                        // `f(args)` on a function handle calls it — handles
                        // cannot be subscripted.
                        Some(Value::Func(h)) => {
                            let argv = self.eval_args(args)?;
                            Ok(self.call_handle(&h, argv, 1)?.into_iter().next())
                        }
                        Some(t) => self.index_into(&t, args).map(Some),
                        None => Err(LangError::new(format!("undefined variable '{name}'"), 0)),
                    }
                } else if self.is_function(name)? {
                    let outs = self.call_function(name, args, 1)?;
                    Ok(outs.into_iter().next())
                } else {
                    self.builtin(name, args)
                }
            }
            Expr::FuncRef(name) => Ok(Some(Value::Func(FuncHandle::Named(name.clone())))),
            Expr::AnonFunc(params, body) => {
                // The closure captures the whole visible workspace now —
                // later edits to those variables don't reach it.
                Ok(Some(Value::Func(FuncHandle::Closure { params: params.clone(), body: body.clone(), env: self.vars.clone() })))
            }
        }
    }

    fn indexed_assign(&mut self, name: &str, index: &[Expr], v: Value, braces: bool) -> Result<(), LangError> {
        if braces {
            // `c{i} = v` — content assignment; creates/grows the cell.
            let ca = match self.vars.remove(name) {
                Some(Value::Cell(c)) => c,
                Some(_) => return Err(LangError::new(format!("'{name}' is not a cell array (clear it to reassign as a cell)"), 0)),
                None => CellArray::new(0, 0),
            };
            let ca = self.cell_brace_apply(ca, index, v)?;
            self.vars.insert(name.to_string(), Value::Cell(ca));
            return Ok(());
        }
        if matches!(self.vars.get(name), Some(Value::Cell(_))) || matches!(v, Value::Cell(_)) {
            return self.cell_paren_assign(name, index, v);
        }
        // `s(i) = t` / `s(i) = []` — element write or delete on a struct
        // array; `paren_apply` handles growth and dimension rules.
        if matches!(self.vars.get(name), Some(Value::Struct(_))) {
            let cur = self.vars.remove(name).unwrap_or(Value::Struct(StructArray::new()));
            let new = self.paren_apply(cur, index, v)?;
            self.vars.insert(name.to_string(), new);
            return Ok(());
        }
        // `s(i) = 'x'` — write into a char array. Doubles and chars both
        // land as code points; growth pads with spaces (M-code semantics).
        if let Some(Value::Str(s)) = self.vars.get(name).cloned() {
            let len = s.chars().count();
            let mut target = Matrix::row_vector(s.chars().map(|c| c as u32 as f64).collect());
            let specs = self.index_specs(index, target.shape(), target.numel().max(1))?;
            let max_i = specs.rows.iter().chain(specs.cols.iter()).copied().max().unwrap_or(0);
            if max_i > len {
                let mut d = target.data().to_vec();
                d.resize(max_i, 32.0);
                target = Matrix::row_vector(d);
            }
            let rhs = v.to_numeric()?;
            // `s(i) = []` deletes characters.
            if rhs.numel() == 0 {
                let kept = matrix_delete(&target, &specs)?;
                let out: String =
                    (0..kept.numel()).filter_map(|i| kept.get_linear(i)).map(|x| char::from_u32(x.max(0.0) as u32).unwrap_or('\u{FFFD}')).collect();
                self.vars.insert(name.to_string(), Value::Str(out));
                return Ok(());
            }
            assign_into(&mut target, &specs, &rhs)?;
            let out: String =
                (0..target.numel()).filter_map(|i| target.get_linear(i)).map(|x| char::from_u32(x.max(0.0) as u32).unwrap_or('\u{FFFD}')).collect();
            self.vars.insert(name.to_string(), Value::Str(out));
            return Ok(());
        }
        // Sparse target: `S(i,j) = v` writes entries, `S(i,j) = []` clears
        // them; writing past the end grows the sparse dims (MATLAB rule).
        if let Some(Value::Sparse(mut s)) = self.vars.get(name).cloned() {
            let rhs = v.to_matrix()?;
            let specs = self.index_specs(index, s.shape(), s.numel().max(1))?;
            if rhs.numel() == 0 {
                // `S(idx) = []` clears the addressed entries.
                for &i in &specs.rows {
                    let i0 = i.saturating_sub(1);
                    if i0 < s.numel() {
                        let (r, c) = (i0 % s.rows().max(1), i0 / s.rows().max(1));
                        s.set(r, c, 0.0);
                    }
                }
            } else {
                let scalar_rhs = rhs.numel() == 1;
                if specs.linear {
                    for (o, &i) in specs.rows.iter().enumerate() {
                        let i0 = i.saturating_sub(1);
                        let (r, c) = (i0 % s.rows().max(1), i0 / s.rows().max(1));
                        s.set(r, c, if scalar_rhs { rhs.get_linear(0).unwrap_or(0.0) } else { rhs.get_linear(o).unwrap_or(0.0) });
                    }
                } else {
                    for (oc, &c) in specs.cols.iter().enumerate() {
                        for (or, &r) in specs.rows.iter().enumerate() {
                            let rv = if scalar_rhs { rhs.get_linear(0).unwrap_or(0.0) } else { rhs.get(or, oc).unwrap_or(0.0) };
                            s.set(r.saturating_sub(1), c.saturating_sub(1), rv);
                        }
                    }
                }
            }
            self.vars.insert(name.to_string(), Value::Sparse(s));
            return Ok(());
        }
        // Complex on either side promotes the target: `x(2) = 3+4i` on a
        // real `x` makes it complex — the M-code storage model.
        if matches!(self.vars.get(name), Some(Value::Cplx(_))) || matches!(v, Value::Cplx(_)) {
            let mut target = match self.vars.get(name) {
                Some(t) => t.to_cmatrix()?,
                None => CMatrix::zeros(0, 0),
            };
            let rhs = v.to_cmatrix()?;
            let specs = self.index_specs(index, target.shape(), target.numel().max(1))?;
            if rhs.numel() == 0 {
                let kept = cmatrix_delete(&target, &specs)?;
                self.vars.insert(name.to_string(), cmat_or_value(kept));
                return Ok(());
            }
            cassign_into(&mut target, &specs, &rhs)?;
            self.vars.insert(name.to_string(), cmat_or_value(target));
            return Ok(());
        }
        let mut target = match self.vars.get(name) {
            Some(t) => t.to_matrix()?,
            None => Matrix::zeros(0, 0),
        };
        // `x(i) = 'a'` — chars promote to code points when written into
        // a numeric array.
        let rhs = v.to_numeric()?;
        let specs = self.index_specs(index, target.shape(), target.numel())?;
        // `A(i) = []` — deletion on a matrix (`A(:,j)`, `A(i,:)` too).
        if rhs.numel() == 0 {
            let kept = matrix_delete(&target, &specs)?;
            self.vars.insert(name.to_string(), Value::Mat(kept));
            return Ok(());
        }
        assign_into(&mut target, &specs, &rhs)?;
        self.vars.insert(name.to_string(), Value::Mat(target));
        Ok(())
    }

    /// `c{i}` — pick cell contents; multi-element picks return a 1×n cell
    /// (comma-separated lists aren't implemented — documented deviation).
    /// Fetch the contents selected by `c{…}` as a list — the building
    /// block of comma-separated-list expansion.
    fn cell_pick(&mut self, ca: &CellArray, args: &[Expr]) -> Result<Vec<Value>, LangError> {
        let specs = self.index_specs(args, ca.shape(), ca.numel().max(1))?;
        if specs.linear {
            specs
                .rows
                .iter()
                .map(|&i| {
                    ca.get_linear(i.saturating_sub(1))
                        .cloned()
                        .ok_or_else(|| LangError::new(format!("index {i} out of bounds ({} cells)", ca.numel()), 0))
                })
                .collect()
        } else {
            let mut out = Vec::with_capacity(specs.rows.len() * specs.cols.len());
            for &c in &specs.cols {
                for &r in &specs.rows {
                    out.push(
                        ca.get(r.saturating_sub(1), c.saturating_sub(1))
                            .cloned()
                            .ok_or_else(|| LangError::new(format!("index ({r},{c}) out of bounds {:?}", ca.shape()), 0))?,
                    );
                }
            }
            Ok(out)
        }
    }

    /// Scalar-context `c{…}`: exactly one element or the M-code error.
    fn cell_content(&mut self, ca: &CellArray, args: &[Expr]) -> Result<Option<Value>, LangError> {
        let picked = self.cell_pick(ca, args)?;
        match picked.len() {
            1 => Ok(picked.into_iter().next()),
            n => Err(LangError::new(format!("expected one output from a curly brace or dot indexing expression, but there were {n} results"), 0)),
        }
    }

    /// Evaluate an expression that may produce a comma-separated list:
    /// `c{1:3}` / `s.c{:}` yield one value per selected cell; any other
    /// expression yields exactly one. Used in argument lists, `[…]`/`{…}`
    /// literals, multi-target assignment, and index positions.
    fn eval_multi(&mut self, e: &Expr) -> Result<Vec<Value>, LangError> {
        match e {
            Expr::BraceIndex(name, args) => {
                let ca = match self.vars.get(name) {
                    Some(Value::Cell(c)) => c.clone(),
                    Some(_) => return Err(LangError::new(format!("'{name}' is not a cell array (use parens for matrix indexing)"), 0)),
                    None => return Err(LangError::new(format!("undefined variable '{name}'"), 0)),
                };
                self.cell_pick(&ca, args)
            }
            Expr::BraceIdx(base, args) => {
                // The base of the chain must yield exactly one value.
                match self.eval(base)? {
                    Value::Cell(ca) => self.cell_pick(&ca, args),
                    _ => Err(LangError::new("brace indexing needs a cell array", 0)),
                }
            }
            Expr::Field(base, field) => {
                // `s.f` on a struct array yields one value per element.
                match self.eval(base)? {
                    Value::Struct(m) => match m.field_values(field) {
                        Some(vals) => Ok(vals),
                        None => Err(LangError::new(format!("reference to non-existent field '{field}'"), 0)),
                    },
                    other => {
                        let _ = other;
                        Ok(self.eval_inner(e)?.into_iter().collect())
                    }
                }
            }
            Expr::DynField(base, name) => {
                let field = match self.eval(name)? {
                    Value::Str(s) => s,
                    _ => return Err(LangError::new("dynamic field name must be a char", 0)),
                };
                match self.eval(base)? {
                    Value::Struct(m) => match m.field_values(&field) {
                        Some(vals) => Ok(vals),
                        None => Err(LangError::new(format!("reference to non-existent field '{field}'"), 0)),
                    },
                    _ => Ok(self.eval_inner(e)?.into_iter().collect()),
                }
            }
            // Non-list expressions yield one value — or none, for builtins
            // that only print (`disp`, `clear`, …).
            _ => Ok(self.eval_inner(e)?.into_iter().collect()),
        }
    }

    /// `s.a.b = v` — assign `v` at the end of `path`, creating intermediate
    /// structs. `index` holds the trailing `(i)`/`{i}` subscript ops
    /// applied to the field's value (may be several: `s.c{1}(i) = v`).
    /// `map` is owned-detached (the caller removed the root from `vars`)
    /// so `self` stays borrowable for index evaluation.
    /// Apply a resolved assignment path to `cur`, returning the updated
    /// container. Steps interleave freely: `Name` descends a struct field
    /// (auto-vivifying `[]`/missing fields into structs), `Paren`/`Brace`
    /// subscript into matrices and cells. Mid-path steps fetch the slot,
    /// recurse, and write the result back — so `s.c{1}.f = v` works.
    ///
    /// `…(i) = []` deletion works at any depth: the leaf `paren_apply`
    /// calls `matrix_delete`, and mid-path steps write the shrunk value back.
    fn path_set(&mut self, cur: Value, path: &[PathStep], v: Value, root: &str) -> Result<Value, LangError> {
        let Some((step, rest)) = path.split_first() else { return Ok(v) };
        match step {
            PathStep::Name(field) => {
                let mut sa = match cur {
                    Value::Struct(m) => m,
                    // Assigning a field into `[]`/nothing creates a struct.
                    Value::Mat(e) if e.numel() == 0 => StructArray::new(),
                    _ => return Err(LangError::new(format!("'{root}' is not a struct — clear it to reassign"), 0)),
                };
                if !sa.is_scalar() {
                    return Err(LangError::new("dot name assignment on a non-scalar struct — subscript it first (s(i).f = v)", 0));
                }
                let sub = sa.field_at(0, field).cloned().unwrap_or_else(|| Value::Mat(Matrix::zeros(0, 0)));
                let new = self.path_set(sub, rest, v, root)?;
                sa.set_field(0, field, new)?;
                Ok(Value::Struct(sa))
            }
            // Dynamic names are resolved before the root is detached.
            PathStep::Dyn(_) => Err(LangError::new("internal: unresolved dynamic field step", 0)),
            PathStep::Paren(ix) => {
                if rest.is_empty() {
                    return self.paren_apply(cur, ix, v);
                }
                // `s(k).f = v` past the end grows the struct array — the
                // missing element starts as an empty 1x1 struct.
                let inner = match self.index_into(&cur, ix) {
                    Ok(inner) => inner,
                    Err(e) if matches!(cur, Value::Struct(_)) && matches!(rest.first(), Some(PathStep::Name(_))) => {
                        let _ = e;
                        Value::Struct(StructArray::new())
                    }
                    Err(e) => return Err(e),
                };
                let new = self.path_set(inner, rest, v, root)?;
                self.paren_apply(cur, ix, new)
            }
            PathStep::Brace(ix) => {
                let ca = match cur {
                    Value::Cell(c) => c,
                    Value::Mat(m) if m.numel() == 0 => CellArray::new(0, 0),
                    _ => return Err(LangError::new("brace indexing needs a cell array", 0)),
                };
                if rest.is_empty() {
                    return Ok(Value::Cell(self.cell_brace_apply(ca, ix, v)?));
                }
                let inner = self.cell_content(&ca, ix)?.ok_or_else(|| LangError::new("empty cell selection", 0))?;
                let new = self.path_set(inner, rest, v, root)?;
                Ok(Value::Cell(self.cell_brace_apply(ca, ix, new)?))
            }
        }
    }

    /// `x(i) = v` on an owned value — a paren subscript inside a chain or
    /// a single `s.f(i) = v`. Parens on a cell (or a cell RHS) assign
    /// cell elements via `cell_paren_apply`; otherwise it's a numeric
    /// `assign_into` write.
    fn paren_apply(&mut self, target: Value, index: &[Expr], v: Value) -> Result<Value, LangError> {
        if let Value::Struct(sa) = target {
            let mut sa = sa;
            let specs = self.index_specs(index, (sa.rows(), sa.cols()), sa.numel().max(1))?;
            // Linear indices into the element list (1-based).
            let idx: Vec<usize> = if specs.linear {
                specs.rows.iter().map(|i| i.saturating_sub(1)).collect()
            } else {
                let mut out = Vec::with_capacity(specs.rows.len() * specs.cols.len());
                for &c in &specs.cols {
                    for &r in &specs.rows {
                        out.push(r.saturating_sub(1) + c.saturating_sub(1) * sa.rows());
                    }
                }
                out
            };
            match v {
                // `s(i) = []` deletes elements; a null assignment may have
                // only one non-colon index (same rule as matrices).
                Value::Mat(m) if m.numel() == 0 => {
                    let total = sa.numel();
                    let mut keep = vec![true; total];
                    for &i in &idx {
                        if i >= total {
                            return Err(LangError::new("index exceeds the number of struct array elements", 0));
                        }
                        keep[i] = false;
                    }
                    let kept = keep.iter().filter(|&&k| k).count();
                    let (r, c) = if sa.cols() == 1 { (kept, 1) } else { (1, kept.max(1)) };
                    sa.delete_elements(&keep, if kept == 0 { 0 } else { r }, if kept == 0 { 0 } else { c });
                    return Ok(Value::Struct(sa));
                }
                Value::Struct(src) => {
                    sa.assign_elements(&idx, &src)?;
                    return Ok(Value::Struct(sa));
                }
                _ => return Err(LangError::new("subscripted assignment into a struct needs a struct value", 0)),
            }
        }
        if matches!(target, Value::Cell(_)) || matches!(v, Value::Cell(_)) {
            let ca = match target {
                Value::Cell(c) => c,
                Value::Mat(m) if m.numel() == 0 => CellArray::new(0, 0),
                _ => return Err(LangError::new("a matrix, not a cell — cannot assign cell contents", 0)),
            };
            return Ok(Value::Cell(self.cell_paren_apply(ca, index, v)?));
        }
        if let Value::Str(s) = &target {
            // Char-array write through a chain (`s.f(2) = 'x'`).
            let len = s.chars().count();
            let mut m = Matrix::row_vector(s.chars().map(|c| c as u32 as f64).collect());
            let specs = self.index_specs(index, m.shape(), m.numel().max(1))?;
            let max_i = specs.rows.iter().chain(specs.cols.iter()).copied().max().unwrap_or(0);
            if max_i > len {
                let mut d = m.data().to_vec();
                d.resize(max_i, 32.0);
                m = Matrix::row_vector(d);
            }
            let rhs = v.to_numeric()?;
            if rhs.numel() == 0 {
                let kept = matrix_delete(&m, &specs)?;
                let out: String =
                    (0..kept.numel()).filter_map(|i| kept.get_linear(i)).map(|x| char::from_u32(x.max(0.0) as u32).unwrap_or('\u{FFFD}')).collect();
                return Ok(Value::Str(out));
            }
            assign_into(&mut m, &specs, &rhs)?;
            let out: String =
                (0..m.numel()).filter_map(|i| m.get_linear(i)).map(|x| char::from_u32(x.max(0.0) as u32).unwrap_or('\u{FFFD}')).collect();
            return Ok(Value::Str(out));
        }
        if matches!(target, Value::Cplx(_)) || matches!(v, Value::Cplx(_)) {
            let mut m = target.to_cmatrix()?;
            let rhs = v.to_cmatrix()?;
            let specs = self.index_specs(index, m.shape(), m.numel().max(1))?;
            if rhs.numel() == 0 {
                return Ok(cmat_or_value(cmatrix_delete(&m, &specs)?));
            }
            cassign_into(&mut m, &specs, &rhs)?;
            return Ok(cmat_or_value(m));
        }
        let mut m = target.to_matrix()?;
        let rhs = v.to_numeric()?;
        let specs = self.index_specs(index, m.shape(), m.numel())?;
        if rhs.numel() == 0 {
            return Ok(Value::Mat(matrix_delete(&m, &specs)?));
        }
        assign_into(&mut m, &specs, &rhs)?;
        Ok(Value::Mat(m))
    }

    /// `c{i} = v` — content assignment into an owned cell array; creates and
    /// grows the cell. Shared by variable and field assignment.
    fn cell_brace_apply(&mut self, mut ca: CellArray, index: &[Expr], v: Value) -> Result<CellArray, LangError> {
        let specs = self.index_specs(index, ca.shape(), ca.numel().max(1))?;
        if specs.linear {
            let max_i = specs.rows.iter().copied().max().unwrap_or(0);
            let (gr, gc) = if ca.cols == 1 && ca.rows > 1 {
                (max_i, 1)
            } else if ca.rows <= 1 {
                (1, max_i)
            } else {
                (max_i.div_ceil(ca.cols.max(1)), ca.cols)
            };
            ca.grow_capped(gr, gc)?;
            for &i in &specs.rows {
                ca.set_linear(i.saturating_sub(1), v.clone());
            }
        } else {
            let max_r = specs.rows.iter().copied().max().unwrap_or(0);
            let max_c = specs.cols.iter().copied().max().unwrap_or(0);
            ca.grow_capped(max_r, max_c)?;
            for &c in &specs.cols {
                for &r in &specs.rows {
                    ca.set(r.saturating_sub(1), c.saturating_sub(1), v.clone());
                }
            }
        }
        Ok(ca)
    }

    /// `c(i) = v` — paren assignment into a cell. `v` must be a cell (each
    /// slot gets the corresponding cell) or `[]` to delete those cells.
    fn cell_paren_assign(&mut self, name: &str, index: &[Expr], v: Value) -> Result<(), LangError> {
        let ca = match self.vars.remove(name) {
            Some(Value::Cell(c)) => c,
            Some(other) => {
                self.vars.insert(name.to_string(), other);
                return Err(LangError::new(format!("'{name}' is a matrix, not a cell — cannot assign cell contents"), 0));
            }
            None => CellArray::new(0, 0),
        };
        let ca = self.cell_paren_apply(ca, index, v)?;
        self.vars.insert(name.to_string(), Value::Cell(ca));
        Ok(())
    }

    /// The `c(i) = v` operation on an owned cell array — shared by variable
    /// assignment and `s.f(i) = …` field assignment.
    fn cell_paren_apply(&mut self, mut ca: CellArray, index: &[Expr], v: Value) -> Result<CellArray, LangError> {
        let specs = self.index_specs(index, ca.shape(), ca.numel().max(1))?;
        match v {
            Value::Cell(rhs) => {
                // Grow first, same orientation rules as brace assignment.
                let (gr, gc) = if specs.linear {
                    let max_i = specs.rows.iter().copied().max().unwrap_or(0);
                    if ca.cols == 1 && ca.rows > 1 {
                        (max_i, 1)
                    } else if ca.rows <= 1 {
                        (1, max_i)
                    } else {
                        (max_i.div_ceil(ca.cols.max(1)), ca.cols)
                    }
                } else {
                    (specs.rows.iter().copied().max().unwrap_or(0), specs.cols.iter().copied().max().unwrap_or(0))
                };
                ca.grow_capped(gr, gc)?;
                let idxs: Vec<usize> = if specs.linear {
                    specs.rows.clone()
                } else {
                    let mut lin = Vec::with_capacity(specs.rows.len() * specs.cols.len());
                    for &c in &specs.cols {
                        for &r in &specs.rows {
                            lin.push((c - 1) * ca.rows + r);
                        }
                    }
                    lin
                };
                let broadcast = rhs.numel() == 1;
                if !broadcast && rhs.numel() != idxs.len() {
                    return Err(LangError::new("cell assignment needs a scalar cell or a matching number of cells", 0));
                }
                for (k, &i) in idxs.iter().enumerate() {
                    let src = if broadcast { 0 } else { k };
                    if let Some(x) = rhs.get_linear(src) {
                        ca.set_linear(i.saturating_sub(1), x.clone());
                    }
                }
                Ok(ca)
            }
            Value::Mat(m) if m.numel() == 0 => {
                if specs.linear {
                    // `c(i) = []` deletes cells.
                    ca.remove_linear(&specs.rows)?;
                    return Ok(ca);
                }
                // `c(:,j) = []` / `c(i,:) = []` — whole column/row delete.
                // A null assignment may have only one non-colon index.
                let all_rows = specs.rows.len() == ca.rows;
                let all_cols = specs.cols.len() == ca.cols;
                match (all_rows, all_cols) {
                    (true, false) => ca.remove_cols(&specs.cols),
                    (false, true) => ca.remove_rows(&specs.rows),
                    (true, true) => ca = CellArray::new(0, 0),
                    (false, false) => {
                        return Err(LangError::new("a null assignment can have only one non-colon index", 0));
                    }
                }
                Ok(ca)
            }
            _ => Err(LangError::new("conversion to cell from a non-cell value — write c{i} = value instead", 0)),
        }
    }

    fn index_into(&mut self, target: &Value, args: &[Expr]) -> Result<Value, LangError> {
        if let Value::Cell(ca) = target {
            // `c(:)` on an empty cell yields an empty cell, not an index error.
            if ca.numel() == 0 && matches!(args, [Expr::Colon]) {
                return Ok(Value::Cell(CellArray::new(0, 0)));
            }
            // `c(i)` / `c(i, j)` — paren indexing keeps cells.
            let specs = self.index_specs(args, ca.shape(), ca.numel().max(1))?;
            if specs.linear {
                let mut out = Vec::with_capacity(specs.rows.len());
                for &i in &specs.rows {
                    match ca.get_linear(i.saturating_sub(1)) {
                        Some(v) => out.push(v.clone()),
                        None => return Err(LangError::new(format!("index {i} out of bounds ({} cells)", ca.numel()), 0)),
                    }
                }
                // `c(:)` always columnizes; a column cell keeps its shape.
                let col = matches!(args.first(), Some(Expr::Colon)) || (ca.cols == 1 && ca.rows > 1);
                return Ok(Value::Cell(if col { CellArray::col(out) } else { CellArray::row(out) }));
            }
            let mut out = CellArray::new(specs.rows.len(), specs.cols.len());
            for (oc, &c) in specs.cols.iter().enumerate() {
                for (or, &r) in specs.rows.iter().enumerate() {
                    match ca.get(r.saturating_sub(1), c.saturating_sub(1)) {
                        Some(v) => out.set(or, oc, v.clone()),
                        None => return Err(LangError::new(format!("index ({r},{c}) out of bounds {:?}", ca.shape()), 0)),
                    }
                }
            }
            return Ok(Value::Cell(out));
        }
        if let Value::Str(s) = target {
            // Strings index as 1xN row vectors of chars.
            let chars: Vec<char> = s.chars().collect();
            let specs = self.index_specs(args, (1, chars.len()), chars.len())?;
            let idxs = if specs.linear { &specs.rows } else { &specs.cols };
            let mut out = String::new();
            for &i in idxs {
                if let Some(&ch) = chars.get(i.saturating_sub(1)) {
                    out.push(ch);
                }
            }
            return Ok(Value::Str(out));
        }
        if let Value::Sparse(s) = target {
            // `S(i,j)` reads sparse → a sparse result (or scalar Num).
            let specs = self.index_specs(args, s.shape(), s.numel().max(1))?;
            if specs.linear {
                // Single-element linear read returns a scalar double.
                if specs.rows.len() == 1 {
                    let i0 = specs.rows.first().copied().unwrap_or(1).saturating_sub(1);
                    if i0 >= s.numel() {
                        return Err(LangError::new(format!("index out of bounds ({}x{} sparse)", s.rows(), s.cols()), 0));
                    }
                    return Ok(Value::Num(s.get(i0 % s.rows().max(1), i0 / s.rows().max(1))));
                }
                let mut out = matcore::SparseMat::zeros(specs.rows.len(), 1);
                for (o, &i) in specs.rows.iter().enumerate() {
                    let i0 = i.saturating_sub(1);
                    if i0 >= s.numel() {
                        return Err(LangError::new(format!("index {i} out of bounds ({}x{} sparse)", s.rows(), s.cols()), 0));
                    }
                    let (r, c) = (i0 % s.rows().max(1), i0 / s.rows().max(1));
                    let v = s.get(r, c);
                    if v != 0.0 {
                        out.set(o, 0, v);
                    }
                }
                return Ok(Value::Sparse(out));
            }
            // A 1x1 selection reads as a scalar double.
            if specs.rows.len() == 1 && specs.cols.len() == 1 {
                let (r0, c0) =
                    (specs.rows.first().copied().unwrap_or(1).saturating_sub(1), specs.cols.first().copied().unwrap_or(1).saturating_sub(1));
                if r0 >= s.rows() || c0 >= s.cols() {
                    return Err(LangError::new(format!("index out of bounds ({}x{} sparse)", s.rows(), s.cols()), 0));
                }
                return Ok(Value::Num(s.get(r0, c0)));
            }
            let mut out = matcore::SparseMat::zeros(specs.rows.len(), specs.cols.len());
            for (oc, &c) in specs.cols.iter().enumerate() {
                for (or, &r) in specs.rows.iter().enumerate() {
                    let (r0, c0) = (r.saturating_sub(1), c.saturating_sub(1));
                    if r0 >= s.rows() || c0 >= s.cols() {
                        return Err(LangError::new(format!("index ({r},{c}) out of bounds ({}x{} sparse)", s.rows(), s.cols()), 0));
                    }
                    let v = s.get(r0, c0);
                    if v != 0.0 {
                        out.set(or, oc, v);
                    }
                }
            }
            return Ok(Value::Sparse(out));
        }
        if let Value::Struct(sa) = target {
            let specs = self.index_specs(args, (sa.rows(), sa.cols()), sa.numel().max(1))?;
            if specs.linear {
                let idx: Vec<usize> = specs.rows.iter().map(|i| i.saturating_sub(1)).collect();
                // `s(:)` columnizes; a column vector stays a column.
                let col = matches!(args.first(), Some(Expr::Colon)) || (sa.cols() == 1 && sa.rows() > 1);
                let (r, c) = if idx.is_empty() {
                    (0, 0)
                } else if col {
                    (idx.len(), 1)
                } else {
                    (1, idx.len())
                };
                return Ok(Value::Struct(sa.pick(&idx, r, c)?));
            }
            let mut idx = Vec::with_capacity(specs.rows.len() * specs.cols.len());
            for &c in &specs.cols {
                for &r in &specs.rows {
                    idx.push(r.saturating_sub(1) + c.saturating_sub(1) * sa.rows());
                }
            }
            return Ok(Value::Struct(sa.pick(&idx, specs.rows.len().max(1), specs.cols.len().max(1))?));
        }
        if let Value::Cplx(m) = target {
            if m.numel() == 0 && matches!(args, [Expr::Colon]) {
                return Ok(Value::Mat(Matrix::zeros(0, 0)));
            }
            let specs = self.index_specs(args, m.shape(), m.numel())?;
            if specs.linear {
                let mut data = Vec::with_capacity(specs.rows.len());
                for i in &specs.rows {
                    match m.get_linear(i.saturating_sub(1)) {
                        Some(v) => data.push(v),
                        None => return Err(LangError::new(format!("index {i} out of bounds ({})", m.numel()), 0)),
                    }
                }
                let col = matches!(args.first(), Some(Expr::Colon)) || (m.cols() == 1 && m.rows() > 1);
                let (rows, cols) = if col { (data.len(), 1) } else { (1, data.len()) };
                let out = CMatrix::from_col_major(rows, cols, data).map_err(lang)?;
                return Ok(cmat_or_value(out));
            }
            let mut out = CMatrix::zeros(specs.rows.len(), specs.cols.len());
            for (oc, &c) in specs.cols.iter().enumerate() {
                for (or, &r) in specs.rows.iter().enumerate() {
                    match m.get(r.saturating_sub(1), c.saturating_sub(1)) {
                        Some(v) => {
                            let _ = out.set(or, oc, v);
                        }
                        None => return Err(LangError::new(format!("index ({r},{c}) out of bounds {:?}", m.shape()), 0)),
                    }
                }
            }
            return Ok(cmat_or_value(out));
        }
        let m = target.to_matrix()?;
        if m.numel() == 0 && matches!(args, [Expr::Colon]) {
            return Ok(Value::Mat(Matrix::zeros(0, 0)));
        }
        let specs = self.index_specs(args, m.shape(), m.numel())?;
        if specs.linear {
            let mut data = Vec::with_capacity(specs.rows.len());
            for i in &specs.rows {
                match m.get_linear(i.saturating_sub(1)) {
                    Some(v) => data.push(v),
                    None => return Err(LangError::new(format!("index {i} out of bounds ({})", m.numel()), 0)),
                }
            }
            // `A(:)` always columnizes; a column vector keeps its shape.
            let col = matches!(args.first(), Some(Expr::Colon)) || (m.cols() == 1 && m.rows() > 1);
            return Ok(Value::Mat(if col { Matrix::col_vector(data) } else { Matrix::row_vector(data) }));
        }
        let mut out = Matrix::zeros(specs.rows.len(), specs.cols.len());
        for (oc, &c) in specs.cols.iter().enumerate() {
            for (or, &r) in specs.rows.iter().enumerate() {
                match m.get(r.saturating_sub(1), c.saturating_sub(1)) {
                    Some(v) => {
                        let _ = out.set(or, oc, v);
                    }
                    None => return Err(LangError::new(format!("index ({r},{c}) out of bounds {:?}", m.shape()), 0)),
                }
            }
        }
        Ok(mat_or_num(out))
    }

    /// Evaluate index arguments to concrete 1-based index lists.
    /// Comma-separated lists expand: `A(c{1:2})` indexes as `A(1,2)`.
    fn index_specs(&mut self, args: &[Expr], shape: (usize, usize), numel: usize) -> Result<IndexSpec, LangError> {
        /// One declared subscript: the symbolic colon/end, or evaluated values.
        enum Sub {
            Colon,
            End,
            V(Vec<Value>),
        }
        let mut flat: Vec<Sub> = Vec::with_capacity(args.len());
        for (i, a) in args.iter().enumerate() {
            match a {
                Expr::Colon => flat.push(Sub::Colon),
                Expr::End => flat.push(Sub::End),
                _ => {
                    // `end` is resolved against the declared position's dim
                    // before evaluation (`A(1:end)`), then CSLs flatten.
                    let dim = if args.len() == 1 {
                        numel.max(1)
                    } else if i == 0 {
                        shape.0.max(1)
                    } else {
                        shape.1.max(1)
                    };
                    let resolved = resolve_end(a, dim);
                    flat.push(Sub::V(self.eval_multi(&resolved)?));
                }
            }
        }
        // Each expanded element is one subscript; e.g. one declared arg that
        // expands to two values becomes a (row, col) pair.
        let subs: Vec<Sub> = flat
            .into_iter()
            .flat_map(|s| match s {
                Sub::V(vals) => vals.into_iter().map(|v| Sub::V(vec![v])).collect::<Vec<_>>(),
                other => vec![other],
            })
            .collect();
        let conv = |sub: &Sub, dim_len: usize| -> Result<Vec<usize>, LangError> {
            match sub {
                Sub::Colon => Ok((1..=dim_len).collect()),
                Sub::End => Ok(vec![dim_len]),
                Sub::V(vals) => {
                    let v = vals.first().ok_or_else(|| LangError::new("empty index", 0))?;
                    index_list(v, dim_len)
                }
            }
        };
        match subs.len() {
            1 => Ok(IndexSpec { rows: conv(&subs[0], numel.max(1))?, cols: Vec::new(), linear: true }),
            2 => Ok(IndexSpec { rows: conv(&subs[0], shape.0.max(1))?, cols: conv(&subs[1], shape.1.max(1))?, linear: false }),
            n => Err(LangError::new(format!("expected 1 or 2 indices, got {n}"), 0)),
        }
    }

    fn binary(&mut self, op: BinOp, a: Value, b: Value) -> Result<Value, LangError> {
        // Either side complex → the whole op runs in complex arithmetic.
        if matches!(a, Value::Cplx(_)) || matches!(b, Value::Cplx(_)) {
            return self.cbinary(op, &a, &b);
        }
        // Both sparse → sparse ops (`+`,`-`,`.*`,`*`,`==`,`~=`); a mixed
        // sparse/full pair densifies via `to_numeric` (MATLAB rule).
        if let (Value::Sparse(sa), Value::Sparse(sb)) = (&a, &b) {
            let sparse = |r: Result<matcore::SparseMat, MatError>| r.map(Value::Sparse).map_err(lang);
            return match op {
                BinOp::Add => sparse(sa.add(sb)),
                BinOp::Sub => sparse(sa.sub(sb)),
                BinOp::ElemMul => sparse(sa.mul_elem(sb)),
                BinOp::Mul => sparse(sa.matmul(sb)),
                // Comparisons on sparse return a dense logical result
                // (we have no sparse-logical type).
                BinOp::Eq => cmp(&sa.full(), &sb.full(), |x, y| x == y),
                BinOp::Neq => cmp(&sa.full(), &sb.full(), |x, y| x != y),
                BinOp::Lt => cmp(&sa.full(), &sb.full(), |x, y| x < y),
                BinOp::Le => cmp(&sa.full(), &sb.full(), |x, y| x <= y),
                BinOp::Gt => cmp(&sa.full(), &sb.full(), |x, y| x > y),
                BinOp::Ge => cmp(&sa.full(), &sb.full(), |x, y| x >= y),
                _ => Err(LangError::new("that operator is not defined for two sparse matrices — convert with full()", 0)),
            };
        }
        // Sparse ∘ scalar: `S * 2`, `S .* k` stay sparse; `S + k` densifies
        // (falls through — `to_numeric` materializes the full matrix).
        if let (Value::Sparse(sa), Value::Num(k)) = (&a, &b) {
            match op {
                BinOp::Mul | BinOp::ElemMul => return Ok(Value::Sparse(sa.map(|v| v * k))),
                BinOp::ElemDiv => return Ok(Value::Sparse(sa.map(|v| v / k))),
                BinOp::ElemPow => return Ok(Value::Sparse(sa.map(|v| v.powf(*k)))),
                _ => {}
            }
        } else if let (Value::Num(k), Value::Sparse(sb)) = (&a, &b) {
            if matches!(op, BinOp::Mul | BinOp::ElemMul) {
                return Ok(Value::Sparse(sb.map(|v| v * k)));
            }
        }
        // Chars promote to double code points in numeric context.
        let ma = a.to_numeric()?;
        let mb = b.to_numeric()?;
        match op {
            BinOp::Add => ma.broadcast(&mb, |x, y| x + y).map(mat_or_num).map_err(lang),
            BinOp::Sub => ma.broadcast(&mb, |x, y| x - y).map(mat_or_num).map_err(lang),
            BinOp::ElemMul => ma.broadcast(&mb, |x, y| x * y).map(mat_or_num).map_err(lang),
            BinOp::ElemDiv => ma.broadcast(&mb, |x, y| x / y).map(mat_or_num).map_err(lang),
            BinOp::ElemPow => ma.broadcast(&mb, |x, y| x.powf(y)).map(mat_or_num).map_err(lang),
            BinOp::Mul => {
                // A (r×k) @ (k×c) product can blow up even when both operands
                // are legal (column × row outer product).
                check_elems(ma.rows().saturating_mul(mb.cols()))?;
                ma.matmul(&mb).map(mat_or_num).map_err(lang)
            }
            BinOp::Div => {
                // mrdivide: A/B == A * inv(B); scalars broadcast.
                if ma.is_scalar() || mb.is_scalar() {
                    ma.broadcast(&mb, |x, y| x / y).map(mat_or_num).map_err(lang)
                } else {
                    // x*B = A  =>  x = (B' \ A')'
                    mb.transpose().solve(&ma.transpose()).map(|m| mat_or_num(m.transpose())).map_err(lang)
                }
            }
            BinOp::Backslash => {
                // mldivide: A\B solves A*x = B; scalars broadcast.
                if ma.is_scalar() || mb.is_scalar() {
                    mb.broadcast(&ma, |x, y| x / y).map(mat_or_num).map_err(lang)
                } else {
                    ma.solve(&mb).map(mat_or_num).map_err(lang)
                }
            }
            BinOp::Pow => {
                if let (Some(x), Some(y)) = (ma.as_scalar(), mb.as_scalar()) {
                    Ok(Value::Num(x.powf(y)))
                } else if let Some(p) = mb.as_scalar() {
                    matrix_power(&ma, p).map(mat_or_num).map_err(lang)
                } else {
                    Err(LangError::new("matrix exponent must be scalar; use .^", 0))
                }
            }
            BinOp::And => ma.broadcast(&mb, |x, y| if x != 0.0 && y != 0.0 { 1.0 } else { 0.0 }).map(mat_or_num).map_err(lang),
            BinOp::Or => ma.broadcast(&mb, |x, y| if x != 0.0 || y != 0.0 { 1.0 } else { 0.0 }).map(mat_or_num).map_err(lang),
            BinOp::ScAnd | BinOp::ScOr => Err(LangError::new("internal: short-circuit handled in eval", 0)),
            BinOp::Eq => cmp(&ma, &mb, |x, y| x == y),
            BinOp::Neq => cmp(&ma, &mb, |x, y| x != y),
            BinOp::Lt => cmp(&ma, &mb, |x, y| x < y),
            BinOp::Le => cmp(&ma, &mb, |x, y| x <= y),
            BinOp::Gt => cmp(&ma, &mb, |x, y| x > y),
            BinOp::Ge => cmp(&ma, &mb, |x, y| x >= y),
        }
    }

    /// Complex arithmetic: either operand complex promotes both via
    /// `to_cmatrix`. Results with all-zero imaginary parts collapse back
    /// to real (`(3+4i)+(1-4i)` → `4`).
    fn cbinary(&mut self, op: BinOp, a: &Value, b: &Value) -> Result<Value, LangError> {
        let ca = a.to_cmatrix()?;
        let cb = b.to_cmatrix()?;
        match op {
            BinOp::Add => ca.broadcast(&cb, |x, y| x.add(y)).map(cmat_or_value).map_err(lang),
            BinOp::Sub => ca.broadcast(&cb, |x, y| x.sub(y)).map(cmat_or_value).map_err(lang),
            BinOp::ElemMul => ca.broadcast(&cb, |x, y| x.mul(y)).map(cmat_or_value).map_err(lang),
            BinOp::ElemDiv => ca.broadcast(&cb, |x, y| x.div(y)).map(cmat_or_value).map_err(lang),
            BinOp::ElemPow => ca.broadcast(&cb, |x, y| x.powc(y)).map(cmat_or_value).map_err(lang),
            BinOp::Mul => {
                check_elems(ca.rows().saturating_mul(cb.cols()))?;
                ca.matmul(&cb).map(cmat_or_value).map_err(lang)
            }
            BinOp::Div => {
                if ca.is_scalar() || cb.is_scalar() {
                    ca.broadcast(&cb, |x, y| x.div(y)).map(cmat_or_value).map_err(lang)
                } else {
                    cb.ctranspose().solve(&ca.ctranspose()).map(|m| cmat_or_value(m.ctranspose())).map_err(lang)
                }
            }
            BinOp::Backslash => {
                if ca.is_scalar() || cb.is_scalar() {
                    cb.broadcast(&ca, |x, y| x.div(y)).map(cmat_or_value).map_err(lang)
                } else {
                    ca.solve(&cb).map(cmat_or_value).map_err(lang)
                }
            }
            BinOp::Pow => {
                if let (Some(x), Some(y)) = (ca.as_scalar(), cb.as_scalar()) {
                    // `z^n` integer exponent is exact; fractional uses
                    // the polar form.
                    if y.is_real() && y.re.fract() == 0.0 && y.re.abs() < 1e9 {
                        return Ok(cmat_or_value(CMatrix::scalar(x.powi(y.re as i32))));
                    }
                    return Ok(cmat_or_value(CMatrix::scalar(x.powc(y))));
                }
                if let Some(p) = cb.as_scalar() {
                    if p.is_real() {
                        // Real exponent on a complex matrix: reuse the
                        // real matrix-power path when the base is real.
                        if let (Some(mr),) = (ca.real_if_real(),) {
                            return matrix_power(&mr, p.re).map(mat_or_num).map_err(lang);
                        }
                    }
                }
                Err(LangError::new("matrix exponent must be scalar; use .^", 0))
            }
            // Comparisons: `==`/`~=` compare re+im; ordering operators use
            // only the real part (documented M-code semantics).
            BinOp::Eq => ccmp(&ca, &cb, |x, y| x == y),
            BinOp::Neq => ccmp(&ca, &cb, |x, y| x != y),
            BinOp::Lt => ccmp(&ca, &cb, |x, y| x.re < y.re),
            BinOp::Le => ccmp(&ca, &cb, |x, y| x.re <= y.re),
            BinOp::Gt => ccmp(&ca, &cb, |x, y| x.re > y.re),
            BinOp::Ge => ccmp(&ca, &cb, |x, y| x.re >= y.re),
            // `&`/`|` on complex error — logical ops need reals.
            BinOp::And | BinOp::Or | BinOp::ScAnd | BinOp::ScOr => Err(LangError::new("logical operators require real operands (complex input)", 0)),
        }
    }

    fn constant(&self, name: &str) -> Option<Value> {
        match name {
            "pi" => Some(Value::Num(std::f64::consts::PI)),
            "eps" => Some(Value::Num(f64::EPSILON)),
            "inf" | "Inf" => Some(Value::Num(f64::INFINITY)),
            "nan" | "NaN" => Some(Value::Num(f64::NAN)),
            "true" => Some(Value::Num(1.0)),
            "false" => Some(Value::Num(0.0)),
            "i" | "j" => Some(Value::Cplx(CMatrix::scalar(CNum::I))),
            _ => None,
        }
    }

    fn next_f64(&mut self) -> f64 {
        // SplitMix64 — fast, dependency-free PRNG.
        self.rng = self.rng.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Builtin functions. Returns `Ok(None)` for commands that only print.
    fn builtin(&mut self, name: &str, args: &[Expr]) -> Result<Option<Value>, LangError> {
        match name {
            "zeros" | "ones" | "eye" | "rand" | "randn" => {
                let (r, c) = self.dims(args)?;
                let m = match name {
                    "zeros" => Matrix::zeros(r, c),
                    "ones" => Matrix::ones(r, c),
                    "eye" => Matrix::eye(r, c),
                    "rand" => {
                        let mut data = Vec::with_capacity(r.saturating_mul(c));
                        for _ in 0..r.saturating_mul(c) {
                            data.push(self.next_f64());
                        }
                        Matrix::from_col_major(r, c, data).map_err(lang)?
                    }
                    _ => {
                        // randn: sum-of-12 uniform approximation to N(0,1).
                        let mut data = Vec::with_capacity(r.saturating_mul(c));
                        for _ in 0..r.saturating_mul(c) {
                            let mut acc = 0.0;
                            for _ in 0..12 {
                                acc += self.next_f64();
                            }
                            data.push(acc - 6.0);
                        }
                        Matrix::from_col_major(r, c, data).map_err(lang)?
                    }
                };
                Ok(Some(mat_or_num(m)))
            }
            "linspace" => {
                let v = self.eval_args(args)?;
                let (a, b) = (v.scalar_at(0)?, v.scalar_at(1)?);
                let n = if v.len() > 2 { v.scalar_at(2)? as usize } else { 100 };
                check_elems(n)?;
                let mut data = Vec::with_capacity(n);
                for i in 0..n {
                    let t = if n > 1 { i as f64 / (n - 1) as f64 } else { 0.0 };
                    data.push(a + (b - a) * t);
                }
                Ok(Some(Value::Mat(Matrix::row_vector(data))))
            }
            "magic" => {
                let v = self.eval_args(args)?;
                let n = v.scalar_at(0)? as usize;
                check_elems(n.saturating_mul(n))?;
                Ok(Some(Value::Mat(magic_square(n))))
            }
            "size" => {
                let v = self.eval_args(args)?;
                let (r, c) = v.first().map(Value::dims).unwrap_or((0, 0));
                Ok(Some(Value::Mat(Matrix::row_vector(vec![r as f64, c as f64]))))
            }
            "numel" => {
                let v = self.eval_args(args)?;
                let (r, c) = v.first().map(Value::dims).unwrap_or((0, 0));
                Ok(Some(Value::Num((r * c) as f64)))
            }
            "length" => {
                let v = self.eval_args(args)?;
                let (r, c) = v.first().map(Value::dims).unwrap_or((0, 0));
                Ok(Some(Value::Num(r.max(c) as f64)))
            }
            "cell" => {
                // cell(n) | cell(m, n) | cell([m n]) — `[]`-filled array.
                let v = self.eval_args(args)?;
                let (r, c) = match v.len() {
                    0 => (0, 0),
                    1 => {
                        let m = v[0].to_matrix()?;
                        if m.numel() == 1 {
                            let n = m.data().first().copied().unwrap_or(0.0) as usize;
                            (n, n)
                        } else {
                            (m.get_linear(0).unwrap_or(0.0) as usize, m.get_linear(1).unwrap_or(0.0) as usize)
                        }
                    }
                    _ => (v.scalar_at(0)? as usize, v.scalar_at(1)? as usize),
                };
                if r.saturating_mul(c) > MAX_CELL_ELEMS {
                    return Err(LangError::new("allocation exceeds limit", 0));
                }
                Ok(Some(Value::Cell(CellArray::new(r, c))))
            }
            "iscell" => {
                let v = self.eval_args(args)?;
                Ok(Some(Value::Num(matches!(v.first(), Some(Value::Cell(_))) as u8 as f64)))
            }
            "iscellstr" => {
                let v = self.eval_args(args)?;
                let ok = matches!(v.first(), Some(Value::Cell(c)) if c.items.iter().all(|x| matches!(x, Value::Str(_))));
                Ok(Some(Value::Num(ok as u8 as f64)))
            }
            "cell2mat" => {
                let v = self.eval_args(args)?;
                let ca = match v.first() {
                    Some(Value::Cell(c)) => c,
                    _ => return Err(LangError::new("cell2mat needs a cell array", 0)),
                };
                let mut built: Option<Matrix> = None;
                for r in 0..ca.rows {
                    let mut rowmat: Option<Matrix> = None;
                    for c in 0..ca.cols {
                        let m = ca.get(r, c).ok_or_else(|| LangError::new("cell2mat: bad index", 0))?.to_matrix()?;
                        rowmat = Some(match rowmat {
                            None => m,
                            Some(prev) => {
                                check_elems(prev.numel().saturating_add(m.numel()))?;
                                Matrix::hcat(&prev, &m).map_err(lang)?
                            }
                        });
                    }
                    if let Some(rm) = rowmat {
                        built = Some(match built {
                            None => rm,
                            Some(prev) => {
                                check_elems(prev.numel().saturating_add(rm.numel()))?;
                                Matrix::vcat(&prev, &rm).map_err(lang)?
                            }
                        });
                    }
                }
                Ok(Some(Value::Mat(built.unwrap_or_else(|| Matrix::zeros(0, 0)))))
            }
            // ---- sparse builtins ----
            "sparse" => {
                let v = self.eval_args(args)?;
                match (v.first(), v.get(1)) {
                    // sparse(m,n) → empty; sparse(A) → sparsify; the
                    // triplet forms sparse(i,j,v[,m,n]).
                    (Some(x), None) => match x {
                        Value::Sparse(s) => Ok(Some(Value::Sparse(s.clone()))),
                        _ => {
                            let m = x.to_matrix()?;
                            Ok(Some(Value::Sparse(matcore::SparseMat::from_dense(&m))))
                        }
                    },
                    (Some(mv), Some(nv)) if v.len() == 2 => {
                        // Two scalars → `sparse(m,n)` empty; two vectors →
                        // `sparse(i,j)` with implicit 1.0 values.
                        let (ma, mb) = (mv.to_matrix()?, nv.to_matrix()?);
                        if ma.is_scalar() && mb.is_scalar() {
                            let (m, n) = (ma.as_scalar().unwrap_or(0.0) as usize, mb.as_scalar().unwrap_or(0.0) as usize);
                            check_elems(m.saturating_mul(n))?;
                            Ok(Some(Value::Sparse(matcore::SparseMat::zeros(m, n))))
                        } else {
                            let ones = vec![1.0; ma.numel().max(mb.numel())];
                            let max_i = ma.data().iter().fold(0.0f64, |a, &x| a.max(x)) as usize;
                            let max_j = mb.data().iter().fold(0.0f64, |a, &x| a.max(x)) as usize;
                            let s = matcore::SparseMat::from_triplets(max_i, max_j, ma.data(), mb.data(), &ones).map_err(lang)?;
                            Ok(Some(Value::Sparse(s)))
                        }
                    }
                    _ => {
                        // sparse(i,j,v) or sparse(i,j,v,m,n).
                        let (is, js, vs) = (
                            v.first().ok_or_else(|| LangError::new("sparse(i,j,v[,m,n]) expected", 0))?.to_matrix()?,
                            v.get(1).ok_or_else(|| LangError::new("sparse(i,j,v[,m,n]) expected", 0))?.to_matrix()?,
                            v.get(2).ok_or_else(|| LangError::new("sparse(i,j,v[,m,n]) expected", 0))?.to_matrix()?,
                        );
                        let max_i = is.data().iter().fold(0.0f64, |a, &x| a.max(x)) as usize;
                        let max_j = js.data().iter().fold(0.0f64, |a, &x| a.max(x)) as usize;
                        let m = v.get(3).map(|x| x.scalar()).transpose()?.unwrap_or(max_i as f64) as usize;
                        let n = v.get(4).map(|x| x.scalar()).transpose()?.unwrap_or(max_j as f64) as usize;
                        check_elems(m.saturating_mul(n))?;
                        let s = matcore::SparseMat::from_triplets(m, n, is.data(), js.data(), vs.data()).map_err(lang)?;
                        Ok(Some(Value::Sparse(s)))
                    }
                }
            }
            "full" => {
                let v = self.eval_args(args)?;
                match v.first() {
                    Some(Value::Sparse(s)) => Ok(Some(mat_or_num(s.full()))),
                    Some(x) => Ok(Some(x.clone())),
                    None => Err(LangError::new("full needs an argument", 0)),
                }
            }
            "issparse" => {
                let v = self.eval_args(args)?;
                Ok(Some(Value::Num(matches!(v.first(), Some(Value::Sparse(_))) as u8 as f64)))
            }
            "nnz" | "nzmax" => {
                let v = self.eval_args(args)?;
                match v.first() {
                    Some(Value::Sparse(s)) => Ok(Some(Value::Num(s.nnz() as f64))),
                    Some(x) => {
                        let m = x.to_matrix()?;
                        Ok(Some(Value::Num(m.data().iter().filter(|&&x| x != 0.0).count() as f64)))
                    }
                    None => Err(LangError::new("nnz needs an argument", 0)),
                }
            }
            "nonzeros" => {
                let v = self.eval_args(args)?;
                match v.first() {
                    Some(Value::Sparse(s)) => Ok(Some(mat_or_num(Matrix::col_vector(s.nonzeros())))),
                    Some(x) => {
                        let m = x.to_matrix()?;
                        let nz: Vec<f64> = m.data().iter().copied().filter(|&x| x != 0.0).collect();
                        Ok(Some(mat_or_num(Matrix::col_vector(nz))))
                    }
                    None => Err(LangError::new("nonzeros needs an argument", 0)),
                }
            }
            "speye" => {
                let v = self.eval_args(args)?;
                let n = v.scalar_at(0)? as usize;
                let m = v.get(1).map(|x| x.scalar()).transpose()?.unwrap_or(n as f64) as usize;
                check_elems(n.saturating_mul(m))?;
                let mut s = matcore::SparseMat::zeros(n, m);
                for i in 0..n.min(m) {
                    s.set(i, i, 1.0);
                }
                Ok(Some(Value::Sparse(s)))
            }
            "spalloc" => {
                // spalloc(m,n,nzmax) — nzmax is a capacity hint we ignore.
                let v = self.eval_args(args)?;
                let m = v.scalar_at(0)? as usize;
                let n = v.scalar_at(1)? as usize;
                check_elems(m.saturating_mul(n))?;
                Ok(Some(Value::Sparse(matcore::SparseMat::zeros(m, n))))
            }
            "spones" => {
                let v = self.eval_args(args)?;
                match v.first() {
                    Some(Value::Sparse(s)) => Ok(Some(Value::Sparse(s.map(|_| 1.0)))),
                    Some(x) => {
                        let m = x.to_matrix()?;
                        Ok(Some(mat_or_num(m.map(|x| if x != 0.0 { 1.0 } else { 0.0 }))))
                    }
                    None => Err(LangError::new("spones needs an argument", 0)),
                }
            }
            "num2cell" => {
                let v = self.eval_args(args)?;
                // `num2cell('abc')` → {'a' 'b' 'c'}.
                if let Some(Value::Str(s)) = v.first() {
                    if s.chars().count() > MAX_CELL_ELEMS {
                        return Err(LangError::new("allocation exceeds limit", 0));
                    }
                    let items: Vec<Value> = s.chars().map(|c| Value::Str(c.to_string())).collect();
                    return Ok(Some(Value::Cell(CellArray::row(items))));
                }
                let m = match v.first() {
                    Some(x) => x.to_matrix()?,
                    None => return Err(LangError::new("num2cell needs a matrix", 0)),
                };
                if m.numel() > MAX_CELL_ELEMS {
                    return Err(LangError::new("allocation exceeds limit", 0));
                }
                // `num2cell(A, dim)`: dim 1 wraps each column, dim 2 each row.
                match v.scalar_at(1).unwrap_or(0.0) {
                    1.0 => {
                        let mut ca = CellArray::new(1, m.cols());
                        for c in 0..m.cols() {
                            let col: Vec<f64> = (0..m.rows()).map(|r| m.get(r, c).unwrap_or(0.0)).collect();
                            ca.set(0, c, Value::Mat(Matrix::col_vector(col)));
                        }
                        Ok(Some(Value::Cell(ca)))
                    }
                    2.0 => {
                        let mut ca = CellArray::new(m.rows(), 1);
                        for r in 0..m.rows() {
                            let row: Vec<f64> = (0..m.cols()).map(|c| m.get(r, c).unwrap_or(0.0)).collect();
                            ca.set(r, 0, Value::Mat(Matrix::row_vector(row)));
                        }
                        Ok(Some(Value::Cell(ca)))
                    }
                    0.0 => {
                        let mut ca = CellArray::new(m.rows(), m.cols());
                        for c in 0..m.cols() {
                            for r in 0..m.rows() {
                                ca.set(r, c, Value::Num(m.get(r, c).unwrap_or(0.0)));
                            }
                        }
                        Ok(Some(Value::Cell(ca)))
                    }
                    _ => Err(LangError::new("num2cell supports dims 1 and 2", 0)),
                }
            }
            "celldisp" => {
                let v = self.eval_args(args)?;
                match v.first() {
                    Some(Value::Cell(ca)) => {
                        let long = self.fmt_long;
                        let mut s = String::new();
                        for i in 0..ca.numel() {
                            if let Some(x) = ca.get_linear(i) {
                                s.push_str(&format!("{{{}}} =\n\n{}\n\n", i + 1, fmt_value(x, long)));
                            }
                        }
                        self.print_buf.push_str(&s);
                        Ok(None)
                    }
                    _ => Err(LangError::new("celldisp needs a cell array", 0)),
                }
            }
            "struct" => {
                // struct() → empty scalar struct; struct('f', v, 'g', w, …)
                // builds a 1x1 struct; `struct('f', {c1 c2}, …)` expands a
                // cell's elements into a struct array (all non-scalar cell
                // args must agree in size; scalars replicate).
                let v = self.eval_args(args)?;
                if v.len() % 2 != 0 {
                    return Err(LangError::new("struct needs field/value pairs", 0));
                }
                let mut dims: Option<(usize, usize)> = None;
                for pair in v.chunks(2) {
                    if let Some(Value::Cell(c)) = pair.get(1) {
                        if c.numel() > 1 {
                            match dims {
                                None => dims = Some(c.shape()),
                                Some(d) if d == c.shape() => {}
                                _ => return Err(LangError::new("cell arguments to struct must have matching sizes", 0)),
                            }
                        }
                    }
                }
                let (rows, cols) = dims.unwrap_or((1, 1));
                let n = rows * cols;
                let mut sa = StructArray::with_dims(rows, cols);
                for pair in v.chunks(2) {
                    let name = match pair.first() {
                        Some(Value::Str(s)) => s.clone(),
                        _ => return Err(LangError::new("struct field names must be strings", 0)),
                    };
                    let vals: Vec<Value> = match pair.get(1) {
                        // A cell value expands across the struct array;
                        // a scalar cell stays a single element.
                        Some(Value::Cell(c)) if c.numel() > 1 => {
                            (0..n).map(|i| c.get_linear(i).cloned().unwrap_or(Value::Mat(Matrix::zeros(0, 0)))).collect()
                        }
                        Some(Value::Cell(c)) => vec![c.get_linear(0).cloned().unwrap_or(Value::Mat(Matrix::zeros(0, 0))); n],
                        other => vec![other.cloned().unwrap_or(Value::Mat(Matrix::zeros(0, 0))); n],
                    };
                    sa.fields.push((name, vals));
                }
                Ok(Some(Value::Struct(sa)))
            }
            "fieldnames" => {
                let v = self.eval_args(args)?;
                match v.first() {
                    Some(Value::Struct(m)) => {
                        // MATLAB returns a column cell of char row vectors.
                        let names = m.field_names();
                        let mut c = CellArray::new(names.len(), 1);
                        for (i, k) in names.iter().enumerate() {
                            c.set_linear(i, Value::Str(k.clone()));
                        }
                        Ok(Some(Value::Cell(c)))
                    }
                    _ => Err(LangError::new("fieldnames needs a struct", 0)),
                }
            }
            "isstruct" => {
                let v = self.eval_args(args)?;
                Ok(Some(Value::Num(matches!(v.first(), Some(Value::Struct(_))) as u8 as f64)))
            }
            "isfield" => {
                let v = self.eval_args(args)?;
                let m = match v.first() {
                    Some(Value::Struct(m)) => m,
                    _ => return Err(LangError::new("isfield needs a struct and a field name", 0)),
                };
                match v.get(1) {
                    Some(Value::Str(f)) => Ok(Some(Value::Num(m.has_field(f) as u8 as f64))),
                    // cellstr: element-wise check, logical array the shape of
                    // the cell (isfield(s, {'a','zzz'}) → [1 0]).
                    Some(Value::Cell(c)) => {
                        let mut data = Vec::with_capacity(c.numel());
                        for i in 0..c.numel() {
                            let hit = matches!(c.get_linear(i), Some(Value::Str(f)) if m.has_field(f));
                            data.push(f64::from(hit));
                        }
                        let (r, nc) = c.shape();
                        Ok(Some(mat_or_num(Matrix::from_col_major(r, nc, data).map_err(lang)?)))
                    }
                    _ => Err(LangError::new("isfield needs a struct and a field name", 0)),
                }
            }
            "rmfield" => {
                let v = self.eval_args(args)?;
                let mut m = match v.first().cloned() {
                    Some(Value::Struct(m)) => m,
                    _ => return Err(LangError::new("rmfield needs a struct", 0)),
                };
                match v.get(1) {
                    None => Ok(Some(Value::Struct(m))),
                    Some(Value::Str(f)) => {
                        if !m.remove_field(f) {
                            return Err(LangError::new(format!("reference to non-existent field '{f}'"), 0));
                        }
                        Ok(Some(Value::Struct(m)))
                    }
                    // cellstr removes every named field.
                    Some(Value::Cell(c)) => {
                        for i in 0..c.numel() {
                            match c.get_linear(i) {
                                Some(Value::Str(f)) => {
                                    if !m.remove_field(f) {
                                        return Err(LangError::new(format!("reference to non-existent field '{f}'"), 0));
                                    }
                                }
                                _ => return Err(LangError::new("rmfield needs a struct and a field name", 0)),
                            }
                        }
                        Ok(Some(Value::Struct(m)))
                    }
                    _ => Err(LangError::new("rmfield needs a struct and a field name", 0)),
                }
            }
            "orderfields" => {
                // orderfields(s) → fields sorted alphabetically;
                // orderfields(s, c|p) — cellstr order or a permutation.
                let v = self.eval_args(args)?;
                let mut m = match v.first().cloned() {
                    Some(Value::Struct(m)) => m,
                    _ => return Err(LangError::new("orderfields needs a struct", 0)),
                };
                match v.get(1) {
                    None => {
                        let mut names = m.field_names();
                        names.sort();
                        m.reorder(&names)?;
                    }
                    Some(Value::Cell(c)) => {
                        let mut names = Vec::with_capacity(c.numel());
                        for i in 0..c.numel() {
                            match c.get_linear(i) {
                                Some(Value::Str(f)) => names.push(f.clone()),
                                _ => return Err(LangError::new("orderfields needs field names", 0)),
                            }
                        }
                        m.reorder(&names)?;
                    }
                    Some(x) => {
                        // Numeric permutation: orderfields(s, [2 1 3]).
                        let p = x.to_matrix()?;
                        let names = m.field_names();
                        let mut new_names = Vec::with_capacity(names.len());
                        for i in 0..p.numel() {
                            let k = p.get_linear(i).unwrap_or(0.0) as usize;
                            match names.get(k.saturating_sub(1)) {
                                Some(n) => new_names.push(n.clone()),
                                None => return Err(LangError::new("orderfields permutation out of range", 0)),
                            }
                        }
                        m.reorder(&new_names)?;
                    }
                }
                Ok(Some(Value::Struct(m)))
            }
            "getfield" | "setfield" => {
                let v = self.eval_args(args)?;
                match (v.first().cloned(), v.get(1)) {
                    // getfield(s, i, 'f') on a struct array; 1x1 forms too.
                    (Some(Value::Struct(m)), Some(Value::Str(f))) => {
                        if name == "getfield" {
                            if !m.is_scalar() {
                                return Err(LangError::new(
                                    format!(
                                        "expected one output from a curly brace or dot indexing expression, but there were {} results",
                                        m.numel()
                                    ),
                                    0,
                                ));
                            }
                            match m.field_at(0, f) {
                                Some(x) => Ok(Some(x.clone())),
                                None => Err(LangError::new(format!("reference to non-existent field '{f}'"), 0)),
                            }
                        } else {
                            let mut m = m;
                            if !m.is_scalar() {
                                return Err(LangError::new("setfield on a non-scalar struct needs an index: setfield(s, i, 'f', v)", 0));
                            }
                            let val = v.get(2).cloned().unwrap_or(Value::Mat(Matrix::zeros(0, 0)));
                            m.set_field(0, f, val)?;
                            Ok(Some(Value::Struct(m)))
                        }
                    }
                    // getfield(s, i, 'f') / setfield(s, i, 'f', v).
                    (Some(Value::Struct(m)), Some(idx)) if matches!(idx, Value::Num(_) | Value::Mat(_)) => {
                        let i = idx.scalar()?.max(1.0) as usize;
                        let f = match v.get(2) {
                            Some(Value::Str(f)) => f.clone(),
                            _ => return Err(LangError::new(format!("{name} needs a struct, an index, and a field name"), 0)),
                        };
                        if name == "getfield" {
                            match m.field_at(i - 1, &f) {
                                Some(x) => Ok(Some(x.clone())),
                                None => Err(LangError::new(format!("reference to non-existent field '{f}'"), 0)),
                            }
                        } else {
                            let mut m = m;
                            let val = v.get(3).cloned().unwrap_or(Value::Mat(Matrix::zeros(0, 0)));
                            m.set_field(i - 1, &f, val)?;
                            Ok(Some(Value::Struct(m)))
                        }
                    }
                    _ => Err(LangError::new(format!("{name} needs a struct and a field name"), 0)),
                }
            }
            "reshape" => {
                let v = self.eval_args(args)?;
                let r = v.scalar_at(1)? as usize;
                let c = v.scalar_at(2)? as usize;
                match v.first() {
                    Some(Value::Cell(ca)) => ca.reshape(r, c).map(Value::Cell).map(Some),
                    Some(Value::Cplx(cm)) => cm.reshape(r, c).map(cmat_or_value).map(Some).map_err(lang),
                    Some(x) => {
                        let m = x.to_matrix()?;
                        m.reshape(r, c).map(mat_or_num).map(Some).map_err(lang)
                    }
                    None => Err(LangError::new("reshape needs a matrix", 0)),
                }
            }
            "transpose" | "ctranspose" => {
                let v = self.eval_args(args)?;
                if let Some(Value::Cplx(cm)) = v.first() {
                    // `transpose` is plain; `ctranspose` conjugates — the
                    // same distinction as `.'` vs `'`.
                    return Ok(Some(cmat_or_value(if name == "ctranspose" { cm.ctranspose() } else { cm.transpose() })));
                }
                let m = v.first().ok_or_else(|| LangError::new(format!("{name} needs an argument"), 0))?.to_matrix()?;
                Ok(Some(mat_or_num(m.transpose())))
            }
            "det" => {
                let v = self.eval_args(args)?;
                if let Some(Value::Cplx(cm)) = v.first() {
                    // det(A) = det(L)·ΠU diagonal, via complex LU.
                    return cm.det().map(|z| cmat_or_value(CMatrix::scalar(z))).map(Some).map_err(lang);
                }
                let m = v.first().ok_or_else(|| LangError::new("det needs an argument", 0))?.to_matrix()?;
                m.det().map(Value::Num).map(Some).map_err(lang)
            }
            "inv" => {
                let v = self.eval_args(args)?;
                if let Some(Value::Cplx(cm)) = v.first() {
                    let n = cm.rows();
                    let eye = {
                        let mut e = CMatrix::zeros(n, n);
                        for i in 0..n {
                            let _ = e.set(i, i, CNum::real(1.0));
                        }
                        e
                    };
                    return cm.solve(&eye).map(cmat_or_value).map(Some).map_err(lang);
                }
                let m = v.first().ok_or_else(|| LangError::new("inv needs an argument", 0))?.to_matrix()?;
                m.inverse().map(mat_or_num).map(Some).map_err(lang)
            }
            "norm" => {
                let v = self.eval_args(args)?;
                if let Some(Value::Cplx(cm)) = v.first() {
                    return Ok(Some(Value::Num(cm.norm())));
                }
                let m = v.first().ok_or_else(|| LangError::new("norm needs an argument", 0))?.to_matrix()?;
                Ok(Some(Value::Num(m.norm())))
            }
            "sum" | "mean" | "prod" | "min" | "max" => {
                let v = self.eval_args(args)?;
                // Two-arg min/max is elementwise: max(A, B), max(A, scalar).
                // Check the *expanded* arg count — `max(c{1:2})` → max(1,2)
                // is elementwise in M-code, not a dim form (`max(A,[],dim)`
                // keeps the [] placeholder).
                if (name == "min" || name == "max") && v.len() >= 2 && v.get(1).map(|x| x.dims().0 * x.dims().1 != 0).unwrap_or(false) {
                    let (a, b) = (v.first(), v.get(1));
                    let (v0, v1) = (a.cloned().unwrap_or(Value::Num(0.0)), b.cloned().unwrap_or(Value::Num(0.0)));
                    if matches!(v0, Value::Cplx(_)) || matches!(v1, Value::Cplx(_)) {
                        // Complex elementwise min/max orders by magnitude.
                        let ca = v0.to_cmatrix()?;
                        let cb = v1.to_cmatrix()?;
                        let hi = name == "max";
                        return ca
                            .broadcast(&cb, |x, y| if (x.abs() > y.abs() || (x.abs() == y.abs() && x.arg() > y.arg())) == hi { x } else { y })
                            .map(cmat_or_value)
                            .map(Some)
                            .map_err(lang);
                    }
                    let (a, b) = (v0.to_matrix()?, v1.to_matrix()?);
                    let hi = name == "max";
                    return a.broadcast(&b, |x, y| if (x > y) == hi { x } else { y }).map(mat_or_num).map(Some).map_err(lang);
                }
                let v0 = v.first().ok_or_else(|| LangError::new(format!("{name} needs an argument"), 0))?;
                if let Value::Cplx(cm) = v0 {
                    let dim = self.dim_arg(args, cm.shape())?;
                    return self.creduce(name, cm, dim).map(Some);
                }
                let m = v0.to_matrix()?;
                let dim = self.dim_arg(args, m.shape())?;
                self.reduce(name, &m, dim).map(Some)
            }
            "std" | "var" | "median" => {
                let v = self.eval_args(args)?;
                let v0 = v.first().ok_or_else(|| LangError::new(format!("{name} needs an argument"), 0))?;
                if let Value::Cplx(cm) = v0 {
                    let dim = self.dim_arg(args, cm.shape())?;
                    return self.creduce(name, cm, dim).map(Some);
                }
                let m = v0.to_matrix()?;
                let dim = self.dim_arg(args, m.shape())?;
                self.reduce(name, &m, dim).map(Some)
            }
            "any" | "all" => {
                let v = self.eval_args(args)?;
                let v0 = v.first().ok_or_else(|| LangError::new(format!("{name} needs an argument"), 0))?;
                if let Value::Cplx(cm) = v0 {
                    let dim = self.dim_arg(args, cm.shape())?;
                    return self.creduce(name, cm, dim).map(Some);
                }
                let m = v0.to_matrix()?;
                let dim = self.dim_arg(args, m.shape())?;
                self.reduce(name, &m, dim).map(Some)
            }
            "cumsum" | "cumprod" | "diff" => {
                let v = self.eval_args(args)?;
                let v0 = v.first().ok_or_else(|| LangError::new(format!("{name} needs an argument"), 0))?;
                if let Value::Cplx(cm) = v0 {
                    let dim = self.dim_arg(args, cm.shape())?;
                    return self.crunning(name, cm, dim).map(Some);
                }
                let m = v0.to_matrix()?;
                let dim = self.dim_arg(args, m.shape())?;
                self.running(name, &m, dim).map(Some)
            }
            "sort" => {
                let v = self.eval_args(args)?;
                let first = v.first().ok_or_else(|| LangError::new("sort needs an argument", 0))?;
                // sort(A), sort(A, dim), sort(A, 'descend'), sort(A, dim, 'descend')
                let mut dim: Option<usize> = None;
                let mut desc = false;
                for extra in v.iter().skip(1) {
                    match extra {
                        Value::Str(s) => desc = s.trim() == "descend" || s.trim() == "descending",
                        other => {
                            let d = other.scalar()? as usize;
                            dim = Some(d);
                        }
                    }
                }
                let dim = dim.unwrap_or(1);
                if let Value::Cplx(cm) = first {
                    return self.csort(cm, dim, desc).map(Some);
                }
                let m = first.to_matrix()?;
                self.sort(&m, dim, desc).map(Some)
            }
            "unique" => {
                let m = self.arg_matrix(args, 0)?;
                let mut d = m.data().to_vec();
                d.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                d.dedup();
                Ok(Some(mat_or_num(Matrix::row_vector(d))))
            }
            "find" => {
                let v = self.eval_args(args)?;
                if let Some(Value::Sparse(s)) = v.first() {
                    let idxs: Vec<f64> = s.entries().map(|((r, c), _)| (r + c * s.rows() + 1) as f64).collect();
                    return Ok(Some(mat_or_num(Matrix::col_vector(idxs))));
                }
                if let Some(Value::Cplx(cm)) = v.first() {
                    let idxs: Vec<f64> =
                        cm.data().iter().enumerate().filter(|(_, z)| z.re != 0.0 || z.im != 0.0).map(|(i, _)| (i + 1) as f64).collect();
                    return Ok(Some(mat_or_num(Matrix::col_vector(idxs))));
                }
                let m = v.first().ok_or_else(|| LangError::new("find needs an argument", 0))?.to_matrix()?;
                let idxs: Vec<f64> = m.data().iter().enumerate().filter(|(_, &v)| v != 0.0).map(|(i, _)| (i + 1) as f64).collect();
                Ok(Some(mat_or_num(Matrix::col_vector(idxs))))
            }
            "abs" | "sqrt" | "exp" | "log" | "log10" | "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "floor" | "ceil" | "round" | "sign"
            | "mod" | "rem" | "atan2" | "real" | "imag" | "conj" | "angle" => {
                let v = self.eval_args(args)?;
                let first = v.first().ok_or_else(|| LangError::new(format!("{name} needs an argument"), 0))?;
                // Complex argument: elementwise complex arithmetic, except
                // abs/real/imag/angle which return real matrices.
                if let Value::Cplx(cm) = first {
                    let out = match name {
                        "abs" => return Ok(Some(mat_or_num(cm.abs()))),
                        "angle" => return Ok(Some(mat_or_num(cm.angle()))),
                        "real" => return Ok(Some(mat_or_num(cm.re()))),
                        "imag" => return Ok(Some(mat_or_num(cm.im()))),
                        "conj" => cm.conj(),
                        "sqrt" => cm.map(CNum::sqrt),
                        "exp" => cm.map(CNum::exp),
                        "log" => cm.map(CNum::ln),
                        "log10" => cm.map(|z| z.ln().div(CNum::real(std::f64::consts::LN_10))),
                        "sin" => cm.map(CNum::sin),
                        "cos" => cm.map(CNum::cos),
                        "tan" => cm.map(|z| z.sin().div(z.cos())),
                        "asin" => cm.map(|z| CNum::I.neg().mul(CNum::I.mul(z).add(CNum::real(1.0).sub(z.mul(z)).sqrt()).ln())),
                        "acos" => cm.map(|z| CNum::I.neg().mul(z.add(CNum::I.mul(CNum::real(1.0).sub(z.mul(z)).sqrt())).ln())),
                        "atan" => cm.map(|z| {
                            let iz = CNum::I.mul(z);
                            CNum::real(0.5).mul(CNum::I).mul(CNum::real(1.0).add(iz).ln().sub(CNum::real(1.0).sub(iz).ln()))
                        }),
                        "sign" => cm.map(|z| if z.re == 0.0 && z.im == 0.0 { z } else { z.div(CNum::real(z.abs())) }),
                        "floor" => cm.map(|z| CNum::new(z.re.floor(), z.im.floor())),
                        "ceil" => cm.map(|z| CNum::new(z.re.ceil(), z.im.ceil())),
                        "round" => cm.map(|z| CNum::new(z.re.round(), z.im.round())),
                        _ => return Err(LangError::new(format!("{name} requires real arguments (complex input)"), 0)),
                    };
                    return Ok(Some(cmat_or_value(out)));
                }
                // `real`/`imag`/`conj`/`angle` on real input.
                match name {
                    "real" | "conj" => {
                        let m = first.to_matrix()?;
                        return Ok(Some(mat_or_num(m)));
                    }
                    "imag" => {
                        let m = first.to_matrix()?;
                        return Ok(Some(mat_or_num(Matrix::zeros(m.rows(), m.cols()))));
                    }
                    "angle" => {
                        let m = first.to_matrix()?;
                        return Ok(Some(mat_or_num(m.map(|x| if x < 0.0 { std::f64::consts::PI } else { 0.0 }))));
                    }
                    _ => {}
                }
                let m = first.to_matrix()?;
                // Domain promotion: sqrt/log/asin/acos on out-of-domain reals
                // return complex (`sqrt(-4)` → 2i, `log(-1)` → πi) — same rule
                // M-code follows.
                let needs_complex = match name {
                    "sqrt" | "log" | "log10" => m.data().iter().any(|&x| x < 0.0),
                    "asin" | "acos" => m.data().iter().any(|&x| x.abs() > 1.0),
                    _ => false,
                };
                if needs_complex {
                    let cm = CMatrix::from_real(&m);
                    let out = match name {
                        "sqrt" => cm.map(CNum::sqrt),
                        "log" => cm.map(CNum::ln),
                        "log10" => cm.map(|z| z.ln().div(CNum::real(std::f64::consts::LN_10))),
                        "asin" => cm.map(|z| CNum::I.neg().mul(CNum::I.mul(z).add(CNum::real(1.0).sub(z.mul(z)).sqrt()).ln())),
                        "acos" => cm.map(|z| CNum::I.neg().mul(z.add(CNum::I.mul(CNum::real(1.0).sub(z.mul(z)).sqrt())).ln())),
                        _ => cm,
                    };
                    return Ok(Some(cmat_or_value(out)));
                }
                let unary = |f: fn(f64) -> f64| -> Result<Option<Value>, LangError> { Ok(Some(mat_or_num(m.map(f)))) };
                match name {
                    "abs" => unary(f64::abs),
                    "sqrt" => unary(f64::sqrt),
                    "exp" => unary(f64::exp),
                    "log" => unary(f64::ln),
                    "log10" => unary(f64::log10),
                    "sin" => unary(f64::sin),
                    "cos" => unary(f64::cos),
                    "tan" => unary(f64::tan),
                    "asin" => unary(f64::asin),
                    "acos" => unary(f64::acos),
                    "atan" => unary(f64::atan),
                    "floor" => unary(f64::floor),
                    "ceil" => unary(f64::ceil),
                    "round" => unary(f64::round),
                    "sign" => unary(|x| {
                        if x > 0.0 {
                            1.0
                        } else if x < 0.0 {
                            -1.0
                        } else {
                            0.0
                        }
                    }),
                    "atan2" => {
                        let y = v.get(1).ok_or_else(|| LangError::new("atan2 needs two arguments", 0))?.to_matrix()?;
                        m.broadcast(&y, |a, b| a.atan2(b)).map(mat_or_num).map(Some).map_err(lang)
                    }
                    "mod" | "rem" => {
                        let y = v.get(1).ok_or_else(|| LangError::new(format!("{name} needs two arguments"), 0))?.to_matrix()?;
                        let f = if name == "mod" { |a: f64, b: f64| a - b * (a / b).floor() } else { |a: f64, b: f64| a - b * (a / b).trunc() };
                        m.broadcast(&y, f).map(mat_or_num).map(Some).map_err(lang)
                    }
                    _ => Err(LangError::new(format!("unhandled builtin '{name}'"), 0)),
                }
            }
            "deg2rad" | "rad2deg" | "sinc" | "db2pow" | "pow2db" | "uint8" | "rectpuls" | "tripuls" => {
                let v = self.eval_args(args)?;
                let m = v.first().ok_or_else(|| LangError::new(format!("{name} needs an argument"), 0))?.to_matrix()?;
                let f: fn(f64) -> f64 = match name {
                    "deg2rad" => |x| x * (std::f64::consts::PI / 180.0),
                    "rad2deg" => |x| x * (180.0 / std::f64::consts::PI),
                    "sinc" => |x| {
                        if x == 0.0 {
                            1.0
                        } else {
                            let t = std::f64::consts::PI * x;
                            t.sin() / t
                        }
                    },
                    "db2pow" => |x| 10f64.powf(x / 10.0),
                    "pow2db" => |x| 10.0 * x.log10(),
                    "uint8" => |x| x.round().clamp(0.0, 255.0),
                    "rectpuls" => |x| if x.abs() <= 0.5 { 1.0 } else { 0.0 },
                    _ => |x| (1.0 - x.abs()).max(0.0), // tripuls
                };
                Ok(Some(mat_or_num(m.map(f))))
            }
            // Constructor forms: bare `Inf`/`NaN`/`eps` resolve via constant();
            // these arms cover `Inf(1,n)`, `NaN(size)`, `eps(x)`.
            "inf" | "Inf" | "nan" | "NaN" => {
                let v = self.eval_args(args)?;
                let fill = if name == "inf" || name == "Inf" { f64::INFINITY } else { f64::NAN };
                if v.is_empty() {
                    return Ok(Some(Value::Num(fill)));
                }
                let (r, c) = self.dims(args)?;
                check_elems(r.saturating_mul(c))?;
                Ok(Some(mat_or_num(Matrix::from_col_major(r, c, vec![fill; r * c]).map_err(lang)?)))
            }
            "eps" => {
                let v = self.eval_args(args)?;
                match v.first() {
                    None => Ok(Some(Value::Num(f64::EPSILON))),
                    Some(val) => {
                        let m = val.to_matrix()?;
                        Ok(Some(mat_or_num(m.map(|x| x.abs() * f64::EPSILON))))
                    }
                }
            }
            "mkdir" => {
                let v = self.eval_args(args)?;
                match v.first() {
                    Some(Value::Str(s)) => {
                        std::fs::create_dir_all(s).map_err(|e| LangError::new(format!("mkdir: {e}"), 0))?;
                        Ok(Some(Value::Num(1.0)))
                    }
                    _ => Err(LangError::new("mkdir needs a directory name", 0)),
                }
            }
            "randperm" => {
                let v = self.eval_args(args)?;
                let n = v.scalar_at(0)? as usize;
                check_elems(n)?;
                let k = if v.len() > 1 { v.scalar_at(1)? as usize } else { n };
                if k > n {
                    return Err(LangError::new("randperm: k cannot exceed n", 0));
                }
                let mut p: Vec<f64> = (1..=n).map(|i| i as f64).collect();
                // Fisher–Yates shuffle over the session PRNG.
                for i in (1..n).rev() {
                    let j = ((self.next_f64() * (i + 1) as f64) as usize).min(i);
                    p.swap(i, j);
                }
                p.truncate(k);
                Ok(Some(Value::Mat(Matrix::row_vector(p))))
            }
            "normrnd" => {
                let v = self.eval_args(args)?;
                let mu = v.scalar_at(0)?;
                let sg = v.scalar_at(1)?;
                let (r, c) = match v.len() {
                    0..=2 => (1, 1),
                    3 => (v.scalar_at(2)? as usize, 1),
                    _ => (v.scalar_at(2)? as usize, v.scalar_at(3)? as usize),
                };
                check_elems(r.saturating_mul(c))?;
                let mut data = Vec::with_capacity(r.saturating_mul(c));
                for _ in 0..r.saturating_mul(c) {
                    let mut acc = 0.0;
                    for _ in 0..12 {
                        acc += self.next_f64();
                    }
                    data.push(mu + sg * (acc - 6.0));
                }
                Ok(Some(mat_or_num(Matrix::from_col_major(r, c, data).map_err(lang)?)))
            }
            "assert" => {
                let v = self.eval_args(args)?;
                let ok = match v.first() {
                    Some(val) => val.truthy()?,
                    None => false,
                };
                if ok {
                    Ok(None)
                } else {
                    let msg = match v.get(1) {
                        Some(Value::Str(s)) => s.clone(),
                        Some(val) => fmt_value(val, false).to_string(),
                        None => "assertion failed".to_string(),
                    };
                    Err(LangError::new(msg, 0))
                }
            }
            "rng" => {
                let v = self.eval_args(args)?;
                match v.first() {
                    None => Ok(Some(Value::Num(self.rng as f64))),
                    Some(Value::Str(s)) if s == "default" => {
                        self.rng = 0x9E3779B97F4A7C15;
                        Ok(None)
                    }
                    Some(Value::Str(s)) if s == "shuffle" => {
                        #[cfg(not(target_arch = "wasm32"))]
                        let t = {
                            #[allow(clippy::disallowed_methods)]
                            let now = std::time::SystemTime::now();
                            now.duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos() as u64 ^ d.as_secs()).unwrap_or(0x2545F4914F6CDD1D)
                        };
                        #[cfg(target_arch = "wasm32")]
                        let t = 0x2545F4914F6CDD1Du64;
                        self.rng = t | 1;
                        Ok(None)
                    }
                    Some(val) => {
                        let s = val.scalar()?;
                        self.rng = (s as u64) | 1;
                        Ok(None)
                    }
                }
            }
            "disp" | "display" => {
                let v = self.eval_args(args)?;
                match v.first() {
                    Some(val) => {
                        let long = self.fmt_long;
                        self.print_buf.push_str(&format!("{}\n", fmt_value(val, long)));
                        Ok(None)
                    }
                    None => Err(LangError::new("disp needs an argument", 0)),
                }
            }
            "format" => {
                // `format long`, `format short`, `format compact`, `format` —
                // numeric display mode; spacing modes are accepted and ignored.
                let v = self.eval_args(args)?;
                match v.first() {
                    Some(Value::Str(s)) => {
                        self.fmt_long = matches!(s.as_str(), "long" | "long e" | "longE");
                    }
                    Some(_) => {}
                    // bare `format` resets to short (M-code behaviour)
                    None => self.fmt_long = false,
                }
                Ok(None)
            }
            "fprintf" => {
                let v = self.eval_args(args)?;
                let (f, rest) = split_format_args(&v)?;
                let text = sprintf_impl(&f, &rest)?;
                self.print_buf.push_str(&text);
                Ok(None)
            }
            "sprintf" => {
                let v = self.eval_args(args)?;
                let (f, rest) = split_format_args(&v)?;
                Ok(Some(Value::Str(sprintf_impl(&f, &rest)?)))
            }
            "who" | "whos" => {
                let mut names: Vec<&String> = self.vars.keys().collect();
                names.sort();
                for n in names {
                    if name == "whos" {
                        if let Some(v) = self.vars.get(n) {
                            self.print_buf.push_str(&format!("{n:>12}  {}\n", describe(v)));
                        }
                    } else {
                        self.print_buf.push_str(n);
                        self.print_buf.push('\n');
                    }
                }
                Ok(None)
            }
            "clear" => {
                if args.is_empty() {
                    self.vars.clear();
                } else {
                    for a in args {
                        // Command syntax passes names as Str args.
                        match a {
                            Expr::Ident(n) | Expr::Str(n) => {
                                self.vars.remove(n);
                            }
                            _ => {}
                        }
                    }
                }
                Ok(None)
            }
            // --- function handles -----------------------------------------
            "feval" => {
                let v = self.eval_args(args)?;
                let (target, rest) = match v.split_first() {
                    Some((t, r)) => (t.clone(), r.to_vec()),
                    None => return Err(LangError::new("feval needs a function handle or name", 0)),
                };
                let outs = self.call_feval_target(&target, rest, 1)?;
                Ok(outs.into_iter().next())
            }
            "func2str" => {
                let v = self.eval_args(args)?;
                match v.first() {
                    Some(Value::Func(h)) => Ok(Some(Value::Str(func_text(h)))),
                    _ => Err(LangError::new("func2str needs a function handle", 0)),
                }
            }
            "str2func" => {
                let v = self.eval_args(args)?;
                let s = match v.first() {
                    Some(Value::Str(s)) => s.clone(),
                    _ => return Err(LangError::new("str2func needs a string", 0)),
                };
                Ok(Some(Value::Func(str_to_func(&s)?)))
            }
            "functions" => {
                let v = self.eval_args(args)?;
                let h = match v.first() {
                    Some(Value::Func(h)) => h,
                    _ => return Err(LangError::new("functions needs a function handle", 0)),
                };
                let mut m = std::collections::BTreeMap::new();
                match h {
                    FuncHandle::Named(n) => {
                        m.insert("function".into(), Value::Str(n.clone()));
                        m.insert("type".into(), Value::Str("simple".into()));
                        m.insert("file".into(), Value::Str(String::new()));
                    }
                    FuncHandle::Closure { .. } => {
                        m.insert("function".into(), Value::Str(func_text(h)));
                        m.insert("type".into(), Value::Str("anonymous".into()));
                        m.insert("file".into(), Value::Str(String::new()));
                    }
                }
                Ok(Some(Value::Struct(StructArray::from_map(m))))
            }
            "arrayfun" => {
                // `arrayfun(fh, A)` — apply fh to each element, same-shaped
                // result; non-scalar outputs error (needs the
                // 'UniformOutput',false form, which is not implemented).
                let v = self.eval_args(args)?;
                let (target, rest) = match v.split_first() {
                    Some((t, r)) => (t.clone(), r.to_vec()),
                    None => return Err(LangError::new("arrayfun needs a function handle", 0)),
                };
                let src = match rest.first() {
                    Some(Value::Cell(c)) => (0..c.numel()).map(|i| c.get_linear(i).cloned().unwrap_or_else(CellArray::empty)).collect::<Vec<_>>(),
                    Some(other) => other.to_matrix()?.data().iter().map(|&x| Value::Num(x)).collect::<Vec<_>>(),
                    None => return Err(LangError::new("arrayfun needs an array argument", 0)),
                };
                let (rows, cols) = rest.first().map(Value::dims).unwrap_or((0, 0));
                let mut data = Vec::with_capacity(src.len());
                for item in src {
                    let outs = self.call_feval_target(&target, vec![item], 1)?;
                    let out = outs.into_iter().next().ok_or_else(|| LangError::new("arrayfun: function returned no value", 0))?;
                    data.push(
                        out.scalar().map_err(|_| LangError::new("arrayfun: non-scalar output needs 'UniformOutput', false (not implemented)", 0))?,
                    );
                }
                Ok(Some(mat_or_num(Matrix::from_col_major(rows, cols, data).map_err(lang)?)))
            }
            "cellfun" => {
                // `cellfun(fh, C [, 'UniformOutput', false])` — apply fh to
                // each cell's content. Default collects scalar results into
                // a like-shaped double array; 'UniformOutput',false keeps a
                // cell of whatever each call returned.
                let v = self.eval_args(args)?;
                let (target, rest) = match v.split_first() {
                    Some((t, r)) => (t.clone(), r.to_vec()),
                    None => return Err(LangError::new("cellfun needs a function handle", 0)),
                };
                let ca = match rest.first() {
                    Some(Value::Cell(c)) => c.clone(),
                    _ => return Err(LangError::new("cellfun needs a cell array", 0)),
                };
                let mut uniform = true;
                if let Some(Value::Str(s)) = rest.get(1) {
                    if s.eq_ignore_ascii_case("uniformoutput") {
                        uniform = match rest.get(2) {
                            Some(Value::Str(v)) => !v.eq_ignore_ascii_case("false"),
                            Some(v) => v.scalar().map(|x| x != 0.0).unwrap_or(true),
                            None => true,
                        };
                    }
                }
                let mut outs = Vec::with_capacity(ca.numel());
                for i in 0..ca.numel() {
                    let item = ca.get_linear(i).cloned().unwrap_or_else(CellArray::empty);
                    let r = self.call_feval_target(&target, vec![item], 1)?;
                    outs.push(r.into_iter().next().unwrap_or_else(|| Value::Mat(Matrix::zeros(0, 0))));
                }
                if !uniform {
                    let (rows, cols) = ca.shape();
                    let mut out = CellArray::new(rows, cols);
                    for (i, val) in outs.into_iter().enumerate() {
                        out.set_linear(i, val);
                    }
                    return Ok(Some(Value::Cell(out)));
                }
                let (rows, cols) = ca.shape();
                let mut data = Vec::with_capacity(outs.len());
                for val in &outs {
                    data.push(val.scalar().map_err(|_| LangError::new("cellfun: non-scalar output needs 'UniformOutput', false", 0))?);
                }
                Ok(Some(mat_or_num(Matrix::from_col_major(rows, cols, data).map_err(lang)?)))
            }
            // Inside a function: the counts of the innermost active call.
            "nargin" => Ok(Some(Value::Num(self.call_ctx.map(|c| c.0).unwrap_or(0) as f64))),
            "nargout" => Ok(Some(Value::Num(self.call_ctx.map(|c| c.1).unwrap_or(1) as f64))),
            "narginchk" | "nargoutchk" => {
                let v = self.eval_args(args)?;
                let lo = v.scalar_at(0)? as i64;
                let hi = v.scalar_at(1)? as i64;
                let (n, what) = match &self.call_ctx {
                    Some((na, no)) if name == "narginchk" => (*na as i64, "input"),
                    Some((_, no)) => (*no as i64, "output"),
                    None => return Err(LangError::new(format!("{name} is only valid inside a function"), 0)),
                };
                if n < lo {
                    return Err(LangError::new(format!("not enough {what} arguments"), 0));
                }
                if n > hi {
                    return Err(LangError::new(format!("too many {what} arguments"), 0));
                }
                Ok(None)
            }
            // --- construction / reshape -----------------------------------
            "logspace" => {
                let v = self.eval_args(args)?;
                let (a, b) = (v.scalar_at(0)?, v.scalar_at(1)?);
                let n = if v.len() > 2 { v.scalar_at(2)? as usize } else { 50 };
                check_elems(n)?;
                let mut data = Vec::with_capacity(n);
                for i in 0..n {
                    let t = if n > 1 { i as f64 / (n - 1) as f64 } else { 0.0 };
                    data.push(10f64.powf(a + (b - a) * t));
                }
                Ok(Some(Value::Mat(Matrix::row_vector(data))))
            }
            "repmat" => {
                let v = self.eval_args(args)?;
                let r = v.scalar_at(1)? as usize;
                let c = if v.len() > 2 { v.scalar_at(2)? as usize } else { r };
                match v.first() {
                    Some(Value::Cell(ca)) => ca.repmat(r, c).map(Value::Cell).map(Some),
                    Some(Value::Cplx(cm)) => cplanes(cm, |m| repmat(m, r, c)).map(cmat_or_value).map(Some),
                    Some(x) => {
                        let m = x.to_matrix()?;
                        repmat(&m, r, c).map(mat_or_num).map(Some)
                    }
                    None => Err(LangError::new("repmat needs a matrix", 0)),
                }
            }
            "diag" => {
                let v = self.eval_args(args)?;
                if let Some(Value::Cplx(cm)) = v.first() {
                    return cplanes(cm, diag).map(cmat_or_value).map(Some);
                }
                let m = v.first().ok_or_else(|| LangError::new("diag needs an argument", 0))?.to_matrix()?;
                diag(&m).map(mat_or_num).map(Some)
            }
            "triu" | "tril" => {
                let v = self.eval_args(args)?;
                let k = v.scalar_at(1).unwrap_or(0.0) as i64;
                let band = |m: &Matrix| -> Matrix {
                    let mut out = Matrix::zeros(m.rows(), m.cols());
                    for c in 0..m.cols() {
                        for r in 0..m.rows() {
                            let keep = if name == "triu" { (c as i64 - r as i64) >= k } else { (c as i64 - r as i64) <= k };
                            if keep {
                                let _ = out.set(r, c, m.get(r, c).unwrap_or(0.0));
                            }
                        }
                    }
                    out
                };
                if let Some(Value::Cplx(cm)) = v.first() {
                    return cplanes(cm, |m| Ok(band(m))).map(cmat_or_value).map(Some);
                }
                let m = v.first().ok_or_else(|| LangError::new(format!("{name} needs an argument"), 0))?.to_matrix()?;
                Ok(Some(mat_or_num(band(&m))))
            }
            "flipud" | "fliplr" => {
                let first = self.eval_args(args)?.into_iter().next();
                if let Some(Value::Cell(ca)) = first {
                    return Ok(Some(Value::Cell(ca.flip(name == "fliplr"))));
                }
                let flip = |m: &Matrix| -> Matrix {
                    let mut out = Matrix::zeros(m.rows(), m.cols());
                    for c in 0..m.cols() {
                        for r in 0..m.rows() {
                            let (rr, cc) = if name == "flipud" { (m.rows() - 1 - r, c) } else { (r, m.cols() - 1 - c) };
                            let _ = out.set(rr, cc, m.get(r, c).unwrap_or(0.0));
                        }
                    }
                    out
                };
                if let Some(Value::Cplx(cm)) = &first {
                    return cplanes(cm, |m| Ok(flip(m))).map(cmat_or_value).map(Some);
                }
                let m = first.ok_or_else(|| LangError::new(format!("{name} needs an argument"), 0))?.to_matrix()?;
                Ok(Some(mat_or_num(flip(&m))))
            }
            "kron" => {
                let v = self.eval_args(args)?;
                let a = v.first().ok_or_else(|| LangError::new("kron needs two arguments", 0))?;
                let b = v.get(1).ok_or_else(|| LangError::new("kron needs two arguments", 0))?;
                check_elems(a.dims().0.saturating_mul(a.dims().1).saturating_mul(b.dims().0).saturating_mul(b.dims().1))?;
                if matches!(a, Value::Cplx(_)) || matches!(b, Value::Cplx(_)) {
                    let (ca, cb) = (a.to_cmatrix()?, b.to_cmatrix()?);
                    let mut out = CMatrix::zeros(ca.rows() * cb.rows(), ca.cols() * cb.cols());
                    for bc in 0..cb.cols() {
                        for br in 0..cb.rows() {
                            let bv = cb.get(br, bc).unwrap_or(CNum::real(0.0));
                            for ac in 0..ca.cols() {
                                for ar in 0..ca.rows() {
                                    let _ = out.set(br + ar * cb.rows(), bc + ac * cb.cols(), ca.get(ar, ac).unwrap_or(CNum::real(0.0)).mul(bv));
                                }
                            }
                        }
                    }
                    return Ok(Some(cmat_or_value(out)));
                }
                let (a, b) = (a.to_matrix()?, b.to_matrix()?);
                let mut out = Matrix::zeros(a.rows() * b.rows(), a.cols() * b.cols());
                for cb in 0..b.cols() {
                    for rb in 0..b.rows() {
                        let bv = b.get(rb, cb).unwrap_or(0.0);
                        for ca in 0..a.cols() {
                            for ra in 0..a.rows() {
                                let _ = out.set(rb + ra * b.rows(), cb + ca * b.cols(), a.get(ra, ca).unwrap_or(0.0) * bv);
                            }
                        }
                    }
                }
                Ok(Some(mat_or_num(out)))
            }
            "cat" => {
                let v = self.eval_args(args)?;
                let dim = v.scalar_at(0)? as usize;
                if v.len() < 3 {
                    return Err(LangError::new("cat needs a dim and at least two matrices", 0));
                }
                if v.iter().skip(1).all(|x| matches!(x, Value::Struct(_))) {
                    let mut acc = match v.get(1) {
                        Some(Value::Struct(s)) => s.clone(),
                        _ => return Err(LangError::new("cat needs struct arrays", 0)),
                    };
                    for x in v.iter().skip(2) {
                        if let Value::Struct(s) = x {
                            check_elems(acc.numel().saturating_add(s.numel()))?;
                            acc = match dim {
                                1 => StructArray::vcat(&acc, s)?,
                                2 => StructArray::hcat(&acc, s)?,
                                _ => return Err(LangError::new("cat supports dims 1 and 2", 0)),
                            };
                        }
                    }
                    return Ok(Some(Value::Struct(acc)));
                }
                if v.iter().skip(1).all(|x| matches!(x, Value::Cell(_))) {
                    let mut acc = match v.get(1) {
                        Some(Value::Cell(c)) => c.clone(),
                        _ => return Err(LangError::new("cat needs cell arrays", 0)),
                    };
                    for x in v.iter().skip(2) {
                        // Guarded by the all-cells check above.
                        let c = match x {
                            Value::Cell(c) => c,
                            _ => continue,
                        };
                        acc = match dim {
                            1 => CellArray::vcat(&acc, c)?,
                            2 => CellArray::hcat(&acc, c)?,
                            _ => return Err(LangError::new("cat supports dims 1 and 2", 0)),
                        };
                    }
                    return Ok(Some(Value::Cell(acc)));
                }
                if v.iter().skip(1).any(|x| matches!(x, Value::Cplx(_))) {
                    let mut acc = v.get(1).ok_or_else(|| LangError::new("cat needs matrices", 0))?.to_cmatrix()?;
                    for x in v.iter().skip(2) {
                        let m = x.to_cmatrix()?;
                        check_elems(acc.numel().saturating_add(m.numel()))?;
                        acc = match dim {
                            1 => CMatrix::vcat(&acc, &m).map_err(lang)?,
                            2 => CMatrix::hcat(&acc, &m).map_err(lang)?,
                            _ => return Err(LangError::new("cat supports dims 1 and 2", 0)),
                        };
                    }
                    return Ok(Some(cmat_or_value(acc)));
                }
                let mut acc = v.get(1).ok_or_else(|| LangError::new("cat needs matrices", 0))?.to_matrix()?;
                for x in v.iter().skip(2) {
                    let m = x.to_matrix()?;
                    check_elems(acc.numel().saturating_add(m.numel()))?;
                    acc = match dim {
                        1 => Matrix::vcat(&acc, &m).map_err(lang)?,
                        2 => Matrix::hcat(&acc, &m).map_err(lang)?,
                        _ => return Err(LangError::new("cat supports dims 1 and 2", 0)),
                    };
                }
                Ok(Some(mat_or_num(acc)))
            }
            // `[a b]` / `[a; b]` spelled as calls. Cells concat as cells;
            // chars pull the result to char; otherwise numeric.
            "horzcat" | "vertcat" => {
                let v = self.eval_args(args)?;
                if v.len() < 2 {
                    return Err(LangError::new("horzcat/vertcat need at least two arguments", 0));
                }
                let horizontal = name == "horzcat";
                if v.iter().all(|x| matches!(x, Value::Sparse(_))) {
                    let mut acc = match v.first() {
                        Some(Value::Sparse(s)) => s.clone(),
                        _ => return Err(LangError::new("concat needs sparse matrices", 0)),
                    };
                    for x in v.iter().skip(1) {
                        if let Value::Sparse(s) = x {
                            acc = if horizontal { acc.hcat(s).map_err(lang)? } else { acc.vcat(s).map_err(lang)? };
                        }
                    }
                    return Ok(Some(Value::Sparse(acc)));
                }
                if v.iter().all(|x| matches!(x, Value::Struct(_))) {
                    let mut acc = match v.first() {
                        Some(Value::Struct(s)) => s.clone(),
                        _ => return Err(LangError::new("concat needs struct arrays", 0)),
                    };
                    for x in v.iter().skip(1) {
                        if let Value::Struct(s) = x {
                            check_elems(acc.numel().saturating_add(s.numel()))?;
                            acc = if horizontal { StructArray::hcat(&acc, s)? } else { StructArray::vcat(&acc, s)? };
                        }
                    }
                    return Ok(Some(Value::Struct(acc)));
                }
                if v.iter().all(|x| matches!(x, Value::Cell(_))) {
                    let mut acc = match v.first() {
                        Some(Value::Cell(c)) => c.clone(),
                        _ => return Err(LangError::new("concat needs cell arrays", 0)),
                    };
                    for x in v.iter().skip(1) {
                        if let Value::Cell(c) = x {
                            acc = if horizontal { CellArray::hcat(&acc, c)? } else { CellArray::vcat(&acc, c)? };
                        }
                    }
                    return Ok(Some(Value::Cell(acc)));
                }
                let any_char = v.iter().any(|x| matches!(x, Value::Str(_)));
                if !any_char && v.iter().any(|x| matches!(x, Value::Cplx(_))) {
                    let mut acc = v.first().ok_or_else(|| LangError::new("concat needs arguments", 0))?.to_cmatrix()?;
                    for x in v.iter().skip(1) {
                        let m = x.to_cmatrix()?;
                        check_elems(acc.numel().saturating_add(m.numel()))?;
                        acc = if horizontal { CMatrix::hcat(&acc, &m).map_err(lang)? } else { CMatrix::vcat(&acc, &m).map_err(lang)? };
                    }
                    return Ok(Some(cmat_or_value(acc)));
                }
                let mut acc = match v.first() {
                    Some(x) if any_char => x.to_numeric()?,
                    Some(x) => x.to_matrix()?,
                    None => return Err(LangError::new("concat needs arguments", 0)),
                };
                for x in v.iter().skip(1) {
                    let m = if any_char { x.to_numeric()? } else { x.to_matrix()? };
                    check_elems(acc.numel().saturating_add(m.numel()))?;
                    acc = if horizontal { Matrix::hcat(&acc, &m).map_err(lang)? } else { Matrix::vcat(&acc, &m).map_err(lang)? };
                }
                if any_char {
                    if acc.rows() > 1 {
                        return Err(LangError::new("multi-row char arrays are not supported (char is a row vector)", 0));
                    }
                    let s: String =
                        (0..acc.cols()).filter_map(|i| acc.get(0, i)).map(|x| char::from_u32(x.max(0.0) as u32).unwrap_or('\u{FFFD}')).collect();
                    return Ok(Some(Value::Str(s)));
                }
                Ok(Some(mat_or_num(acc)))
            }
            "ndims" => Ok(Some(Value::Num(2.0))),
            "squeeze" => {
                let v = self.eval_args(args)?;
                Ok(v.first().cloned())
            }
            "circshift" => {
                let v = self.eval_args(args)?;
                let first = v.first().ok_or_else(|| LangError::new("circshift needs an argument", 0))?;
                let k = v.scalar_at(1)? as i64;
                let n = (first.dims().0 * first.dims().1) as i64;
                if n == 0 {
                    return Ok(Some(first.clone()));
                }
                let shift = |m: &Matrix| -> Matrix {
                    let mut out = Matrix::zeros(m.rows(), m.cols());
                    for i in 0..n {
                        let to = (i + k).rem_euclid(n) as usize;
                        let _ = out.set(to % m.rows(), to / m.rows(), m.get_linear(i as usize).unwrap_or(0.0));
                    }
                    out
                };
                if let Value::Cplx(cm) = first {
                    return cplanes(cm, |m| Ok(shift(m))).map(cmat_or_value).map(Some);
                }
                Ok(Some(Value::Mat(shift(&first.to_matrix()?))))
            }
            "rot90" => {
                let v = self.eval_args(args)?;
                let first = v.first().ok_or_else(|| LangError::new("rot90 needs an argument", 0))?;
                let k = v.scalar_at(1).unwrap_or(1.0).rem_euclid(4.0) as usize;
                let rot = |m: &Matrix| -> Matrix {
                    let mut out = m.clone();
                    for _ in 0..k {
                        let (r, c) = out.shape();
                        let mut next = Matrix::zeros(c, r);
                        for i in 0..r {
                            for j in 0..c {
                                let _ = next.set(c - 1 - j, i, out.get(i, j).unwrap_or(0.0));
                            }
                        }
                        out = next;
                    }
                    out
                };
                if let Value::Cplx(cm) = first {
                    return cplanes(cm, |m| Ok(rot(m))).map(cmat_or_value).map(Some);
                }
                let out = rot(&first.to_matrix()?);
                Ok(Some(mat_or_num(out)))
            }
            // --- statistics / elementwise extras ---------------------------
            "dot" => {
                let a = self.arg_matrix(args, 0)?;
                let b = self.arg_matrix(args, 1)?;
                if a.numel() != b.numel() {
                    return Err(LangError::new("dot needs equal-length vectors", 0));
                }
                Ok(Some(Value::Num(a.data().iter().zip(b.data()).map(|(x, y)| x * y).sum())))
            }
            "cross" => {
                let a = self.arg_matrix(args, 0)?;
                let b = self.arg_matrix(args, 1)?;
                if a.numel() != 3 || b.numel() != 3 {
                    return Err(LangError::new("cross needs 3-element vectors", 0));
                }
                let (x, y) = (a.data(), b.data());
                let out = [
                    x.get(1).copied().unwrap_or(0.0) * y.get(2).copied().unwrap_or(0.0)
                        - x.get(2).copied().unwrap_or(0.0) * y.get(1).copied().unwrap_or(0.0),
                    x.get(2).copied().unwrap_or(0.0) * y.first().copied().unwrap_or(0.0)
                        - x.first().copied().unwrap_or(0.0) * y.get(2).copied().unwrap_or(0.0),
                    x.first().copied().unwrap_or(0.0) * y.get(1).copied().unwrap_or(0.0)
                        - x.get(1).copied().unwrap_or(0.0) * y.first().copied().unwrap_or(0.0),
                ];
                let m = if a.rows() == 3 { Matrix::col_vector(out.to_vec()) } else { Matrix::row_vector(out.to_vec()) };
                Ok(Some(Value::Mat(m)))
            }
            "fix" => self.unary_mat(args, f64::trunc),
            "log2" | "expm1" | "log1p" | "cosh" | "sinh" | "tanh" | "sech" | "csch" | "coth" => {
                let f: fn(f64) -> f64 = match name {
                    "log2" => f64::log2,
                    "expm1" => f64::exp_m1,
                    "log1p" => f64::ln_1p,
                    "cosh" => f64::cosh,
                    "sinh" => f64::sinh,
                    "tanh" => f64::tanh,
                    "sech" => |x| 1.0 / x.cosh(),
                    "csch" => |x| 1.0 / x.sinh(),
                    _ => |x| 1.0 / x.tanh(),
                };
                self.unary_mat(args, f)
            }
            "hypot" => {
                let a = self.arg_matrix(args, 0)?;
                let b = self.arg_matrix(args, 1)?;
                a.broadcast(&b, |x, y| x.hypot(y)).map(mat_or_num).map(Some).map_err(lang)
            }
            "nthroot" => {
                let a = self.arg_matrix(args, 0)?;
                let b = self.arg_matrix(args, 1)?;
                a.broadcast(&b, |x, y| if x < 0.0 && y.fract() == 0.0 && (y as i64) % 2 != 0 { -(-x).powf(1.0 / y) } else { x.powf(1.0 / y) })
                    .map(mat_or_num)
                    .map(Some)
                    .map_err(lang)
            }
            "nextpow2" => {
                let m = self.arg_matrix(args, 0)?;
                Ok(Some(mat_or_num(m.map(|x| if x == 0.0 { 0.0 } else { x.abs().log2().ceil() }))))
            }
            "pow2" => {
                let m = self.arg_matrix(args, 0)?;
                Ok(Some(mat_or_num(m.map(|x| 2f64.powf(x)))))
            }
            "randi" => {
                let v = self.eval_args(args)?;
                let imax = v.scalar_at(0)?.max(1.0) as u64;
                let (r, c) = if v.len() > 2 {
                    (v.scalar_at(1)? as usize, v.scalar_at(2)? as usize)
                } else if v.len() == 2 {
                    let n = v.scalar_at(1)? as usize;
                    (n, n)
                } else {
                    (1, 1)
                };
                check_elems(r.saturating_mul(c))?;
                let mut data = Vec::with_capacity(r.saturating_mul(c));
                for _ in 0..r.saturating_mul(c) {
                    data.push((self.next_f64() * imax as f64).floor() + 1.0);
                }
                Ok(Some(mat_or_num(Matrix::from_col_major(r, c, data).map_err(lang)?)))
            }
            // --- predicates ---------------------------------------------------
            "isnan" | "isinf" | "isfinite" => {
                let m = self.arg_matrix(args, 0)?;
                let f: fn(f64) -> bool = match name {
                    "isnan" => f64::is_nan,
                    "isinf" => f64::is_infinite,
                    _ => f64::is_finite,
                };
                Ok(Some(mat_or_num(m.map(|x| if f(x) { 1.0 } else { 0.0 }))))
            }
            "isempty" => {
                let v = self.eval_args(args)?;
                let e = match v.first() {
                    Some(x) => {
                        let (r, c) = x.dims();
                        r * c == 0
                    }
                    None => true,
                };
                Ok(Some(Value::Num(if e { 1.0 } else { 0.0 })))
            }
            "isscalar" | "isvector" | "isrow" | "iscolumn" | "ismatrix" | "isnumeric" | "ischar" | "islogical" | "isreal" | "isa" | "isequal"
            | "isequaln" => self.predicate(name, args),
            "complex" => {
                let v = self.eval_args(args)?;
                let re = v.first().ok_or_else(|| LangError::new("complex needs real and imaginary parts", 0))?.to_numeric()?;
                let im = v.get(1).ok_or_else(|| LangError::new("complex needs two arguments", 0))?.to_numeric()?;
                Ok(Some(cmat_or_value(CMatrix::from_parts(&re, &im).map_err(lang)?)))
            }
            // --- polynomials / fitting -------------------------------------
            "polyval" => {
                let p = self.arg_matrix(args, 0)?;
                let x = self.arg_matrix(args, 1)?;
                let coefs = p.data();
                Ok(Some(mat_or_num(x.map(|x| {
                    let mut acc = 0.0;
                    for &c in coefs {
                        acc = acc * x + c;
                    }
                    acc
                }))))
            }
            "polyfit" => {
                let x = self.arg_matrix(args, 0)?;
                let y = self.arg_matrix(args, 1)?;
                let d = self.eval_args(args)?.scalar_at(2)? as usize;
                polyfit(&x, &y, d).map(|c| Value::Mat(Matrix::row_vector(c))).map(Some)
            }
            "roots" => {
                let p = self.arg_matrix(args, 0)?;
                let r = roots(&p)?;
                let n = r.len();
                Ok(Some(cmat_or_value(CMatrix::from_col_major(n, 1, r).map_err(lang)?)))
            }
            "poly" => {
                let r = self.arg_matrix(args, 0)?;
                let mut c = vec![1.0];
                for &root in r.data() {
                    let mut next = vec![0.0; c.len() + 1];
                    for (i, &a) in c.iter().enumerate() {
                        if let Some(x) = next.get_mut(i) {
                            *x += a;
                        }
                        if let Some(x) = next.get_mut(i + 1) {
                            *x -= a * root;
                        }
                    }
                    c = next;
                }
                Ok(Some(Value::Mat(Matrix::row_vector(c))))
            }
            "conv" => {
                let a = self.arg_matrix(args, 0)?;
                let b = self.arg_matrix(args, 1)?;
                let (x, h) = (a.data(), b.data());
                check_elems(x.len().saturating_add(h.len()))?;
                let mut out = vec![0.0; x.len().saturating_add(h.len()).saturating_sub(1)];
                for (i, &a) in x.iter().enumerate() {
                    for (j, &b) in h.iter().enumerate() {
                        if let Some(s) = out.get_mut(i + j) {
                            *s += a * b;
                        }
                    }
                }
                let m = if a.cols() == 1 && b.cols() == 1 { Matrix::col_vector(out) } else { Matrix::row_vector(out) };
                Ok(Some(Value::Mat(m)))
            }
            "deconv" => {
                let a = self.arg_matrix(args, 0)?; // dividend
                let b = self.arg_matrix(args, 1)?; // divisor
                let (num, den) = (a.data().to_vec(), b.data().to_vec());
                if den.is_empty() || den.first().copied().unwrap_or(0.0) == 0.0 {
                    return Err(LangError::new("deconv: leading divisor coefficient is zero", 0));
                }
                let mut q = vec![0.0; num.len().saturating_sub(den.len()) + 1];
                let mut rem = num;
                let d0 = den.first().copied().unwrap_or(1.0);
                for i in 0..q.len() {
                    let coef = rem.get(i).copied().unwrap_or(0.0) / d0;
                    if let Some(slot) = q.get_mut(i) {
                        *slot = coef;
                    }
                    for (j, &d) in den.iter().enumerate() {
                        if let Some(slot) = rem.get_mut(i + j) {
                            *slot -= coef * d;
                        }
                    }
                }
                Ok(Some(Value::Mat(Matrix::row_vector(q))))
            }
            // --- DSP ------------------------------------------------------
            "fft" | "ifft" | "fft2" | "ifft2" => {
                let inv = name.starts_with('i');
                let two_d = name.ends_with('2');
                Ok(Some(self.dsp_fft(args, two_d, inv)?))
            }
            "fftshift" | "ifftshift" => {
                let v = self.eval_args(args)?;
                let x = v.first().ok_or_else(|| LangError::new("fftshift needs an argument", 0))?;
                let order = if name == "fftshift" { matcore::dsp::fftshift_order } else { matcore::dsp::ifftshift_order };
                match x {
                    Value::Cplx(cm) => {
                        let d = shift_both(cm.rows(), cm.cols(), order, |r, c| cm.get(r, c).unwrap_or(CNum::real(0.0)));
                        Ok(Some(cmat_or_value(CMatrix::from_col_major(cm.rows(), cm.cols(), d).map_err(lang)?)))
                    }
                    _ => {
                        let m = x.to_numeric()?;
                        let d = shift_both(m.rows(), m.cols(), order, |r, c| m.get(r, c).unwrap_or(0.0));
                        Ok(Some(mat_or_num(Matrix::from_col_major(m.rows(), m.cols(), d).map_err(lang)?)))
                    }
                }
            }
            "filter" => {
                let (y, _zf) = self.dsp_filter(args)?;
                Ok(Some(cmat_or_value(y)))
            }
            "xcorr" => {
                let (r, _lags) = self.dsp_xcorr(args)?;
                Ok(Some(cmat_or_value(r)))
            }
            "freqz" => {
                let (h, _w) = self.dsp_freqz(args)?;
                Ok(Some(cmat_or_value(h)))
            }
            "dftmtx" => {
                let n = self.eval_args(args)?.first().map(|v| v.scalar()).transpose()?.unwrap_or(0.0);
                if n <= 0.0 || n.fract() != 0.0 {
                    return Err(LangError::new("dftmtx needs a positive integer", 0));
                }
                let n = n as usize;
                check_elems(n.saturating_mul(n))?;
                let mut d = Vec::with_capacity(n * n);
                for k in 0..n {
                    for j in 0..n {
                        let a = -2.0 * std::f64::consts::PI * (j as f64) * (k as f64) / n as f64;
                        d.push(CNum::new(a.cos(), a.sin()));
                    }
                }
                Ok(Some(cmat_or_value(CMatrix::from_col_major(n, n, d).map_err(lang)?)))
            }
            "unwrap" => {
                let m = self.arg_matrix(args, 0)?;
                let tol = self.eval_args(args)?.scalar_at(1).unwrap_or(std::f64::consts::PI);
                let (rows, cols) = m.shape();
                let mut out = Matrix::zeros(rows, cols);
                if rows == 1 && cols > 1 {
                    let row: Vec<f64> = (0..cols).map(|c| m.get(0, c).unwrap_or(0.0)).collect();
                    for (c, v) in matcore::dsp::unwrap_phase(&row, tol).iter().enumerate() {
                        let _ = out.set(0, c, *v);
                    }
                } else {
                    for c in 0..cols {
                        let col: Vec<f64> = (0..rows).map(|r| m.get(r, c).unwrap_or(0.0)).collect();
                        for (r, v) in matcore::dsp::unwrap_phase(&col, tol).iter().enumerate() {
                            let _ = out.set(r, c, *v);
                        }
                    }
                }
                Ok(Some(mat_or_num(out)))
            }
            "hamming" | "hann" | "blackman" | "bartlett" | "rectwin" | "triang" | "kaiser" | "gausswin" => {
                let v = self.eval_args(args)?;
                let n = v.first().map(|x| x.scalar()).transpose()?.unwrap_or(0.0);
                if n < 0.0 || n.fract() != 0.0 {
                    return Err(LangError::new(format!("{name} needs a nonnegative integer length"), 0));
                }
                let n = n as usize;
                check_elems(n)?;
                let periodic = v.iter().skip(1).any(|x| matches!(x, Value::Str(s) if s.trim().eq_ignore_ascii_case("periodic")));
                let w = match name {
                    "hamming" => matcore::dsp::hamming(n, periodic),
                    "hann" => matcore::dsp::hann(n, periodic),
                    "blackman" => matcore::dsp::blackman(n, periodic),
                    "bartlett" => matcore::dsp::bartlett(n),
                    "rectwin" => matcore::dsp::rectwin(n),
                    "triang" => matcore::dsp::triang(n),
                    "kaiser" => matcore::dsp::kaiser(n, v.scalar_at(1).unwrap_or(0.5)),
                    "gausswin" => matcore::dsp::gausswin(n, v.scalar_at(1).unwrap_or(2.5)),
                    _ => Vec::new(),
                };
                Ok(Some(Value::Mat(Matrix::col_vector(w))))
            }
            // --- interpolation / integration / solving ----------------------
            "interp1" => {
                let v = self.eval_args(args)?;
                let x = v.first().ok_or_else(|| LangError::new("interp1 needs x", 0))?.to_matrix()?;
                let yv = v.get(1).ok_or_else(|| LangError::new("interp1 needs y", 0))?.to_matrix()?;
                let xq = v.get(2).ok_or_else(|| LangError::new("interp1 needs query points", 0))?.to_matrix()?;
                interp1(&x, &yv, &xq).map(mat_or_num).map(Some)
            }
            "trapz" => {
                let v = self.eval_args(args)?;
                let (x, y) = if v.len() == 1 {
                    let y = v.first().ok_or_else(|| LangError::new("trapz needs data", 0))?.to_matrix()?;
                    let xs: Vec<f64> = (0..y.numel()).map(|i| i as f64).collect();
                    (Matrix::row_vector(xs), y)
                } else {
                    (
                        v.first().ok_or_else(|| LangError::new("trapz needs x", 0))?.to_matrix()?,
                        v.get(1).ok_or_else(|| LangError::new("trapz needs y", 0))?.to_matrix()?,
                    )
                };
                Ok(Some(Value::Num(trapz(&x, &y))))
            }
            "fzero" | "fminsearch" | "integral" | "quad" => self.solve_scalar(name, args),
            // --- linear algebra ---------------------------------------------
            "eig" | "svd" | "qr" | "lu" | "chol" | "rref" | "pinv" | "rank" | "cond" | "trace" | "null" | "orth" => self.linalg(name, args),
            // --- strings -----------------------------------------------------
            "num2str" => {
                let v = self.eval_args(args)?;
                let m = v.first().ok_or_else(|| LangError::new("num2str needs a number", 0))?.to_matrix()?;
                let long = self.fmt_long;
                let parts: Vec<String> = m.data().iter().map(|&x| fmt_num(x, long)).collect();
                Ok(Some(Value::Str(parts.join("  "))))
            }
            "str2double" | "str2num" => {
                let v = self.eval_args(args)?;
                match v.first() {
                    Some(Value::Str(s)) => Ok(Some(Value::Num(s.trim().parse::<f64>().unwrap_or(f64::NAN)))),
                    Some(x) => Ok(Some(Value::Num(x.scalar()?))),
                    None => Err(LangError::new("str2double needs a string", 0)),
                }
            }
            "strcmp" | "strcmpi" => {
                let v = self.eval_args(args)?;
                let (a, b) = (v.first(), v.get(1));
                let eq = match (a, b) {
                    (Some(Value::Str(x)), Some(Value::Str(y))) => {
                        if name == "strcmpi" {
                            x.eq_ignore_ascii_case(y)
                        } else {
                            x == y
                        }
                    }
                    (Some(x), Some(y)) => values_equal(x, y),
                    _ => false,
                };
                Ok(Some(Value::Num(if eq { 1.0 } else { 0.0 })))
            }
            "strcat" => {
                let v = self.eval_args(args)?;
                let mut s = String::new();
                for x in &v {
                    match x {
                        Value::Str(t) => s.push_str(t),
                        other => s.push_str(&fmt_value(other, false).to_string()),
                    }
                }
                Ok(Some(Value::Str(s)))
            }
            "upper" | "lower" => {
                let v = self.eval_args(args)?;
                match v.first() {
                    Some(Value::Str(s)) => Ok(Some(Value::Str(if name == "upper" { s.to_uppercase() } else { s.to_lowercase() }))),
                    _ => Err(LangError::new(format!("{name} needs a string"), 0)),
                }
            }
            "strtrim" => {
                let v = self.eval_args(args)?;
                match v.first() {
                    Some(Value::Str(s)) => Ok(Some(Value::Str(s.trim().to_string()))),
                    _ => Err(LangError::new("strtrim needs a string", 0)),
                }
            }
            "strrep" => {
                let v = self.eval_args(args)?;
                let get_str = |i: usize| -> Result<String, LangError> {
                    match v.get(i) {
                        Some(Value::Str(s)) => Ok(s.clone()),
                        _ => Err(LangError::new("strrep needs three strings", 0)),
                    }
                };
                Ok(Some(Value::Str(get_str(0)?.replace(&get_str(1)?, &get_str(2)?))))
            }
            "strfind" => {
                let v = self.eval_args(args)?;
                let (s, pat) = match (v.first(), v.get(1)) {
                    (Some(Value::Str(a)), Some(Value::Str(b))) => (a.clone(), b.clone()),
                    _ => return Err(LangError::new("strfind needs two strings", 0)),
                };
                if pat.is_empty() {
                    return Ok(Some(Value::Mat(Matrix::zeros(0, 0))));
                }
                let idxs: Vec<f64> = s.match_indices(&pat).map(|(i, _)| (i + 1) as f64).collect();
                Ok(Some(mat_or_num(Matrix::row_vector(idxs))))
            }
            "double" | "logical" => {
                let v = self.eval_args(args)?;
                match v.first() {
                    Some(Value::Str(s)) => Ok(Some(Value::Mat(Matrix::row_vector(s.chars().map(|c| c as u32 as f64).collect())))),
                    Some(x) => Ok(Some(mat_or_num(x.to_matrix()?))),
                    None => Err(LangError::new(format!("{name} needs an argument"), 0)),
                }
            }
            "char" => {
                let v = self.eval_args(args)?;
                // `char({'abc'})` unwraps a cellstr. A multi-element cellstr
                // would need a 2-D char array we don't have — error instead
                // of silently losing rows.
                if let Some(Value::Cell(ca)) = v.first() {
                    let mut parts: Vec<&str> = Vec::with_capacity(ca.numel());
                    for i in 0..ca.numel() {
                        match ca.get_linear(i) {
                            Some(Value::Str(s)) => parts.push(s),
                            _ => return Err(LangError::new("char needs a cell array of strings", 0)),
                        }
                    }
                    match parts.as_slice() {
                        [] => return Ok(Some(Value::Str(String::new()))),
                        [s] => return Ok(Some(Value::Str((*s).to_string()))),
                        _ => {
                            return Err(LangError::new(
                                "char of a multi-element cellstr needs a 2-D char array (unsupported) — index the cell instead",
                                0,
                            ));
                        }
                    }
                }
                let m = v.first().ok_or_else(|| LangError::new("char needs an argument", 0))?.to_matrix()?;
                let s: String = m.data().iter().map(|&x| char::from_u32(x as u32).unwrap_or(' ')).collect();
                Ok(Some(Value::Str(s)))
            }
            // --- environment / errors --------------------------------------
            "eval" => {
                let v = self.eval_args(args)?;
                match v.first() {
                    Some(Value::Str(code)) => {
                        let out = self.run(code.clone().as_str())?;
                        self.print_buf.push_str(&out);
                        Ok(None)
                    }
                    _ => Err(LangError::new("eval needs a string", 0)),
                }
            }
            "error" => {
                let v = self.eval_args(args)?;
                // `error('id:tag', 'message')` carries an identifier; a first
                // arg containing ':' before any '%' reads as the error ID.
                let (id, msg) = match v.as_slice() {
                    [Value::Str(a), b, ..] if a.contains(':') && !a.contains('%') => (
                        a.clone(),
                        match b {
                            Value::Str(s) => s.clone(),
                            x => fmt_value(x, false).to_string(),
                        },
                    ),
                    _ => (
                        String::new(),
                        match v.first() {
                            Some(Value::Str(s)) => s.clone(),
                            Some(x) => fmt_value(x, false).to_string(),
                            None => "error".into(),
                        },
                    ),
                };
                Err(LangError::with_id(id, msg, 0))
            }
            "lasterror" => {
                let mut m = std::collections::BTreeMap::new();
                if let Some(e) = &self.last_err {
                    m.insert("identifier".into(), Value::Str(e.id.clone()));
                    m.insert("message".into(), Value::Str(e.message.clone()));
                }
                Ok(Some(Value::Struct(StructArray::from_map(m))))
            }
            "warning" => {
                let v = self.eval_args(args)?;
                let msg = match v.first() {
                    Some(Value::Str(s)) => s.clone(),
                    Some(x) => fmt_value(x, false).to_string(),
                    None => "warning".into(),
                };
                self.print_buf.push_str(&format!("Warning: {msg}\n"));
                Ok(None)
            }
            "exist" => {
                let v = self.eval_args(args)?;
                let name = match v.first() {
                    Some(Value::Str(s)) => s.clone(),
                    _ => return Err(LangError::new("exist needs a name", 0)),
                };
                let code = if self.vars.contains_key(&name) {
                    1.0
                } else if std::path::Path::new(&name).exists() || self.paths.iter().any(|d| d.join(format!("{name}.m")).is_file()) {
                    2.0
                } else if self.is_function(&name)? || self.is_builtin(&name) {
                    5.0
                } else {
                    0.0
                };
                Ok(Some(Value::Num(code)))
            }
            "which" => {
                let v = self.eval_args(args)?;
                let name = match v.first() {
                    Some(Value::Str(s)) => s.clone(),
                    _ => return Err(LangError::new("which needs a name", 0)),
                };
                for dir in &self.paths {
                    let f = dir.join(format!("{name}.m"));
                    if f.is_file() {
                        self.print_buf.push_str(&format!("{}\n", f.display()));
                        return Ok(None);
                    }
                }
                if self.is_function(&name)? {
                    self.print_buf.push_str(&format!("{name}: user function in session\n"));
                } else if self.is_builtin(&name) {
                    self.print_buf.push_str(&format!("{name}: built-in function\n"));
                } else {
                    self.print_buf.push_str(&format!("'{name}' not found.\n"));
                }
                Ok(None)
            }
            "clc" | "home" => Ok(None),
            "addpath" | "rmpath" => {
                let v = self.eval_args(args)?;
                for x in &v {
                    if let Value::Str(s) = x {
                        // `addpath(genpath(pwd))` passes a `;`-separated list
                        // (MATLAB pathsep on all platforms here).
                        for part in s.split(';') {
                            if part.is_empty() {
                                continue;
                            }
                            let dir = std::path::PathBuf::from(part);
                            if name == "addpath" {
                                if !self.paths.contains(&dir) {
                                    self.paths.push(dir);
                                }
                            } else {
                                self.paths.retain(|p| p != &dir);
                            }
                        }
                    }
                }
                Ok(None)
            }
            "genpath" => {
                let v = self.eval_args(args)?;
                let root = match v.first() {
                    Some(Value::Str(s)) => s.clone(),
                    _ => return Err(LangError::new("genpath needs a directory", 0)),
                };
                fn walk(d: &std::path::Path, out: &mut Vec<String>, depth: usize) {
                    if depth > 32 {
                        return;
                    }
                    if let Ok(rd) = std::fs::read_dir(d) {
                        for e in rd.flatten() {
                            let p = e.path();
                            if p.is_dir() {
                                out.push(p.display().to_string());
                                walk(&p, out, depth + 1);
                            }
                        }
                    }
                }
                let mut dirs = Vec::new();
                walk(std::path::Path::new(&root), &mut dirs, 0);
                Ok(Some(Value::Str(dirs.join(";"))))
            }
            "fullfile" => {
                let v = self.eval_args(args)?;
                let mut p = std::path::PathBuf::new();
                for x in &v {
                    if let Value::Str(s) = x {
                        p.push(s);
                    }
                }
                Ok(Some(Value::Str(p.display().to_string())))
            }
            "csvread" => {
                let v = self.eval_args(args)?;
                let fname = match v.first() {
                    Some(Value::Str(s)) => s.clone(),
                    _ => return Err(LangError::new("csvread needs a filename", 0)),
                };
                let text = std::fs::read_to_string(&fname).map_err(|e| LangError::new(format!("csvread: {e}"), 0))?;
                let mut rows: Vec<Vec<f64>> = Vec::new();
                for line in text.lines() {
                    let row: Vec<f64> = line
                        .split([',', ';', '\t', ' '])
                        .filter(|s| !s.trim().is_empty())
                        .map(|s| s.trim().parse::<f64>().unwrap_or(f64::NAN))
                        .collect();
                    if !row.is_empty() {
                        rows.push(row);
                    }
                }
                let r = rows.len();
                let c = rows.iter().map(|x| x.len()).max().unwrap_or(0);
                let mut data = vec![0.0; r * c];
                for (i, row) in rows.iter().enumerate() {
                    for (j, &val) in row.iter().enumerate() {
                        if let Some(slot) = data.get_mut(j * r + i) {
                            *slot = val;
                        }
                    }
                }
                Ok(Some(Value::Mat(Matrix::from_col_major(r, c, data).map_err(lang)?)))
            }
            "path" => {
                for d in &self.paths {
                    self.print_buf.push_str(&format!("{}\n", d.display()));
                }
                Ok(None)
            }
            "pwd" => Ok(Some(Value::Str(std::env::current_dir().map(|d| d.display().to_string()).unwrap_or_default()))),
            "cd" => {
                let v = self.eval_args(args)?;
                if let Some(Value::Str(s)) = v.first() {
                    std::env::set_current_dir(s).map_err(|e| LangError::new(format!("cd: {e}"), 0))?;
                    // Keep the working directory on the search path.
                    if let Ok(d) = std::env::current_dir() {
                        if self.paths.first() == Some(&d) || !self.paths.contains(&d) {
                            self.paths.insert(0, d);
                        }
                    }
                }
                Ok(Some(Value::Str(std::env::current_dir().map(|d| d.display().to_string()).unwrap_or_default())))
            }
            "dir" | "ls" => {
                let v = self.eval_args(args)?;
                let pat = match v.first() {
                    Some(Value::Str(s)) => s.clone(),
                    _ => "*".to_string(),
                };
                let dir = std::env::current_dir().map_err(|e| LangError::new(format!("dir: {e}"), 0))?;
                let read = std::fs::read_dir(&dir).map_err(|e| LangError::new(format!("dir: {e}"), 0))?;
                for ent in read.flatten() {
                    let name = ent.file_name().to_string_lossy().to_string();
                    if glob_match(&pat, &name) {
                        self.print_buf.push_str(&name);
                        self.print_buf.push('\n');
                    }
                }
                Ok(None)
            }
            "tic" => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    #[allow(clippy::disallowed_methods)]
                    {
                        self.tic = Some(std::time::Instant::now());
                    }
                }
                Ok(None)
            }
            "toc" => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    #[allow(clippy::disallowed_methods)]
                    let now = std::time::Instant::now();
                    let secs = self.tic.map(|t| now.duration_since(t).as_secs_f64()).unwrap_or(0.0);
                    self.print_buf.push_str(&format!("Elapsed time is {} seconds.\n", fmt_num(secs, true)));
                    Ok(Some(Value::Num(secs)))
                }
                #[cfg(target_arch = "wasm32")]
                {
                    Ok(Some(Value::Num(0.0)))
                }
            }
            "pause" => Ok(None),
            // Multi-output-only builtins called with one target return the
            // first output (meshgrid → X, histcounts → counts).
            "meshgrid" | "histcounts" => match self.builtin_multi(name, args, 1)? {
                Some(mut outs) => Ok(outs.drain(..1).next()),
                None => Err(LangError::new(format!("'{name}' needs output targets"), 0)),
            },
            // --- file I/O ----------------------------------------------------
            "save" | "load" => self.file_io(name, args),
            "type" => {
                let v = self.eval_args(args)?;
                let fname = match v.first() {
                    Some(Value::Str(s)) => s.clone(),
                    _ => return Err(LangError::new("type needs a filename", 0)),
                };
                let path = if std::path::Path::new(&fname).is_file() {
                    std::path::PathBuf::from(&fname)
                } else {
                    let mut found = None;
                    for dir in &self.paths {
                        let f = dir.join(&fname);
                        if f.is_file() {
                            found = Some(f);
                            break;
                        }
                        let f = dir.join(format!("{fname}.m"));
                        if f.is_file() {
                            found = Some(f);
                            break;
                        }
                    }
                    match found {
                        Some(f) => f,
                        None => return Err(LangError::new(format!("file '{fname}' not found"), 0)),
                    }
                };
                let text = std::fs::read_to_string(&path).map_err(|e| LangError::new(format!("cannot read {}: {e}", path.display()), 0))?;
                self.print_buf.push_str(&text);
                if !text.ends_with('\n') {
                    self.print_buf.push('\n');
                }
                Ok(None)
            }
            _ => self.fig_command(name, args),
        }
    }

    /// Current figure, creating figure 1 on first use (M-code auto-creates).
    fn figure_mut(&mut self) -> &mut matplot::Figure {
        if self.cur_fig == 0 || self.cur_fig > self.figs.len() {
            self.figs.push(matplot::Figure::new());
            self.cur_fig = self.figs.len();
        }
        &mut self.figs[self.cur_fig - 1]
    }

    /// Evaluate args then apply a figure-related command.
    fn fig_command(&mut self, name: &str, args: &[Expr]) -> Result<Option<Value>, LangError> {
        use matplot::SeriesKind;
        match name {
            "figure" => {
                let v = self.eval_args(args)?;
                let n = match v.first() {
                    Some(val) => val.scalar()? as usize,
                    None => self.figs.len() + 1,
                };
                if n > MAX_FIGURES {
                    return Err(LangError::new("figure number exceeds limit", 0));
                }
                while self.figs.len() < n {
                    self.figs.push(matplot::Figure::new());
                }
                self.cur_fig = n.max(1);
                Ok(Some(Value::Num(self.cur_fig as f64)))
            }
            "gcf" => {
                let _ = self.figure_mut();
                Ok(Some(Value::Num(self.cur_fig as f64)))
            }
            "subplot" => {
                // subplot(m,n,p) — p scalar or vector (axes spans the bounding
                // box of the cells). subplot(mnp) decodes the packed form.
                let v = self.eval_args(args)?;
                let (rows, cols, pos): (usize, usize, Vec<usize>) = if v.len() == 1 {
                    let n = v[0].scalar()? as u32;
                    if !(111..=999).contains(&n) {
                        return Err(LangError::new("subplot: packed form needs three digits 1-9", 0));
                    }
                    let (m, nn, p) = ((n / 100) as usize, ((n / 10) % 10) as usize, (n % 10) as usize);
                    if nn == 0 || p == 0 || p > m * nn {
                        return Err(LangError::new("subplot: invalid packed position", 0));
                    }
                    (m, nn, vec![p])
                } else if v.len() >= 3 {
                    let m = v[0].scalar()? as usize;
                    let n = v[1].scalar()? as usize;
                    let p: Vec<usize> = match &v[2] {
                        Value::Num(x) => vec![*x as usize],
                        Value::Mat(mm) => mm.data().iter().map(|&x| x.max(0.0) as usize).collect(),
                        _ => return Err(LangError::new("subplot: position must be numeric", 0)),
                    };
                    if m == 0 || n == 0 {
                        return Err(LangError::new("subplot grid dimensions must be at least 1", 0));
                    }
                    if m.saturating_mul(n) > MAX_ELEMS {
                        return Err(LangError::new("subplot grid too large", 0));
                    }
                    if p.iter().any(|&k| k == 0 || k > m.saturating_mul(n)) {
                        return Err(LangError::new("subplot index exceeds the number of tiles", 0));
                    }
                    (m, n, p)
                } else {
                    return Err(LangError::new("usage: subplot(m,n,p) or subplot(mnp)", 0));
                };
                self.figure_mut().subplot(rows, cols, &pos);
                Ok(None)
            }
            "clf" => {
                self.figure_mut().clear();
                Ok(None)
            }
            "close" => {
                let v = self.eval_args(args)?;
                match v.first() {
                    Some(Value::Str(s)) if s == "all" => {
                        self.figs.clear();
                        self.cur_fig = 0;
                    }
                    Some(val) => {
                        let n = val.scalar()? as usize;
                        if n >= 1 && n <= self.figs.len() {
                            self.figs.remove(n - 1);
                            if self.cur_fig > self.figs.len() {
                                self.cur_fig = self.figs.len();
                            }
                        }
                    }
                    None => {
                        if self.cur_fig >= 1 && self.cur_fig <= self.figs.len() {
                            self.figs.remove(self.cur_fig - 1);
                            self.cur_fig = self.cur_fig.min(self.figs.len());
                        }
                    }
                }
                Ok(None)
            }
            "plot" | "scatter" | "stem" | "bar" | "stairs" | "area" | "semilogx" | "semilogy" | "loglog" => {
                let kind = match name {
                    "scatter" => SeriesKind::Scatter,
                    "stem" => SeriesKind::Stem,
                    "stairs" => SeriesKind::Stairs,
                    "area" => SeriesKind::Area,
                    _ if name == "bar" => SeriesKind::Bar,
                    _ => SeriesKind::Line,
                };
                {
                    let fig = self.figure_mut();
                    match name {
                        "semilogx" => fig.xlog = true,
                        "semilogy" => fig.ylog = true,
                        "loglog" => {
                            fig.xlog = true;
                            fig.ylog = true;
                        }
                        _ => {}
                    }
                }
                self.add_series(kind, args)
            }
            "errorbar" => {
                // errorbar(x, y, e [, spec]) — or errorbar(y, e).
                let v = self.eval_args(args)?;
                let mut spec = matplot::LineSpec::new();
                let mut nums: Vec<&Value> = Vec::new();
                for val in &v {
                    match val {
                        Value::Str(s) => spec = matplot::LineSpec::parse(s),
                        other => nums.push(other),
                    }
                }
                let (x, y, e) = match nums.len() {
                    2 => {
                        // errorbar(y, err): x defaults to 1..numel.
                        let y = nums[0].to_matrix()?;
                        let e = nums[1].to_matrix()?;
                        let xs: Vec<f64> = (1..=y.numel()).map(|k| k as f64).collect();
                        (Matrix::row_vector(xs), y, e)
                    }
                    3 => (nums[0].to_matrix()?, nums[1].to_matrix()?, nums[2].to_matrix()?),
                    _ => return Err(LangError::new("errorbar needs (y, e) or (x, y, e)", 0)),
                };
                if !self.figure_mut().hold {
                    self.figure_mut().series.clear();
                }
                self.figure_mut().push_err(SeriesKind::ErrorBar, x.data().to_vec(), y.data().to_vec(), Some(e.data().to_vec()), spec);
                Ok(None)
            }
            "histogram" => {
                let v = self.eval_args(args)?;
                let m = v.first().ok_or_else(|| LangError::new("histogram needs data", 0))?.to_matrix()?;
                let nb = if v.len() > 1 { v.scalar_at(1)? as usize } else { 10 };
                check_elems(nb)?;
                let (counts, edges) = histcounts(&m, nb);
                let centres: Vec<f64> = edges.windows(2).map(|w| (w[0] + w[1]) / 2.0).collect();
                if !self.figure_mut().hold {
                    self.figure_mut().series.clear();
                }
                self.figure_mut().push(SeriesKind::Bar, centres, counts, matplot::LineSpec::new());
                Ok(None)
            }
            "xscale" | "yscale" => {
                let v = self.eval_args(args)?;
                let s = match v.first() {
                    Some(Value::Str(s)) => s.as_str(),
                    _ => return Err(LangError::new(format!("{name} needs 'log' or 'linear'"), 0)),
                };
                let fig = self.figure_mut();
                let log = s == "log";
                if name == "xscale" {
                    fig.xlog = log;
                } else {
                    fig.ylog = log;
                }
                Ok(None)
            }
            "hold" => {
                let v = self.eval_args(args)?;
                let fig = self.figure_mut();
                fig.hold = match v.first() {
                    Some(Value::Str(s)) => s != "off",
                    _ => !fig.hold,
                };
                Ok(None)
            }
            "grid" => {
                let v = self.eval_args(args)?;
                let fig = self.figure_mut();
                fig.grid = match v.first() {
                    Some(Value::Str(s)) => s != "off",
                    _ => !fig.grid,
                };
                Ok(None)
            }
            "title" | "xlabel" | "ylabel" => {
                let v = self.eval_args(args)?;
                let s = match v.first() {
                    Some(Value::Str(s)) => s.clone(),
                    Some(val) => fmt_value(val, self.fmt_long).to_string(),
                    None => return Err(LangError::new(format!("{name} needs a string"), 0)),
                };
                let fig = self.figure_mut();
                match name {
                    "title" => fig.title = Some(s),
                    "xlabel" => fig.xlabel = Some(s),
                    _ => fig.ylabel = Some(s),
                }
                Ok(None)
            }
            "legend" => {
                let v = self.eval_args(args)?;
                let fig = self.figure_mut();
                fig.legend = v
                    .iter()
                    .map(|val| match val {
                        Value::Str(s) => s.clone(),
                        other => fmt_value(other, false).to_string(),
                    })
                    .collect();
                Ok(None)
            }
            "xlim" | "ylim" | "zlim" => {
                let v = self.eval_args(args)?;
                if v.is_empty() {
                    let fig = self.figure_mut();
                    let lim = match name {
                        "xlim" => fig.xlim,
                        "ylim" => fig.ylim,
                        _ => fig.zlim,
                    };
                    let (a, b) = lim.unwrap_or((f64::NAN, f64::NAN));
                    return Ok(Some(Value::Mat(Matrix::row_vector(vec![a, b]))));
                }
                let m = v[0].to_matrix()?;
                let d = m.data();
                if d.len() != 2 {
                    return Err(LangError::new(format!("{name} needs [lo hi]"), 0));
                }
                let lim = (d[0], d[1]);
                let fig = self.figure_mut();
                match name {
                    "xlim" => fig.xlim = Some(lim),
                    "ylim" => fig.ylim = Some(lim),
                    _ => fig.zlim = Some(lim),
                }
                Ok(None)
            }
            "axis" => {
                let v = self.eval_args(args)?;
                match v.first() {
                    None => {
                        let fig = self.figure_mut();
                        let (x0, x1) = fig.xlim.unwrap_or((f64::NAN, f64::NAN));
                        let (y0, y1) = fig.ylim.unwrap_or((f64::NAN, f64::NAN));
                        Ok(Some(Value::Mat(Matrix::row_vector(vec![x0, x1, y0, y1]))))
                    }
                    Some(Value::Str(s)) => {
                        let fig = self.figure_mut();
                        match s.as_str() {
                            "ij" => fig.yrev = true,
                            "xy" => fig.yrev = false,
                            "auto" | "normal" | "tight" => {
                                fig.xlim = None;
                                fig.ylim = None;
                                fig.zlim = None;
                            }
                            // equal/square/off/on/fill/image/vis3d/manual:
                            // accepted; no visible effect (documented gap).
                            _ => {}
                        }
                        Ok(None)
                    }
                    Some(val) => {
                        let m = val.to_matrix()?;
                        let d = m.data();
                        if d.len() != 4 && d.len() != 6 {
                            return Err(LangError::new("axis needs [x0 x1 y0 y1]", 0));
                        }
                        let fig = self.figure_mut();
                        fig.xlim = Some((d[0], d[1]));
                        fig.ylim = Some((d[2], d[3]));
                        if d.len() == 6 {
                            fig.zlim = Some((d[4], d[5]));
                        }
                        Ok(None)
                    }
                }
            }
            "box" => {
                let v = self.eval_args(args)?;
                let fig = self.figure_mut();
                fig.boxed = match v.first() {
                    Some(Value::Str(s)) => s != "off",
                    _ => !fig.boxed,
                };
                Ok(None)
            }
            "zlabel" => {
                let v = self.eval_args(args)?;
                let s = match v.first() {
                    Some(Value::Str(s)) => s.clone(),
                    Some(val) => fmt_value(val, self.fmt_long).to_string(),
                    None => return Err(LangError::new("zlabel needs a string", 0)),
                };
                self.figure_mut().zlabel = Some(s);
                Ok(None)
            }
            "view" => {
                let v = self.eval_args(args)?;
                let fig = self.figure_mut();
                match v.len() {
                    0 => Ok(Some(Value::Mat(Matrix::row_vector(vec![fig.view.0, fig.view.1])))),
                    1 => {
                        // view(2)/view(3) presets or a two-element [az el].
                        let m = v[0].to_matrix()?;
                        let d = m.data();
                        if d.len() == 2 {
                            fig.view = (d[0], d[1]);
                        } else {
                            let k = d.first().copied().unwrap_or(3.0);
                            fig.view = if (k - 2.0).abs() < f64::EPSILON { (0.0, 90.0) } else { (-37.5, 30.0) };
                        }
                        Ok(None)
                    }
                    _ => {
                        fig.view = (v[0].scalar()?, v[1].scalar()?);
                        Ok(None)
                    }
                }
            }
            "colormap" => {
                let v = self.eval_args(args)?;
                if let Some(Value::Str(s)) = v.first() {
                    let cm = matplot::Colormap::named(s).ok_or_else(|| LangError::new(format!("unknown colormap '{s}'"), 0))?;
                    self.figure_mut().cmap = cm;
                    return Ok(None);
                }
                Ok(Some(Value::Str(self.figure_mut().cmap.name().to_string())))
            }
            "colorbar" => {
                let v = self.eval_args(args)?;
                let fig = self.figure_mut();
                fig.colorbar = match v.first() {
                    Some(Value::Str(s)) => s != "off",
                    _ => !fig.colorbar,
                };
                Ok(None)
            }
            "caxis" | "clim" => {
                let v = self.eval_args(args)?;
                if v.is_empty() {
                    let (a, b) = self.figure_mut().clim.unwrap_or((f64::NAN, f64::NAN));
                    return Ok(Some(Value::Mat(Matrix::row_vector(vec![a, b]))));
                }
                let m = v[0].to_matrix()?;
                let d = m.data();
                if d.len() != 2 {
                    return Err(LangError::new(format!("{name} needs [lo hi]"), 0));
                }
                self.figure_mut().clim = Some((d[0], d[1]));
                Ok(None)
            }
            "imagesc" | "image" => {
                // imagesc(C) | imagesc(x, y, C) — x/y are the edge-extending
                // column/row coordinates of the image's first/last cells.
                let v = self.eval_args(args)?;
                let (xv, yv, cm) = match v.len() {
                    1 => {
                        let m = v[0].to_matrix()?;
                        let x: Vec<f64> = (1..=m.cols()).map(|k| k as f64).collect();
                        let y: Vec<f64> = (1..=m.rows()).map(|k| k as f64).collect();
                        (x, y, m)
                    }
                    3 => {
                        let (xm, ym, cm) = (v[0].to_matrix()?, v[1].to_matrix()?, v[2].to_matrix()?);
                        (grid_coords(&xm, cm.cols()), grid_coords(&ym, cm.rows()), cm)
                    }
                    _ => return Err(LangError::new(format!("{name} needs (C) or (x,y,C)"), 0)),
                };
                let (rows, cols) = cm.shape();
                check_elems(rows.saturating_mul(cols))?;
                if !self.figure_mut().hold {
                    self.figure_mut().series.clear();
                }
                {
                    let fig = self.figure_mut();
                    fig.yrev = true;
                    fig.push_grid(SeriesKind::Image, xv, yv, cm.data().to_vec(), rows, cols, Vec::new(), matplot::LineSpec::new());
                }
                Ok(None)
            }
            "pie" => {
                // pie(x) | pie(x, labels) | pie(x, explode)
                let v = self.eval_args(args)?;
                let m = v.first().ok_or_else(|| LangError::new("pie needs data", 0))?.to_matrix()?;
                let mut labels = Vec::new();
                if let Some(second) = v.get(1) {
                    match second {
                        Value::Cell(c) => {
                            for e in &c.items {
                                labels.push(match e {
                                    Value::Str(s) => s.clone(),
                                    other => fmt_value(other, false).to_string(),
                                });
                            }
                        }
                        Value::Str(s) => labels.push(s.clone()),
                        _ => {}
                    }
                }
                while labels.len() < m.numel() {
                    labels.push(format!("{}", labels.len() + 1));
                }
                if !self.figure_mut().hold {
                    self.figure_mut().series.clear();
                }
                self.figure_mut().push_pie(m.data().to_vec(), labels);
                Ok(None)
            }
            "contour" => {
                // contour(Z[,n|v]) | contour(X,Y,Z[,n|v])
                let v = self.eval_args(args)?;
                let (xv, yv, zm, spec_arg): (Vec<f64>, Vec<f64>, Matrix, Option<&Value>) = match v.len() {
                    1 | 2 => {
                        let m = v[0].to_matrix()?;
                        let x: Vec<f64> = (1..=m.cols()).map(|k| k as f64).collect();
                        let y: Vec<f64> = (1..=m.rows()).map(|k| k as f64).collect();
                        (x, y, m, v.get(1))
                    }
                    3 | 4 => {
                        let (xm, ym, zm) = (v[0].to_matrix()?, v[1].to_matrix()?, v[2].to_matrix()?);
                        // Accept meshgrid matrices or coordinate vectors.
                        let x: Vec<f64> =
                            if xm.numel() == zm.cols() { xm.data().to_vec() } else { (0..zm.cols()).map(|c| xm.get(0, c).unwrap_or(0.0)).collect() };
                        let y: Vec<f64> =
                            if ym.numel() == zm.rows() { ym.data().to_vec() } else { (0..zm.rows()).map(|r| ym.get(r, 0).unwrap_or(0.0)).collect() };
                        (x, y, zm, v.get(3))
                    }
                    _ => return Err(LangError::new("contour needs (Z), (Z,n|v) or (X,Y,Z[,n|v])", 0)),
                };
                let (rows, cols) = zm.shape();
                check_elems(rows.saturating_mul(cols))?;
                let levels = match spec_arg {
                    Some(val) => {
                        let lm = val.to_matrix()?;
                        if lm.numel() == 1 {
                            // Scalar n → n evenly spaced levels across the data range.
                            let n = lm.get_linear(0).unwrap_or(1.0).max(1.0) as usize;
                            let (lo, hi) =
                                zm.data()
                                    .iter()
                                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &v| if v.is_finite() { (a.min(v), b.max(v)) } else { (a, b) });
                            if hi > lo {
                                (1..=n.min(50)).map(|k| lo + (hi - lo) * k as f64 / (n as f64 + 1.0)).collect()
                            } else {
                                Vec::new()
                            }
                        } else {
                            lm.data().to_vec()
                        }
                    }
                    None => Vec::new(),
                };
                if !self.figure_mut().hold {
                    self.figure_mut().series.clear();
                }
                self.figure_mut().push_grid(SeriesKind::Contour, xv, yv, zm.data().to_vec(), rows, cols, levels, matplot::LineSpec::new());
                Ok(None)
            }
            "plot3" => {
                // plot3(x,y,z[,spec]) per-triplet, like add_series.
                let v = self.eval_args(args)?;
                if !self.figure_mut().hold {
                    self.figure_mut().series.clear();
                }
                let mut i = 0;
                let mut added = false;
                while i + 2 < v.len() {
                    let (x, y, z) = (v[i].to_matrix()?, v[i + 1].to_matrix()?, v[i + 2].to_matrix()?);
                    if x.numel() != y.numel() || y.numel() != z.numel() {
                        return Err(LangError::new("plot3 vectors must be the same length", 0));
                    }
                    let spec = match v.get(i + 3) {
                        Some(Value::Str(s)) => {
                            i += 1;
                            matplot::LineSpec::parse(s)
                        }
                        _ => matplot::LineSpec::new(),
                    };
                    self.figure_mut().push_xyz(x.data().to_vec(), y.data().to_vec(), z.data().to_vec(), spec);
                    i += 3;
                    added = true;
                }
                if !added {
                    return Err(LangError::new("plot3 needs (x, y, z) vectors", 0));
                }
                Ok(None)
            }
            "surf" | "mesh" => {
                // surf(Z) | surf(X,Y,Z) — X/Y vectors or meshgrid matrices.
                let kind = if name == "surf" { SeriesKind::Surf } else { SeriesKind::Mesh };
                let v = self.eval_args(args)?;
                let zm = match v.len() {
                    1 => v[0].to_matrix()?,
                    3 => v[2].to_matrix()?,
                    _ => return Err(LangError::new(format!("{name} needs (Z) or (X,Y,Z)"), 0)),
                };
                let (rows, cols) = zm.shape();
                check_elems(rows.saturating_mul(cols))?;
                if rows < 2 || cols < 2 {
                    return Err(LangError::new(format!("{name} needs a matrix with at least 2 rows and columns"), 0));
                }
                // Per-vertex x/y, column-major to match z.
                let mut xf = vec![0.0; rows * cols];
                let mut yf = vec![0.0; rows * cols];
                if v.len() == 3 {
                    let (xm, ym) = (v[0].to_matrix()?, v[1].to_matrix()?);
                    if xm.numel() == rows * cols && ym.numel() == rows * cols {
                        xf = xm.data().to_vec();
                        yf = ym.data().to_vec();
                    } else if xm.numel() == cols && ym.numel() == rows {
                        for c in 0..cols {
                            for r in 0..rows {
                                xf[c * rows + r] = xm.get_linear(c).unwrap_or(0.0);
                                yf[c * rows + r] = ym.get_linear(r).unwrap_or(0.0);
                            }
                        }
                    } else {
                        return Err(LangError::new(format!("{name}: X/Y must be vectors matching Z's size or meshgrid matrices"), 0));
                    }
                } else {
                    for c in 0..cols {
                        for r in 0..rows {
                            xf[c * rows + r] = (c + 1) as f64;
                            yf[c * rows + r] = (r + 1) as f64;
                        }
                    }
                }
                if !self.figure_mut().hold {
                    self.figure_mut().series.clear();
                }
                self.figure_mut().push_grid(kind, xf, yf, zm.data().to_vec(), rows, cols, Vec::new(), matplot::LineSpec::new());
                Ok(None)
            }
            "saveas" | "exportgraphics" => {
                let v = self.eval_args(args)?;
                // saveas(file) | saveas(figNo, file) | exportgraphics(gcf, file)
                let file = match v.iter().rev().find_map(|val| match val {
                    Value::Str(s) => Some(s.clone()),
                    _ => None,
                }) {
                    Some(f) => f,
                    None => return Err(LangError::new(format!("{name} needs a filename"), 0)),
                };
                let fig = self.figure_mut();
                let bytes = export_figure(fig, &file, None)?;
                std::fs::write(&file, &bytes).map_err(|e| LangError::new(format!("cannot write {file}: {e}"), 0))?;
                self.print_buf.push_str(&format!("wrote {file} ({} bytes)\n", bytes.len()));
                Ok(None)
            }
            "print" => {
                let v = self.eval_args(args)?;
                // print('-dpng', 'file') — the -d flag picks the format,
                // else the file extension does (svg default).
                let mut fmt: Option<String> = None;
                let mut file: Option<String> = None;
                for val in &v {
                    if let Value::Str(s) = val {
                        if let Some(f) = s.strip_prefix("-d") {
                            fmt = Some(f.to_string());
                        } else if !s.starts_with('-') {
                            file = Some(s.clone());
                        }
                    }
                }
                let file = file.ok_or_else(|| LangError::new("print needs a filename", 0))?;
                let bytes = export_figure(self.figure_mut(), &file, fmt.as_deref())?;
                std::fs::write(&file, &bytes).map_err(|e| LangError::new(format!("cannot write {file}: {e}"), 0))?;
                self.print_buf.push_str(&format!("wrote {file} ({} bytes)\n", bytes.len()));
                Ok(None)
            }
            _ => Err(LangError::new(format!("unknown function or variable '{name}'"), 0)),
        }
    }

    /// Parse `plot`-family arguments into series on the current figure:
    /// `(y)`, `(x, y)`, `(x, y, spec)`, and multi-triplets; matrix `y`
    /// expands to one series per column.
    fn add_series(&mut self, kind: matplot::SeriesKind, args: &[Expr]) -> Result<Option<Value>, LangError> {
        let v = self.eval_args(args)?;
        if !self.figure_mut().hold {
            self.figure_mut().series.clear();
        }
        // `plot(z)` on a single complex vector plots re vs im (documented
        // M-code behaviour) — unwrap that pair before the real path.
        let mut v = v;
        if v.len() == 1 {
            if let Some(Value::Cplx(cm)) = v.first() {
                v = vec![Value::Mat(cm.re()), Value::Mat(cm.im())];
            }
        }
        let mut i = 0;
        let mut added = 0;
        while i < v.len() {
            let first = v.get(i).ok_or_else(|| LangError::new("plot needs data", 0))?;
            let first_m = match first {
                Value::Cplx(cm) => cm.re(),
                other => other.to_matrix()?,
            };
            let (x, y) = if i + 1 < v.len() && matches!(v.get(i + 1), Some(Value::Mat(_)) | Some(Value::Num(_)) | Some(Value::Cplx(_))) {
                let ym = match v.get(i + 1) {
                    Some(Value::Cplx(cm)) => cm.re(),
                    Some(o) => o.to_matrix()?,
                    None => return Err(LangError::new("plot needs data", 0)),
                };
                i += 2;
                (first_m, ym)
            } else {
                // plot(y): x defaults to 1..numel for a vector, 1..rows for
                // a matrix (columns are the series).
                let (r, _) = first_m.shape();
                let n = if r == 1 { first_m.numel() } else { r };
                let xs: Vec<f64> = (1..=n).map(|k| k as f64).collect();
                i += 1;
                (Matrix::row_vector(xs), first_m)
            };
            let spec = match v.get(i) {
                Some(Value::Str(s)) => {
                    i += 1;
                    matplot::LineSpec::parse(s)
                }
                _ => matplot::LineSpec::new(),
            };
            let (yr, yc) = y.shape();
            let yd = y.data();
            let xs: Vec<f64> = x.data().to_vec();
            if x.numel() == yr {
                // length(x) == rows(y): each column is a series (the
                // vector case is the yc==1 specialisation).
                for c in 0..yc {
                    let ys: Vec<f64> = yd[c * yr..c * yr + yr].to_vec();
                    self.figure_mut().push(kind, xs.clone(), ys, spec.clone());
                    added += 1;
                }
            } else if x.numel() == yc {
                // length(x) == columns(y): each row is a series (covers the
                // common row-vector plot(x_row, y_row) as yr==1).
                for r in 0..yr {
                    let ys: Vec<f64> = (0..yc).map(|c| yd[c * yr + r]).collect();
                    self.figure_mut().push(kind, xs.clone(), ys, spec.clone());
                    added += 1;
                }
            } else {
                return Err(LangError::new(format!("plot x length {} doesn't match y dims {yr}x{yc}", x.numel()), 0));
            }
        }
        if added == 0 {
            return Err(LangError::new("plot needs data", 0));
        }
        Ok(None)
    }

    fn eval_args(&mut self, args: &[Expr]) -> Result<Vec<Value>, LangError> {
        let mut out = Vec::with_capacity(args.len());
        for a in args {
            // Comma-separated lists expand: `f(c{:})` passes each cell
            // content as a separate argument.
            out.extend(self.eval_multi(a)?);
        }
        Ok(out)
    }

    /// True when `name` resolves to a function — already defined or loadable
    /// from a `<name>.m` file on the search path.
    fn is_function(&mut self, name: &str) -> Result<bool, LangError> {
        if self.funcs.contains_key(name) {
            return Ok(true);
        }
        self.load_function_file(name)
    }

    /// Look for `<name>.m` on the search path; if found, hoist every
    /// `function` in it and report whether `name` is now defined.
    fn load_function_file(&mut self, name: &str) -> Result<bool, LangError> {
        for dir in self.paths.clone() {
            let file = dir.join(format!("{name}.m"));
            if !file.is_file() {
                continue;
            }
            let src = std::fs::read_to_string(&file).map_err(|e| LangError::new(format!("cannot read {}: {e}", file.display()), 0))?;
            let stmts = crate::parser::parse(&src).map_err(|mut e| {
                e.message = format!("{}: {}", file.display(), e.message);
                e
            })?;
            self.hoist_functions(&stmts);
            return Ok(self.funcs.contains_key(name));
        }
        Ok(false)
    }

    /// Call a registered function: evaluate args, run its body in a fresh
    /// local scope, and return the bound outputs.
    fn call_function(&mut self, name: &str, arg_exprs: &[Expr], nargout: usize) -> Result<Vec<Value>, LangError> {
        let argv = self.eval_args(arg_exprs)?;
        self.call_function_argv(name, argv, nargout)
    }

    /// `call_function` with pre-evaluated argument values — used by
    /// `feval`/function-handle dispatch where args arrive as `Value`s.
    fn call_function_argv(&mut self, name: &str, argv: Vec<Value>, nargout: usize) -> Result<Vec<Value>, LangError> {
        let def = self.funcs.get(name).cloned().ok_or_else(|| LangError::new(format!("unknown function '{name}'"), 0))?;
        if self.call_depth >= MAX_CALL_DEPTH {
            return Err(LangError::new("call depth limit exceeded (runaway recursion?)", 0));
        }
        self.call_depth += 1;
        // On early return the depth must still be restored — route through
        // a scope guard pattern: compute, then decrement before `?`.
        let result = self.call_function_argv_inner(name, &def, argv, nargout);
        self.call_depth = self.call_depth.saturating_sub(1);
        result
    }

    fn call_function_argv_inner(&mut self, name: &str, def: &FuncDef, argv: Vec<Value>, nargout: usize) -> Result<Vec<Value>, LangError> {
        // Fresh local scope: caller workspace is fully swapped out.
        let saved_vars = std::mem::take(&mut self.vars);
        let saved_ctx = self.call_ctx.replace((argv.len(), nargout));
        let vararg = def.params.last().map(|p| p == "varargin").unwrap_or(false);
        let fixed = if vararg { def.params.len().saturating_sub(1) } else { def.params.len() };
        for (i, p) in def.params.iter().take(fixed).enumerate() {
            if p == "~" {
                continue; // ignored input slot — consumes argv[i], binds nothing
            }
            let v = argv.get(i).cloned().unwrap_or_else(|| Value::Mat(Matrix::zeros(0, 0)));
            self.vars.insert(p.clone(), v);
        }
        if vararg {
            // `varargin` is a 1×n cell of the extra arguments.
            let extras: Vec<Value> = argv.iter().skip(fixed).cloned().collect();
            self.vars.insert("varargin".to_string(), Value::Cell(CellArray::row(extras)));
        }

        // Function-body echoes (unsuppressed assigns, disp) accumulate in the
        // shared print buffer, same as script output.
        let mut fbuf = std::mem::take(&mut self.print_buf);
        let result: Result<Vec<Value>, LangError> = (|| {
            let flow = self.exec_block(&def.body, &mut fbuf)?;
            match flow {
                Flow::Normal | Flow::Return => {}
                Flow::Break | Flow::Continue => return Err(LangError::new("break/continue outside a loop", 0)),
            }
            // A trailing `varargout` output expands its cells into the
            // remaining output slots.
            let vargout = def.outputs.last().map(|o| o == "varargout").unwrap_or(false);
            let fixed_out = if vargout { def.outputs.len().saturating_sub(1) } else { def.outputs.len() };
            let mut outs: Vec<Value> = Vec::with_capacity(nargout.max(fixed_out));
            for o in def.outputs.iter().take(fixed_out) {
                // `[~, b] = f(…)` — an ignored output never needs assignment;
                // the slot exists so positions line up.
                if o == "~" {
                    outs.push(Value::Mat(Matrix::zeros(0, 0)));
                    continue;
                }
                outs.push(self.vars.get(o).cloned().ok_or_else(|| LangError::new(format!("output '{o}' of '{name}' was never assigned"), 0))?);
            }
            if vargout && outs.len() < nargout {
                if let Some(v) = self.vars.get("varargout") {
                    match v {
                        Value::Cell(ca) => {
                            for i in outs.len()..nargout {
                                outs.push(ca.get_linear(i).cloned().unwrap_or_else(CellArray::empty));
                            }
                        }
                        _ => {
                            let m = v.to_matrix()?;
                            for i in outs.len()..nargout {
                                outs.push(Value::Num(m.get_linear(i).unwrap_or(0.0)));
                            }
                        }
                    }
                }
            }
            Ok(outs)
        })();

        fbuf.push_str(&self.print_buf);
        self.print_buf = fbuf;
        self.vars = saved_vars;
        self.call_ctx = saved_ctx;
        result
    }

    /// Call a function handle with evaluated argument values; returns up to
    /// `nout` outputs (an anonymous function always has exactly one).
    fn call_handle(&mut self, h: &FuncHandle, argv: Vec<Value>, nout: usize) -> Result<Vec<Value>, LangError> {
        match h {
            FuncHandle::Named(name) => self.call_named(name, argv, nout),
            FuncHandle::Closure { params, body, env } => {
                if self.call_depth >= MAX_CALL_DEPTH {
                    return Err(LangError::new("call depth limit exceeded (runaway recursion?)", 0));
                }
                self.call_depth += 1;
                // Child scope: the captured workspace plus bound params —
                // the caller's variables are swapped out like a `function`
                // call. Params shadow same-named captured variables.
                let saved_vars = std::mem::replace(&mut self.vars, env.clone());
                let saved_ctx = self.call_ctx.replace((argv.len(), 1));
                for (i, p) in params.iter().enumerate() {
                    let v = argv.get(i).cloned().unwrap_or_else(|| Value::Mat(Matrix::zeros(0, 0)));
                    self.vars.insert(p.clone(), v);
                }
                let mut fbuf = std::mem::take(&mut self.print_buf);
                let result = self.eval(body).map(|v| vec![v]);
                fbuf.push_str(&self.print_buf);
                self.print_buf = fbuf;
                self.vars = saved_vars;
                self.call_ctx = saved_ctx;
                self.call_depth = self.call_depth.saturating_sub(1);
                result
            }
        }
    }

    /// Call `name` (user function or builtin) with evaluated arg values.
    fn call_named(&mut self, name: &str, argv: Vec<Value>, nout: usize) -> Result<Vec<Value>, LangError> {
        if self.is_function(name)? {
            return self.call_function_argv(name, argv, nout);
        }
        if self.is_builtin(name) {
            return self.builtin_values(name, argv, nout);
        }
        Err(LangError::new(format!("'{name}' is not a function"), 0))
    }

    /// `feval`-style dispatch on a handle *or* a function-name char —
    /// MATLAB's `feval` accepts both.
    fn call_feval_target(&mut self, target: &Value, argv: Vec<Value>, nout: usize) -> Result<Vec<Value>, LangError> {
        match target {
            Value::Func(h) => self.call_handle(h, argv, nout),
            Value::Str(s) => self.call_named(&s.clone(), argv, nout),
            _ => Err(LangError::new("first argument must be a function handle or a function name", 0)),
        }
    }

    /// Run a builtin whose arguments are already evaluated `Value`s. Each
    /// value is bound to a hidden temporary name (unspellable in source —
    /// it contains a control char) so the expr-driven builtin table can
    /// consume it; the bindings are restored afterwards.
    fn builtin_values(&mut self, name: &str, argv: Vec<Value>, nout: usize) -> Result<Vec<Value>, LangError> {
        let mut exprs = Vec::with_capacity(argv.len());
        let mut saved: Vec<(String, Option<Value>)> = Vec::with_capacity(argv.len());
        for (i, v) in argv.into_iter().enumerate() {
            let tmp = format!("\u{1}harg{i}");
            saved.push((tmp.clone(), self.vars.insert(tmp.clone(), v)));
            exprs.push(Expr::Ident(tmp));
        }
        // No `?` here — the temporaries must be restored even on error.
        let out = if nout > 1 {
            match self.builtin_multi(name, &exprs, nout) {
                Ok(Some(outs)) => Ok(outs),
                Ok(None) => self.builtin(name, &exprs).map(|o| o.into_iter().collect()),
                Err(e) => Err(e),
            }
        } else {
            self.builtin(name, &exprs).map(|o| o.into_iter().collect())
        };
        for (tmp, old) in saved {
            match old {
                Some(v) => {
                    self.vars.insert(tmp, v);
                }
                None => {
                    self.vars.remove(&tmp);
                }
            }
        }
        out
    }

    /// The `i`-th call argument as a matrix. Args go through `eval_args`
    /// so comma-separated lists (`f(c{:})`) expand before indexing —
    /// the position counts the flattened list, as M-code defines.
    fn arg_matrix(&mut self, args: &[Expr], i: usize) -> Result<Matrix, LangError> {
        let v = self.eval_args(args)?;
        v.get(i).ok_or_else(|| LangError::new("missing argument", 0))?.to_matrix()
    }

    /// Multi-output builtins for `[a, b, …] = f(x)`: `size`, `min`/`max`
    /// (value + argmin/argmax), `find` (row/col), `eig`, `svd`, `qr`, `lu`,
    /// `meshgrid`, `histcounts`. Returns `None` when `name` has no
    /// multi-output form (caller reports "not a function").
    fn builtin_multi(&mut self, name: &str, args: &[Expr], nout: usize) -> Result<Option<Vec<Value>>, LangError> {
        let out = match name {
            "feval" => {
                // `[a,b] = feval(@f, x)` — multi-output call through a handle.
                let v = self.eval_args(args)?;
                match v.split_first() {
                    Some((t, r)) => self.call_feval_target(t, r.to_vec(), nout)?,
                    None => return Err(LangError::new("feval needs a function handle or name", 0)),
                }
            }
            "size" => {
                let v = self.eval_args(args)?;
                let (r, c) = v.first().map(Value::dims).unwrap_or((0, 0));
                vec![Value::Num(r as f64), Value::Num(c as f64)]
            }
            "min" | "max" => {
                let m = self.arg_matrix(args, 0)?;
                let hi = name == "max";
                let dim = self.dim_arg(args, m.shape())?;
                let (vals, idxs) = reduce_with_index(&m, dim, hi);
                vec![mat_or_num(vals), mat_or_num(idxs)]
            }
            "find" => {
                let v = self.eval_args(args)?;
                if let Some(Value::Sparse(s)) = v.first() {
                    let (mut rows, mut cols, mut vals) = (Vec::new(), Vec::new(), Vec::new());
                    for ((r, c), x) in s.entries() {
                        rows.push((r + 1) as f64);
                        cols.push((c + 1) as f64);
                        vals.push(x);
                    }
                    let mut out = vec![mat_or_num(Matrix::col_vector(rows)), mat_or_num(Matrix::col_vector(cols))];
                    if nout >= 3 {
                        out.push(mat_or_num(Matrix::col_vector(vals)));
                    }
                    return Ok(Some(out));
                }
                let m = self.arg_matrix(args, 0)?;
                let mut rows = Vec::new();
                let mut cols = Vec::new();
                for c in 0..m.cols() {
                    for r in 0..m.rows() {
                        if m.get(r, c).unwrap_or(0.0) != 0.0 {
                            rows.push((r + 1) as f64);
                            cols.push((c + 1) as f64);
                        }
                    }
                }
                vec![mat_or_num(Matrix::col_vector(rows)), mat_or_num(Matrix::col_vector(cols))]
            }
            "deal" => {
                // [a,b] = deal(x,y,…) maps inputs to outputs; deal(x)
                // replicates one input across all outputs. Comma-separated
                // args (deal(s.f)) arrive already expanded.
                let v = self.eval_args(args)?;
                if v.is_empty() {
                    return Err(LangError::new("deal needs at least one input", 0));
                }
                if v.len() == 1 {
                    return Ok(Some(vec![v.first().cloned().unwrap_or(Value::Num(0.0)); nout.max(1)]));
                }
                let mut out = v;
                out.truncate(nout.max(1));
                while out.len() < nout {
                    out.push(out.last().cloned().unwrap_or(Value::Num(0.0)));
                }
                out
            }
            "meshgrid" => {
                let v = self.eval_args(args)?;
                let x = v.first().ok_or_else(|| LangError::new("meshgrid needs x", 0))?.to_matrix()?;
                let y = v.get(1).map(|t| t.to_matrix()).transpose()?.unwrap_or_else(|| x.clone());
                let (nx, ny) = (x.numel(), y.numel());
                check_elems(nx.saturating_mul(ny))?;
                let mut gx = Matrix::zeros(ny, nx);
                let mut gy = Matrix::zeros(ny, nx);
                for r in 0..ny {
                    for c in 0..nx {
                        let _ = gx.set(r, c, x.get_linear(c).unwrap_or(0.0));
                        let _ = gy.set(r, c, y.get_linear(r).unwrap_or(0.0));
                    }
                }
                vec![Value::Mat(gx), Value::Mat(gy)]
            }
            "eig" => {
                let m = self.arg_matrix(args, 0)?;
                if nout < 2 {
                    let w = matcore::eigvals_c(&m).map_err(lang)?;
                    let cm = CMatrix::from_col_major(w.len(), 1, w).map_err(lang)?;
                    return Ok(Some(vec![cmat_or_value(cm)]));
                }
                // Symmetric fast path (Jacobi gives eigenvectors too).
                if let Ok((w, v)) = matcore::eig_symmetric(&m) {
                    let mut d = Matrix::zeros(w.len(), w.len());
                    for (i, x) in w.iter().enumerate() {
                        let _ = d.set(i, i, *x);
                    }
                    return Ok(Some(vec![Value::Mat(v), Value::Mat(d)]));
                }
                // General real spectrum: eigenvalues by Hessenberg+QR, then
                // each eigenvector as null(A - λI). Complex spectra need
                // complex eigenvector solves — not implemented yet.
                let wc = matcore::eigvals_c(&m).map_err(lang)?;
                let w: Vec<f64> = wc
                    .iter()
                    .map(|z| {
                        if z.is_real() {
                            Ok(z.re)
                        } else {
                            Err(LangError::new("[V,D] = eig(A) for a complex spectrum is not implemented yet; use eig(A)", 0))
                        }
                    })
                    .collect::<Result<_, _>>()?;
                let n = m.rows();
                let mut v = Matrix::zeros(n, n);
                let mut d = Matrix::zeros(n, n);
                for (i, &lam) in w.iter().enumerate() {
                    let shifted = m.elemwise(&Matrix::eye(n, n), |a, b| a - b * lam).map_err(lang)?;
                    let s = matcore::svd(&shifted).map_err(lang)?;
                    // Last column of V = smallest singular direction.
                    let k = s.v.cols().saturating_sub(1);
                    for r in 0..n {
                        let _ = v.set(r, i, s.v.get(r, k).unwrap_or(0.0));
                    }
                    let _ = d.set(i, i, lam);
                }
                vec![Value::Mat(v), Value::Mat(d)]
            }
            "svd" => {
                let m = self.arg_matrix(args, 0)?;
                let s = matcore::svd(&m).map_err(lang)?;
                if nout >= 3 {
                    vec![Value::Mat(s.u), Value::Mat(s.s), Value::Mat(s.v)]
                } else {
                    let mut sv = Vec::new();
                    for i in 0..s.s.rows().min(s.s.cols()) {
                        sv.push(s.s.get(i, i).unwrap_or(0.0));
                    }
                    vec![mat_or_num(Matrix::col_vector(sv))]
                }
            }
            "qr" => {
                let m = self.arg_matrix(args, 0)?;
                let (q, r) = matcore::qr(&m).map_err(lang)?;
                vec![Value::Mat(q), Value::Mat(r)]
            }
            "lu" => {
                let m = self.arg_matrix(args, 0)?;
                let (l, u, p) = matcore::lu(&m).map_err(lang)?;
                vec![Value::Mat(l), Value::Mat(u), Value::Mat(p)]
            }
            "histcounts" => {
                let v = self.eval_args(args)?;
                let m = v.first().ok_or_else(|| LangError::new("histcounts needs data", 0))?.to_matrix()?;
                let nb = if v.len() > 1 { v.scalar_at(1)? as usize } else { 10 };
                check_elems(nb)?;
                let (counts, edges) = histcounts(&m, nb);
                vec![Value::Mat(Matrix::row_vector(counts)), Value::Mat(Matrix::row_vector(edges))]
            }
            "filter" => {
                let (y, zf) = self.dsp_filter(args)?;
                vec![cmat_or_value(y), cmat_or_value(zf)]
            }
            "xcorr" => {
                let (r, lags) = self.dsp_xcorr(args)?;
                vec![cmat_or_value(r), Value::Mat(lags)]
            }
            "freqz" => {
                let (h, w) = self.dsp_freqz(args)?;
                vec![cmat_or_value(h), Value::Mat(w)]
            }
            _ => return Ok(None),
        };
        Ok(Some(out))
    }

    /// `fft`/`ifft`/`fft2`/`ifft2` — column-wise over matrices, whole-vector
    /// for vectors (row vectors stay rows); `n` pads or truncates.
    fn dsp_fft(&mut self, args: &[Expr], two_d: bool, inverse: bool) -> Result<Value, LangError> {
        let v = self.eval_args(args)?;
        let cm = v.first().ok_or_else(|| LangError::new("fft needs an input", 0))?.to_cmatrix()?;
        let t = if inverse { matcore::dsp::ifft } else { matcore::dsp::fft };
        if two_d {
            // fft2(A[,m,n]) — transform each column then each row of the
            // (optionally resized) m×n matrix.
            let mr = v.scalar_at(1).unwrap_or(cm.rows() as f64);
            let nc = v.scalar_at(2).unwrap_or(cm.cols() as f64);
            if mr < 0.0 || nc < 0.0 || mr.fract() != 0.0 || nc.fract() != 0.0 {
                return Err(LangError::new("fft2 size arguments must be nonnegative integers", 0));
            }
            let (mr, nc) = (mr as usize, nc as usize);
            check_elems(mr.saturating_mul(nc))?;
            let mut resized = CMatrix::zeros(mr, nc);
            for c in 0..nc.min(cm.cols()) {
                for r in 0..mr.min(cm.rows()) {
                    let _ = resized.set(r, c, cm.get(r, c).unwrap_or(CNum::real(0.0)));
                }
            }
            let cols = fft_cols(&resized, mr, t)?;
            let rows = fft_cols(&cols.transpose(), nc, t)?;
            return Ok(cmat_or_value(rows.transpose()));
        }
        let rowvec = cm.rows() == 1 && cm.cols() > 1;
        let src = if rowvec { cm.transpose() } else { cm };
        let n = v.scalar_at(1).unwrap_or(src.rows() as f64);
        if n < 0.0 || n.fract() != 0.0 {
            return Err(LangError::new("fft length must be a nonnegative integer", 0));
        }
        let out = fft_cols(&src, n as usize, t)?;
        Ok(cmat_or_value(if rowvec { out.transpose() } else { out }))
    }

    /// `filter(b,a,x[,zi])` → (y, zf); column-wise for matrix `x`, the whole
    /// sequence for vectors.
    fn dsp_filter(&mut self, args: &[Expr]) -> Result<(CMatrix, CMatrix), LangError> {
        let v = self.eval_args(args)?;
        let b = v.first().ok_or_else(|| LangError::new("filter needs b", 0))?.to_cmatrix()?;
        let a = v.get(1).map(|x| x.to_cmatrix()).transpose()?.unwrap_or_else(|| CMatrix::scalar(CNum::real(1.0)));
        let x = v.get(2).ok_or_else(|| LangError::new("filter needs x", 0))?.to_cmatrix()?;
        if a.data().first().map(|z| z.re == 0.0 && z.im == 0.0).unwrap_or(true) {
            return Err(LangError::new("filter: a(1) must be nonzero", 0));
        }
        let zi = v.get(3).map(|x| x.to_cmatrix()).transpose()?.unwrap_or_else(|| CMatrix::zeros(0, 0));
        let rowvec = x.rows() == 1 && x.cols() > 1;
        let src = if rowvec { x.transpose() } else { x };
        let order = a.numel().max(b.numel()).saturating_sub(1);
        let mut ycols = Vec::with_capacity(src.numel());
        let mut zf = CMatrix::zeros(order, src.cols());
        for c in 0..src.cols() {
            let col: Vec<CNum> = (0..src.rows()).map(|r| src.get(r, c).unwrap_or(CNum::real(0.0))).collect();
            // zi as a per-column matrix picks column c; a vector broadcasts.
            let zic: Vec<CNum> = if zi.cols() == src.cols() && zi.cols() > 1 {
                (0..order).map(|i| zi.get(i, c).unwrap_or(CNum::real(0.0))).collect()
            } else {
                (0..order).map(|i| zi.get_linear(i).unwrap_or(CNum::real(0.0))).collect()
            };
            let (yc, fc) = matcore::dsp::filter_df2t(b.data(), a.data(), &col, &zic);
            ycols.extend(yc);
            for r in 0..order {
                let _ = zf.set(r, c, fc.get(r).copied().unwrap_or(CNum::real(0.0)));
            }
        }
        let y = CMatrix::from_col_major(src.rows(), src.cols(), ycols).map_err(lang)?;
        Ok((if rowvec { y.transpose() } else { y }, zf))
    }

    /// `xcorr(x[,y][,maxlag][,'scale'])` → (r, lags).
    fn dsp_xcorr(&mut self, args: &[Expr]) -> Result<(CMatrix, Matrix), LangError> {
        let v = self.eval_args(args)?;
        let x = v.first().ok_or_else(|| LangError::new("xcorr needs an input", 0))?.to_cmatrix()?;
        let y = match v.get(1) {
            Some(t) if !matches!(t, Value::Str(_)) => t.to_cmatrix()?.data().to_vec(),
            _ => x.data().to_vec(),
        };
        let mut maxlag = None;
        let mut norm = 0u8;
        for a in v.iter().skip(2) {
            match a {
                Value::Str(_) => {}
                other => {
                    let m = other.to_matrix()?;
                    if m.numel() != 1 || m.as_scalar().unwrap_or(-1.0) < 0.0 || m.as_scalar().unwrap_or(0.0).fract() != 0.0 {
                        return Err(LangError::new("xcorr: maxlag must be a nonnegative integer", 0));
                    }
                    maxlag = Some(m.as_scalar().unwrap_or(0.0) as usize);
                }
            }
        }
        for a in v.iter().skip(1) {
            if let Value::Str(s) = a {
                norm = match s.trim() {
                    "biased" => 1,
                    "unbiased" => 2,
                    "coeff" | "normalized" => 3,
                    "none" => 0,
                    other => return Err(LangError::new(format!("xcorr: unknown scale option '{other}'"), 0)),
                };
            }
        }
        let (r, lags) = matcore::dsp::xcorr(x.data(), &y, maxlag, norm);
        let n = lags.len();
        Ok((CMatrix::from_col_major(n, 1, r).map_err(lang)?, Matrix::from_col_major(n, 1, lags).map_err(lang)?))
    }

    /// `freqz(b,a,n)` / `freqz(b,a,w)` / `freqz(b,a,n,'whole')` /
    /// `freqz(b,a,f,fs)` → (h, w) with `w` in rad/sample (or `f` in Hz when
    /// `fs` is given).
    fn dsp_freqz(&mut self, args: &[Expr]) -> Result<(CMatrix, Matrix), LangError> {
        let v = self.eval_args(args)?;
        let b = v.first().ok_or_else(|| LangError::new("freqz needs b", 0))?.to_cmatrix()?;
        let a = v.get(1).map(|x| x.to_cmatrix()).transpose()?.unwrap_or_else(|| CMatrix::scalar(CNum::real(1.0)));
        let third = v.get(2).ok_or_else(|| LangError::new("freqz needs a point count or frequency vector", 0))?;
        // Numeric 4th arg is `fs` (Hz); string 'whole' means the full circle.
        let fs = v.get(3).and_then(|x| x.to_matrix().ok()).and_then(|m| m.as_scalar());
        let whole = v.iter().any(|x| matches!(x, Value::Str(s) if s.trim().eq_ignore_ascii_case("whole")));
        let tm = third.to_matrix()?;
        // Integer scalar → point count; any other numeric → frequency vector.
        let w = if tm.numel() == 1 && fs.is_none() && tm.as_scalar().unwrap_or(0.0).fract() == 0.0 {
            let n = tm.as_scalar().unwrap_or(64.0);
            if n <= 0.0 {
                return Err(LangError::new("freqz point count must be a positive integer", 0));
            }
            let n = n as usize;
            check_elems(n)?;
            let span = if whole { 2.0 } else { 1.0 };
            (0..n).map(|k| span * std::f64::consts::PI * k as f64 / n as f64).collect::<Vec<f64>>()
        } else {
            tm.data().to_vec()
        };
        let wrad: Vec<f64> = match fs {
            Some(f) => w.iter().map(|&x| 2.0 * std::f64::consts::PI * x / f).collect(),
            None => w.clone(),
        };
        let h = matcore::dsp::freqz_at(b.data(), a.data(), &wrad);
        let n = w.len();
        Ok((CMatrix::from_col_major(n, 1, h).map_err(lang)?, Matrix::from_col_major(n, 1, w).map_err(lang)?))
    }

    /// Dim argument for reductions: `f(A)`, `f(A, 2)`, `f(A, 'all')`,
    /// `f(A, [], 2)`. Returns 0 for "all" (reduce to scalar).
    fn dim_arg(&mut self, args: &[Expr], dims: (usize, usize)) -> Result<usize, LangError> {
        // Expanded argument values — positions count comma-separated lists.
        for v in self.eval_args(args)?.iter().skip(1) {
            match v {
                Value::Str(s) if s.trim() == "all" => return Ok(0),
                Value::Str(_) => continue,
                other => {
                    let m2 = other.to_matrix()?;
                    if m2.numel() == 0 {
                        continue;
                    }
                    return Ok(other.scalar()?.max(1.0) as usize);
                }
            }
        }
        // Default: vectors reduce fully; matrices reduce along columns.
        if dims.0 == 1 || dims.1 == 1 {
            return Ok(0);
        }
        Ok(1)
    }

    /// Reduce a matrix along `dim` (0 = all → scalar, 1 = per column → row
    /// vector, 2 = per row → column vector).
    fn reduce(&mut self, name: &str, m: &Matrix, dim: usize) -> Result<Value, LangError> {
        let reduce_vec = |d: &[f64]| -> f64 {
            match name {
                "sum" => d.iter().sum(),
                "mean" => {
                    if d.is_empty() {
                        f64::NAN
                    } else {
                        d.iter().sum::<f64>() / d.len() as f64
                    }
                }
                "prod" => d.iter().product(),
                "min" => d.iter().fold(f64::INFINITY, |a, &b| if b.is_nan() { a } else { a.min(b) }),
                "max" => d.iter().fold(f64::NEG_INFINITY, |a, &b| if b.is_nan() { a } else { a.max(b) }),
                "median" => {
                    let mut s = d.to_vec();
                    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                    let n = s.len();
                    if n == 0 {
                        f64::NAN
                    } else if n % 2 == 1 {
                        s.get(n / 2).copied().unwrap_or(f64::NAN)
                    } else {
                        (s.get(n / 2 - 1).copied().unwrap_or(0.0) + s.get(n / 2).copied().unwrap_or(0.0)) / 2.0
                    }
                }
                "std" | "var" => {
                    let n = d.len();
                    if n < 2 {
                        0.0
                    } else {
                        let mean = d.iter().sum::<f64>() / n as f64;
                        let ss: f64 = d.iter().map(|x| (x - mean).powi(2)).sum();
                        let v = ss / (n - 1) as f64;
                        if name == "std" {
                            v.sqrt()
                        } else {
                            v
                        }
                    }
                }
                "any" => {
                    if d.iter().any(|&x| x != 0.0) {
                        1.0
                    } else {
                        0.0
                    }
                }
                "all" => {
                    if d.iter().all(|&x| x != 0.0) {
                        1.0
                    } else {
                        0.0
                    }
                }
                _ => f64::NAN,
            }
        };
        match dim {
            0 => Ok(Value::Num(reduce_vec(m.data()))),
            2 => {
                let mut out = Vec::with_capacity(m.rows());
                for r in 0..m.rows() {
                    let row: Vec<f64> = (0..m.cols()).map(|c| m.get(r, c).unwrap_or(0.0)).collect();
                    out.push(reduce_vec(&row));
                }
                Ok(mat_or_num(Matrix::col_vector(out)))
            }
            _ => {
                let mut out = Vec::with_capacity(m.cols());
                for c in 0..m.cols() {
                    let col: Vec<f64> = (0..m.rows()).map(|r| m.get(r, c).unwrap_or(0.0)).collect();
                    out.push(reduce_vec(&col));
                }
                Ok(mat_or_num(Matrix::row_vector(out)))
            }
        }
    }

    /// Running operations (`cumsum`/`cumprod`/`diff`) along `dim`.
    fn running(&mut self, name: &str, m: &Matrix, dim: usize) -> Result<Value, LangError> {
        // `dim_arg` returns 0 for vectors ("along the data"): map it to the
        // vector's non-singleton direction so row/column output is preserved.
        let dim = if dim == 0 {
            if m.rows() == 1 {
                2
            } else {
                1
            }
        } else {
            dim
        };
        let op = |acc: f64, x: f64| -> f64 {
            match name {
                "cumsum" => acc + x,
                "cumprod" => acc * x,
                _ => acc + x,
            }
        };
        if name == "diff" {
            // First differences along dim (default: rows → along columns).
            let (rows, cols) = m.shape();
            if dim == 2 {
                let mut out = Matrix::zeros(rows, cols.saturating_sub(1));
                for c in 0..cols.saturating_sub(1) {
                    for r in 0..rows {
                        let _ = out.set(r, c, m.get(r, c + 1).unwrap_or(0.0) - m.get(r, c).unwrap_or(0.0));
                    }
                }
                return Ok(mat_or_num(out));
            }
            let mut out = Matrix::zeros(rows.saturating_sub(1), cols);
            for c in 0..cols {
                for r in 0..rows.saturating_sub(1) {
                    let _ = out.set(r, c, m.get(r + 1, c).unwrap_or(0.0) - m.get(r, c).unwrap_or(0.0));
                }
            }
            return Ok(mat_or_num(out));
        }
        let (rows, cols) = m.shape();
        let mut out = Matrix::zeros(rows, cols);
        if dim == 2 {
            for r in 0..rows {
                let mut acc = if name == "cumprod" { 1.0 } else { 0.0 };
                for c in 0..cols {
                    acc = op(acc, m.get(r, c).unwrap_or(0.0));
                    let _ = out.set(r, c, acc);
                }
            }
        } else {
            for c in 0..cols {
                let mut acc = if name == "cumprod" { 1.0 } else { 0.0 };
                for r in 0..rows {
                    acc = op(acc, m.get(r, c).unwrap_or(0.0));
                    let _ = out.set(r, c, acc);
                }
            }
        }
        Ok(mat_or_num(out))
    }

    /// Complex reductions — `sum`/`mean`/`prod`/`any`/`all`/`median`/`std`/`var`
    /// work directly; `min`/`max` order by magnitude then phase, matching
    /// M-code's documented complex ordering.
    fn creduce(&mut self, name: &str, m: &CMatrix, dim: usize) -> Result<Value, LangError> {
        let mag_cmp = |a: &CNum, b: &CNum| {
            a.abs()
                .partial_cmp(&b.abs())
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.arg().partial_cmp(&b.arg()).unwrap_or(std::cmp::Ordering::Equal))
        };
        let reduce_vec = |d: &[CNum]| -> CNum {
            match name {
                "sum" => d.iter().fold(CNum::real(0.0), |a, &b| a.add(b)),
                "mean" => {
                    if d.is_empty() {
                        CNum::new(f64::NAN, f64::NAN)
                    } else {
                        d.iter().fold(CNum::real(0.0), |a, &b| a.add(b)).div(CNum::real(d.len() as f64))
                    }
                }
                "prod" => d.iter().fold(CNum::real(1.0), |a, &b| a.mul(b)),
                "min" => d
                    .iter()
                    .copied()
                    .filter(|z| !(z.re.is_nan() || z.im.is_nan()))
                    .reduce(|a, b| if mag_cmp(&a, &b) == std::cmp::Ordering::Less { a } else { b })
                    .unwrap_or(CNum::real(f64::NAN)),
                "max" => d
                    .iter()
                    .copied()
                    .filter(|z| !(z.re.is_nan() || z.im.is_nan()))
                    .reduce(|a, b| if mag_cmp(&a, &b) == std::cmp::Ordering::Greater { a } else { b })
                    .unwrap_or(CNum::real(f64::NAN)),
                "median" => {
                    let mut s = d.to_vec();
                    s.sort_by(mag_cmp);
                    let n = s.len();
                    if n == 0 {
                        CNum::new(f64::NAN, f64::NAN)
                    } else if n % 2 == 1 {
                        s.get(n / 2).copied().unwrap_or(CNum::real(f64::NAN))
                    } else {
                        s.get(n / 2 - 1)
                            .copied()
                            .unwrap_or(CNum::real(0.0))
                            .add(s.get(n / 2).copied().unwrap_or(CNum::real(0.0)))
                            .div(CNum::real(2.0))
                    }
                }
                // std/var use squared magnitudes of deviations — result is
                // real, wrapped in CNum::real so it flows back as double.
                "std" | "var" => {
                    let n = d.len();
                    if n < 2 {
                        CNum::real(0.0)
                    } else {
                        let mean = d.iter().fold(CNum::real(0.0), |a, &b| a.add(b)).div(CNum::real(n as f64));
                        let ss: f64 = d.iter().map(|z| z.sub(mean).abs2()).sum();
                        let v = ss / (n - 1) as f64;
                        CNum::real(if name == "std" { v.sqrt() } else { v })
                    }
                }
                "any" => CNum::real(if d.iter().any(|z| z.re != 0.0 || z.im != 0.0) { 1.0 } else { 0.0 }),
                "all" => CNum::real(if d.iter().all(|z| z.re != 0.0 || z.im != 0.0) { 1.0 } else { 0.0 }),
                _ => CNum::real(f64::NAN),
            }
        };
        let scalar_or = |cm: CMatrix| -> Value {
            // any/all/var/std produce real scalars; the demoter handles it
            // anyway since their imag parts are zero.
            cmat_or_value(cm)
        };
        match dim {
            0 => Ok(scalar_or(CMatrix::scalar(reduce_vec(m.data())))),
            2 => {
                let mut out = Vec::with_capacity(m.rows());
                for r in 0..m.rows() {
                    let row: Vec<CNum> = (0..m.cols()).map(|c| m.get(r, c).unwrap_or(CNum::real(0.0))).collect();
                    out.push(reduce_vec(&row));
                }
                Ok(scalar_or(CMatrix::from_col_major(out.len(), 1, out).map_err(lang)?))
            }
            _ => {
                let mut out = Vec::with_capacity(m.cols());
                for c in 0..m.cols() {
                    let col: Vec<CNum> = (0..m.rows()).map(|r| m.get(r, c).unwrap_or(CNum::real(0.0))).collect();
                    out.push(reduce_vec(&col));
                }
                Ok(scalar_or(CMatrix::from_col_major(1, out.len(), out).map_err(lang)?))
            }
        }
    }

    /// Running ops (`cumsum`/`cumprod`/`diff`) on complex data.
    fn crunning(&mut self, name: &str, m: &CMatrix, dim: usize) -> Result<Value, LangError> {
        let dim = if dim == 0 {
            if m.rows() == 1 {
                2
            } else {
                1
            }
        } else {
            dim
        };
        let (rows, cols) = m.shape();
        if name == "diff" {
            let out = if dim == 2 {
                let mut o = CMatrix::zeros(rows, cols.saturating_sub(1));
                for c in 0..cols.saturating_sub(1) {
                    for r in 0..rows {
                        let _ = o.set(r, c, m.get(r, c + 1).unwrap_or(CNum::real(0.0)).sub(m.get(r, c).unwrap_or(CNum::real(0.0))));
                    }
                }
                o
            } else {
                let mut o = CMatrix::zeros(rows.saturating_sub(1), cols);
                for c in 0..cols {
                    for r in 0..rows.saturating_sub(1) {
                        let _ = o.set(r, c, m.get(r + 1, c).unwrap_or(CNum::real(0.0)).sub(m.get(r, c).unwrap_or(CNum::real(0.0))));
                    }
                }
                o
            };
            return Ok(cmat_or_value(out));
        }
        let mut out = CMatrix::zeros(rows, cols);
        if dim == 2 {
            for r in 0..rows {
                let mut acc = if name == "cumprod" { CNum::real(1.0) } else { CNum::real(0.0) };
                for c in 0..cols {
                    acc = if name == "cumprod" {
                        acc.mul(m.get(r, c).unwrap_or(CNum::real(0.0)))
                    } else {
                        acc.add(m.get(r, c).unwrap_or(CNum::real(0.0)))
                    };
                    let _ = out.set(r, c, acc);
                }
            }
        } else {
            for c in 0..cols {
                let mut acc = if name == "cumprod" { CNum::real(1.0) } else { CNum::real(0.0) };
                for r in 0..rows {
                    acc = if name == "cumprod" {
                        acc.mul(m.get(r, c).unwrap_or(CNum::real(0.0)))
                    } else {
                        acc.add(m.get(r, c).unwrap_or(CNum::real(0.0)))
                    };
                    let _ = out.set(r, c, acc);
                }
            }
        }
        Ok(cmat_or_value(out))
    }

    /// Complex sort — by magnitude, ties by phase (M-code ordering).
    fn csort(&mut self, m: &CMatrix, dim: usize, desc: bool) -> Result<Value, LangError> {
        let cmp = |a: &CNum, b: &CNum| {
            let o = a
                .abs()
                .partial_cmp(&b.abs())
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.arg().partial_cmp(&b.arg()).unwrap_or(std::cmp::Ordering::Equal));
            if desc {
                o.reverse()
            } else {
                o
            }
        };
        let (rows, cols) = m.shape();
        if rows == 1 || cols == 1 {
            let mut d = m.data().to_vec();
            d.sort_by(cmp);
            let out = if rows == 1 {
                CMatrix::from_col_major(1, d.len(), d).map_err(lang)?
            } else {
                CMatrix::from_col_major(d.len(), 1, d).map_err(lang)?
            };
            return Ok(cmat_or_value(out));
        }
        let mut out = CMatrix::zeros(rows, cols);
        if dim == 2 {
            for r in 0..rows {
                let mut row: Vec<CNum> = (0..cols).map(|c| m.get(r, c).unwrap_or(CNum::real(0.0))).collect();
                row.sort_by(cmp);
                for (c, v) in row.iter().enumerate() {
                    let _ = out.set(r, c, *v);
                }
            }
        } else {
            for c in 0..cols {
                let mut col: Vec<CNum> = (0..rows).map(|r| m.get(r, c).unwrap_or(CNum::real(0.0))).collect();
                col.sort_by(cmp);
                for (r, v) in col.iter().enumerate() {
                    let _ = out.set(r, c, *v);
                }
            }
        }
        Ok(cmat_or_value(out))
    }

    /// Sort vectors or each column/row of a matrix; `desc` reverses.
    fn sort(&mut self, m: &Matrix, dim: usize, desc: bool) -> Result<Value, LangError> {
        let cmp = |a: &f64, b: &f64| {
            let o = a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal);
            if desc {
                o.reverse()
            } else {
                o
            }
        };
        let (rows, cols) = m.shape();
        if rows == 1 || cols == 1 || dim == 0 {
            let mut d = m.data().to_vec();
            d.sort_by(cmp);
            let out = if rows == 1 { Matrix::row_vector(d) } else { Matrix::col_vector(d) };
            return Ok(mat_or_num(out));
        }
        let mut out = Matrix::zeros(rows, cols);
        if dim == 2 {
            for r in 0..rows {
                let mut row: Vec<f64> = (0..cols).map(|c| m.get(r, c).unwrap_or(0.0)).collect();
                row.sort_by(cmp);
                for (c, v) in row.iter().enumerate() {
                    let _ = out.set(r, c, *v);
                }
            }
        } else {
            for c in 0..cols {
                let mut col: Vec<f64> = (0..rows).map(|r| m.get(r, c).unwrap_or(0.0)).collect();
                col.sort_by(cmp);
                for (r, v) in col.iter().enumerate() {
                    let _ = out.set(r, c, *v);
                }
            }
        }
        Ok(mat_or_num(out))
    }

    /// Unary elementwise builtin on the first argument.
    fn unary_mat(&mut self, args: &[Expr], f: fn(f64) -> f64) -> Result<Option<Value>, LangError> {
        let m = self.arg_matrix(args, 0)?;
        Ok(Some(mat_or_num(m.map(f))))
    }

    /// Type/shape predicates.
    fn predicate(&mut self, name: &str, args: &[Expr]) -> Result<Option<Value>, LangError> {
        let v = self.eval_args(args)?;
        let first = v.first();
        let b = match name {
            "isscalar" => matches!(first, Some(x) if x.dims().0 * x.dims().1 == 1),
            "isvector" => matches!(first, Some(x) if x.dims().0 == 1 || x.dims().1 == 1),
            "isrow" => matches!(first, Some(x) if x.dims().0 == 1),
            "iscolumn" => matches!(first, Some(x) if x.dims().1 == 1),
            "ismatrix" => matches!(first, Some(Value::Mat(_)) | Some(Value::Num(_)) | Some(Value::Cplx(_))),
            "isnumeric" => matches!(first, Some(Value::Mat(_)) | Some(Value::Num(_)) | Some(Value::Cplx(_))),
            // `isreal` is false only for a complex value with a nonzero
            // imaginary part — auto-demoted Cplx are real again.
            "isreal" => matches!(first, Some(x) if !matches!(x, Value::Cplx(m) if !m.is_real())),
            "ischar" => matches!(first, Some(Value::Str(_))),
            "islogical" => matches!(first, Some(x) if x.to_matrix().map(|m| m.data().iter().all(|&v| v == 0.0 || v == 1.0)).unwrap_or(false)),
            "isa" => {
                let cls = match v.get(1) {
                    Some(Value::Str(s)) => s.as_str(),
                    _ => "",
                };
                let actual = match first {
                    Some(Value::Num(_)) | Some(Value::Mat(_)) => "double",
                    // Complex values class as `double` with `isreal` == 0.
                    Some(Value::Cplx(_)) => "double",
                    Some(Value::Sparse(_)) => "double",
                    Some(Value::Str(_)) => "char",
                    Some(Value::Cell(_)) => "cell",
                    Some(Value::Struct(_)) => "struct",
                    Some(Value::Func(_)) => "function_handle",
                    None => "",
                };
                cls == actual || cls == "numeric" && actual == "double"
            }
            "isequal" | "isequaln" => {
                let mut eq = v.len() >= 2;
                for w in v.windows(2) {
                    let (a, b) = (w.first(), w.get(1));
                    match (a, b) {
                        (Some(Value::Str(x)), Some(Value::Str(y))) => {
                            if x != y {
                                eq = false;
                            }
                        }
                        (Some(Value::Cell(x)), Some(Value::Cell(y))) => {
                            if x != y {
                                eq = false;
                            }
                        }
                        (Some(Value::Struct(x)), Some(Value::Struct(y))) => {
                            if x != y {
                                eq = false;
                            }
                        }
                        (Some(x), Some(y)) if matches!(x, Value::Cplx(_)) || matches!(y, Value::Cplx(_)) => {
                            // Complex compare: equal shape, equal re+im.
                            match (x.to_cmatrix(), y.to_cmatrix()) {
                                (Ok(mx), Ok(my)) => {
                                    if mx.shape() != my.shape() || mx.data().iter().zip(my.data()).any(|(&p, &q)| p != q) {
                                        eq = false;
                                    }
                                }
                                _ => eq = false,
                            }
                        }
                        (Some(x), Some(y)) => {
                            let (mx, my) = (x.to_matrix(), y.to_matrix());
                            match (mx, my) {
                                (Ok(mx), Ok(my)) => {
                                    if mx.shape() != my.shape()
                                        || mx.data().iter().zip(my.data()).any(|(&p, &q)| {
                                            if name == "isequaln" && p.is_nan() && q.is_nan() {
                                                false
                                            } else {
                                                p != q
                                            }
                                        })
                                    {
                                        eq = false;
                                    }
                                }
                                _ => eq = false,
                            }
                        }
                        _ => eq = false,
                    }
                }
                eq
            }
            _ => false,
        };
        Ok(Some(Value::Num(if b { 1.0 } else { 0.0 })))
    }

    /// `fzero`/`fminsearch`/`integral`/`quad` on a function handle (`@f`,
    /// `@(x) expr`) or a function-name char (`fzero('sin', 1)`).
    fn solve_scalar(&mut self, name: &str, args: &[Expr]) -> Result<Option<Value>, LangError> {
        let v = self.eval_args(args)?;
        let target = match v.first() {
            Some(t @ (Value::Func(_) | Value::Str(_))) => t.clone(),
            Some(Value::Num(_)) | Some(Value::Mat(_)) => {
                return Err(LangError::new(format!("{name}: first argument must be a function handle or name (e.g. {name}(@sin, 1))"), 0))
            }
            _ => return Err(LangError::new(format!("{name} needs a function handle or name"), 0)),
        };
        let f = |s: &mut Session, x: f64| -> Result<f64, LangError> {
            let outs = s.call_feval_target(&target, vec![Value::Num(x)], 1)?;
            outs.first().ok_or_else(|| LangError::new("function returned no value", 0))?.scalar()
        };
        match name {
            "fzero" => {
                // Brent-style safeguarded secant with a bisection fallback on
                // a bracketing interval; with one x0, grow a bracket first.
                let x0 = v.scalar_at(1).unwrap_or(0.0);
                let mut a = x0;
                let mut b = x0 + 1.0;
                let mut fa = f(self, a)?;
                let mut fb = f(self, b)?;
                if fa == 0.0 {
                    return Ok(Some(Value::Num(a)));
                }
                let mut tries = 0;
                while fa * fb > 0.0 && tries < 60 {
                    // Expand geometrically around x0.
                    a = x0 - (x0 - a) * 2.0 - 0.1;
                    fa = f(self, a)?;
                    if fa * fb <= 0.0 {
                        break;
                    }
                    b = x0 + (b - x0) * 2.0 + 0.1;
                    fb = f(self, b)?;
                    tries += 1;
                }
                if fa * fb > 0.0 {
                    return Err(LangError::new("fzero: could not bracket a sign change", 0));
                }
                for _ in 0..200 {
                    if fb == fa {
                        break;
                    }
                    let c = b - fb * (b - a) / (fb - fa);
                    let fc = f(self, c)?;
                    if fc == 0.0 || (b - a).abs() < 1e-14 {
                        return Ok(Some(Value::Num(c)));
                    }
                    if fa * fc < 0.0 {
                        b = c;
                        fb = fc;
                    } else {
                        a = c;
                        fa = fc;
                    }
                }
                // False position may pin one endpoint — return the tighter one.
                Ok(Some(Value::Num(if fa.abs() <= fb.abs() { a } else { b })))
            }
            "fminsearch" => {
                // Nelder–Mead on a scalar parameter.
                let x0 = v.scalar_at(1).unwrap_or(0.0);
                let (mut lo, mut hi) = (x0, if x0 == 0.0 { 1.0 } else { x0 * 1.05 + 0.00025 });
                if hi == lo {
                    hi = lo + 1.0;
                }
                let (mut flo, mut fhi) = (f(self, lo)?, f(self, hi)?);
                for _ in 0..500 {
                    if flo > fhi {
                        std::mem::swap(&mut lo, &mut hi);
                        std::mem::swap(&mut flo, &mut fhi);
                    }
                    if (hi - lo).abs() < 1e-12 {
                        break;
                    }
                    let mid = (lo + hi) / 2.0;
                    let reflect = mid + (mid - hi); // reflect worst
                    let fr = f(self, reflect)?;
                    if fr < flo {
                        let expand = mid + 2.0 * (mid - hi);
                        let fe = f(self, expand)?;
                        if fe < fr {
                            hi = expand;
                            fhi = fe;
                        } else {
                            hi = reflect;
                            fhi = fr;
                        }
                    } else if fr < fhi {
                        hi = reflect;
                        fhi = fr;
                    } else {
                        hi = mid;
                        fhi = f(self, mid)?;
                    }
                }
                Ok(Some(Value::Num(if flo <= fhi { lo } else { hi })))
            }
            _ => {
                // integral / quad: adaptive Simpson.
                let a = v.scalar_at(1)?;
                let b = v.scalar_at(2)?;
                let fa = f(self, a)?;
                let fb = f(self, b)?;
                let m = (a + b) / 2.0;
                let fm = f(self, m)?;
                let whole = (b - a) * (fa + 4.0 * fm + fb) / 6.0;
                Ok(Some(Value::Num(self.adaptive_simpson(&f, a, b, fa, fb, fm, whole, 20)?)))
            }
        }
    }

    /// Recursive adaptive Simpson quadrature for `integral`/`quad`.
    #[allow(clippy::too_many_arguments)]
    fn adaptive_simpson(
        &mut self,
        f: &dyn Fn(&mut Session, f64) -> Result<f64, LangError>,
        a: f64,
        b: f64,
        fa: f64,
        fb: f64,
        fm: f64,
        whole: f64,
        depth: usize,
    ) -> Result<f64, LangError> {
        if depth == 0 {
            return Ok(whole);
        }
        let m = (a + b) / 2.0;
        let lm = (a + m) / 2.0;
        let rm = (m + b) / 2.0;
        let flm = f(self, lm)?;
        let frm = f(self, rm)?;
        let left = (m - a) * (fa + 4.0 * flm + fm) / 6.0;
        let right = (b - m) * (fm + 4.0 * frm + fb) / 6.0;
        let delta = left + right - whole;
        if delta.abs() <= 15.0 * 1e-10 {
            return Ok(left + right + delta / 15.0);
        }
        Ok(self.adaptive_simpson(f, a, m, fa, fm, flm, left, depth - 1)? + self.adaptive_simpson(f, m, b, fm, fb, frm, right, depth - 1)?)
    }

    /// Linear-algebra builtins, single output.
    fn linalg(&mut self, name: &str, args: &[Expr]) -> Result<Option<Value>, LangError> {
        // Complex input: only `trace` is defined for now (diagonal sum).
        // The rest error through `arg_matrix`'s real-only conversion.
        let v = self.eval_args(args)?;
        if let Some(Value::Cplx(cm)) = v.first() {
            if name == "trace" {
                let t = (0..cm.rows().min(cm.cols())).fold(CNum::real(0.0), |a, i| a.add(cm.get(i, i).unwrap_or(CNum::real(0.0))));
                return Ok(Some(cmat_or_value(CMatrix::scalar(t))));
            }
            return Err(LangError::new(format!("{name} on a complex matrix is not implemented yet"), 0));
        }
        let m = v.first().ok_or_else(|| LangError::new(format!("{name} needs an argument"), 0))?.to_matrix()?;
        match name {
            "eig" => {
                let w = matcore::eigvals_c(&m).map_err(lang)?;
                let cm = CMatrix::from_col_major(w.len(), 1, w).map_err(lang)?;
                Ok(Some(cmat_or_value(cm)))
            }
            "svd" => {
                let sv = matcore::singular_values(&m).map_err(lang)?;
                Ok(Some(mat_or_num(Matrix::col_vector(sv))))
            }
            "qr" => {
                // R alone for the single-output form.
                let (_, r) = matcore::qr(&m).map_err(lang)?;
                Ok(Some(Value::Mat(r)))
            }
            "lu" => {
                // Single output: L+U combined (unit diagonal implied).
                let (l, u, _) = matcore::lu(&m).map_err(lang)?;
                let combined = l.elemwise(&Matrix::eye(l.rows(), l.cols()), |a, b| a - b).and_then(|x| x.elemwise(&u, |a, b| a + b)).map_err(lang)?;
                Ok(Some(Value::Mat(combined)))
            }
            "chol" => Ok(Some(Value::Mat(matcore::chol(&m).map_err(lang)?))),
            "rref" => Ok(Some(Value::Mat(matcore::rref(&m).map_err(lang)?))),
            "pinv" => Ok(Some(mat_or_num(matcore::pinv(&m).map_err(lang)?))),
            "rank" => Ok(Some(Value::Num(matcore::rank(&m).map_err(lang)? as f64))),
            "cond" => Ok(Some(Value::Num(matcore::cond(&m).map_err(lang)?))),
            "trace" => Ok(Some(Value::Num(matcore::trace(&m)))),
            "null" | "orth" => {
                let s = matcore::svd(&m).map_err(lang)?;
                let sv: Vec<f64> = (0..s.s.rows().min(s.s.cols())).map(|i| s.s.get(i, i).unwrap_or(0.0)).collect();
                let smax = sv.iter().copied().fold(0.0f64, f64::max);
                let tol = smax * (m.rows().max(m.cols()) as f64) * f64::EPSILON * 32.0;
                if name == "orth" {
                    // Columns of U with s > tol.
                    let keep: Vec<usize> = (0..sv.len().min(s.u.cols())).filter(|&i| sv.get(i).copied().unwrap_or(0.0) > tol).collect();
                    let mut out = Matrix::zeros(s.u.rows(), keep.len());
                    for (nc, &sc) in keep.iter().enumerate() {
                        for r in 0..s.u.rows() {
                            let _ = out.set(r, nc, s.u.get(r, sc).unwrap_or(0.0));
                        }
                    }
                    Ok(Some(mat_or_num(out)))
                } else {
                    // Columns of V with s <= tol.
                    let keep: Vec<usize> = (0..sv.len().min(s.v.cols())).filter(|&i| sv.get(i).copied().unwrap_or(0.0) <= tol).collect();
                    let mut out = Matrix::zeros(s.v.rows(), keep.len());
                    for (nc, &sc) in keep.iter().enumerate() {
                        for r in 0..s.v.rows() {
                            let _ = out.set(r, nc, s.v.get(r, sc).unwrap_or(0.0));
                        }
                    }
                    Ok(Some(mat_or_num(out)))
                }
            }
            _ => Err(LangError::new(format!("unhandled linalg '{name}'"), 0)),
        }
    }

    /// ASCII `save`/`load`. `save file [vars…]` writes a tagged text format
    /// (`# matcraft 1` header); `load file` restores tagged files, or reads a
    /// plain numeric table into a variable named after the file (M-code).
    fn file_io(&mut self, name: &str, args: &[Expr]) -> Result<Option<Value>, LangError> {
        let v = self.eval_args(args)?;
        let path = match v.first() {
            Some(Value::Str(s)) => s.clone(),
            _ => return Err(LangError::new(format!("{name} needs a filename"), 0)),
        };
        if name == "save" {
            let ascii = v.iter().any(|x| matches!(x, Value::Str(s) if s.eq_ignore_ascii_case("-ascii") || s.eq_ignore_ascii_case("-ascii-")));
            let is_mat = path.to_ascii_lowercase().ends_with(".mat") && !ascii;
            let mut out = String::from("# matcraft 1\n");
            let named: Vec<String> = v
                .iter()
                .skip(1)
                .filter_map(|x| match x {
                    Value::Str(s) if !s.starts_with('-') => Some(s.clone()),
                    _ => None,
                })
                .collect();
            // No names at all (or only flags like '-ascii') → every variable.
            let names: Vec<String> = if named.is_empty() { self.var_names() } else { named };
            // MATLAB errors "Variable 'B' not found" for undefined names.
            let missing: Vec<String> = names.iter().filter(|n| !self.vars.contains_key(*n)).map(|n| format!("'{n}'")).collect();
            if !missing.is_empty() {
                return Err(LangError::new(format!("variable {} not found", missing.join(", ")), 0));
            }
            if is_mat {
                let vars: Vec<(String, Value)> = names.iter().filter_map(|n| self.vars.get(n).map(|val| (n.clone(), val.clone()))).collect();
                crate::matfile::write(&path, &vars).map_err(|e| LangError::new(e, 0))?;
                self.print_buf.push_str(&format!("saved {} variable(s) to {path} (MAT v5)\n", vars.len()));
                return Ok(None);
            }
            for n in &names {
                if let Some(val) = self.vars.get(n) {
                    match val {
                        Value::Num(x) => out.push_str(&format!("# name {n} double 1 1\n{x:.17e}\n")),
                        Value::Str(s) => out.push_str(&format!("# name {n} char 1 {}\n{s}\n", s.chars().count())),
                        Value::Mat(m) => {
                            out.push_str(&format!("# name {n} double {} {}\n", m.rows(), m.cols()));
                            for r in 0..m.rows() {
                                for c in 0..m.cols() {
                                    if c > 0 {
                                        out.push(' ');
                                    }
                                    out.push_str(&format!("{:.17e}", m.get(r, c).unwrap_or(0.0)));
                                }
                                out.push('\n');
                            }
                        }
                        // Complex ASCII save writes `re im` pairs per element.
                        Value::Cplx(m) => {
                            out.push_str(&format!("# name {n} complex {} {}\n", m.rows(), m.cols()));
                            for r in 0..m.rows() {
                                for c in 0..m.cols() {
                                    if c > 0 {
                                        out.push(' ');
                                    }
                                    let z = m.get(r, c).unwrap_or(CNum::real(0.0));
                                    out.push_str(&format!("{:.17e} {:.17e}", z.re, z.im));
                                }
                                out.push('\n');
                            }
                        }
                        // Sparse ASCII save writes the dense form (loads
                        // back as a full matrix — mxSPARSE lives in .mat).
                        Value::Sparse(s) => {
                            let m = s.full();
                            out.push_str(&format!("# name {n} double {} {}\n", m.rows(), m.cols()));
                            for r in 0..m.rows() {
                                for c in 0..m.cols() {
                                    if c > 0 {
                                        out.push(' ');
                                    }
                                    out.push_str(&format!("{:.17e}", m.get(r, c).unwrap_or(0.0)));
                                }
                                out.push('\n');
                            }
                        }
                        Value::Cell(_) => return Err(LangError::new(format!("cannot save cell array '{n}' to an ASCII file"), 0)),
                        Value::Struct(_) => return Err(LangError::new(format!("cannot save struct '{n}' to an ASCII file"), 0)),
                        Value::Func(_) => return Err(LangError::new(format!("cannot save function handle '{n}' to an ASCII file"), 0)),
                    }
                }
            }
            std::fs::write(&path, &out).map_err(|e| LangError::new(format!("cannot write {path}: {e}"), 0))?;
            self.print_buf.push_str(&format!("saved {} variable(s) to {path}\n", names.len()));
            return Ok(None);
        }
        // load — sniff the format: MAT v5 binary, tagged ASCII, or a plain
        // numeric table. Function form `S = load('f')` with no variable
        // names returns a scalar struct of the loaded variables (the file's
        // contents are loaded into the workspace as well — the AST can't
        // tell `S = load('f')` from a bare `load 'f'` apart).
        let raw = std::fs::read(&path).map_err(|e| LangError::new(format!("cannot read {path}: {e}"), 0))?;
        let name_filter: Vec<String> = v
            .iter()
            .skip(1)
            .filter_map(|x| match x {
                Value::Str(s) if !s.starts_with('-') => Some(s.clone()),
                _ => None,
            })
            .collect();
        if crate::matfile::looks_like_v5(&raw) {
            let vars = crate::matfile::read(&raw).map_err(|e| LangError::new(e, 0))?;
            let mut loaded: Vec<(String, Value)> = Vec::with_capacity(vars.len());
            for (n, val) in vars {
                if !name_filter.is_empty() && !name_filter.iter().any(|f| f == &n) {
                    continue;
                }
                self.vars.insert(n.clone(), val.clone());
                loaded.push((n, val));
            }
            self.print_buf.push_str(&format!("loaded {} variable(s) from {path}\n", loaded.len()));
            if name_filter.is_empty() {
                let fields: std::collections::BTreeMap<String, Value> = loaded.into_iter().collect();
                return Ok(Some(Value::Struct(StructArray::from_map(fields))));
            }
            return Ok(None);
        }
        if raw.starts_with(b"# matcraft") {
            let text = String::from_utf8_lossy(&raw).into_owned();
            // (name, kind, accumulated numeric rows) — kind: 0 double, 1 char,
            // 2 complex (each element stored as a `re im` pair).
            let mut cur: Option<(String, u8, Vec<Vec<f64>>)> = None;
            let mut loaded: Vec<(String, Value)> = Vec::new();
            for line in text.lines() {
                if let Some(rest) = line.strip_prefix("# name ") {
                    if let Some((n, kind, rows)) = cur.take() {
                        push_num_var(&mut loaded, n, kind, rows);
                    }
                    let parts: Vec<&str> = rest.split_whitespace().collect();
                    let n = parts.first().copied().unwrap_or("ans").to_string();
                    let kind = match parts.get(1).copied() {
                        Some("char") => 1,
                        Some("complex") => 2,
                        _ => 0,
                    };
                    cur = Some((n, kind, Vec::new()));
                } else if line.starts_with('#') {
                    continue;
                } else if let Some((n, kind, rows)) = cur.as_mut() {
                    if *kind == 1 {
                        // The line after a char header is the raw string.
                        loaded.push((n.clone(), Value::Str(line.to_string())));
                        cur = None;
                    } else {
                        let nums: Vec<f64> = line.split_whitespace().filter_map(|t| t.parse::<f64>().ok()).collect();
                        if !nums.is_empty() {
                            rows.push(nums);
                        }
                    }
                }
            }
            if let Some((n, kind, rows)) = cur {
                push_num_var(&mut loaded, n, kind, rows);
            }
            let mut count = 0;
            let mut fields = std::collections::BTreeMap::new();
            for (n, val) in loaded {
                if !name_filter.is_empty() && !name_filter.iter().any(|f| f == &n) {
                    continue;
                }
                self.vars.insert(n.clone(), val.clone());
                fields.insert(n, val);
                count += 1;
            }
            self.print_buf.push_str(&format!("loaded {count} variable(s) from {path}\n"));
            if name_filter.is_empty() {
                return Ok(Some(Value::Struct(StructArray::from_map(fields))));
            }
            return Ok(None);
        }
        // A `.mat` file that isn't MAT v5 (and isn't our tagged ASCII) is
        // corrupt — never fall through to the table reader.
        if path.to_ascii_lowercase().ends_with(".mat") {
            return Err(LangError::new(format!("{path} is not a MAT-file v5"), 0));
        }
        // Plain numeric table → variable named after the file stem.
        let text = String::from_utf8_lossy(&raw).into_owned();
        let mut rows: Vec<Vec<f64>> = Vec::new();
        for line in text.lines() {
            let nums: Vec<f64> =
                line.split(|c: char| c == ',' || c.is_whitespace()).filter(|t| !t.is_empty()).filter_map(|t| t.parse::<f64>().ok()).collect();
            if !nums.is_empty() {
                rows.push(nums);
            }
        }
        if rows.is_empty() {
            return Err(LangError::new(format!("cannot load {path}: no numeric data"), 0));
        }
        let cols = rows.first().map(|r| r.len()).unwrap_or(0);
        let flat: Vec<f64> = rows.iter().flatten().copied().collect();
        let stem = std::path::Path::new(&path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "ans".to_string());
        let stem = if stem.chars().next().map(|c| c.is_alphabetic()).unwrap_or(false) { stem } else { "ans".to_string() };
        let m = Matrix::from_row_major(rows.len(), cols, &flat).map_err(lang)?;
        let out = mat_or_num(m);
        self.vars.insert(stem.clone(), out.clone());
        self.print_buf.push_str(&format!("{stem} loaded from {path}\n"));
        Ok(Some(out))
    }

    /// Names the builtin table handles (for `exist`).
    fn is_builtin(&self, name: &str) -> bool {
        BUILTIN_NAMES.contains(&name)
    }

    /// (rows, cols) from a dims call: `f()` = 1x1, `f(n)` = nxn, `f(r, c)` = rxc.
    /// Shared choke point for `zeros`/`ones`/`eye`/`rand`/`randn`: the
    /// element count is capped before any allocation is attempted.
    fn dims(&mut self, args: &[Expr]) -> Result<(usize, usize), LangError> {
        let (r, c) = match args.len() {
            0 => (1, 1),
            1 => {
                let m = self.eval(&args[0])?.to_matrix()?;
                if m.numel() == 1 {
                    let n = m.as_scalar().unwrap_or(0.0).max(0.0) as usize;
                    (n, n)
                } else if m.numel() >= 2 {
                    (m.get_linear(0).unwrap_or(0.0).max(0.0) as usize, m.get_linear(1).unwrap_or(0.0).max(0.0) as usize)
                } else {
                    (0, 0)
                }
            }
            _ => (self.eval(&args[0])?.scalar()? as usize, self.eval(&args[1])?.scalar()? as usize),
        };
        check_elems(r.saturating_mul(c))?;
        Ok((r, c))
    }
}

impl Default for Session {
    fn default() -> Self {
        Session::new()
    }
}

struct IndexSpec {
    rows: Vec<usize>,
    cols: Vec<usize>,
    linear: bool,
}

/// An evaluated index value → a list of 1-based indices into a dimension
/// of `dim_len`. Carries the logical-mask heuristic and strict integer
/// validation shared by every indexing path.
fn index_list(v: &Value, dim_len: usize) -> Result<Vec<usize>, LangError> {
    let m = v.to_matrix()?;
    // Logical indexing: a 0/1 mask the length of the indexed dimension
    // (`A(A>0)`, `A(mask)`). Only masks containing a zero take this path —
    // all-ones 0/1 vectors are ambiguous with index lists and stay indices.
    if m.numel() == dim_len && dim_len > 0 && m.data().iter().all(|&x| x == 0.0 || x == 1.0) && m.data().contains(&0.0) {
        return Ok(m.data().iter().enumerate().filter(|(_, &x)| x != 0.0).map(|(i, _)| i + 1).collect());
    }
    let mut out = Vec::with_capacity(m.numel());
    for &x in m.data() {
        // Indices are 1-based whole numbers: no rounding of 1.5, no
        // zero/negative, no NaN/Inf.
        if !x.is_finite() || x.fract() != 0.0 || x < 1.0 || x > usize::MAX as f64 {
            return Err(LangError::new("indices must be positive integers", 0));
        }
        out.push(x as usize);
    }
    Ok(out)
}

/// Replace `end` inside an index expression with the dimension length so
/// `A(2:end)` and `A(end-1)` evaluate correctly.
fn resolve_end(e: &Expr, dim_len: usize) -> Expr {
    match e {
        Expr::End => Expr::Num(dim_len as f64),
        Expr::Range(a, step, b) => Expr::Range(
            Box::new(resolve_end(a, dim_len)),
            step.as_ref().map(|s| Box::new(resolve_end(s, dim_len))),
            Box::new(resolve_end(b, dim_len)),
        ),
        Expr::Binary(op, a, b) => Expr::Binary(*op, Box::new(resolve_end(a, dim_len)), Box::new(resolve_end(b, dim_len))),
        Expr::Unary(op, a) => Expr::Unary(*op, Box::new(resolve_end(a, dim_len))),
        Expr::Transpose(a, c) => Expr::Transpose(Box::new(resolve_end(a, dim_len)), *c),
        _ => e.clone(),
    }
}

/// `A(i) = []`, `A(:,j) = []`, `A(i,:) = []` — null-assignment deletion
/// on a matrix. A linear delete collapses a 2-D matrix to a row vector
/// (vectors keep orientation); a subscripted delete needs exactly one
/// non-colon index.
fn matrix_delete(m: &Matrix, specs: &IndexSpec) -> Result<Matrix, LangError> {
    // Deletion cannot grow: every index must be in range.
    let oob = if specs.linear {
        specs.rows.iter().any(|&i| i > m.numel())
    } else {
        specs.rows.iter().any(|&i| i > m.rows()) || specs.cols.iter().any(|&i| i > m.cols())
    };
    if oob {
        return Err(LangError::new("index out of range for deletion", 0));
    }
    if specs.linear {
        let mut data = Vec::with_capacity(m.numel());
        for i in 0..m.numel() {
            if !specs.rows.contains(&(i + 1)) {
                data.push(m.get_linear(i).unwrap_or(0.0));
            }
        }
        return Ok(if m.cols() == 1 && m.rows() > 1 { Matrix::col_vector(data) } else { Matrix::row_vector(data) });
    }
    let all_rows = specs.rows.len() == m.rows();
    let all_cols = specs.cols.len() == m.cols();
    match (all_rows, all_cols) {
        (true, false) => {
            let keep: Vec<usize> = (1..=m.cols()).filter(|c| !specs.cols.contains(c)).collect();
            let mut out = Matrix::zeros(m.rows(), keep.len());
            for (nc, &oc) in keep.iter().enumerate() {
                for r in 0..m.rows() {
                    let _ = out.set(r, nc, m.get(r, oc - 1).unwrap_or(0.0));
                }
            }
            Ok(out)
        }
        (false, true) => {
            let keep: Vec<usize> = (1..=m.rows()).filter(|r| !specs.rows.contains(r)).collect();
            let mut out = Matrix::zeros(keep.len(), m.cols());
            for c in 0..m.cols() {
                for (nr, &or) in keep.iter().enumerate() {
                    let _ = out.set(nr, c, m.get(or - 1, c).unwrap_or(0.0));
                }
            }
            Ok(out)
        }
        (true, true) => Ok(Matrix::zeros(0, 0)),
        (false, false) => Err(LangError::new("a null assignment can have only one non-colon index", 0)),
    }
}

fn assign_into(target: &mut Matrix, specs: &IndexSpec, rhs: &Matrix) -> Result<(), LangError> {
    if specs.linear {
        // Grow to fit the largest linear index (column-vector semantics).
        let top = specs.rows.iter().copied().max().unwrap_or(0);
        if top > target.numel() {
            if target.cols() > 1 && target.numel() > 0 {
                return Err(LangError::new("linear index exceeds matrix dimensions", 0));
            }
            check_elems(top)?;
            target.grow(top, 1);
        }
        for (i, &idx) in specs.rows.iter().enumerate() {
            let v = rhs.get_linear(if rhs.is_scalar() { 0 } else { i }).unwrap_or(0.0);
            let (r, c) = if target.rows() > 0 { ((idx - 1) % target.rows(), (idx - 1) / target.rows()) } else { (0, 0) };
            target.set(r, c, v).map_err(lang)?;
        }
        return Ok(());
    }
    let maxr = specs.rows.iter().copied().max().unwrap_or(0);
    let maxc = specs.cols.iter().copied().max().unwrap_or(0);
    check_elems(maxr.max(target.rows()).saturating_mul(maxc.max(target.cols())))?;
    target.grow(maxr, maxc);
    for (oc, &c) in specs.cols.iter().enumerate() {
        for (or, &r) in specs.rows.iter().enumerate() {
            let v = if rhs.is_scalar() { rhs.as_scalar().unwrap_or(0.0) } else { rhs.get(or, oc).unwrap_or(0.0) };
            let _ = target.set(r.saturating_sub(1), c.saturating_sub(1), v);
        }
    }
    Ok(())
}

fn matrix_power(m: &Matrix, p: f64) -> Result<Matrix, MatError> {
    let n = p.round() as i64;
    if (n as f64 - p).abs() > 1e-12 {
        return Err(MatError::DimensionMismatch { a: m.shape(), b: (1, 1) });
    }
    if n == 0 {
        return Ok(Matrix::eye(m.rows(), m.cols()));
    }
    let (base, count) = if n < 0 { (m.inverse()?, (-n) as u64) } else { (m.clone(), n as u64) };
    // Exponentiation by squaring.
    let mut result = Matrix::eye(m.rows(), m.cols());
    let mut b = base;
    let mut e = count;
    while e > 0 {
        if e % 2 == 1 {
            result = result.matmul(&b)?;
        }
        e /= 2;
        if e > 0 {
            b = b.matmul(&b)?;
        }
    }
    Ok(result)
}

fn cmp(a: &Matrix, b: &Matrix, f: impl Fn(f64, f64) -> bool) -> Result<Value, LangError> {
    a.broadcast(b, |x, y| if f(x, y) { 1.0 } else { 0.0 }).map(mat_or_num).map_err(lang)
}

/// `A(i) = v` on a complex target — mirrors `assign_into` with `CNum`s.
fn cassign_into(target: &mut CMatrix, specs: &IndexSpec, rhs: &CMatrix) -> Result<(), LangError> {
    if specs.linear {
        let top = specs.rows.iter().copied().max().unwrap_or(0);
        if top > target.numel() {
            if target.cols() > 1 && target.numel() > 0 {
                return Err(LangError::new("linear index exceeds matrix dimensions", 0));
            }
            check_elems(top)?;
            target.grow(top, 1);
        }
        for (i, &idx) in specs.rows.iter().enumerate() {
            let v = rhs.get_linear(if rhs.is_scalar() { 0 } else { i }).unwrap_or(CNum::real(0.0));
            let (r, c) = if target.rows() > 0 { ((idx - 1) % target.rows(), (idx - 1) / target.rows()) } else { (0, 0) };
            target.set(r, c, v).map_err(lang)?;
        }
        return Ok(());
    }
    let maxr = specs.rows.iter().copied().max().unwrap_or(0);
    let maxc = specs.cols.iter().copied().max().unwrap_or(0);
    check_elems(maxr.max(target.rows()).saturating_mul(maxc.max(target.cols())))?;
    target.grow(maxr, maxc);
    for (oc, &c) in specs.cols.iter().enumerate() {
        for (or, &r) in specs.rows.iter().enumerate() {
            let v = if rhs.is_scalar() { rhs.as_scalar().unwrap_or(CNum::real(0.0)) } else { rhs.get(or, oc).unwrap_or(CNum::real(0.0)) };
            let _ = target.set(r.saturating_sub(1), c.saturating_sub(1), v);
        }
    }
    Ok(())
}

/// `A(i) = []` on a complex target — mirrors `matrix_delete`.
fn cmatrix_delete(m: &CMatrix, specs: &IndexSpec) -> Result<CMatrix, LangError> {
    let oob = if specs.linear {
        specs.rows.iter().any(|&i| i > m.numel())
    } else {
        specs.rows.iter().any(|&i| i > m.rows()) || specs.cols.iter().any(|&i| i > m.cols())
    };
    if oob {
        return Err(LangError::new("index out of range for deletion", 0));
    }
    if specs.linear {
        let mut data = Vec::with_capacity(m.numel());
        for i in 0..m.numel() {
            if !specs.rows.contains(&(i + 1)) {
                data.push(m.get_linear(i).unwrap_or(CNum::real(0.0)));
            }
        }
        let (r, c) = if m.cols() == 1 && m.rows() > 1 { (data.len(), 1) } else { (1, data.len()) };
        return CMatrix::from_col_major(r, c, data).map_err(lang);
    }
    let all_rows = specs.rows.len() == m.rows();
    let all_cols = specs.cols.len() == m.cols();
    match (all_rows, all_cols) {
        (true, false) => {
            let keep: Vec<usize> = (1..=m.cols()).filter(|c| !specs.cols.contains(c)).collect();
            let mut out = CMatrix::zeros(m.rows(), keep.len());
            for (nc, &oc) in keep.iter().enumerate() {
                for r in 0..m.rows() {
                    let _ = out.set(r, nc, m.get(r, oc - 1).unwrap_or(CNum::real(0.0)));
                }
            }
            Ok(out)
        }
        (false, true) => {
            let keep: Vec<usize> = (1..=m.rows()).filter(|r| !specs.rows.contains(r)).collect();
            let mut out = CMatrix::zeros(keep.len(), m.cols());
            for c in 0..m.cols() {
                for (nr, &or) in keep.iter().enumerate() {
                    let _ = out.set(nr, c, m.get(or - 1, c).unwrap_or(CNum::real(0.0)));
                }
            }
            Ok(out)
        }
        (true, true) => Ok(CMatrix::zeros(0, 0)),
        (false, false) => Err(LangError::new("a null assignment can have only one non-colon index", 0)),
    }
}

/// `switch` case matching: strings compare literally, numbers elementwise-any.
fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Cell(x), Value::Cell(y)) => x == y,
        (Value::Struct(x), Value::Struct(y)) => x == y,
        _ => {
            // Complex either side compares elementwise on re+im.
            if matches!(a, Value::Cplx(_)) || matches!(b, Value::Cplx(_)) {
                let (x, y) = (a.to_cmatrix(), b.to_cmatrix());
                return match (x, y) {
                    (Ok(ma), Ok(mb)) => ma
                        .broadcast(&mb, |p, q| if p == q { CNum::real(1.0) } else { CNum::real(0.0) })
                        .map(|m| m.data().iter().any(|z| z.re != 0.0 || z.im != 0.0))
                        .unwrap_or(false),
                    _ => false,
                };
            }
            let (x, y) = (a.to_matrix(), b.to_matrix());
            match (x, y) {
                (Ok(ma), Ok(mb)) => {
                    ma.broadcast(&mb, |p, q| if p == q { 1.0 } else { 0.0 }).map(|m| m.data().iter().any(|&v| v != 0.0)).unwrap_or(false)
                }
                _ => false,
            }
        }
    }
}

fn mat_or_num(m: Matrix) -> Value {
    match m.as_scalar() {
        Some(v) => Value::Num(v),
        None => Value::Mat(m),
    }
}

/// Complex result → `Value`; a fully-real result collapses to Mat/Num.
fn cmat_or_value(m: CMatrix) -> Value {
    if let Some(r) = m.real_if_real() {
        return mat_or_num(r);
    }
    Value::Cplx(m)
}

/// Column-wise transform `t` (`fft`/`ifft`) on every column of `cm`,
/// padding or truncating each to length `n`.
fn fft_cols(cm: &CMatrix, n: usize, t: fn(&[CNum]) -> Vec<CNum>) -> Result<CMatrix, LangError> {
    check_elems(n.saturating_mul(cm.cols()))?;
    let mut out = Vec::with_capacity(n * cm.cols());
    for c in 0..cm.cols() {
        let mut col: Vec<CNum> = (0..cm.rows()).map(|r| cm.get(r, c).unwrap_or(CNum::real(0.0))).collect();
        col.resize(n, CNum::real(0.0));
        col.truncate(n);
        out.extend(t(&col));
    }
    CMatrix::from_col_major(n, cm.cols(), out).map_err(lang)
}

/// `fftshift`/`ifftshift` on both dims: `out(r,c) = get(ro[r], co[c])`,
/// gathered column-major.
fn shift_both<T>(rows: usize, cols: usize, order: fn(usize) -> Vec<usize>, get: impl Fn(usize, usize) -> T) -> Vec<T> {
    let ro = order(rows);
    let co = order(cols);
    let mut out = Vec::with_capacity(rows.saturating_mul(cols));
    for c in 0..cols {
        for r in 0..rows {
            out.push(get(ro.get(r).copied().unwrap_or(0), co.get(c).copied().unwrap_or(0)));
        }
    }
    out
}

/// Apply a real-matrix transform to a complex matrix's re/im planes and
/// recombine — works for any shape/position op (reshape, repmat, flip,
/// circshift, rot90, triu/tril, diag).
fn cplanes(cm: &CMatrix, f: impl Fn(&Matrix) -> Result<Matrix, LangError>) -> Result<CMatrix, LangError> {
    let re = f(&cm.re())?;
    let im = f(&cm.im())?;
    CMatrix::from_parts(&re, &im).map_err(lang)
}

/// Element-wise complex comparison returning a real 0/1 mask.
fn ccmp(a: &CMatrix, b: &CMatrix, f: impl Fn(CNum, CNum) -> bool) -> Result<Value, LangError> {
    a.broadcast(b, |x, y| CNum::real(if f(x, y) { 1.0 } else { 0.0 })).map(cmat_or_value).map_err(lang)
}

/// Guard against user-driven allocation bombs (`zeros(1e9)`, `A(1e9)=1`,
/// `linspace(0,1,1e12)`): the count is the elements about to be allocated.
fn check_elems(n: usize) -> Result<(), LangError> {
    if n > MAX_ELEMS {
        return Err(LangError::new("allocation exceeds limit", 0));
    }
    Ok(())
}

/// `magic(n)`: an n-by-n magic square — every row, column and main
/// diagonal sums to n(n²+1)/2. Odd orders use the Siamese method, doubly
/// even orders the complement pattern, singly even orders Strachey's
/// method on an odd-order base square. Order 2 has no magic square; the
/// conventional placeholder `[1 3; 4 2]` is returned instead.
fn magic_square(n: usize) -> Matrix {
    let mut m = vec![0.0; n.saturating_mul(n)];
    let put = |m: &mut [f64], r: usize, c: usize, v: f64| {
        if let Some(e) = m.get_mut(c * n + r) {
            *e = v;
        }
    };
    if n % 2 == 1 {
        // Siamese method: start at the middle of the top row and step
        // up-right with wraparound; drop one row on an occupied cell.
        let (mut r, mut c) = (0usize, n / 2);
        for v in 1..=(n * n) {
            put(&mut m, r, c, v as f64);
            let (nr, nc) = ((r + n - 1) % n, (c + 1) % n);
            if m.get(nc * n + nr).copied().unwrap_or(0.0) != 0.0 {
                r = (r + 1) % n;
            } else {
                r = nr;
                c = nc;
            }
        }
    } else if n == 2 {
        // Placeholder — its line sums are deliberately not all equal.
        for (i, &v) in [1.0, 4.0, 3.0, 2.0].iter().enumerate() {
            put(&mut m, i % 2, i / 2, v);
        }
    } else if n.is_multiple_of(4) {
        // Doubly even: sequential fill, then complement the cells on the
        // 4-by-4 block diagonals (r%4 == c%4 or r%4 + c%4 == 3).
        let total = (n * n + 1) as f64;
        for r in 0..n {
            for c in 0..n {
                let v = (r * n + c + 1) as f64;
                put(&mut m, r, c, if r % 4 == c % 4 || r % 4 + c % 4 == 3 { total - v } else { v });
            }
        }
    } else {
        // Singly even (n = 2k, k odd): Strachey's method — fill four
        // k-by-k quadrants from an odd-order base square with offsets
        // TL +0, TR +2k², BL +3k², BR +k², then exchange columns:
        // every left-half row swaps its first p cells TL↔BL except the
        // middle row, which swaps cells p..2p; every right-half row
        // swaps its last p−1 cells TR↔BR.
        let k = n / 2;
        let kk = (k * k) as f64;
        let p = (k - 1) / 2;
        let s = magic_square(k);
        for r in 0..k {
            for c in 0..k {
                let sv = s.data().get(c * k + r).copied().unwrap_or(0.0);
                put(&mut m, r, c, sv);
                put(&mut m, r, c + k, sv + 2.0 * kk);
                put(&mut m, r + k, c, sv + 3.0 * kk);
                put(&mut m, r + k, c + k, sv + kk);
            }
        }
        let mid = k / 2;
        for r in 0..k {
            let lo = if r == mid { p } else { 0 };
            for c in lo..(lo + p) {
                m.swap(c * n + r, c * n + r + k);
            }
            for c in (k + 1 - p)..k {
                m.swap((c + k) * n + r, (c + k) * n + r + k);
            }
        }
    }
    Matrix::from_col_major(n, n, m).unwrap_or_else(|_| Matrix::zeros(0, 0))
}

/// MATLAB field names must be valid identifiers: a letter or `_` first, then
/// letters/digits/`_`, and never a reserved word (`s.('for')` is rejected).
fn valid_field_name(s: &str) -> bool {
    const KEYWORDS: &[&str] = &[
        "break",
        "case",
        "catch",
        "classdef",
        "continue",
        "else",
        "elseif",
        "end",
        "events",
        "for",
        "function",
        "global",
        "if",
        "methods",
        "otherwise",
        "parfor",
        "persistent",
        "properties",
        "return",
        "spmd",
        "switch",
        "try",
        "while",
    ];
    let mut chars = s.chars();
    if !matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_') && !KEYWORDS.contains(&s)
}

/// `repmat`: tile `m` r×c times.
fn repmat(m: &Matrix, r: usize, c: usize) -> Result<Matrix, LangError> {
    check_elems(m.numel().saturating_mul(r).saturating_mul(c))?;
    let mut out = Matrix::zeros(m.rows().saturating_mul(r), m.cols().saturating_mul(c));
    for tc in 0..c {
        for tr in 0..r {
            for cj in 0..m.cols() {
                for ri in 0..m.rows() {
                    let _ = out.set(tr * m.rows() + ri, tc * m.cols() + cj, m.get(ri, cj).unwrap_or(0.0));
                }
            }
        }
    }
    Ok(out)
}

/// `diag`: vector → diagonal matrix; matrix → main-diagonal column vector.
fn diag(m: &Matrix) -> Result<Matrix, LangError> {
    if m.rows() == 1 || m.cols() == 1 {
        let n = m.numel();
        check_elems(n.saturating_mul(n))?;
        let mut out = Matrix::zeros(n, n);
        for i in 0..n {
            let _ = out.set(i, i, m.get_linear(i).unwrap_or(0.0));
        }
        Ok(out)
    } else {
        let n = m.rows().min(m.cols());
        let d: Vec<f64> = (0..n).map(|i| m.get(i, i).unwrap_or(0.0)).collect();
        Ok(Matrix::col_vector(d))
    }
}

/// Column-wise (dim 1) or row-wise (dim 2) min/max with 1-based arg indices.
fn reduce_with_index(m: &Matrix, dim: usize, hi: bool) -> (Matrix, Matrix) {
    let pick = |a: f64, b: f64| -> bool {
        if hi {
            b > a
        } else {
            b < a
        }
    };
    let (rows, cols) = m.shape();
    let row_mode = dim == 2 || (rows == 1);
    let n_outer = if row_mode { rows } else { cols };
    let n_inner = if row_mode { cols } else { rows };
    let mut vals = Vec::with_capacity(n_outer);
    let mut idxs = Vec::with_capacity(n_outer);
    for o in 0..n_outer {
        let mut bv = f64::NAN;
        let mut bi = 0usize;
        for i in 0..n_inner {
            let v = if row_mode { m.get(o, i).unwrap_or(0.0) } else { m.get(i, o).unwrap_or(0.0) };
            if v.is_nan() {
                continue;
            }
            if bv.is_nan() || pick(bv, v) {
                bv = v;
                bi = i;
            }
        }
        vals.push(bv);
        idxs.push((bi + 1) as f64);
    }
    let (v, i) = if row_mode { (Matrix::col_vector(vals), Matrix::col_vector(idxs)) } else { (Matrix::row_vector(vals), Matrix::row_vector(idxs)) };
    (v, i)
}

/// Least-squares polynomial fit, degree `d`: solves the Vandermonde system
/// via QR. Returns coefficients in descending powers (M-code order).
fn polyfit(x: &Matrix, y: &Matrix, d: usize) -> Result<Vec<f64>, LangError> {
    let n = x.numel().min(y.numel());
    let d = d.min(n.saturating_sub(1));
    check_elems(n.saturating_mul(d.saturating_add(1)))?;
    let mut v = Matrix::zeros(n, d + 1);
    for i in 0..n {
        let xi = x.get_linear(i).unwrap_or(0.0);
        let mut p = 1.0;
        for c in 0..=d {
            // Columns ascending power; we reverse to descending at the end.
            let _ = v.set(i, d - c, p);
            p *= xi;
        }
    }
    let yv: Vec<f64> = (0..n).map(|i| y.get_linear(i).unwrap_or(0.0)).collect();
    let ym = Matrix::col_vector(yv);
    // Least squares via normal equations on QR of the Vandermonde matrix.
    let (q, r) = matcore::qr(&v).map_err(lang)?;
    let qty = q.transpose().matmul(&ym).map_err(lang)?;
    // R is (d+1)x(d+1) upper block at the top.
    let mut rsq = Matrix::zeros(d + 1, d + 1);
    for c in 0..=d {
        for row in 0..=d {
            let _ = rsq.set(row, c, r.get(row, c).unwrap_or(0.0));
        }
    }
    let mut qty_top = Matrix::zeros(d + 1, 1);
    for i in 0..=d {
        let _ = qty_top.set(i, 0, qty.get(i, 0).unwrap_or(0.0));
    }
    let coefs = rsq.solve(&qty_top).map_err(lang)?;
    Ok(coefs.data().to_vec())
}

/// Polynomial roots (descending-power coefficients) as eigenvalues of the
/// companion matrix — complex pairs included via `eigvals_c`.
fn roots(p: &Matrix) -> Result<Vec<CNum>, LangError> {
    let mut c: Vec<f64> = p.data().to_vec();
    while c.first().copied().unwrap_or(0.0) == 0.0 && c.len() > 1 {
        c.remove(0);
    }
    if c.len() <= 1 {
        return Ok(Vec::new());
    }
    let lead = c.first().copied().unwrap_or(1.0);
    let n = c.len() - 1;
    if n == 0 {
        return Ok(Vec::new());
    }
    check_elems(n.saturating_mul(n))?;
    let mut comp = Matrix::zeros(n, n);
    for i in 0..n {
        let _ = comp.set(0, i, -c.get(i + 1).copied().unwrap_or(0.0) / lead);
    }
    for i in 1..n {
        let _ = comp.set(i, i - 1, 1.0);
    }
    matcore::eigvals_c(&comp).map_err(lang)
}

/// Linear interpolation of `y` at `xq` (x must be sorted ascending);
/// extrapolation clamps to the end slopes like M-code linear extrapolation.
fn interp1(x: &Matrix, y: &Matrix, xq: &Matrix) -> Result<Matrix, LangError> {
    if x.numel() != y.numel() {
        return Err(LangError::new("interp1: x and y must be the same length", 0));
    }
    let n = x.numel();
    if n < 2 {
        return Err(LangError::new("interp1 needs at least two points", 0));
    }
    let out: Vec<f64> = xq
        .data()
        .iter()
        .map(|&q| {
            // Binary search for the segment.
            let mut lo = 0usize;
            let mut hi = n - 1;
            while hi - lo > 1 {
                let mid = (lo + hi) / 2;
                if x.get_linear(mid).unwrap_or(0.0) <= q {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            if q <= x.get_linear(0).unwrap_or(0.0) {
                lo = 0;
                hi = 1;
            } else if q >= x.get_linear(n - 1).unwrap_or(0.0) {
                lo = n - 2;
                hi = n - 1;
            }
            let (x0, x1) = (x.get_linear(lo).unwrap_or(0.0), x.get_linear(hi).unwrap_or(0.0));
            let (y0, y1) = (y.get_linear(lo).unwrap_or(0.0), y.get_linear(hi).unwrap_or(0.0));
            if x1 == x0 {
                return y0;
            }
            y0 + (y1 - y0) * (q - x0) / (x1 - x0)
        })
        .collect();
    Matrix::from_col_major(xq.rows(), xq.cols(), out).map_err(lang)
}

/// Trapezoidal integration of `y` over `x`.
fn trapz(x: &Matrix, y: &Matrix) -> f64 {
    let n = x.numel().min(y.numel());
    let mut acc = 0.0;
    for i in 1..n {
        let dx = x.get_linear(i).unwrap_or(0.0) - x.get_linear(i - 1).unwrap_or(0.0);
        let h = (y.get_linear(i).unwrap_or(0.0) + y.get_linear(i - 1).unwrap_or(0.0)) / 2.0;
        acc += dx * h;
    }
    acc
}

/// Histogram bin counts and edges over `nb` equal-width bins.
/// Pick an output format for `saveas`/`exportgraphics`/`print` and render
/// the figure to bytes. `fmt` overrides the extension (`-dpng` &c.).
#[cfg(not(target_arch = "wasm32"))]
fn export_figure(fig: &matplot::Figure, file: &str, fmt: Option<&str>) -> Result<Vec<u8>, LangError> {
    let kind = fmt.map(str::to_string).unwrap_or_else(|| file.rsplit('.').next().unwrap_or("svg").to_ascii_lowercase());
    match kind.as_str() {
        "svg" => Ok(matplot::to_svg(fig).into_bytes()),
        "png" => matplot::to_png(fig, 2.0).map_err(|e| LangError::new(format!("cannot render PNG: {e}"), 0)),
        "pdf" => matplot::to_pdf(fig).map_err(|e| LangError::new(format!("cannot render PDF: {e}"), 0)),
        other => Err(LangError::new(format!("unsupported export format '{other}' (use svg/png/pdf)"), 0)),
    }
}

#[cfg(target_arch = "wasm32")]
fn export_figure(fig: &matplot::Figure, file: &str, fmt: Option<&str>) -> Result<Vec<u8>, LangError> {
    let kind = fmt.map(str::to_string).unwrap_or_else(|| file.rsplit('.').next().unwrap_or("svg").to_ascii_lowercase());
    match kind.as_str() {
        "svg" => Ok(matplot::to_svg(fig).into_bytes()),
        other => Err(LangError::new(format!("export format '{other}' needs a native renderer (use svg)"), 0)),
    }
}

/// `imagesc`-style coordinate vector → per-cell centres. A two-element
/// vector is the [first last] corner form and expands linearly; anything
/// else of the right length is used as-is; else 1..=n.
fn grid_coords(m: &Matrix, n: usize) -> Vec<f64> {
    let d = m.data();
    if d.len() == n && n > 2 {
        return d.to_vec();
    }
    if d.len() >= 2 && n >= 2 {
        let (a, b) = (d[0], d[d.len() - 1]);
        return (0..n).map(|k| a + (b - a) * k as f64 / (n - 1) as f64).collect();
    }
    (1..=n).map(|k| k as f64).collect()
}

fn histcounts(m: &Matrix, nb: usize) -> (Vec<f64>, Vec<f64>) {
    let nb = nb.max(1);
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for &v in m.data() {
        if v.is_finite() {
            lo = lo.min(v);
            hi = hi.max(v);
        }
    }
    if !lo.is_finite() {
        return (vec![0.0; nb], (0..=nb).map(|i| i as f64).collect());
    }
    if hi == lo {
        hi = lo + 1.0;
    }
    let w = (hi - lo) / nb as f64;
    let mut counts = vec![0.0; nb];
    for &v in m.data() {
        if !v.is_finite() {
            continue;
        }
        let b = ((v - lo) / w).floor() as usize;
        let b = b.min(nb - 1);
        if let Some(c) = counts.get_mut(b) {
            *c += 1.0;
        }
    }
    let edges: Vec<f64> = (0..=nb).map(|i| lo + w * i as f64).collect();
    (counts, edges)
}

/// Every name `builtin()` recognises — used by `exist` and by the engine's
/// function catalogue. Keep in sync with the `match` above.
pub const BUILTIN_NAMES: &[&str] = &[
    "zeros",
    "ones",
    "eye",
    "magic",
    "rand",
    "randn",
    "randi",
    "linspace",
    "logspace",
    "size",
    "numel",
    "length",
    "ndims",
    "reshape",
    "squeeze",
    "transpose",
    "ctranspose",
    "repmat",
    "diag",
    "triu",
    "tril",
    "flipud",
    "fliplr",
    "kron",
    "cat",
    "horzcat",
    "vertcat",
    "cell",
    "iscell",
    "iscellstr",
    "cell2mat",
    "num2cell",
    "sparse",
    "full",
    "issparse",
    "nnz",
    "nzmax",
    "nonzeros",
    "speye",
    "spalloc",
    "spones",
    "celldisp",
    "struct",
    "isstruct",
    "fieldnames",
    "isfield",
    "rmfield",
    "orderfields",
    "getfield",
    "setfield",
    "deal",
    "circshift",
    "rot90",
    "det",
    "inv",
    "norm",
    "sum",
    "mean",
    "prod",
    "min",
    "max",
    "std",
    "var",
    "median",
    "any",
    "all",
    "cumsum",
    "cumprod",
    "diff",
    "sort",
    "unique",
    "find",
    "abs",
    "sqrt",
    "exp",
    "log",
    "log10",
    "log2",
    "expm1",
    "log1p",
    "sin",
    "cos",
    "tan",
    "asin",
    "acos",
    "atan",
    "atan2",
    "sinh",
    "cosh",
    "tanh",
    "sech",
    "csch",
    "coth",
    "floor",
    "ceil",
    "round",
    "fix",
    "sign",
    "mod",
    "rem",
    "hypot",
    "nthroot",
    "nextpow2",
    "pow2",
    "dot",
    "cross",
    "isnan",
    "isinf",
    "isfinite",
    "isempty",
    "isscalar",
    "isvector",
    "isrow",
    "iscolumn",
    "ismatrix",
    "isnumeric",
    "ischar",
    "islogical",
    "isreal",
    "complex",
    "real",
    "imag",
    "conj",
    "angle",
    "isa",
    "isequal",
    "isequaln",
    "polyval",
    "polyfit",
    "roots",
    "poly",
    "conv",
    "deconv",
    "fft",
    "ifft",
    "fft2",
    "ifft2",
    "fftshift",
    "ifftshift",
    "dftmtx",
    "filter",
    "xcorr",
    "freqz",
    "unwrap",
    "hamming",
    "hann",
    "blackman",
    "bartlett",
    "rectwin",
    "triang",
    "kaiser",
    "gausswin",
    "interp1",
    "trapz",
    "fzero",
    "fminsearch",
    "integral",
    "quad",
    "eig",
    "svd",
    "qr",
    "lu",
    "chol",
    "rref",
    "pinv",
    "rank",
    "cond",
    "trace",
    "null",
    "orth",
    "num2str",
    "str2double",
    "str2num",
    "strcmp",
    "strcmpi",
    "strcat",
    "upper",
    "lower",
    "strtrim",
    "strrep",
    "strfind",
    "double",
    "logical",
    "char",
    "eval",
    "error",
    "warning",
    "exist",
    "which",
    "clc",
    "home",
    "tic",
    "toc",
    "pause",
    "save",
    "load",
    "type",
    "disp",
    "display",
    "format",
    "addpath",
    "rmpath",
    "path",
    "pwd",
    "cd",
    "dir",
    "ls",
    "fprintf",
    "sprintf",
    "who",
    "whos",
    "clear",
    "nargin",
    "nargout",
    "narginchk",
    "nargoutchk",
    "feval",
    "func2str",
    "str2func",
    "functions",
    "arrayfun",
    "cellfun",
    "lasterror",
    "figure",
    "gcf",
    "clf",
    "close",
    "subplot",
    "plot",
    "scatter",
    "stem",
    "bar",
    "hold",
    "grid",
    "title",
    "xlabel",
    "ylabel",
    "legend",
    "xlim",
    "ylim",
    "zlim",
    "zlabel",
    "view",
    "colormap",
    "colorbar",
    "caxis",
    "clim",
    "imagesc",
    "image",
    "pie",
    "contour",
    "plot3",
    "surf",
    "mesh",
    "saveas",
    "exportgraphics",
    "print",
    "histcounts",
    "meshgrid",
    "errorbar",
    "histogram",
    "semilogx",
    "semilogy",
    "loglog",
    "area",
    "stairs",
    "xscale",
    "yscale",
    "deg2rad",
    "rad2deg",
    "sinc",
    "db2pow",
    "pow2db",
    "uint8",
    "rectpuls",
    "tripuls",
    "randperm",
    "normrnd",
    "assert",
    "rng",
    "genpath",
    "fullfile",
    "csvread",
    "axis",
    "box",
    "inf",
    "Inf",
    "nan",
    "NaN",
    "eps",
    "mkdir",
];

fn lang(e: MatError) -> LangError {
    LangError::new(e.to_string(), 0)
}

/// Collect a finished `# name <n> <kind> r c` block from a tagged-ASCII file
/// as a (name, Value) pair for `load`. kind 2 = complex (`re im` pairs).
fn push_num_var(loaded: &mut Vec<(String, Value)>, n: String, kind: u8, rows: Vec<Vec<f64>>) {
    let cols = rows.first().map(|r| r.len()).unwrap_or(0);
    let flat: Vec<f64> = rows.iter().flatten().copied().collect();
    if kind == 2 {
        let data: Vec<CNum> =
            flat.as_chunks::<2>().0.iter().map(|pair| CNum::new(pair.first().copied().unwrap_or(0.0), pair.get(1).copied().unwrap_or(0.0))).collect();
        let nrows = rows.len();
        let ncols = cols / 2;
        let mut m = CMatrix::zeros(nrows, ncols);
        for (r, row) in data.chunks(ncols.max(1)).enumerate() {
            for (c, z) in row.iter().enumerate() {
                let _ = m.set(r, c, *z);
            }
        }
        if nrows * ncols == data.len() {
            loaded.push((n, Value::Cplx(m)));
        }
        return;
    }
    if kind != 0 {
        return;
    }
    if let Ok(m) = Matrix::from_row_major(rows.len(), cols, &flat) {
        let v2 = if m.is_scalar() { Value::Num(m.as_scalar().unwrap_or(0.0)) } else { Value::Mat(m) };
        loaded.push((n, v2));
    }
}

/// Minimal `*`-wildcard matcher for `dir`/`ls` patterns.
fn glob_match(pat: &str, name: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut pi, mut ni) = (0usize, 0usize);
    let (mut star, mut star_n) = (usize::MAX, 0usize);
    while ni < n.len() {
        let pc = p.get(pi).copied();
        if pc == Some('*') {
            star = pi;
            star_n = ni;
            pi += 1;
        } else if pc == Some('?') || pc == n.get(ni).copied() {
            pi += 1;
            ni += 1;
        } else if star != usize::MAX {
            pi = star + 1;
            star_n += 1;
            ni = star_n;
        } else {
            return false;
        }
    }
    while p.get(pi) == Some(&'*') {
        pi += 1;
    }
    pi == p.len()
}

/// Short class description used by `whos` and `workspace.list`.
pub fn describe(v: &Value) -> String {
    match v {
        Value::Num(_) => "1x1 double".into(),
        Value::Mat(m) => format!("{}x{} double", m.rows(), m.cols()),
        Value::Cplx(m) => format!("{}x{} complex", m.rows(), m.cols()),
        Value::Str(s) => format!("1x{} char", s.chars().count()),
        Value::Cell(c) => format!("{}x{} cell", c.shape().0, c.shape().1),
        Value::Struct(s) => format!("{}x{} struct", s.rows(), s.cols()),
        Value::Sparse(s) => format!("{}x{} sparse double", s.rows(), s.cols()),
        Value::Func(_) => "1x1 function_handle".into(),
    }
}

trait Scalar {
    fn scalar(&self) -> Result<f64, LangError>;
    fn scalar_at(&self, i: usize) -> Result<f64, LangError>;
}

impl Scalar for Value {
    fn scalar(&self) -> Result<f64, LangError> {
        match self {
            Value::Num(v) => Ok(*v),
            Value::Mat(m) => m.as_scalar().ok_or_else(|| LangError::new("expected a scalar", 0)),
            // A complex scalar reads as its real part when the imaginary
            // part is zero (`dim` args etc.); genuinely complex errors.
            Value::Cplx(m) => {
                m.as_scalar().filter(|z| z.is_real()).map(|z| z.re).ok_or_else(|| LangError::new("expected a real scalar, got a complex value", 0))
            }
            Value::Str(_) => Err(LangError::new("expected a scalar, got a string", 0)),
            Value::Cell(_) => Err(LangError::new("expected a scalar, got a cell array", 0)),
            Value::Struct(_) => Err(LangError::new("expected a scalar, got a struct", 0)),
            Value::Func(_) => Err(LangError::new("expected a scalar, got a function handle", 0)),
            // A 1x1 sparse reads as its stored value (0 when empty).
            Value::Sparse(s) => {
                if s.numel() == 1 {
                    Ok(s.get(0, 0))
                } else {
                    Err(LangError::new("expected a scalar, got a sparse matrix", 0))
                }
            }
        }
    }
    fn scalar_at(&self, _i: usize) -> Result<f64, LangError> {
        self.scalar()
    }
}

impl Scalar for Vec<Value> {
    fn scalar(&self) -> Result<f64, LangError> {
        self.scalar_at(0)
    }
    fn scalar_at(&self, i: usize) -> Result<f64, LangError> {
        self.get(i).ok_or_else(|| LangError::new(format!("missing argument {}", i + 1), 0))?.scalar()
    }
}

pub fn fmt_num(v: f64, long: bool) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Inf".into() } else { "-Inf".into() };
    }
    if v == 0.0 {
        return "0".into();
    }
    if v.fract() == 0.0 && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    let (prec, sci_at) = if long { (14, 1e12) } else { (4, 1e5) };
    if v.abs() >= sci_at || v.abs() < 1e-4 {
        return format!("{:.*e}", prec.min(4), v);
    }
    let s = format!("{:.*}", prec, v);
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    if s.is_empty() {
        "0".into()
    } else {
        s
    }
}

/// Handle text form: `@sin` or `@(x,y)x.*y` — what `disp`, `func2str`
/// and a handle's echo print.
pub fn func_text(h: &FuncHandle) -> String {
    match h {
        FuncHandle::Named(n) => format!("@{n}"),
        FuncHandle::Closure { params, body, .. } => format!("@({}){}", params.join(","), expr_text(body)),
    }
}

/// Compact M-code text for an expression — used by `func2str`/`disp` for
/// anonymous functions. Output re-parses to the same semantics (binary
/// children are parenthesized rather than relying on precedence).
fn expr_text(e: &Expr) -> String {
    match e {
        Expr::Num(v) => {
            // Shortest round-trip float repr; `format!` gives it for f64.
            format!("{v}")
        }
        Expr::CNum(v) => format!("{v}i"),
        Expr::Str(s) => format!("'{}'", s.replace('\'', "''")),
        Expr::Ident(n) => n.clone(),
        Expr::Colon => ":".into(),
        Expr::End => "end".into(),
        Expr::Range(a, step, b) => match step {
            Some(s) => format!("{}:{}:{}", pexpr(a), pexpr(s), pexpr(b)),
            None => format!("{}:{}", pexpr(a), pexpr(b)),
        },
        Expr::Matrix(rows) => {
            let rs: Vec<String> = rows.iter().map(|r| r.iter().map(expr_text).collect::<Vec<_>>().join(" ")).collect();
            format!("[{}]", rs.join("; "))
        }
        Expr::CellMat(rows) => {
            let rs: Vec<String> = rows.iter().map(|r| r.iter().map(expr_text).collect::<Vec<_>>().join(" ")).collect();
            format!("{{{}}}", rs.join("; "))
        }
        Expr::BraceIndex(n, args) => format!("{}{{{}}}", n, expr_text_args(args)),
        Expr::BraceIdx(b, args) => format!("{}{{{}}}", pexpr(b), expr_text_args(args)),
        Expr::Field(b, f) => format!("{}.{}", pexpr(b), f),
        Expr::DynField(b, n) => format!("{}.({})", pexpr(b), expr_text(n)),
        Expr::Index(b, args) => format!("{}({})", pexpr(b), expr_text_args(args)),
        Expr::CallOrIndex(n, args) => format!("{}({})", n, expr_text_args(args)),
        Expr::Unary(op, e) => {
            let o = match op {
                UnOp::Neg => "-",
                UnOp::Pos => "+",
                UnOp::Not => "~",
            };
            format!("{}{}", o, pexpr(e))
        }
        Expr::Binary(op, a, b) => {
            let o = match op {
                BinOp::Add => "+",
                BinOp::Sub => "-",
                BinOp::Mul => "*",
                BinOp::Div => "/",
                BinOp::Backslash => "\\",
                BinOp::Pow => "^",
                BinOp::ElemMul => ".*",
                BinOp::ElemDiv => "./",
                BinOp::ElemPow => ".^",
                BinOp::Eq => "==",
                BinOp::Neq => "~=",
                BinOp::Lt => "<",
                BinOp::Le => "<=",
                BinOp::Gt => ">",
                BinOp::Ge => ">=",
                BinOp::And => "&",
                BinOp::Or => "|",
                BinOp::ScAnd => "&&",
                BinOp::ScOr => "||",
            };
            format!("{} {o} {}", pexpr(a), pexpr(b))
        }
        Expr::Transpose(e, conj) => format!("{}{}", pexpr(e), if *conj { "'" } else { ".'" }),
        Expr::FuncRef(n) => format!("@{n}"),
        Expr::AnonFunc(params, body) => format!("@({}){}", params.join(","), expr_text(body)),
    }
}

fn expr_text_args(args: &[Expr]) -> String {
    args.iter().map(expr_text).collect::<Vec<_>>().join(",")
}

/// `expr_text` for a child position — non-atomic expressions get parens.
fn pexpr(e: &Expr) -> String {
    match e {
        Expr::Binary(..) | Expr::Range(..) | Expr::Unary(..) => format!("({})", expr_text(e)),
        _ => expr_text(e),
    }
}

/// `str2func`: `'name'` or `'@name'` → a named handle; `'@(args) expr'` →
/// a closure. Per M-code, a handle built from a string does NOT capture
/// the caller's workspace — the closure env starts empty.
fn str_to_func(s: &str) -> Result<FuncHandle, LangError> {
    let t = s.trim();
    let stmts = crate::parser::parse(t).map_err(|e| LangError::new(format!("str2func: {}", e.message), e.line))?;
    match stmts.as_slice() {
        [Stmt::Expr(Expr::AnonFunc(params, body), _)] => Ok(FuncHandle::Closure { params: params.clone(), body: body.clone(), env: HashMap::new() }),
        [Stmt::Expr(Expr::FuncRef(n), _)] => Ok(FuncHandle::Named(n.clone())),
        [Stmt::Expr(Expr::Ident(n), _)] if !t.starts_with('@') => Ok(FuncHandle::Named(n.clone())),
        _ => Err(LangError::new("str2func: not a valid function specification", 0)),
    }
}

/// One cell rendered compactly inside a `disp` of a cell array:
/// `[1]` for scalars, `'a'` for chars, `2x2 double` for bigger matrices.
fn fmt_cell_item(v: &Value, long: bool) -> String {
    match v {
        Value::Num(n) => format!("[{}]", fmt_num(*n, long)),
        Value::Mat(m) if m.numel() == 0 => "[]".into(),
        Value::Mat(m) if m.numel() == 1 => format!("[{}]", fmt_num(m.get_linear(0).unwrap_or(0.0), long)),
        Value::Mat(m) => format!("{}x{} double", m.rows(), m.cols()),
        Value::Cplx(m) if m.numel() == 1 => {
            let z = m.get_linear(0).unwrap_or(CNum::real(0.0));
            format!("[{} + {}i]", fmt_num(z.re, long), fmt_num(z.im, long))
        }
        Value::Cplx(m) => format!("{}x{} complex", m.rows(), m.cols()),
        Value::Str(s) => format!("'{s}'"),
        Value::Cell(c) => format!("{}x{} cell", c.shape().0, c.shape().1),
        Value::Struct(s) => format!("{}x{} struct", s.rows(), s.cols()),
        Value::Sparse(s) => format!("{}x{} sparse double", s.rows(), s.cols()),
        Value::Func(h) => func_text(h),
    }
}

pub fn fmt_value(v: &Value, long: bool) -> impl fmt::Display + '_ {
    struct F<'a>(&'a Value, bool);
    impl fmt::Display for F<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self.0 {
                Value::Num(v) => write!(f, "   {}", fmt_num(*v, self.1)),
                Value::Cplx(m) => {
                    // MATLAB prints `a + bi` per element.
                    for r in 0..m.rows() {
                        write!(f, "   ")?;
                        for c in 0..m.cols() {
                            let z = m.get(r, c).unwrap_or(CNum::real(0.0));
                            let im = if z.im < 0.0 || z.im.is_sign_negative() {
                                format!("{} - {}i", fmt_num(z.re, self.1), fmt_num(-z.im, self.1))
                            } else {
                                format!("{} + {}i", fmt_num(z.re, self.1), fmt_num(z.im, self.1))
                            };
                            write!(f, "{:>20}", im)?;
                        }
                        writeln!(f)?;
                    }
                    Ok(())
                }
                Value::Str(s) => write!(f, "{s}"),
                Value::Func(h) => write!(f, "   {}", func_text(h)),
                Value::Sparse(s) => {
                    // MATLAB's disp form for sparse: a (row, col) value list.
                    if s.nnz() == 0 {
                        return writeln!(f, "   All zero sparse: {}x{}", s.rows(), s.cols());
                    }
                    for ((r, c), v) in s.entries() {
                        writeln!(f, "   ({},{}) {:>12}", r + 1, c + 1, fmt_num(v, self.1))?;
                    }
                    Ok(())
                }
                Value::Mat(m) => {
                    for r in 0..m.rows() {
                        write!(f, "   ")?;
                        for c in 0..m.cols() {
                            let v = m.get(r, c).unwrap_or(0.0);
                            write!(f, "{:>12}", fmt_num(v, self.1))?;
                        }
                        writeln!(f)?;
                    }
                    Ok(())
                }
                Value::Cell(ca) => {
                    // MATLAB's disp form: `{'a'}    {[1]}    [2x2 double]`.
                    let (rows, cols) = ca.shape();
                    for r in 0..rows {
                        write!(f, "   ")?;
                        for c in 0..cols {
                            let item = match ca.get(r, c) {
                                Some(v) => fmt_cell_item(v, self.1),
                                None => "[]".to_string(),
                            };
                            write!(f, "{item:<20}")?;
                        }
                        writeln!(f)?;
                    }
                    Ok(())
                }
                Value::Struct(m) => {
                    // MATLAB's disp form: a scalar struct prints each field
                    // line; an array prints `NxM struct array with fields:`.
                    if m.field_names().is_empty() {
                        return write!(f, "   struct with no fields.");
                    }
                    if !m.is_scalar() {
                        writeln!(f, "   {}x{} struct array with fields:", m.rows(), m.cols())?;
                        for name in m.field_names() {
                            writeln!(f, "       {name}")?;
                        }
                        return Ok(());
                    }
                    for name in m.field_names() {
                        let val = m.field_at(0, &name).cloned().unwrap_or(Value::Mat(Matrix::zeros(0, 0)));
                        let item = fmt_cell_item(&val, self.1);
                        writeln!(f, "   {name}: {item}")?;
                    }
                    Ok(())
                }
            }
        }
    }
    F(v, long)
}

/// One argument to a `*printf` conversion, in order.
#[derive(Clone)]
enum FmtArg {
    Num(f64),
    Txt(String),
}

fn split_format_args(v: &[Value]) -> Result<(String, Vec<FmtArg>), LangError> {
    let fmt = match v.first() {
        Some(Value::Str(s)) => s.clone(),
        Some(Value::Num(_)) => return Err(LangError::new("first argument must be a format string", 0)),
        _ => return Err(LangError::new("fprintf needs a format string", 0)),
    };
    let mut args: Vec<FmtArg> = Vec::new();
    for a in v.iter().skip(1) {
        match a {
            Value::Num(n) => args.push(FmtArg::Num(*n)),
            Value::Mat(m) => args.extend(m.data().iter().map(|&x| FmtArg::Num(x))),
            // `%f` on complex prints the real part (M-code behaviour).
            Value::Cplx(m) => args.extend(m.data().iter().map(|z| FmtArg::Num(z.re))),
            Value::Str(s) => args.push(FmtArg::Txt(s.clone())),
            Value::Cell(c) => args.push(FmtArg::Txt(format!("{}x{} cell", c.shape().0, c.shape().1))),
            Value::Struct(s) => args.push(FmtArg::Txt(format!("{}x{} struct", s.rows(), s.cols()))),
            Value::Sparse(s) => args.extend(s.full().data().iter().map(|&x| FmtArg::Num(x))),
            Value::Func(h) => args.push(FmtArg::Txt(func_text(h))),
        }
    }
    Ok((fmt, args))
}

/// `*printf` formatting: `%d %i %f %e %g %s %c %x %%`, `\n \t \\` escapes,
/// width/precision (`%8.2f`, `%-12s`), args consumed cyclically like M-code.
fn sprintf_impl(fmt: &str, args: &[FmtArg]) -> Result<String, LangError> {
    let chars: Vec<char> = fmt.chars().collect();
    let mut out = String::new();
    let mut i = 0usize;
    let mut arg_i = 0usize;
    // The format is re-scanned until all args are consumed (M-code cycling).
    loop {
        let scanned = arg_i;
        while i < chars.len() {
            let c = chars[i];
            i += 1;
            match c {
                '\\' => match chars.get(i) {
                    Some('n') => {
                        out.push('\n');
                        i += 1;
                    }
                    Some('t') => {
                        out.push('\t');
                        i += 1;
                    }
                    Some('\\') => {
                        out.push('\\');
                        i += 1;
                    }
                    _ => out.push(c),
                },
                '%' => {
                    let (text, ni, consumed) = format_one(&chars, i, args, arg_i)?;
                    i = ni;
                    if consumed {
                        arg_i += 1;
                    }
                    out.push_str(&text);
                }
                _ => out.push(c),
            }
        }
        // Restart the format if args remain and conversions happened.
        if arg_i >= args.len() || arg_i == scanned {
            break;
        }
        i = 0;
    }
    Ok(out)
}

/// Format one `%` conversion; returns (text, next index, whether an arg was
/// consumed).
fn format_one(chars: &[char], start: usize, args: &[FmtArg], arg_i: usize) -> Result<(String, usize, bool), LangError> {
    let mut i = start;
    let mut flags = String::new();
    while i < chars.len() && matches!(chars[i], '-' | '+' | ' ' | '0' | '#') {
        flags.push(chars[i]);
        i += 1;
    }
    let mut width_s = String::new();
    while i < chars.len() && chars[i].is_ascii_digit() {
        width_s.push(chars[i]);
        i += 1;
    }
    let mut prec: Option<usize> = None;
    if chars.get(i) == Some(&'.') {
        i += 1;
        let mut p = String::new();
        while i < chars.len() && chars[i].is_ascii_digit() {
            p.push(chars[i]);
            i += 1;
        }
        prec = p.parse::<usize>().ok();
    }
    let spec = chars.get(i).copied().unwrap_or('\0');
    if spec == '\0' {
        return Ok(("%".to_string(), i, false));
    }
    i += 1;
    if spec == '%' {
        return Ok(("%".to_string(), i, false));
    }
    let arg = args.get(arg_i).ok_or_else(|| LangError::new("not enough arguments for format", 0))?;
    let width: usize = width_s.parse().unwrap_or(0);
    let left = flags.contains('-');
    let pad = |s: String| -> String {
        if s.len() >= width {
            return s;
        }
        let pad = " ".repeat(width - s.len());
        if left {
            format!("{s}{pad}")
        } else {
            format!("{pad}{s}")
        }
    };
    let num = |fallback: f64| -> f64 {
        match arg {
            FmtArg::Num(n) => *n,
            FmtArg::Txt(_) => fallback,
        }
    };
    let txt = || -> String {
        match arg {
            FmtArg::Txt(s) => s.clone(),
            FmtArg::Num(n) => fmt_num(*n, false),
        }
    };
    let body = match spec {
        'd' | 'i' | 'u' => pad(format!("{}", num(0.0).round() as i64)),
        'f' | 'F' => pad(format!("{:.*}", prec.unwrap_or(6), num(0.0))),
        'e' | 'E' => pad(format!("{:.*e}", prec.unwrap_or(6), num(0.0))),
        'g' | 'G' => pad(fmt_num(num(0.0), false)),
        'x' => pad(format!("{:x}", num(0.0).round() as i64)),
        'X' => pad(format!("{:X}", num(0.0).round() as i64)),
        's' => pad(txt()),
        'c' => match arg {
            FmtArg::Txt(s) => pad(s.chars().next().unwrap_or(' ').to_string()),
            FmtArg::Num(n) => pad(char::from_u32(n.round() as u32).unwrap_or(' ').to_string()),
        },
        other => return Err(LangError::new(format!("unsupported conversion '%{other}'"), 0)),
    };
    Ok((body, i, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> String {
        let mut s = Session::new();
        s.run(src).unwrap()
    }

    #[test]
    fn arithmetic() {
        assert!(run("x = 1 + 2 * 3").contains("x =\n\n   7"));
    }

    #[test]
    fn matrix_build_and_index() {
        let mut s = Session::new();
        s.run("A = [1 2; 3 4]; y = A(2,1)").unwrap();
        assert_eq!(s.get("y"), Some(&Value::Num(3.0)));
    }

    #[test]
    fn mldivide() {
        let mut s = Session::new();
        s.run("A = [2 1; 1 3]; b = [5; 5]; x = A\\b").unwrap();
        match s.get("x") {
            Some(Value::Mat(m)) => {
                assert!((m.get(0, 0).unwrap() - 2.0).abs() < 1e-9);
                assert!((m.get(1, 0).unwrap() - 1.0).abs() < 1e-9);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn for_loop_sums() {
        let mut s = Session::new();
        s.run("s = 0; for k = 1:5\ns = s + k;\nend").unwrap();
        assert_eq!(s.get("s"), Some(&Value::Num(15.0)));
    }

    #[test]
    fn while_loop() {
        let mut s = Session::new();
        s.run("n = 1; while n < 100\nn = n * 2;\nend").unwrap();
        assert_eq!(s.get("n"), Some(&Value::Num(128.0)));
    }

    #[test]
    fn indexed_growth() {
        let mut s = Session::new();
        s.run("A(3) = 9;").unwrap();
        match s.get("A") {
            Some(Value::Mat(m)) => {
                assert_eq!(m.numel(), 3);
                assert_eq!(m.get(2, 0), Some(9.0));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn magic_square_sums() {
        for (src, want) in [("M = magic(3)", 15.0), ("M = magic(4)", 34.0), ("M = magic(5)", 65.0), ("M = magic(6)", 111.0), ("M = magic(10)", 505.0)]
        {
            let mut s = Session::new();
            s.run(src).unwrap();
            let Some(Value::Mat(m)) = s.get("M") else { panic!("expected a matrix") };
            let n = m.rows();
            assert_eq!(n, m.cols());
            for i in 0..n {
                let mut row = 0.0;
                let mut col = 0.0;
                for j in 0..n {
                    row += m.get(i, j).unwrap();
                    col += m.get(j, i).unwrap();
                }
                assert_eq!(row, want);
                assert_eq!(col, want);
            }
            let d1: f64 = (0..n).map(|i| m.get(i, i).unwrap()).sum();
            let d2: f64 = (0..n).map(|i| m.get(i, n - 1 - i).unwrap()).sum();
            assert_eq!(d1, want);
            assert_eq!(d2, want);
        }
        // magic(3) uses the Siamese arrangement.
        let mut s = Session::new();
        s.run("M = magic(3)").unwrap();
        let Some(Value::Mat(m)) = s.get("M") else { panic!("expected a matrix") };
        let want = [8.0, 1.0, 6.0, 3.0, 5.0, 7.0, 4.0, 9.0, 2.0];
        for r in 0..3 {
            for c in 0..3 {
                assert_eq!(m.get(r, c), Some(want[r * 3 + c]));
            }
        }
        // Order 2 has no magic square — the conventional placeholder.
        let mut s = Session::new();
        s.run("M = magic(2)").unwrap();
        let Some(Value::Mat(m)) = s.get("M") else { panic!("expected a matrix") };
        assert_eq!(m.get(0, 0), Some(1.0));
        assert_eq!(m.get(0, 1), Some(3.0));
        assert_eq!(m.get(1, 0), Some(4.0));
        assert_eq!(m.get(1, 1), Some(2.0));
    }

    #[test]
    fn disp_and_semicolon() {
        let out = run("x = 5; disp(x * 2)");
        assert!(out.contains("10"));
        assert!(!out.contains("ans"));
    }

    #[test]
    fn comparisons_and_if() {
        let mut s = Session::new();
        s.run("x = 3; if x > 2\nr = 1;\nelse\nr = 0;\nend").unwrap();
        assert_eq!(s.get("r"), Some(&Value::Num(1.0)));
    }

    #[test]
    fn break_and_continue() {
        let mut s = Session::new();
        s.run("s = 0; for k = 1:10\nif k == 5\ncontinue\nend\nif k > 8\nbreak\nend\ns = s + k;\nend").unwrap();
        // sums 1+2+3+4+6+7+8 (skips 5, breaks before 9 and 10) = 31
        assert_eq!(s.get("s"), Some(&Value::Num(31.0)));
    }

    #[test]
    fn return_stops_script() {
        let mut s = Session::new();
        s.run("a = 1; return; a = 2").unwrap();
        assert_eq!(s.get("a"), Some(&Value::Num(1.0)));
    }

    #[test]
    fn switch_cases() {
        let mut s = Session::new();
        s.run("x = 2; switch x\ncase 1\nr = 10;\ncase 2\nr = 20;\notherwise\nr = 0;\nend").unwrap();
        assert_eq!(s.get("r"), Some(&Value::Num(20.0)));
    }

    #[test]
    fn logical_operators() {
        let mut s = Session::new();
        s.run("a = 1 && 0; b = 0 || 1; c = [1 0] & [1 1]; d = c | 0").unwrap();
        assert_eq!(s.get("a"), Some(&Value::Num(0.0)));
        assert_eq!(s.get("b"), Some(&Value::Num(1.0)));
        match s.get("d") {
            Some(Value::Mat(m)) => assert_eq!(m.data(), &[1.0, 0.0]),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn short_circuit_skips_side_effects() {
        let mut s = Session::new();
        // If short-circuiting didn't work, these would error on the
        // unknown function instead of returning 0/1.
        s.run("r = 0 && undefinedfn(1); q = 1 || undefinedfn(2)").unwrap();
        assert_eq!(s.get("r"), Some(&Value::Num(0.0)));
        assert_eq!(s.get("q"), Some(&Value::Num(1.0)));
    }

    #[test]
    fn command_syntax() {
        let mut s = Session::new();
        s.run("x = 1; clear x; who").unwrap();
        assert!(s.get("x").is_none());
    }

    #[test]
    fn sprintf_formats() {
        let mut s = Session::new();
        s.run("t = sprintf('%d plus %.2f is %s\\n', 3, 0.5, 'x')").unwrap();
        assert_eq!(s.get("t"), Some(&Value::Str("3 plus 0.50 is x\n".into())));
    }

    #[test]
    fn format_long() {
        let mut s = Session::new();
        let out = s.run("format long; x = 1/3").unwrap();
        assert!(out.contains("0.33333333333333"));
        let out2 = s.run("format short; y = 1/3").unwrap();
        assert!(out2.contains("0.3333"));
    }

    #[test]
    fn function_basic() {
        let mut s = Session::new();
        s.run("r = double_it(21)\nfunction y = double_it(x)\ny = 2 * x;\nend").unwrap();
        assert_eq!(s.get("r"), Some(&Value::Num(42.0)));
    }

    #[test]
    fn function_multi_output() {
        let mut s = Session::new();
        s.run("[lo, hi] = minmax([3 1 4])\nfunction [a, b] = minmax(v)\na = min(v); b = max(v);\nend").unwrap();
        assert_eq!(s.get("lo"), Some(&Value::Num(1.0)));
        assert_eq!(s.get("hi"), Some(&Value::Num(4.0)));
    }

    #[test]
    fn function_local_scope() {
        let mut s = Session::new();
        s.run("x = 1; leak()\nx\nfunction leak()\nx = 99; t = 7;\nend").unwrap();
        // The function's `x` and `t` must not escape into the workspace.
        assert_eq!(s.get("x"), Some(&Value::Num(1.0)));
        assert!(s.get("t").is_none());
    }

    #[test]
    fn function_recursion_and_return() {
        let mut s = Session::new();
        s.run("r = fact(5)\nfunction r = fact(n)\nif n <= 1\nr = 1; return;\nend\nr = n * fact(n - 1);\nend").unwrap();
        assert_eq!(s.get("r"), Some(&Value::Num(120.0)));
    }

    #[test]
    fn function_nargin() {
        let mut s = Session::new();
        s.run("r = pick(10)\nfunction r = pick(a, b)\nif nargin < 2\nr = a;\nelse\nr = b;\nend\nend").unwrap();
        assert_eq!(s.get("r"), Some(&Value::Num(10.0)));
    }

    #[test]
    fn function_file_loading() {
        let dir = std::env::temp_dir().join(format!("matcraft_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let fpath = dir.join("triple.m");
        std::fs::write(&fpath, "function y = triple(x)\ny = 3 * x;\nend\n").unwrap();
        let mut s = Session::new();
        s.add_path(&dir);
        s.run("r = triple(7)").unwrap();
        assert_eq!(s.get("r"), Some(&Value::Num(21.0)));
        let _ = std::fs::remove_file(&fpath);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn plot_creates_figure_and_saves() {
        let dir = std::env::temp_dir().join(format!("matcraft_plot_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("out.svg").to_string_lossy().replace('\\', "/");
        let mut s = Session::new();
        let out = s
            .run(&format!(
                "plot(0:3, [0 1 0 1], 'r--o'); title('demo'); xlabel('t'); \
legend('sig'); grid on; saveas('{f}')"
            ))
            .unwrap();
        assert!(out.contains("wrote"));
        let svg = std::fs::read_to_string(&f).unwrap();
        assert!(svg.contains("<svg") && svg.contains("demo") && svg.contains("sig"));
        let _ = std::fs::remove_file(&f);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn plot_hold_and_replacement() {
        let mut s = Session::new();
        s.run("plot(1:3); hold on; plot(4:6); hold off; plot(7:9)").unwrap();
        let fig = &s.figs[0];
        // hold off ⇒ the last plot call replaced everything: one series.
        assert_eq!(fig.series.len(), 1);
    }

    #[test]
    fn plot_y_matrix_expands_to_series() {
        let mut s = Session::new();
        s.run("plot([1 2; 3 4])").unwrap();
        assert_eq!(s.figs[0].series.len(), 2);
    }

    #[test]
    fn figure_and_gcf() {
        let mut s = Session::new();
        s.run("f = figure; figure(3); g = gcf").unwrap();
        assert_eq!(s.get("f"), Some(&Value::Num(1.0)));
        assert_eq!(s.get("g"), Some(&Value::Num(3.0)));
    }

    #[test]
    fn subplot_tiles_keep_state() {
        let mut s = Session::new();
        s.run(
            "x = 1:4; subplot(2,1,1); plot(x); title('top'); \
              subplot(2,1,2); stem(x); title('bot')",
        )
        .unwrap();
        let fig = &s.figs[0];
        let sub = fig.sub.as_ref().unwrap();
        assert_eq!((sub.rows, sub.cols), (2, 1));
        // One stashed tile (tile 1) plus the live axes (tile 2).
        assert_eq!(sub.tiles.len(), 1);
        assert_eq!(sub.tiles[0].axes.title.as_deref(), Some("top"));
        assert_eq!(fig.title.as_deref(), Some("bot"));
        assert_eq!(fig.series.len(), 1);
        // SVG: two axes, both titles.
        let svg = matplot::to_svg(fig);
        assert!(svg.contains("ax0") && svg.contains("ax1"));
        assert!(svg.contains("top") && svg.contains("bot"));
    }

    #[test]
    fn subplot_packed_and_span() {
        let mut s = Session::new();
        // Packed mnp form and a multi-cell span.
        s.run("subplot(211); plot(1:3); subplot(2,2,[3 4]); plot(1:3)").unwrap();
        let sub = s.figs[0].sub.as_ref().unwrap();
        assert_eq!((sub.rows, sub.cols), (2, 2));
        assert_eq!(sub.cur_rect, (1, 0, 1, 1));
        // Revisiting a cell restores its axes.
        s.run("subplot(2,2,1); title('one'); subplot(2,2,3); subplot(2,2,1)").unwrap();
        let sub = s.figs[0].sub.as_ref().unwrap();
        // `subplot(2,2,1)` visited twice; its axes (titled 'one') is live again.
        assert_eq!(s.figs[0].title.as_deref(), Some("one"));
        assert_eq!(sub.cur_rect, (0, 0, 0, 0));
    }

    // ---- language growth batch -------------------------------------------

    #[test]
    fn matrix_literal_negative_elements() {
        // `[0 -2 5]` is three elements (space before `-`, none after);
        // `[0 - 2]` and `[0-2]` are one (binary minus).
        let mut s = Session::new();
        s.run("a = [0 -2 5]; b = [0 - 2]; c = [0-2]; d = [0 -2 5; 0 3 0]").unwrap();
        match s.get("a") {
            Some(Value::Mat(m)) => assert_eq!(m.shape(), (1, 3)),
            other => panic!("unexpected {other:?}"),
        }
        // Matrix literals keep `Mat` type even when 1×1.
        match (s.get("b"), s.get("c")) {
            (Some(Value::Mat(b)), Some(Value::Mat(c))) => {
                assert_eq!(b.data(), &[-2.0]);
                assert_eq!(c.data(), &[-2.0]);
            }
            other => panic!("unexpected {other:?}"),
        }
        match s.get("d") {
            Some(Value::Mat(m)) => assert_eq!(m.shape(), (2, 3)),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn logical_indexing() {
        let mut s = Session::new();
        s.run("B = [0 -2 5; 0 3 0]; v = B(B > 0); B(B < 0) = 9").unwrap();
        match s.get("v") {
            Some(Value::Mat(m)) => assert_eq!(m.data(), &[3.0, 5.0]),
            other => panic!("unexpected {other:?}"),
        }
        match s.get("B") {
            Some(Value::Mat(m)) => assert_eq!(m.get(0, 1), Some(9.0)),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn reductions_dim_aware() {
        let mut s = Session::new();
        s.run("A = [1 2; 3 4]; c = sum(A); r = sum(A, 2); t = sum(A, 'all'); m = mean(A); mx = max(A, [], 2)").unwrap();
        match s.get("c") {
            Some(Value::Mat(m)) => assert_eq!(m.data(), &[4.0, 6.0]),
            other => panic!("unexpected {other:?}"),
        }
        match s.get("r") {
            Some(Value::Mat(m)) => assert_eq!(m.data(), &[3.0, 7.0]),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(s.get("t"), Some(&Value::Num(10.0)));
        match s.get("m") {
            Some(Value::Mat(m)) => assert_eq!(m.data(), &[2.0, 3.0]),
            other => panic!("unexpected {other:?}"),
        }
        match s.get("mx") {
            Some(Value::Mat(m)) => assert_eq!(m.data(), &[2.0, 4.0]),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn minmax_two_outputs() {
        let mut s = Session::new();
        s.run("[M, I] = max([3 1 4]); [m, i] = min([3 1 4])").unwrap();
        assert_eq!(s.get("M"), Some(&Value::Num(4.0)));
        assert_eq!(s.get("I"), Some(&Value::Num(3.0)));
        assert_eq!(s.get("m"), Some(&Value::Num(1.0)));
        assert_eq!(s.get("i"), Some(&Value::Num(2.0)));
    }

    #[test]
    fn size_two_outputs() {
        let mut s = Session::new();
        s.run("A = zeros(2, 3); [r, c] = size(A)").unwrap();
        assert_eq!(s.get("r"), Some(&Value::Num(2.0)));
        assert_eq!(s.get("c"), Some(&Value::Num(3.0)));
    }

    #[test]
    fn meshgrid_outputs() {
        let mut s = Session::new();
        s.run("[X, Y] = meshgrid(1:3, 10:11)").unwrap();
        match s.get("X") {
            Some(Value::Mat(m)) => {
                assert_eq!(m.shape(), (2, 3));
                assert_eq!(m.get(0, 2), Some(3.0));
            }
            other => panic!("unexpected {other:?}"),
        }
        match s.get("Y") {
            Some(Value::Mat(m)) => {
                assert_eq!(m.shape(), (2, 3));
                assert_eq!(m.get(1, 0), Some(11.0));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn linalg_builtins() {
        let mut s = Session::new();
        s.run(
            "A = [4 2; 1 3]; w = eig(A); [V, D] = eig(A); E = A*V - V*D; [Q, R] = qr(A); Qm = Q*R; \
               s = svd(A); r = rank(A); c = cond(A); t = trace(A); rr = rref([1 2 3; 4 5 6]); U = chol([4 2; 2 3])",
        )
        .unwrap();
        match s.get("w") {
            Some(Value::Mat(m)) => {
                assert!((m.get(0, 0).unwrap() - 2.0).abs() < 1e-8);
                assert!((m.get(1, 0).unwrap() - 5.0).abs() < 1e-8);
            }
            other => panic!("unexpected {other:?}"),
        }
        match s.get("E") {
            Some(Value::Mat(m)) => assert!(m.data().iter().all(|v| v.abs() < 1e-8)),
            other => panic!("unexpected {other:?}"),
        }
        // Q*R reconstructs A.
        match (s.get("Qm"), s.get("A")) {
            (Some(Value::Mat(qr)), Some(Value::Mat(a))) => {
                assert!(qr.data().iter().zip(a.data()).all(|(x, y)| (x - y).abs() < 1e-8));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(s.get("r"), Some(&Value::Num(2.0)));
        match s.get("c") {
            // σ(A)² = eig(AᵀA) = eig([17 11; 11 13]) = 15±5√5 → cond = 2.618.
            Some(Value::Num(c)) => assert!((c - 2.618).abs() < 1e-3),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(s.get("t"), Some(&Value::Num(7.0)));
        match s.get("rr") {
            Some(Value::Mat(m)) => {
                assert_eq!(m.get(0, 2), Some(-1.0));
                assert_eq!(m.get(1, 2), Some(2.0));
            }
            other => panic!("unexpected {other:?}"),
        }
        // chol([4 2; 2 3]) = [2 1; 0 sqrt(2)].
        match s.get("U") {
            Some(Value::Mat(m)) => {
                assert!((m.get(0, 0).unwrap() - 2.0).abs() < 1e-9);
                assert!((m.get(1, 1).unwrap() - 2f64.sqrt()).abs() < 1e-9);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn stats_and_running_builtins() {
        let mut s = Session::new();
        s.run(
            "v = [2 4 4 4 5 5 7 9]; sd = std(v); md = median([1 3 2]); cs = cumsum([1 2 3]); \
               df = diff([1 4 9 16]); srt = sort([3 1 2], 'descend'); u = unique([2 1 2 3 1])",
        )
        .unwrap();
        match s.get("sd") {
            Some(Value::Num(v)) => assert!((v - 2.1381).abs() < 1e-3),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(s.get("md"), Some(&Value::Num(2.0)));
        match s.get("cs") {
            Some(Value::Mat(m)) => assert_eq!(m.data(), &[1.0, 3.0, 6.0]),
            other => panic!("unexpected {other:?}"),
        }
        match s.get("df") {
            Some(Value::Mat(m)) => assert_eq!(m.data(), &[3.0, 5.0, 7.0]),
            other => panic!("unexpected {other:?}"),
        }
        match s.get("srt") {
            Some(Value::Mat(m)) => assert_eq!(m.data(), &[3.0, 2.0, 1.0]),
            other => panic!("unexpected {other:?}"),
        }
        match s.get("u") {
            Some(Value::Mat(m)) => assert_eq!(m.data(), &[1.0, 2.0, 3.0]),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn varargin_and_varargout() {
        let mut s = Session::new();
        s.run("s = mysum(1, 2, 3)\nfunction s = mysum(a, varargin)\ns = a;\nfor k = 1:numel(varargin)\ns = s + varargin{k};\nend\nend").unwrap();
        assert_eq!(s.get("s"), Some(&Value::Num(6.0)));
        let mut s = Session::new();
        s.run("[lo, hi, mu] = spread([4 1 7])\nfunction varargout = spread(v)\nvarargout = [min(v) max(v) mean(v)];\nend").unwrap();
        assert_eq!(s.get("lo"), Some(&Value::Num(1.0)));
        assert_eq!(s.get("hi"), Some(&Value::Num(7.0)));
        assert_eq!(s.get("mu"), Some(&Value::Num(4.0)));
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("matcraft_save_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("ws.mat").to_string_lossy().replace('\\', "/");
        let mut s = Session::new();
        s.run(&format!("a = 42; M = [1 2; 3 4]; t = 'hi there'; save '{f}'; clear; load '{f}'")).unwrap();
        assert_eq!(s.get("a"), Some(&Value::Num(42.0)));
        match s.get("M") {
            Some(Value::Mat(m)) => assert_eq!(m.data(), &[1.0, 3.0, 2.0, 4.0]),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(s.get("t"), Some(&Value::Str("hi there".into())));
        let _ = std::fs::remove_file(&f);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn tic_toc() {
        let mut s = Session::new();
        s.run("tic; t = toc").unwrap();
        match s.get("t") {
            Some(Value::Num(v)) => assert!(*v >= 0.0),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn solvers_and_fit() {
        let mut s = Session::new();
        s.run(
            "r = fzero('cos', 1); q = integral('sin', 0, pi); p = polyfit([0 1 2 3], [1 3 5 7], 1); \
               pv = polyval([1 -3 2], 5); ix = interp1([0 1 2], [0 10 20], [0.5 1.5]); tz = trapz([0 1 2], [0 1 4])",
        )
        .unwrap();
        match s.get("r") {
            Some(Value::Num(v)) => assert!((v - std::f64::consts::FRAC_PI_2).abs() < 1e-6),
            other => panic!("unexpected {other:?}"),
        }
        match s.get("q") {
            Some(Value::Num(v)) => assert!((v - 2.0).abs() < 1e-6),
            other => panic!("unexpected {other:?}"),
        }
        match s.get("p") {
            Some(Value::Mat(m)) => {
                assert!((m.get(0, 0).unwrap() - 2.0).abs() < 1e-8);
                assert!((m.get(0, 1).unwrap() - 1.0).abs() < 1e-8);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(s.get("pv"), Some(&Value::Num(12.0)));
        match s.get("ix") {
            Some(Value::Mat(m)) => assert_eq!(m.data(), &[5.0, 15.0]),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(s.get("tz"), Some(&Value::Num(3.0)));
    }

    #[test]
    fn string_builtins() {
        let mut s = Session::new();
        s.run(
            "a = strcmp('x', 'x'); b = strcmpi('A', 'a'); c = upper('hi'); d = strrep('aa bb', 'bb', 'cc'); \
               e = strfind('hello world', 'o'); f = str2double('3.5'); g = strcat('a', 'b'); h = strtrim('  x  ')",
        )
        .unwrap();
        assert_eq!(s.get("a"), Some(&Value::Num(1.0)));
        assert_eq!(s.get("b"), Some(&Value::Num(1.0)));
        assert_eq!(s.get("c"), Some(&Value::Str("HI".into())));
        assert_eq!(s.get("d"), Some(&Value::Str("aa cc".into())));
        match s.get("e") {
            Some(Value::Mat(m)) => assert_eq!(m.data(), &[5.0, 8.0]),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(s.get("f"), Some(&Value::Num(3.5)));
        assert_eq!(s.get("g"), Some(&Value::Str("ab".into())));
        assert_eq!(s.get("h"), Some(&Value::Str("x".into())));
    }

    #[test]
    fn exist_and_which() {
        let mut s = Session::new();
        s.run("x = 1; a = exist('x'); b = exist('nosuchvar'); c = exist('sin')").unwrap();
        assert_eq!(s.get("a"), Some(&Value::Num(1.0)));
        assert_eq!(s.get("b"), Some(&Value::Num(0.0)));
        assert_eq!(s.get("c"), Some(&Value::Num(5.0)));
    }

    #[test]
    fn new_plot_verbs() {
        let mut s = Session::new();
        s.run(
            "stairs([0 1 2], [0 1 0]); clf; area([0 1 2], [1 2 1]); clf; \
               errorbar([1 2 3], [1 4 2], [0.1 0.2 0.3]); clf; histogram([1 1 2 3], 3); clf; loglog([1 10 100], [1 10 100])",
        )
        .unwrap();
        use matplot::SeriesKind;
        assert_eq!(s.figs[0].series.len(), 1);
        assert_eq!(s.figs[0].series[0].kind, SeriesKind::Line);
        assert!(s.figs[0].xlog && s.figs[0].ylog);
        // The SVG must render without error for every new kind.
        s.run("clf; stairs([0 1 2], [0 1 0]); area([0 1 2], [1 2 1]); errorbar([1 2], [1 2], [0.1 0.2])").unwrap();
        let svg = matplot::to_svg(&s.figs[0]);
        assert!(svg.contains("<svg"));
    }

    #[test]
    fn plotting_breadth() {
        use matplot::SeriesKind;
        let mut s = Session::new();
        s.run("imagesc([1 9; 4 2]); colorbar").unwrap();
        let fig = &s.figs[0];
        assert_eq!(fig.series[0].kind, SeriesKind::Image);
        assert_eq!(fig.series[0].zshape, (2, 2));
        assert!(fig.yrev && fig.colorbar);
        let svg = matplot::to_svg(fig);
        assert!(svg.contains("<svg") && svg.contains("linearGradient"));

        s.run("clf; pie([3 1 2], {'a','b','c'})").unwrap();
        assert_eq!(s.figs[0].series[0].kind, SeriesKind::Pie);
        assert_eq!(s.figs[0].series[0].labels.len(), 3);
        assert!(matplot::to_svg(&s.figs[0]).contains("<path"));

        s.run("clf; [X,Y]=meshgrid(-2:2,-2:2); contour(X.^2+Y.^2, 3)").unwrap();
        let fig = &s.figs[0];
        assert_eq!(fig.series[0].kind, SeriesKind::Contour);
        assert_eq!(fig.series[0].levels.len(), 3);
        assert!(matplot::to_svg(fig).contains("<path"));

        s.run("clf; [X,Y]=meshgrid(-1:1,-1:1); surf(X,Y,X.^2+Y.^2)").unwrap();
        let fig = &s.figs[0];
        assert_eq!(fig.series[0].kind, SeriesKind::Surf);
        assert_eq!(fig.series[0].zshape, (3, 3));
        assert!(matplot::to_svg(fig).contains("<path"));

        s.run("clf; mesh(-1:1, -1:1, [1 2 3; 4 5 6; 7 8 9])").unwrap();
        assert_eq!(s.figs[0].series[0].kind, SeriesKind::Mesh);

        s.run("clf; t = 0:0.5:6; plot3(sin(t), cos(t), t)").unwrap();
        assert_eq!(s.figs[0].series[0].kind, SeriesKind::Line3);
        assert!(matplot::to_svg(&s.figs[0]).contains("<path"));

        // view(2)/view(3) presets and the two-vector form.
        s.run("v0 = view; view(2); v2 = view; view(-37.5, 30); view([10 20]); v3 = view").unwrap();
        if let Some(Value::Mat(m)) = s.get("v0") {
            assert_eq!(m.data(), &[-37.5, 30.0]);
        } else {
            panic!("view did not return a vector");
        }
        if let Some(Value::Mat(m)) = s.get("v2") {
            assert_eq!(m.data(), &[0.0, 90.0]);
        } else {
            panic!("view(2) failed");
        }
        if let Some(Value::Mat(m)) = s.get("v3") {
            assert_eq!(m.data(), &[10.0, 20.0]);
        } else {
            panic!("view([10 20]) failed");
        }

        s.run("zlim([0 5]); zlabel('h'); colormap('jet'); caxis([0 2])").unwrap();
        let fig = &s.figs[0];
        assert_eq!(fig.zlim, Some((0.0, 5.0)));
        assert_eq!(fig.zlabel.as_deref(), Some("h"));
        assert_eq!(fig.clim, Some((0.0, 2.0)));

        // axis([..]) set + query, axis ij/xy, box off.
        s.run("axis([0 10 -1 1]); v = axis; axis ij; box off").unwrap();
        if let Some(Value::Mat(m)) = s.get("v") {
            assert_eq!(m.data(), &[0.0, 10.0, -1.0, 1.0]);
        } else {
            panic!("axis query failed");
        }
        assert!(s.figs[0].yrev && !s.figs[0].boxed);
        s.run("axis xy; box on").unwrap();
        assert!(!s.figs[0].yrev && s.figs[0].boxed);
    }

    #[test]
    fn corpus_gap_builtins() {
        let mut s = Session::new();
        // Parenthesised for-header (real-world syntax).
        s.run("t = 0; for(i=1:4) t = t + i; end").unwrap();
        assert_eq!(s.get("t").unwrap().scalar().unwrap(), 10.0);

        // Cheap builtins surfaced by the GitHub corpus sweep.
        s.run("d = deg2rad(180)").unwrap();
        assert!((s.get("d").unwrap().scalar().unwrap() - std::f64::consts::PI).abs() < 1e-12);
        s.run("x = sinc(0); y = sinc(1)").unwrap();
        assert_eq!(s.get("x").unwrap().scalar().unwrap(), 1.0);
        assert!(s.get("y").unwrap().scalar().unwrap().abs() < 1e-12);
        s.run("u = uint8([-3 300 7.6])").unwrap();
        if let Some(Value::Mat(m)) = s.get("u") {
            assert_eq!(m.data(), &[0.0, 255.0, 8.0]);
        }
        s.run("p = sort(randperm(8))").unwrap();
        if let Some(Value::Mat(m)) = s.get("p") {
            assert_eq!(m.data(), &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]);
        }
        s.run("rng(5); a = rand; rng(5); b = rand").unwrap();
        assert_eq!(s.get("a").unwrap().scalar().unwrap(), s.get("b").unwrap().scalar().unwrap());
        assert!(s.run("assert(1 == 2)").is_err());
        s.run("assert(1 == 1)").unwrap();
        s.run("r = normrnd(3, 1, 2, 4)").unwrap();
        if let Some(Value::Mat(m)) = s.get("r") {
            assert_eq!(m.shape(), (2, 4));
        }
        s.run("f = fullfile('a', 'b', 'c.m')").unwrap();
        if let Some(Value::Str(f)) = s.get("f") {
            assert!(f.ends_with("c.m") && f.contains('b'));
        }
        s.run("rp = rectpuls(0.4); tp = tripuls(0.9); dp = db2pow(20)").unwrap();
        assert_eq!(s.get("rp").unwrap().scalar().unwrap(), 1.0);
        assert!((s.get("tp").unwrap().scalar().unwrap() - 0.1).abs() < 1e-12);
        assert!((s.get("dp").unwrap().scalar().unwrap() - 100.0).abs() < 1e-9);
    }

    #[test]
    fn export_png_pdf() {
        let dir = std::env::temp_dir().join(format!("matcraft_d10_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let png = dir.join("f.png").to_string_lossy().replace('\\', "/");
        let pdf = dir.join("f.pdf").to_string_lossy().replace('\\', "/");
        let mut s = Session::new();
        s.run("plot(0:3, [0 1 0 1])").unwrap();
        let out = s.run(&format!("print('-dpng', '{png}'); print('-dpdf', '{pdf}')")).unwrap();
        assert!(out.contains("wrote"));
        let png_b = std::fs::read(&png).unwrap();
        assert_eq!(&png_b[..4], &[0x89, 0x50, 0x4e, 0x47]);
        let pdf_b = std::fs::read(&pdf).unwrap();
        assert_eq!(&pdf_b[..5], b"%PDF-");
        // Extension-based dispatch via saveas.
        let png2 = dir.join("g.png").to_string_lossy().replace('\\', "/");
        s.run(&format!("saveas('{png2}')")).unwrap();
        assert_eq!(&std::fs::read(&png2).unwrap()[..4], &[0x89, 0x50, 0x4e, 0x47]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn predicates() {
        let mut s = Session::new();
        s.run("a = isscalar(5); b = isvector([1 2]); c = ischar('x'); d = isnan(nan); e = isempty([]); f = isequal([1 2], [1 2])").unwrap();
        assert_eq!(s.get("a"), Some(&Value::Num(1.0)));
        assert_eq!(s.get("b"), Some(&Value::Num(1.0)));
        assert_eq!(s.get("c"), Some(&Value::Num(1.0)));
        assert_eq!(s.get("d"), Some(&Value::Num(1.0)));
        assert_eq!(s.get("e"), Some(&Value::Num(1.0)));
        assert_eq!(s.get("f"), Some(&Value::Num(1.0)));
    }

    fn cell_of(s: &Session, name: &str) -> CellArray {
        match s.get(name) {
            Some(Value::Cell(c)) => c.clone(),
            other => panic!("{name} is not a cell: {other:?}"),
        }
    }

    #[test]
    fn cell_literal_and_brace_index() {
        let mut s = Session::new();
        s.run("c = {1, 'two', [3 4]}; a = c{1}; b = c{2}; m = c{3};").unwrap();
        assert_eq!(s.get("a"), Some(&Value::Num(1.0)));
        assert_eq!(s.get("b"), Some(&Value::Str("two".into())));
        match s.get("m") {
            Some(Value::Mat(m)) => assert_eq!(m.shape(), (1, 2)),
            other => panic!("unexpected {other:?}"),
        }
        let c = cell_of(&s, "c");
        assert_eq!(c.shape(), (1, 3));
    }

    #[test]
    fn cell_2d_literal_and_paren_index() {
        let mut s = Session::new();
        s.run("c = {1 2; 'x' 'y'}; sub = c(1,:);").unwrap();
        let c = cell_of(&s, "c");
        assert_eq!(c.shape(), (2, 2));
        let sub = cell_of(&s, "sub");
        assert_eq!(sub.shape(), (1, 2));
        // c(1,2) returns a 1x1 cell, not the content.
        s.run("one = c(1,2); t = one{1};").unwrap();
        assert_eq!(s.get("t"), Some(&Value::Num(2.0)));
    }

    #[test]
    fn cell_brace_and_paren_assign() {
        let mut s = Session::new();
        s.run("c = {1 2}; c{2} = 'text'; c(1) = {[9 9]};").unwrap();
        let c = cell_of(&s, "c");
        assert_eq!(c.get(0, 1), Some(&Value::Str("text".into())));
        match c.get(0, 0) {
            Some(Value::Mat(m)) => assert_eq!(m.get(0, 0), Some(9.0)),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn cell_growth_and_delete() {
        let mut s = Session::new();
        s.run("c = {1}; c{3} = 'x'; n = numel(c);").unwrap();
        assert_eq!(s.get("n"), Some(&Value::Num(3.0)));
        s.run("c(2) = [];").unwrap();
        let c = cell_of(&s, "c");
        assert_eq!(c.numel(), 2);
        assert_eq!(c.get(0, 1), Some(&Value::Str("x".into())));
    }

    #[test]
    fn cell_builtins() {
        let mut s = Session::new();
        s.run("c = {'a' 'b'}; e = cell(2,2); n = numel(e); p = iscell(c); q = iscellstr(c); r = iscellstr({1 'x'});").unwrap();
        assert_eq!(s.get("n"), Some(&Value::Num(4.0)));
        assert_eq!(s.get("p"), Some(&Value::Num(1.0)));
        assert_eq!(s.get("q"), Some(&Value::Num(1.0)));
        assert_eq!(s.get("r"), Some(&Value::Num(0.0)));
        s.run("d = num2cell([1 2 3]); v = cell2mat({1 2; 3 4});").unwrap();
        assert_eq!(cell_of(&s, "d").numel(), 3);
        match s.get("v") {
            Some(Value::Mat(m)) => assert_eq!(m.get(1, 0), Some(3.0)),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn cell_concat_and_for() {
        let mut s = Session::new();
        s.run("a = {1}; b = {'x'}; c = [a b]; n = numel(c); t = 0; for x = {10 20}\nt = t + x{1};\nend").unwrap();
        assert_eq!(s.get("n"), Some(&Value::Num(2.0)));
        assert_eq!(s.get("t"), Some(&Value::Num(30.0)));
    }

    #[test]
    fn cell_varargin() {
        let mut s = Session::new();
        s.run(
            "function n = countem(varargin)\n\
             n = numel(varargin);\n\
             end\n\
             a = countem(1, 'x', [2 3]);",
        )
        .unwrap();
        assert_eq!(s.get("a"), Some(&Value::Num(3.0)));
    }

    fn struct_field(s: &Session, name: &str, field: &str) -> Value {
        match s.get(name) {
            Some(Value::Struct(m)) => m.field_at(0, field).cloned().unwrap_or(Value::Num(-999.0)),
            _ => Value::Num(-998.0),
        }
    }

    fn eval_to(s: &mut Session, expr: &str) -> Value {
        s.run(&format!("__t = {expr};")).unwrap();
        s.get("__t").cloned().unwrap_or(Value::Num(f64::NAN))
    }

    #[test]
    fn struct_dot_assign_and_read() {
        let mut s = Session::new();
        s.run("s.name = 'bob'; s.age = 41; s.data = [1 2; 3 4];").unwrap();
        assert_eq!(struct_field(&s, "s", "age"), Value::Num(41.0));
        assert_eq!(struct_field(&s, "s", "name"), Value::Str("bob".into()));
        assert_eq!(eval_to(&mut s, "s.name"), Value::Str("bob".into()));
        assert_eq!(eval_to(&mut s, "s.age + 1"), Value::Num(42.0));
        match eval_to(&mut s, "s.data") {
            Value::Mat(m) => assert_eq!(m.get(1, 1), Some(4.0)),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn struct_function_pairs_and_empty() {
        let mut s = Session::new();
        s.run("e = struct(); s = struct('a', 1, 'b', 'x');").unwrap();
        assert!(matches!(s.get("e"), Some(Value::Struct(m)) if m.field_names().is_empty()));
        assert_eq!(struct_field(&s, "s", "a"), Value::Num(1.0));
        assert_eq!(struct_field(&s, "s", "b"), Value::Str("x".into()));
        assert_eq!(eval_to(&mut s, "isstruct(s)"), Value::Num(1.0));
        assert_eq!(eval_to(&mut s, "isstruct(5)"), Value::Num(0.0));
    }

    #[test]
    fn struct_nested_fields() {
        let mut s = Session::new();
        s.run("s.a.b.c = 7; s.a.x = 'hi';").unwrap();
        assert_eq!(eval_to(&mut s, "s.a.b.c"), Value::Num(7.0));
        assert_eq!(eval_to(&mut s, "s.a.x"), Value::Str("hi".into()));
        assert!(matches!(s.get("s"), Some(Value::Struct(_))));
    }

    #[test]
    fn struct_field_indexing() {
        let mut s = Session::new();
        s.run("s.v = [10 20 30]; s.v(2) = 99; s.m(2,2) = 5; s.c{1} = 'ok';").unwrap();
        // `s.v(2)` is a 1x1 result like `A(2)` — scalar-vs-matrix is
        // cosmetic inside the engine.
        assert_eq!(eval_to(&mut s, "s.v(2)").to_matrix().unwrap().get(0, 0), Some(99.0));
        match eval_to(&mut s, "s.m") {
            Value::Mat(m) => assert_eq!(m.get(1, 1), Some(5.0)),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(eval_to(&mut s, "s.c{1}"), Value::Str("ok".into()));
    }

    #[test]
    fn struct_field_builtins() {
        let mut s = Session::new();
        s.run("s = struct('b', 2, 'a', 1);").unwrap();
        match eval_to(&mut s, "fieldnames(s)") {
            Value::Cell(c) => {
                assert_eq!(c.numel(), 2);
                // Insertion order, like MATLAB — `orderfields` reorders.
                assert_eq!(c.get(0, 0), Some(&Value::Str("b".into())));
                assert_eq!(c.get(1, 0), Some(&Value::Str("a".into())));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(eval_to(&mut s, "isfield(s, 'a')"), Value::Num(1.0));
        assert_eq!(eval_to(&mut s, "isfield(s, 'z')"), Value::Num(0.0));
        assert_eq!(eval_to(&mut s, "getfield(s, 'b')"), Value::Num(2.0));
        s.run("t = setfield(s, 'q', 'new'); r = rmfield(s, 'b');").unwrap();
        assert_eq!(struct_field(&s, "t", "q"), Value::Str("new".into()));
        assert_eq!(struct_field(&s, "r", "b"), Value::Num(-999.0));
        assert_eq!(struct_field(&s, "r", "a"), Value::Num(1.0));
    }

    #[test]
    fn struct_errors_and_isequal() {
        let mut s = Session::new();
        assert!(s.run("x = 5; r = x.f").is_err());
        assert!(s.run("y = 'abc'; r = y.f").is_err());
        assert!(s.run("s.a = 1; r = s.b").is_err());
        s.run("s.a = 1; s.b = 2; t.a = 1; t.b = 2; u.a = 9;").unwrap();
        assert_eq!(eval_to(&mut s, "isequal(s, t)"), Value::Num(1.0));
        assert_eq!(eval_to(&mut s, "isequal(s, u)"), Value::Num(0.0));
        // struct in a cell survives round-trips
        s.run("c = {s 'x'}; q = c{1};").unwrap();
        assert_eq!(struct_field(&s, "q", "a"), Value::Num(1.0));
    }

    #[test]
    fn struct_dynamic_field_names() {
        let mut s = Session::new();
        s.run("s = struct('x', 3); k = 'x';").unwrap();
        assert_eq!(eval_to(&mut s, "getfield(s, k)"), Value::Num(3.0));
        s.run("f = 'newf'; s.(f) = 42;").unwrap();
        assert_eq!(struct_field(&s, "s", "newf"), Value::Num(42.0));
        assert_eq!(eval_to(&mut s, "s.(k)"), Value::Num(3.0));
    }

    // ---- robustness / strictness batch ------------------------------------

    fn run_err(src: &str) -> String {
        let mut s = Session::new();
        s.run(src).unwrap_err().message
    }

    #[test]
    fn alloc_cap_errors_not_aborts() {
        // Every one of these used to attempt a multi-GB/impossible vec.
        for src in [
            "zeros(1e9)",
            "zeros(1e9, 1e9)",
            "ones([1e9 2])",
            "eye(1e9)",
            "rand(1e9)",
            "randn(1e9, 2)",
            "randi(10, 1e9)",
            "linspace(0, 1, 1e12)",
            "logspace(0, 1, 1e12)",
            "repmat([1 2], 1e9)",
            "cell(1e9)",
            "histcounts(1:10, 1e12)",
            "1:1e12",
            "diag(1:1e5)",
            "figure(1e9)",
        ] {
            assert!(run_err(src).contains("exceeds limit") || run_err(src).contains("range too long"), "{src} did not error");
        }
        // Indexed-assignment growth is capped the same way.
        assert!(run_err("A(1e9) = 1").contains("exceeds limit"));
        assert!(run_err("A(1e9, 1e9) = 1").contains("exceeds limit"));
        assert!(run_err("c{1e9} = 1").contains("exceeds limit"));
        assert!(run_err("c = {1}; c(1e9) = {2}").contains("exceeds limit"));
        assert!(run_err("s.v(1e9) = 1").contains("exceeds limit"));
    }

    #[test]
    fn load_garbage_errors() {
        let dir = std::env::temp_dir().join(format!("matcraft_badload_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let mat = dir.join("garbage.mat").to_string_lossy().replace('\\', "/");
        let txt = dir.join("garbage.txt").to_string_lossy().replace('\\', "/");
        std::fs::write(&mat, "this is not a mat file at all").unwrap();
        std::fs::write(&txt, "no numbers here, just words").unwrap();
        let mut s = Session::new();
        assert!(s.run(&format!("load '{mat}'")).is_err());
        assert!(s.get("garbage").is_none());
        assert!(s.run(&format!("load '{txt}'")).is_err());
        assert!(s.get("garbage").is_none());
        let _ = std::fs::remove_file(&mat);
        let _ = std::fs::remove_file(&txt);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn save_undefined_variable_errors() {
        let dir = std::env::temp_dir().join(format!("matcraft_savmiss_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("ws.mat").to_string_lossy().replace('\\', "/");
        let mut s = Session::new();
        let e = s.run(&format!("a = 1; save '{f}' a b")).unwrap_err();
        assert!(e.message.contains("'b'"), "unexpected: {}", e.message);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_ascii_cell_and_struct_error() {
        let dir = std::env::temp_dir().join(format!("matcraft_savasc_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("ws.txt").to_string_lossy().replace('\\', "/");
        let mut s = Session::new();
        // With only flags named, save must still see every workspace var.
        s.run("a = 1; c = {1};").unwrap();
        assert!(s.run(&format!("save('{f}', '-ascii')")).is_err());
        let mut s = Session::new();
        s.run("a = 1; st.x = 2;").unwrap();
        assert!(s.run(&format!("save('{f}', '-ascii')")).is_err());
        // Explicit names hit the same per-variable check.
        let mut s = Session::new();
        let e = s.run(&format!("st.x = 2; save('{f}', 'st', '-ascii')")).unwrap_err();
        assert!(e.message.contains("ASCII"), "unexpected: {}", e.message);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_function_form_returns_struct() {
        let dir = std::env::temp_dir().join(format!("matcraft_ldstruct_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("ws.mat").to_string_lossy().replace('\\', "/");
        let mut s = Session::new();
        s.run(&format!("a = 42; b = [1 2]; save '{f}' a b; clear a b")).unwrap();
        assert_eq!(eval_to(&mut s, &format!("load('{f}')")).dims(), (1, 1));
        s.run(&format!("S = load('{f}')")).unwrap();
        assert_eq!(struct_field(&s, "S", "a"), Value::Num(42.0));
        match struct_field(&s, "S", "b") {
            Value::Mat(m) => assert_eq!(m.data(), &[1.0, 2.0]),
            other => panic!("unexpected {other:?}"),
        }
        // Selective statement form still loads named vars into the workspace.
        let mut s2 = Session::new();
        s2.run(&format!("load '{f}' a")).unwrap();
        assert_eq!(s2.get("a"), Some(&Value::Num(42.0)));
        assert!(s2.get("b").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn indices_must_be_positive_integers() {
        let mut s = Session::new();
        s.run("m = [10 20; 30 40]; c = {1 2 3};").unwrap();
        for src in ["r = m(1.5)", "r = m(0)", "r = m(-2)", "r = m(nan)", "r = m(inf)", "r = m(1.2, 1)"] {
            assert!(s.run(src).is_err(), "{src} did not error");
        }
        assert!(s.run("r = c{1.5}").is_err());
        assert!(s.run("r = c{0}").is_err());
        // end / colon / logical masks still work.
        s.run("a = m(end); b = m(:); k = m(m > 15);").unwrap();
        assert_eq!(s.get("a").and_then(|v| v.to_matrix().ok()).and_then(|m| m.get_linear(0)), Some(40.0));
        match s.get("k") {
            Some(Value::Mat(m)) => assert_eq!(m.data(), &[30.0, 20.0, 40.0]),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn colon_subscript_columnizes() {
        let mut s = Session::new();
        s.run("c = {10 20 30}; r = c(:); A = [1 2; 3 4]; v = A(:);").unwrap();
        match s.get("r") {
            Some(Value::Cell(c)) => {
                assert_eq!(c.shape(), (3, 1));
                assert_eq!(c.get(1, 0), Some(&Value::Num(20.0)));
            }
            other => panic!("unexpected {other:?}"),
        }
        match s.get("v") {
            Some(Value::Mat(m)) => {
                assert_eq!(m.shape(), (4, 1));
                assert_eq!(m.data(), &[1.0, 3.0, 2.0, 4.0]);
            }
            other => panic!("unexpected {other:?}"),
        }
        // A column cell keeps its orientation on plain linear indexing too.
        s.run("d = {1; 2; 3}; e = d(1:2);").unwrap();
        match s.get("e") {
            Some(Value::Cell(c)) => assert_eq!(c.shape(), (2, 1)),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn subplot_bounds_checked() {
        let mut s = Session::new();
        assert!(s.run("subplot(2,1,3)").is_err());
        assert!(s.run("subplot(2,1,0)").is_err());
        assert!(s.run("subplot(0,1,1)").is_err());
        assert!(s.run("subplot(2,2,[1 5])").is_err());
        // Valid calls still tile.
        s.run("subplot(2,1,1); plot(1:3); subplot(2,1,2); plot(1:3)").unwrap();
        let sub = s.figs[0].sub.as_ref().unwrap();
        assert_eq!((sub.rows, sub.cols), (2, 1));
    }

    #[test]
    fn struct_builtins_strictness() {
        let mut s = Session::new();
        s.run("s = struct('a', 1, 'b', 2);").unwrap();
        // isfield rejects non-char names.
        assert!(s.run("r = isfield(s, 5)").is_err());
        assert!(s.run("r = isfield(5, 'a')").is_err());
        // rmfield errors on a missing field.
        let e = s.run("r = rmfield(s, 'missing')").unwrap_err();
        assert!(e.message.contains("'missing'"), "unexpected: {}", e.message);
        // isfield with a cellstr returns a same-shaped logical array.
        match eval_to(&mut s, "isfield(s, {'a' 'zzz'})") {
            Value::Mat(m) => assert_eq!(m.data(), &[1.0, 0.0]),
            other => panic!("unexpected {other:?}"),
        }
        // rmfield with a cellstr removes each field.
        s.run("r = rmfield(s, {'a' 'b'});").unwrap();
        assert_eq!(eval_to(&mut s, "numel(fieldnames(r))"), Value::Num(0.0));
    }

    #[test]
    fn dynamic_field_names_must_be_identifiers() {
        let mut s = Session::new();
        for bad in ["'a-b'", "'for'", "'9lives'", "''", "'a b'"] {
            let e = s.run(&format!("s.({bad}) = 1")).unwrap_err();
            assert!(e.message.contains("invalid field name"), "{bad}: {}", e.message);
        }
        // Valid dynamic names still assign and read.
        s.run("s.('ok_1') = 7; s.('_priv') = 2;").unwrap();
        assert_eq!(struct_field(&s, "s", "ok_1"), Value::Num(7.0));
        assert_eq!(struct_field(&s, "s", "_priv"), Value::Num(2.0));
    }

    // ---- chained subscript writes ------------------------------------------

    #[test]
    fn chain_cell_brace_brace() {
        let mut s = Session::new();
        s.run("c = {{1 2}; {3 4}}; c{1}{2} = 9;").unwrap();
        // c{1} is now {1 9}; c{2} is untouched.
        assert_eq!(eval_to(&mut s, "c{1}{1}"), Value::Num(1.0));
        assert_eq!(eval_to(&mut s, "c{1}{2}"), Value::Num(9.0));
        assert_eq!(eval_to(&mut s, "c{2}{2}"), Value::Num(4.0));
        let inner = cell_of(&s, "c");
        match inner.get(0, 0) {
            Some(Value::Cell(ic)) => assert_eq!(ic.shape(), (1, 2)),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn chain_cell_brace_then_paren() {
        let mut s = Session::new();
        s.run("c = {[1 2; 3 4]}; c{1}(2,2) = 9;").unwrap();
        match eval_to(&mut s, "c{1}") {
            Value::Mat(m) => {
                assert_eq!(m.get(1, 1), Some(9.0));
                assert_eq!(m.get(0, 0), Some(1.0));
            }
            other => panic!("unexpected {other:?}"),
        }
        // Linear form on the same field's matrix.
        s.run("d = {[1 2; 3 4]}; d{1}(2) = 9;").unwrap();
        match eval_to(&mut s, "d{1}") {
            Value::Mat(m) => assert_eq!(m.get(1, 0), Some(9.0)),
            other => panic!("unexpected {other:?}"),
        }
        // Growth through a chain: `e{1}(4)` extends the inner matrix.
        s.run("e = {[1]}; e{1}(4) = 9;").unwrap();
        match eval_to(&mut s, "e{1}") {
            Value::Mat(m) => assert_eq!(m.data(), &[1.0, 0.0, 0.0, 9.0]),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn chain_three_deep() {
        let mut s = Session::new();
        s.run("c = {{{'a' 'b'}}}; c{1}{1}{1} = 'x';").unwrap();
        assert_eq!(eval_to(&mut s, "c{1}{1}{1}"), Value::Str("x".into()));
        assert_eq!(eval_to(&mut s, "c{1}{1}{2}"), Value::Str("b".into()));
    }

    #[test]
    fn chain_through_struct_field() {
        let mut s = Session::new();
        // `s.c{1}(1)` — brace then paren on a cell field of numbers.
        s.run("s.c = {1 2}; s.c{1}(1) = 9;").unwrap();
        assert_eq!(eval_to(&mut s, "s.c{1}").to_matrix().unwrap().get(0, 0), Some(9.0));
        // `s.d{1}{1}` — brace/brace on a cell field of cells.
        s.run("s.d = {{1 2}}; s.d{1}{1} = 9;").unwrap();
        assert_eq!(eval_to(&mut s, "s.d{1}{1}"), Value::Num(9.0));
        // `s.m{1} = {5}` stores a cell inside the field's cell.
        s.run("s.m{1} = {5};").unwrap();
        assert_eq!(eval_to(&mut s, "iscell(s.m{1})"), Value::Num(1.0));
        assert_eq!(eval_to(&mut s, "s.m{1}{1}"), Value::Num(5.0));
        // `s.a{1}(2)` — a cell field of matrices.
        s.run("s.a = {[1 2; 3 4]}; s.a{1}(2) = 9;").unwrap();
        match eval_to(&mut s, "s.a{1}") {
            Value::Mat(m) => assert_eq!(m.get(1, 0), Some(9.0)),
            other => panic!("unexpected {other:?}"),
        }
        // Deeper path still works: `s.x.y{1} = v`.
        s.run("s.x.y = {10}; s.x.y{1} = 20;").unwrap();
        assert_eq!(eval_to(&mut s, "s.x.y{1}"), Value::Num(20.0));
    }

    #[test]
    fn chain_assign_errors() {
        let mut s = Session::new();
        s.run("x = 5; c = {1};").unwrap();
        // Brace chain into a non-cell.
        assert!(s.run("x{1}{1} = 2").is_err());
        assert!(s.run("x{1}(1) = 2").is_err());
        // Out-of-bounds fetch mid-chain propagates.
        assert!(s.run("c{99}{1} = 2").is_err());
        // `c{1}(i) = []` — deletion inside a chain deletes the element.
        s.run("c{1}(1) = [];").unwrap();
        match eval_to(&mut s, "c{1}") {
            Value::Mat(m) => assert_eq!(m.numel(), 0),
            other => panic!("unexpected {other:?}"),
        }
        s.run("c = {1};").unwrap();
        // `[]` *content* through a brace tail is fine though — the
        // deletion guard only fires on a `()` last op.
        s.run("d = {{5}}; d{1}{1} = [];").unwrap();
        match eval_to(&mut s, "d{1}{1}") {
            Value::Mat(m) => assert_eq!(m.numel(), 0),
            other => panic!("unexpected {other:?}"),
        }
        // A failed chain assign keeps the old binding.
        s.run("v = 7;").unwrap();
        assert!(s.run("v{1}{1} = 2").is_err());
        assert_eq!(s.get("v"), Some(&Value::Num(7.0)));
    }

    #[test]
    fn assign_forms_nonregression() {
        let mut s = Session::new();
        // Every previously-supported lvalue shape, in one session.
        s.run("x = 1 + 2; A = [1 2; 3 4]; A(2,1) = 9; q = [1; 2]; q(4) = 7;").unwrap();
        s.run("c = {1 2 3}; c{1} = 'v'; c(2) = {[8]}; c(3) = [];").unwrap();
        s.run("s.f = 1; s.a.b = 2; s.v = [10 20]; s.v(2) = 99; k = 'g'; s.(k) = 42; s.cc{1} = 3;").unwrap();
        assert_eq!(s.get("x"), Some(&Value::Num(3.0)));
        match s.get("A") {
            Some(Value::Mat(m)) => assert_eq!(m.data(), &[1.0, 9.0, 2.0, 4.0]),
            other => panic!("unexpected {other:?}"),
        }
        match s.get("q") {
            Some(Value::Mat(m)) => assert_eq!(m.data(), &[1.0, 2.0, 0.0, 7.0]),
            other => panic!("unexpected {other:?}"),
        }
        let c = cell_of(&s, "c");
        assert_eq!(c.numel(), 2);
        assert_eq!(c.get(0, 0), Some(&Value::Str("v".into())));
        assert_eq!(eval_to(&mut s, "s.f"), Value::Num(1.0));
        assert_eq!(eval_to(&mut s, "s.g"), Value::Num(42.0));
        assert_eq!(eval_to(&mut s, "s.a.b"), Value::Num(2.0));
        assert_eq!(eval_to(&mut s, "s.v(2)").to_matrix().unwrap().get(0, 0), Some(99.0));
        assert_eq!(eval_to(&mut s, "s.cc{1}"), Value::Num(3.0));
        // AssignMany still works.
        s.run("[lo, hi] = bounds([3 1 4])\nfunction [a, b] = bounds(v)\na = min(v); b = max(v);\nend").unwrap();
        assert_eq!(s.get("lo"), Some(&Value::Num(1.0)));
        assert_eq!(s.get("hi"), Some(&Value::Num(4.0)));
    }

    // ---- B1: char→double promotion -------------------------------

    #[test]
    fn char_arithmetic_and_compare() {
        let mut s = Session::new();
        s.run("a = 'a' + 1; e = 'a' == 'a'; v = 'abc' == 'abc'; n = -'a';").unwrap();
        assert_eq!(s.get("a"), Some(&Value::Num(98.0)));
        assert_eq!(s.get("e"), Some(&Value::Num(1.0)));
        match s.get("v") {
            Some(Value::Mat(m)) => assert_eq!(m.data(), &[1.0, 1.0, 1.0]),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(s.get("n"), Some(&Value::Num(-97.0)));
        // Broadcasting: scalar char vs row.
        match eval_to(&mut s, "'a' == 'ab'") {
            Value::Mat(m) => assert_eq!(m.data(), &[1.0, 0.0]),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn char_transpose_is_code_column() {
        let mut s = Session::new();
        // `'abc''` lexes as unterminated — `'` after a string literal is a
        // quote escape, not transpose (same as M-code). Transpose binds
        // after identifiers/parens/brackets.
        s.run("x = 'abc'; y = x';").unwrap();
        match s.get("y") {
            Some(Value::Mat(m)) => {
                assert_eq!(m.shape(), (3, 1));
                assert_eq!(m.data(), &[97.0, 98.0, 99.0]);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn char_literal_concat() {
        let mut s = Session::new();
        // Char context pulls numerics to code points; result stays char.
        s.run("a = ['a' 'b']; b = [72 'i'];").unwrap();
        assert_eq!(s.get("a"), Some(&Value::Str("ab".into())));
        assert_eq!(s.get("b"), Some(&Value::Str("Hi".into())));
        // Multi-row char arrays need a 2-D char type — clean error.
        let err = s.run("c = ['ab'; 'cd']").unwrap_err();
        assert!(err.to_string().contains("multi-row char"), "{err}");
    }

    #[test]
    fn char_empty_is_0x0() {
        let mut s = Session::new();
        s.run("z = size(''); e = isempty('');").unwrap();
        match s.get("z") {
            Some(Value::Mat(m)) => assert_eq!(m.data(), &[0.0, 0.0]),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(s.get("e"), Some(&Value::Num(1.0)));
    }

    #[test]
    fn char_index_write_and_numeric_target() {
        let mut s = Session::new();
        // Char target: code-point write, space-padded growth, stays char.
        s.run("s = 'abc'; s(2) = 'z'; t = 'ab'; t(4) = 'd';").unwrap();
        assert_eq!(s.get("s"), Some(&Value::Str("azc".into())));
        assert_eq!(s.get("t"), Some(&Value::Str("ab d".into())));
        // Double rhs into a char array stores the code point.
        s.run("u = 'abc'; u(2) = 120;").unwrap();
        assert_eq!(s.get("u"), Some(&Value::Str("axc".into())));
        // Char rhs into a numeric array stores the code point.
        s.run("x = [1 2]; x(1) = 'a';").unwrap();
        match s.get("x") {
            Some(Value::Mat(m)) => assert_eq!(m.data(), &[97.0, 2.0]),
            other => panic!("unexpected {other:?}"),
        }
        // Reading a char element stays char.
        assert_eq!(eval_to(&mut s, "s(1)"), Value::Str("a".into()));
    }

    #[test]
    fn char_truthy_and_for() {
        let mut s = Session::new();
        s.run("hit = 0; if 'a' hit = 1; end").unwrap();
        assert_eq!(s.get("hit"), Some(&Value::Num(1.0)));
        // `for` over a char iterates code points as doubles.
        s.run("acc = 0; for c = 'ab'\nacc = acc + c;\nend").unwrap();
        assert_eq!(s.get("acc"), Some(&Value::Num(195.0)));
    }

    // ---- B2: cell-aware shape builtins ---------------------------

    #[test]
    fn cell_reshape_repmat_flip() {
        let mut s = Session::new();
        s.run("c = {1 2 3; 4 5 6}; r = reshape(c, 3, 2);").unwrap();
        let r = cell_of(&s, "r");
        assert_eq!(r.shape(), (3, 2));
        assert_eq!(r.get_linear(3), Some(&Value::Num(5.0))); // column-major
        s.run("p = repmat({1 2}, 2, 2); f1 = fliplr({1 2; 3 4}); f2 = flipud({1 2; 3 4});").unwrap();
        let p = cell_of(&s, "p");
        assert_eq!(p.shape(), (2, 4));
        // {1 2} tiled 2x2 → {1 2 1 2; 1 2 1 2}: (row2, col2) = 2.
        assert_eq!(p.get(1, 1), Some(&Value::Num(2.0)));
        let f1 = cell_of(&s, "f1");
        assert_eq!(f1.get(0, 0), Some(&Value::Num(2.0)));
        let f2 = cell_of(&s, "f2");
        assert_eq!(f2.get(0, 0), Some(&Value::Num(3.0)));
        // Reshape must preserve element count.
        assert!(s.run("reshape({1 2 3}, 2, 2)").is_err());
    }

    #[test]
    fn cell_cat_and_hv_cat() {
        let mut s = Session::new();
        s.run("a = {1 2}; b = {3; 4}; h = cat(2, a, a); v = cat(1, a, a);").unwrap();
        assert_eq!(cell_of(&s, "h").shape(), (1, 4));
        assert_eq!(cell_of(&s, "v").shape(), (2, 2));
        s.run("h2 = horzcat(a, a); v2 = vertcat(b, b);").unwrap();
        assert_eq!(cell_of(&s, "h2").shape(), (1, 4));
        assert_eq!(cell_of(&s, "v2").shape(), (4, 1));
        // Numeric and char forms.
        s.run("d = horzcat(1, 2, 3); e = vertcat(1, 2); t = horzcat('a', 'b');").unwrap();
        match s.get("d") {
            Some(Value::Mat(m)) => assert_eq!(m.data(), &[1.0, 2.0, 3.0]),
            other => panic!("unexpected {other:?}"),
        }
        match s.get("e") {
            Some(Value::Mat(m)) => assert_eq!(m.shape(), (2, 1)),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(s.get("t"), Some(&Value::Str("ab".into())));
        // Mixing cells and non-cells still errors (use {…} for that).
        assert!(s.run("horzcat({1}, 2)").is_err());
    }

    #[test]
    fn num2cell_variants_and_char() {
        let mut s = Session::new();
        s.run("d = num2cell('abc');").unwrap();
        let d = cell_of(&s, "d");
        assert_eq!(d.shape(), (1, 3));
        assert_eq!(d.get(0, 1), Some(&Value::Str("b".into())));
        s.run("A = [1 2; 3 4]; c1 = num2cell(A, 1); c2 = num2cell(A, 2);").unwrap();
        let c1 = cell_of(&s, "c1");
        assert_eq!(c1.shape(), (1, 2));
        match c1.get(0, 0) {
            Some(Value::Mat(m)) => assert_eq!(m.data(), &[1.0, 3.0]),
            other => panic!("unexpected {other:?}"),
        }
        let c2 = cell_of(&s, "c2");
        assert_eq!(c2.shape(), (2, 1));
        match c2.get(0, 0) {
            Some(Value::Mat(m)) => assert_eq!(m.data(), &[1.0, 2.0]),
            other => panic!("unexpected {other:?}"),
        }
        // char of a single-string cell unwraps; multi errors cleanly.
        s.run("t = char({'hello'});").unwrap();
        assert_eq!(s.get("t"), Some(&Value::Str("hello".into())));
        assert!(s.run("char({'a', 'b'})").is_err());
    }

    #[test]
    fn cell_row_col_delete() {
        let mut s = Session::new();
        s.run("c = {1 2; 3 4}; c(:, 1) = [];").unwrap();
        let c = cell_of(&s, "c");
        assert_eq!(c.shape(), (2, 1));
        assert_eq!(c.get(1, 0), Some(&Value::Num(4.0)));
        s.run("d = {1 2; 3 4}; d(1, :) = [];").unwrap();
        let d = cell_of(&s, "d");
        assert_eq!(d.shape(), (1, 2));
        assert_eq!(d.get(0, 0), Some(&Value::Num(3.0)));
        // Partial row AND partial column is not a legal null assignment.
        // (A single index that covers a whole dim counts as the colon.)
        s.run("e = {1 2; 3 4};").unwrap();
        assert!(s.run("e(1, 1) = []").is_err());
        // Linear delete on a vector still works.
        s.run("v = {1 2 3}; v(2) = [];").unwrap();
        let v = cell_of(&s, "v");
        assert_eq!(v.shape(), (1, 2));
        assert_eq!(v.get(0, 1), Some(&Value::Num(3.0)));
    }

    // ---- B4: error handling ----------------------------------------

    #[test]
    fn try_catch_basics() {
        let mut s = Session::new();
        s.run("try\nx = nosuchvar;\nhit = 1;\ncatch\nhit = 2;\nend\nafter = 3;").unwrap();
        assert_eq!(s.get("hit"), Some(&Value::Num(2.0)));
        assert_eq!(s.get("after"), Some(&Value::Num(3.0)));
        // No error → catch body skipped.
        s.run("try\nok = 1;\ncatch\nok = 99;\nend").unwrap();
        assert_eq!(s.get("ok"), Some(&Value::Num(1.0)));
        // `catch e` binds message + identifier as a struct.
        s.run("try\nerror('my:tag', 'boom');\ncatch e\nmsg = e.message; id = e.identifier;\nend").unwrap();
        assert_eq!(s.get("msg"), Some(&Value::Str("boom".into())));
        assert_eq!(s.get("id"), Some(&Value::Str("my:tag".into())));
        // lasterror reflects the last failure.
        s.run("le = lasterror();").unwrap();
        match s.get("le") {
            Some(Value::Struct(m)) => assert_eq!(m.field_at(0, "identifier"), Some(&Value::Str("my:tag".into()))),
            other => panic!("unexpected {other:?}"),
        }
        // Errors inside the catch body propagate.
        assert!(s.run("try\nnosuch;\ncatch\nerror('also:bad', 'rethrown');\nend").is_err());
        // break inside try still propagates as control flow.
        s.run("done = 0; for i = 1:3\ntry\nif i == 2\nbreak;\nend\ncatch\nend\nend\ndone = i;").unwrap();
        assert_eq!(s.get("done").map(|v| v.to_matrix().unwrap().as_scalar().unwrap_or(0.0)), Some(2.0));
    }

    #[test]
    fn tilde_ignored_outputs() {
        let mut s = Session::new();
        // `[~, i] = max(...)` — builtin multi-output.
        s.run("[~, idx] = max([1 5 3]);").unwrap();
        assert_eq!(s.get("idx"), Some(&Value::Num(2.0)));
        assert_eq!(s.get("~"), None);
        // User function with a `~` output slot — never assigned, no error.
        s.run("[~, b] = second(7)\nfunction [~, b] = second(x)\nb = x * 2;\nend").unwrap();
        assert_eq!(s.get("b"), Some(&Value::Num(14.0)));
        // `~` input param consumes the slot without binding.
        s.run("r = ign(1, 9)\nfunction r = ign(~, b)\nr = b;\nend").unwrap();
        assert_eq!(s.get("r"), Some(&Value::Num(9.0)));
        // `~expr` still means logical not.
        assert_eq!(eval_to(&mut s, "~0"), Value::Num(1.0));
    }

    #[test]
    fn nargin_nargout_checks() {
        let mut s = Session::new();
        let err = s.run("r = two(1)\nfunction r = two(a, b)\nnarginchk(2, 2);\nr = a;\nend").unwrap_err();
        assert!(err.to_string().contains("not enough input"), "{err}");
        let err = s.run("r = two(1, 2, 3)\nfunction r = two(a, b)\nnarginchk(1, 2);\nr = a;\nend").unwrap_err();
        assert!(err.to_string().contains("too many input"), "{err}");
        // In range → passes.
        s.run("r = two(1, 2)\nfunction r = two(a, b)\nnarginchk(1, 2);\nr = a + b;\nend").unwrap();
        assert_eq!(s.get("r"), Some(&Value::Num(3.0)));
        // Outside a function → clean error.
        assert!(s.run("narginchk(1, 2)").is_err());
        // Error ID plumbing through catch.
        s.run("try\nerror('e:o', 'm');\ncatch ex\ni = ex.identifier;\nend").unwrap();
        assert_eq!(s.get("i"), Some(&Value::Str("e:o".into())));
    }

    // ---- B5: papercuts --------------------------------------------------

    #[test]
    fn field_after_subscript_assign() {
        let mut s = Session::new();
        // `s.c{1}.f = v` — a field step after a brace step.
        s.run("s.c = {struct('f', 0)}; s.c{1}.f = 42;").unwrap();
        let Value::Struct(m) = s.get("s").unwrap() else { panic!("not a struct") };
        let Value::Cell(c) = m.field_at(0, "c").unwrap() else { panic!("not a cell") };
        let Value::Struct(inner) = c.get_linear(0).unwrap() else { panic!("not a struct") };
        assert_eq!(inner.field_at(0, "f"), Some(&Value::Num(42.0)));
        // `s.(s.k) = v` — dynamic name resolved before detaching the root.
        s.run("t.k = 'nf'; t.(t.k) = 7;").unwrap();
        let Value::Struct(t) = s.get("t").unwrap() else { panic!("not a struct") };
        assert_eq!(t.field_at(0, "nf"), Some(&Value::Num(7.0)));
    }

    #[test]
    fn matrix_null_assign_deletion() {
        let mut s = Session::new();
        s.run("A = [1 2 3 4 5]; A(2) = [];").unwrap();
        assert_eq!(eval_to(&mut s, "A"), eval_to(&mut s, "[1 3 4 5]"));
        // Column delete on a matrix.
        s.run("M = [1 2 3; 4 5 6; 7 8 9]; M(:, 2) = [];").unwrap();
        assert_eq!(eval_to(&mut s, "size(M)"), eval_to(&mut s, "[3 2]"));
        // Row delete.
        s.run("M(1, :) = [];").unwrap();
        assert_eq!(eval_to(&mut s, "size(M)"), eval_to(&mut s, "[2 2]"));
        // Vector keeps orientation.
        s.run("V = [1; 2; 3]; V(2) = []; sz = size(V);").unwrap();
        assert_eq!(eval_to(&mut s, "sz"), eval_to(&mut s, "[2 1]"));
        // Out-of-range deletion errors — it does not silently no-op or grow.
        assert!(s.run("A(99) = [];").is_err());
        assert!(s.run("M(:, 9) = [];").is_err());
        // Deletion through a chain: `s.v(2) = []`, `s.c{2}(2) = []`.
        s.run("s.v = [1 2 3]; s.v(2) = []; s.c = {1 [2 3] 4}; s.c{2}(2) = [];").unwrap();
        let Value::Struct(m) = s.get("s").unwrap() else { panic!("not a struct") };
        assert_eq!(m.field_at(0, "v"), Some(&Value::Mat(Matrix::row_vector(vec![1.0, 3.0]))));
        let Value::Cell(c) = m.field_at(0, "c").unwrap() else { panic!("not a cell") };
        assert_eq!(c.get_linear(1), Some(&Value::Mat(Matrix::row_vector(vec![2.0]))));
    }

    // ---- B3: comma-separated lists --------------------------------

    #[test]
    fn csl_expands_in_call_args() {
        let mut s = Session::new();
        s.run("c = {1 2 3};").unwrap();
        // `f(c{1:2})` → f(1, 2): sum(1, 2) reduces scalar along dim 2 → 1.
        assert_eq!(eval_to(&mut s, "sum(c{1:2})"), Value::Num(1.0));
        // varargs forwarding: every content becomes a separate argument.
        s.run("r = addall(c{:})\nfunction t = addall(varargin)\nt = 0;\nfor i = 1:numel(varargin)\nt = t + varargin{i};\nend\nend").unwrap();
        assert_eq!(s.get("r"), Some(&Value::Num(6.0)));
        // A struct field's cell expands the same way.
        s.run("s.x = {5 6}; p = pair(s.x{:})\nfunction r = pair(a, b)\nr = a * 10 + b;\nend").unwrap();
        assert_eq!(s.get("p"), Some(&Value::Num(56.0)));
    }

    #[test]
    fn csl_expands_in_literals() {
        let mut s = Session::new();
        s.run("c = {10 20}; m = [c{:}]; k = {c{:}};").unwrap();
        match s.get("m") {
            Some(Value::Mat(mm)) => assert_eq!(mm.data(), &[10.0, 20.0]),
            other => panic!("unexpected {other:?}"),
        }
        let k = cell_of(&s, "k");
        assert_eq!(k.numel(), 2);
        assert_eq!(k.get(0, 1), Some(&Value::Num(20.0)));
        // Mixed with plain elements.
        s.run("m2 = [0 c{:} 30];").unwrap();
        match s.get("m2") {
            Some(Value::Mat(mm)) => assert_eq!(mm.data(), &[0.0, 10.0, 20.0, 30.0]),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn csl_multi_assign_and_index() {
        let mut s = Session::new();
        s.run("c = {1 2 3}; [a, b] = c{1:2};").unwrap();
        assert_eq!(s.get("a"), Some(&Value::Num(1.0)));
        assert_eq!(s.get("b"), Some(&Value::Num(2.0)));
        // `c{:}` fills all targets.
        s.run("[x, y, z] = c{:};").unwrap();
        assert_eq!(s.get("z"), Some(&Value::Num(3.0)));
        // Count mismatch is an error.
        assert!(s.run("[p, q] = c{:}").is_err());
        // CSL expands in index position: A(c{2 3}) → A(2, 3).
        s.run("A = [10 20 30 40; 50 60 70 80]; idx = {2 3}; got = A(idx{:});").unwrap();
        assert_eq!(s.get("got"), Some(&Value::Num(70.0)));
    }

    #[test]
    fn csl_scalar_context_errors() {
        let mut s = Session::new();
        s.run("c = {1 2};").unwrap();
        let err = s.run("x = c{1:2}").unwrap_err();
        assert!(err.to_string().contains("expected one output"), "{err}");
        // Empty pick is still zero results.
        s.run("d = {};").unwrap();
        assert!(s.run("x = d{1}").is_err());
        // Bare CSL echoes each element through ans.
        let out = s.run("c{1:2}").unwrap();
        assert!(out.matches("ans =").count() == 2, "{out}");
    }

    #[test]
    fn char_strict_contexts_still_error() {
        let mut s = Session::new();
        // Builtins that take matrices keep `to_matrix` strictness; the
        // explicit conversion path is `double`.
        assert!(s.run("mod('c', 'a')").is_err());
        s.run("d = double('ab');").unwrap();
        match s.get("d") {
            Some(Value::Mat(m)) => assert_eq!(m.data(), &[97.0, 98.0]),
            other => panic!("unexpected {other:?}"),
        }
        assert!(s.run("[1 {2}]").is_err());
    }

    // --- complex numbers -------------------------------------------------

    fn cplx_scalar(s: &mut Session, var: &str) -> (f64, f64) {
        match s.get(var) {
            Some(Value::Cplx(m)) => {
                let z = m.as_scalar().unwrap_or(CNum::real(f64::NAN));
                (z.re, z.im)
            }
            other => panic!("expected complex {var}, got {other:?}"),
        }
    }

    #[test]
    fn complex_literals_and_constants() {
        let mut s = Session::new();
        // `i`/`j` constants and glued imaginary literals.
        s.run("z = 3 + 4i; w = 1 - 2j;").unwrap();
        assert_eq!(cplx_scalar(&mut s, "z"), (3.0, 4.0));
        assert_eq!(cplx_scalar(&mut s, "w"), (1.0, -2.0));
        // Shadowing works: `i = 5` is legal M-code.
        s.run("i = 5;").unwrap();
        assert_eq!(s.get("i"), Some(&Value::Num(5.0)));
    }

    #[test]
    fn complex_arithmetic() {
        let mut s = Session::new();
        s.run("z = (1+2i) * (3-4i); q = (1+2i) / (3-4i); n = -(1+2i);").unwrap();
        assert_eq!(cplx_scalar(&mut s, "z"), (11.0, 2.0));
        let (re, im) = cplx_scalar(&mut s, "q");
        assert!((re + 0.2).abs() < 1e-12 && (im - 0.4).abs() < 1e-12, "{re} {im}");
        assert_eq!(cplx_scalar(&mut s, "n"), (-1.0, -2.0));
        // i^2 = -1 (auto-demotes to real).
        assert_eq!(eval_to(&mut s, "i * i"), Value::Num(-1.0));
        // (1+i)*(1-i) = 2, demoted to real.
        assert_eq!(eval_to(&mut s, "(1+i) * (1-i)"), Value::Num(2.0));
    }

    #[test]
    fn complex_matrix_literal_and_index() {
        let mut s = Session::new();
        s.run("z = [1 2i; 3 4j];").unwrap();
        match s.get("z") {
            Some(Value::Cplx(m)) => {
                assert_eq!(m.shape(), (2, 2));
                // Column-major linear indexing.
                assert_eq!(m.get_linear(1), Some(CNum::new(3.0, 0.0)));
                assert_eq!(m.get_linear(2), Some(CNum::new(0.0, 2.0)));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn complex_transposes() {
        let mut s = Session::new();
        // `'` conjugates; `.'` doesn't.
        s.run("z = [1+2i 3-4i]; a = z'; b = z.';").unwrap();
        match s.get("a") {
            Some(Value::Cplx(m)) => {
                assert_eq!(m.get(0, 0), Some(CNum::new(1.0, -2.0)));
                assert_eq!(m.get(1, 0), Some(CNum::new(3.0, 4.0)));
            }
            other => panic!("unexpected {other:?}"),
        }
        match s.get("b") {
            Some(Value::Cplx(m)) => assert_eq!(m.get(0, 0), Some(CNum::new(1.0, 2.0))),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn complex_builtins() {
        let mut s = Session::new();
        s.run("z = 3+4i;").unwrap();
        assert_eq!(eval_to(&mut s, "real(z)"), Value::Num(3.0));
        assert_eq!(eval_to(&mut s, "imag(z)"), Value::Num(4.0));
        assert_eq!(eval_to(&mut s, "abs(z)"), Value::Num(5.0));
        assert_eq!(eval_to(&mut s, "isreal(z)"), Value::Num(0.0));
        assert_eq!(eval_to(&mut s, "isreal(5)"), Value::Num(1.0));
        assert_eq!(eval_to(&mut s, "isnumeric(z)"), Value::Num(1.0));
        s.run("w = conj(z);").unwrap();
        assert_eq!(cplx_scalar(&mut s, "w"), (3.0, -4.0));
    }

    #[test]
    fn complex_conj_angle_complex_ctor() {
        let mut s = Session::new();
        s.run("w = conj(3+4i); a = angle(0+1i); c = complex([1 2], [3 4]);").unwrap();
        assert_eq!(cplx_scalar(&mut s, "w"), (3.0, -4.0));
        match s.get("a") {
            Some(Value::Num(a)) => assert!((a - std::f64::consts::FRAC_PI_2).abs() < 1e-9),
            other => panic!("unexpected {other:?}"),
        }
        match s.get("c") {
            Some(Value::Cplx(m)) => {
                assert_eq!(m.get(0, 0), Some(CNum::new(1.0, 3.0)));
                assert_eq!(m.get(0, 1), Some(CNum::new(2.0, 4.0)));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn complex_domain_promotion() {
        let mut s = Session::new();
        // sqrt/log on negative reals auto-promote (M-code semantics).
        s.run("r = sqrt(-4); l = log(-1);").unwrap();
        assert_eq!(cplx_scalar(&mut s, "r"), (0.0, 2.0));
        let (_, im) = cplx_scalar(&mut s, "l");
        assert!((im - std::f64::consts::PI).abs() < 1e-9);
    }

    #[test]
    fn complex_indexed_assign_and_delete() {
        let mut s = Session::new();
        // Complex RHS promotes a real target.
        s.run("z = zeros(2); z(1,2) = 3+4i;").unwrap();
        match s.get("z") {
            Some(Value::Cplx(m)) => assert_eq!(m.get(0, 1), Some(CNum::new(3.0, 4.0))),
            other => panic!("unexpected {other:?}"),
        }
        // Deletion collapses to a row vector on non-vector linear delete.
        s.run("z(1) = [];").unwrap();
        match s.get("z") {
            Some(Value::Cplx(m)) => assert_eq!(m.numel(), 3),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn complex_compare_and_truthy() {
        let mut s = Session::new();
        assert_eq!(eval_to(&mut s, "1+2i == 1+2i"), Value::Num(1.0));
        assert_eq!(eval_to(&mut s, "1+2i ~= 1+3i"), Value::Num(1.0));
        // Ordering compares real parts only.
        assert_eq!(eval_to(&mut s, "(1+2i) < (3+4i)"), Value::Num(1.0));
        assert_eq!(eval_to(&mut s, "(1+2i) > (3+4i)"), Value::Num(0.0));
        // Nonzero complex is truthy.
        s.run("ok = 0; if 0+1i; ok = 1; end").unwrap();
        assert_eq!(s.get("ok"), Some(&Value::Num(1.0)));
    }

    #[test]
    fn complex_reductions() {
        let mut s = Session::new();
        s.run("t = sum([1+2i 3]); p = prod([1+i 1-i]);").unwrap();
        assert_eq!(cplx_scalar(&mut s, "t"), (4.0, 2.0));
        // All-real result demotes to double.
        assert_eq!(s.get("p"), Some(&Value::Num(2.0)));
        // sort orders by magnitude.
        s.run("v = sort([3+4i 1+i 2]);").unwrap();
        match s.get("v") {
            Some(Value::Cplx(m)) => {
                assert_eq!(m.get(0, 0), Some(CNum::new(1.0, 1.0)));
                assert_eq!(m.get(0, 2), Some(CNum::new(3.0, 4.0)));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn complex_solve_and_det_inv() {
        let mut s = Session::new();
        // (1+i) x = 2 → x = 1-i; diag solve.
        s.run("x = [1+i 0; 0 1-i] \\ [2; 2]; d = det([1+i 0; 0 1-i]);").unwrap();
        match s.get("x") {
            Some(Value::Cplx(m)) => {
                assert!((m.get(0, 0).unwrap_or(CNum::real(0.0)).re - 1.0).abs() < 1e-9);
                assert!((m.get(0, 0).unwrap_or(CNum::real(0.0)).im + 1.0).abs() < 1e-9);
                assert!((m.get(1, 0).unwrap_or(CNum::real(0.0)).im - 1.0).abs() < 1e-9);
            }
            other => panic!("unexpected {other:?}"),
        }
        // det = (1+i)(1-i) = 2, demoted.
        assert_eq!(s.get("d"), Some(&Value::Num(2.0)));
    }

    #[test]
    fn complex_eig_rotation() {
        let mut s = Session::new();
        // Rotation matrix → eigenvalues ±i.
        s.run("w = eig([0 -1; 1 0]);").unwrap();
        match s.get("w") {
            Some(Value::Cplx(m)) => {
                let ims: Vec<f64> = m.data().iter().map(|z| z.im).collect();
                assert!(ims.iter().any(|&x| (x - 1.0).abs() < 1e-9) && ims.iter().any(|&x| (x + 1.0).abs() < 1e-9), "{ims:?}");
            }
            other => panic!("unexpected {other:?}"),
        }
        // Real spectrum stays real.
        s.run("w2 = eig([1 0; 0 2]);").unwrap();
        match s.get("w2") {
            Some(Value::Mat(m)) => assert_eq!(m.numel(), 2),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn complex_for_loop_and_cell() {
        let mut s = Session::new();
        // `for` iterates complex columns.
        s.run("k = 0; for z = [1+i 2+2i]; k = k + z; end").unwrap();
        assert_eq!(cplx_scalar(&mut s, "k"), (3.0, 3.0));
        // Complex values live in cells fine.
        s.run("c = {1+2i 'x'};").unwrap();
        match s.get("c") {
            Some(Value::Cell(ca)) => assert_eq!(ca.numel(), 2),
            other => panic!("unexpected {other:?}"),
        }
    }

    // ---- C7: struct arrays ----------------------------------------------

    #[test]
    fn struct_array_ctor_and_subscript() {
        let mut s = Session::new();
        s.run("s = struct('name', {'a','b','c'}, 'x', {10,20,30});").unwrap();
        assert_eq!(eval_to(&mut s, "size(s)"), eval_to(&mut s, "[1 3]"));
        assert_eq!(eval_to(&mut s, "numel(s)"), Value::Num(3.0));
        assert_eq!(eval_to(&mut s, "s(2).name"), Value::Str("b".into()));
        assert_eq!(eval_to(&mut s, "s(3).x"), Value::Num(30.0));
        // `s(2)` yields a 1x1 sub-struct with the same fields.
        match eval_to(&mut s, "s(2)") {
            Value::Struct(sa) => {
                assert!(sa.is_scalar());
                assert_eq!(sa.field_at(0, "x"), Some(&Value::Num(20.0)));
            }
            other => panic!("unexpected {other:?}"),
        }
        // `s(:)` columnizes.
        assert_eq!(eval_to(&mut s, "size(s(:))"), eval_to(&mut s, "[3 1]"));
    }

    #[test]
    fn struct_array_write_and_growth() {
        let mut s = Session::new();
        s.run("s = struct('x', {1,2,3}); s(5).x = 9; s(5).tag = 'new';").unwrap();
        assert_eq!(eval_to(&mut s, "numel(s)"), Value::Num(5.0));
        assert_eq!(eval_to(&mut s, "s(5).x"), Value::Num(9.0));
        assert_eq!(eval_to(&mut s, "s(5).tag"), Value::Str("new".into()));
        // Back-filled field is [] on the earlier elements.
        assert_eq!(eval_to(&mut s, "isempty(s(1).tag)"), Value::Num(1.0));
        // Whole-element write and delete.
        s.run("s(2) = struct('x', 77, 'tag', 'mid');").unwrap();
        assert_eq!(eval_to(&mut s, "s(2).x"), Value::Num(77.0));
        s.run("s(1) = [];").unwrap();
        assert_eq!(eval_to(&mut s, "numel(s)"), Value::Num(4.0));
        assert_eq!(eval_to(&mut s, "s(1).x"), Value::Num(77.0));
    }

    #[test]
    fn struct_array_csl_and_deal() {
        let mut s = Session::new();
        s.run("s = struct('x', {1,2,3});").unwrap();
        // `[s.x]` expands the dot reference to a comma-separated list.
        assert_eq!(eval_to(&mut s, "[s.x]"), eval_to(&mut s, "[1 2 3]"));
        s.run("[a,b,c] = s.x;").unwrap();
        assert_eq!(s.get("b"), Some(&Value::Num(2.0)));
        s.run("[p,q] = deal(s.x);").unwrap();
        assert_eq!(s.get("q"), Some(&Value::Num(2.0)));
        // Bare `s.f` on an array errors like a bare `c{1:2}`.
        assert!(s.run("y = s.x;").is_err());
    }

    #[test]
    fn struct_array_orderfields_concat() {
        let mut s = Session::new();
        s.run("s = struct('b', {1,2}, 'a', {'x','y'}); t = orderfields(s);").unwrap();
        match eval_to(&mut s, "fieldnames(t)") {
            Value::Cell(c) => {
                assert_eq!(c.get(0, 0), Some(&Value::Str("a".into())));
                assert_eq!(c.get(1, 0), Some(&Value::Str("b".into())));
            }
            other => panic!("unexpected {other:?}"),
        }
        // `[s s]` concatenates struct arrays element-wise.
        s.run("v = [s s];").unwrap();
        assert_eq!(eval_to(&mut s, "size(v)"), eval_to(&mut s, "[1 4]"));
        assert_eq!(eval_to(&mut s, "v(4).b"), Value::Num(2.0));
        assert_eq!(eval_to(&mut s, "v(3).a"), Value::Str("x".into()));
        // getfield/setfield with an explicit element index.
        assert_eq!(eval_to(&mut s, "getfield(v, 2, 'b')"), Value::Num(2.0));
        s.run("w = setfield(v, 2, 'b', 55);").unwrap();
        assert_eq!(eval_to(&mut s, "w(2).b"), Value::Num(55.0));
    }

    // ---- C8: sparse matrices --------------------------------------------

    #[test]
    fn sparse_ctor_index_write() {
        let mut s = Session::new();
        s.run("S = sparse([1 2 3], [1 3 2], [10 20 30]);").unwrap();
        assert_eq!(eval_to(&mut s, "nnz(S)"), Value::Num(3.0));
        assert_eq!(eval_to(&mut s, "size(S)"), eval_to(&mut s, "[3 3]"));
        // Sparse reads come back as scalars.
        assert_eq!(eval_to(&mut s, "S(2,3)"), Value::Num(20.0));
        assert_eq!(eval_to(&mut s, "S(1,2)"), Value::Num(0.0));
        assert_eq!(eval_to(&mut s, "S(8)"), Value::Num(20.0));
        // Writes keep the sparse class; growth extends dims.
        s.run("S(4,4) = 99;").unwrap();
        assert_eq!(eval_to(&mut s, "size(S)"), eval_to(&mut s, "[4 4]"));
        assert_eq!(eval_to(&mut s, "S(4,4)"), Value::Num(99.0));
        s.run("S(4,4) = 0;").unwrap();
        assert_eq!(eval_to(&mut s, "nnz(S)"), Value::Num(3.0));
    }

    #[test]
    fn sparse_arithmetic_rules() {
        let mut s = Session::new();
        s.run("S = sparse([1 2 3], [1 3 2], [10 20 30]);").unwrap();
        // sparse+sparse stays sparse; nnz counts.
        s.run("T = S + S;").unwrap();
        assert_eq!(eval_to(&mut s, "issparse(T)"), Value::Num(1.0));
        assert_eq!(eval_to(&mut s, "T(3,2)"), Value::Num(60.0));
        // sparse*scalar stays sparse.
        s.run("Q = S * 2;").unwrap();
        assert_eq!(eval_to(&mut s, "issparse(Q)"), Value::Num(1.0));
        assert_eq!(eval_to(&mut s, "Q(3,2)"), Value::Num(60.0));
        // sparse .* sparse keeps the intersection only.
        s.run("R = sparse([1 3], [1 2], [5 7], 3, 3); B = S .* R;").unwrap();
        assert_eq!(eval_to(&mut s, "nnz(B)"), Value::Num(2.0));
        assert_eq!(eval_to(&mut s, "B(1,1)"), Value::Num(50.0));
        // sparse matmul: S*S on the checkerboard pattern.
        s.run("P = S * S;").unwrap();
        assert_eq!(eval_to(&mut s, "P(1,1)"), Value::Num(100.0));
        assert_eq!(eval_to(&mut s, "P(2,2)"), Value::Num(600.0));
        // Mixed sparse+full densifies.
        s.run("D = S + 1;").unwrap();
        assert_eq!(eval_to(&mut s, "issparse(D)"), Value::Num(0.0));
        assert_eq!(eval_to(&mut s, "D(1,2)"), Value::Num(1.0));
        // Transpose stays sparse.
        s.run("T2 = S';").unwrap();
        assert_eq!(eval_to(&mut s, "T2(2,3)"), Value::Num(30.0));
        // Negation stays sparse.
        assert_eq!(eval_to(&mut s, "nnz(-S)"), Value::Num(3.0));
    }

    #[test]
    fn sparse_helpers_and_find() {
        let mut s = Session::new();
        s.run("S = sparse([1 2 3], [1 3 2], [10 20 30]);").unwrap();
        assert_eq!(eval_to(&mut s, "numel(speye(3))"), Value::Num(9.0));
        assert_eq!(eval_to(&mut s, "nnz(speye(4))"), Value::Num(4.0));
        assert_eq!(eval_to(&mut s, "size(spalloc(5,5,10))"), eval_to(&mut s, "[5 5]"));
        assert_eq!(eval_to(&mut s, "nnz(spones(S))"), Value::Num(3.0));
        // Column-major (linear-index) order, like MATLAB.
        assert_eq!(eval_to(&mut s, "nonzeros(S)'"), eval_to(&mut s, "[10 30 20]"));
        // find(S) → linear indices; [i,j,v] → triplets.
        assert_eq!(eval_to(&mut s, "find(S)'"), eval_to(&mut s, "[1 6 8]"));
        s.run("[i,j,vv] = find(S);").unwrap();
        assert_eq!(eval_to(&mut s, "i'"), eval_to(&mut s, "[1 3 2]"));
        assert_eq!(eval_to(&mut s, "j'"), eval_to(&mut s, "[1 2 3]"));
        assert_eq!(eval_to(&mut s, "vv'"), eval_to(&mut s, "[10 30 20]"));
        // full round-trip.
        s.run("D = full(S); S2 = sparse(D);").unwrap();
        assert_eq!(eval_to(&mut s, "nnz(S2)"), Value::Num(3.0));
        assert_eq!(eval_to(&mut s, "S2(3,2)"), Value::Num(30.0));
        // Concatenation stays sparse.
        s.run("C = [S S];").unwrap();
        assert_eq!(eval_to(&mut s, "issparse(C)"), Value::Num(1.0));
        assert_eq!(eval_to(&mut s, "nnz(C)"), Value::Num(6.0));
        assert_eq!(eval_to(&mut s, "C(2,6)"), Value::Num(20.0));
    }

    // ---- D11: DSP toolbox ------------------------------------------------

    #[test]
    fn dsp_fft_ifft() {
        let mut s = Session::new();
        // FFT of [1 2 3 4] = [10, -2+2i, -2, -2-2i]; ifft round-trips.
        s.run("X = fft([1 2 3 4]); Y = ifft(X);").unwrap();
        match s.get("X") {
            Some(Value::Cplx(m)) => {
                assert_eq!(m.get(0, 0), Some(CNum::new(10.0, 0.0)));
                let z = m.get(0, 1).unwrap_or(CNum::real(0.0));
                assert!((z.re + 2.0).abs() < 1e-9 && (z.im - 2.0).abs() < 1e-9, "{z:?}");
            }
            other => panic!("expected complex, got {other:?}"),
        }
        assert_eq!(eval_to(&mut s, "norm(Y - [1 2 3 4]) < 1e-9"), Value::Num(1.0));
        // fft(x, n) pads/truncates; matrix input transforms columns.
        assert_eq!(eval_to(&mut s, "length(fft([1 1], 8))"), Value::Num(8.0));
        // Column FFTs of ones(2,3) → each column [2; 0]: column sums [2 2 2].
        assert_eq!(eval_to(&mut s, "norm(sum(fft(ones(2,3))) - [2 2 2]) < 1e-9"), Value::Num(1.0));
        // fft2 transforms both dims; fftshift reorders.
        s.run("F = fft2([1 0; 0 0]);").unwrap();
        assert_eq!(eval_to(&mut s, "F(1,1) + F(2,2)"), Value::Num(2.0));
        assert_eq!(eval_to(&mut s, "fftshift([1 2 3 4 5])"), eval_to(&mut s, "[4 5 1 2 3]"));
        assert_eq!(eval_to(&mut s, "ifftshift([4 5 1 2 3])"), eval_to(&mut s, "[1 2 3 4 5]"));
        // Prime length falls back to the naive DFT and still round-trips.
        assert_eq!(eval_to(&mut s, "norm(ifft(fft([1 2 3 4 5])) - [1 2 3 4 5]) < 1e-9"), Value::Num(1.0));
    }

    #[test]
    fn dsp_filter_xcorr() {
        let mut s = Session::new();
        // FIR: y = moving sum; IIR integrator: y = cumsum.
        assert_eq!(eval_to(&mut s, "filter([1 1], 1, [1 2 3 4])"), eval_to(&mut s, "[1 3 5 7]"));
        assert_eq!(eval_to(&mut s, "filter(1, [1 -1], [1 1 1])"), eval_to(&mut s, "[1 2 3]"));
        s.run("[y, zf] = filter([1 1], 1, [1 2 3]);").unwrap();
        assert_eq!(eval_to(&mut s, "zf"), Value::Num(3.0));
        // a(1) == 0 errors.
        assert!(s.run("filter(1, [0 1], [1 2]);").is_err());
        // xcorr peak at lag +1 for x = delay(y); [r,lags] multi-output.
        s.run("[r, l] = xcorr([0 1 0], [1 0 0]);").unwrap();
        assert_eq!(eval_to(&mut s, "l'"), eval_to(&mut s, "[-2 -1 0 1 2]"));
        assert_eq!(eval_to(&mut s, "r'"), eval_to(&mut s, "[0 0 0 1 0]"));
        // maxlag + 'coeff' normalization.
        s.run("[r2, l2] = xcorr([1 0 0], [1 0 0], 1, 'coeff');").unwrap();
        assert_eq!(eval_to(&mut s, "l2'"), eval_to(&mut s, "[-1 0 1]"));
        assert_eq!(eval_to(&mut s, "r2(2) == 1"), Value::Num(1.0));
    }

    #[test]
    fn dsp_freqz_windows() {
        let mut s = Session::new();
        // H(z) = 1 + z^-1: |H| = 2 at w=0, √2 at π/2, 0 at π.
        s.run("[h, w] = freqz([1 1], 1, 4);").unwrap();
        assert_eq!(eval_to(&mut s, "norm(w' - [0 pi/4 pi/2 3*pi/4]) < 1e-12"), Value::Num(1.0));
        assert_eq!(eval_to(&mut s, "norm(abs(h)' - [2 2*cos(pi/8) sqrt(2) 2*sin(pi/8)]) < 1e-12"), Value::Num(1.0));
        // Explicit frequency vector.
        assert_eq!(eval_to(&mut s, "abs(freqz([1 1], 1, pi)) < 1e-12"), Value::Num(1.0));
        // Hz form: freqz(b,a,f,fs) evaluates at w = 2πf/fs.
        assert_eq!(eval_to(&mut s, "norm(abs(freqz([1 1], 1, [0 200], 800))' - [2 sqrt(2)]) < 1e-12"), Value::Num(1.0));
        // Windows: endpoints and symmetry.
        assert_eq!(eval_to(&mut s, "norm(hamming(4)' - [0.08 0.77 0.77 0.08]) < 1e-12"), Value::Num(1.0));
        assert_eq!(eval_to(&mut s, "norm(hann(3)' - [0 1 0]) < 1e-12"), Value::Num(1.0));
        assert_eq!(eval_to(&mut s, "rectwin(3)'"), eval_to(&mut s, "[1 1 1]"));
        assert_eq!(eval_to(&mut s, "norm(kaiser(3,5)' - [0.0367 1 0.0367]) < 1e-4"), Value::Num(1.0));
        assert_eq!(eval_to(&mut s, "unwrap([0 1 1+2*pi])"), eval_to(&mut s, "[0 1 1]"));
        assert_eq!(eval_to(&mut s, "size(dftmtx(4))"), eval_to(&mut s, "[4 4]"));
        // roots now returns complex pairs.
        s.run("r = roots([1 0 1]);").unwrap();
        match s.get("r") {
            Some(Value::Cplx(m)) => {
                assert!((m.get(0, 0).unwrap_or(CNum::real(0.0)).im - 1.0).abs() < 1e-6);
            }
            other => panic!("expected complex roots, got {other:?}"),
        }
    }
}
