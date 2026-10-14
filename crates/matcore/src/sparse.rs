//! Sparse double matrix — coordinate storage for `Value::Sparse`.
//!
//! Storage is a sorted `(row, col) -> value` map: sparse matrices in M-code
//! workloads are built incrementally (`S(i,j) = v`) and queried by
//! subscript, so a map beats CSC for the sizes we target. `full()`
//! materializes the dense `Matrix`; arithmetic follows the MATLAB rule
//! that mixed sparse+full ops return full, while sparse+sparse stays
//! sparse.

use std::collections::BTreeMap;

use crate::{MatError, Matrix};

/// `rows x cols` sparse matrix of `f64`, storing only nonzero entries.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SparseMat {
    rows: usize,
    cols: usize,
    vals: BTreeMap<(usize, usize), f64>,
}

impl SparseMat {
    /// Empty `rows x cols` sparse matrix (`sparse(m,n)` / `spalloc`).
    pub fn zeros(rows: usize, cols: usize) -> Self {
        SparseMat { rows, cols, vals: BTreeMap::new() }
    }

    /// `sparse(i,j,v,m,n)` — build from 1-based triplet lists; duplicate
    /// (i,j) entries sum, matching MATLAB.
    pub fn from_triplets(rows: usize, cols: usize, is: &[f64], js: &[f64], vs: &[f64]) -> Result<Self, MatError> {
        let mut s = SparseMat::zeros(rows, cols);
        for k in 0..is.len().min(js.len()).min(vs.len()) {
            let i = is[k] as usize;
            let j = js[k] as usize;
            if i == 0 || i > rows || j == 0 || j > cols || is[k].fract() != 0.0 || js[k].fract() != 0.0 {
                return Err(MatError::Other("sparse subscript out of range".to_string()));
            }
            let key = (i - 1, j - 1);
            let e = s.vals.entry(key).or_insert(0.0);
            *e += vs[k];
            if *e == 0.0 {
                s.vals.remove(&key);
            }
        }
        Ok(s)
    }

    /// `sparse(A)` — sparsify a dense matrix, dropping exact zeros.
    pub fn from_dense(m: &Matrix) -> Self {
        let mut s = SparseMat::zeros(m.rows(), m.cols());
        for c in 0..m.cols() {
            for r in 0..m.rows() {
                let v = m.get(r, c).unwrap_or(0.0);
                if v != 0.0 {
                    s.vals.insert((r, c), v);
                }
            }
        }
        s
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn shape(&self) -> (usize, usize) {
        (self.rows, self.cols)
    }

    /// `nnz` — number of stored nonzero entries.
    pub fn nnz(&self) -> usize {
        self.vals.len()
    }

    /// `nonzeros` — values in column-major (MATLAB linear-index) order.
    pub fn nonzeros(&self) -> Vec<f64> {
        self.entries().map(|(_, v)| v).collect()
    }

    /// Triplet list for `find(S)` / iteration — 0-based (r,c), value,
    /// in column-major order like MATLAB's sparse storage.
    pub fn entries(&self) -> impl Iterator<Item = ((usize, usize), f64)> {
        let mut v: Vec<((usize, usize), f64)> = self.vals.iter().map(|(&k, &x)| (k, x)).collect();
        v.sort_by_key(|&((r, c), _)| (c, r));
        v.into_iter()
    }

    /// `S(i,j)` — 0 for a missing entry.
    pub fn get(&self, r: usize, c: usize) -> f64 {
        self.vals.get(&(r, c)).copied().unwrap_or(0.0)
    }

    /// `S(i,j) = v` — writes or deletes; grows the matrix dims like
    /// MATLAB (`S(10,10) = 1` on a 2x2 extends to 10x10).
    pub fn set(&mut self, r: usize, c: usize, v: f64) {
        if v == 0.0 {
            self.vals.remove(&(r, c));
        } else {
            self.vals.insert((r, c), v);
        }
        self.rows = self.rows.max(r + 1);
        self.cols = self.cols.max(c + 1);
    }

    /// `full(S)` — materialize the dense matrix.
    pub fn full(&self) -> Matrix {
        let mut m = Matrix::zeros(self.rows, self.cols);
        for (&(r, c), &v) in &self.vals {
            let _ = m.set(r, c, v);
        }
        m
    }

    /// `speye(n)`.
    pub fn eye(n: usize) -> Self {
        let mut s = SparseMat::zeros(n, n);
        for i in 0..n {
            s.vals.insert((i, i), 1.0);
        }
        s
    }

    /// `S'` — transpose (conjugate transpose is identical for reals).
    pub fn transpose(&self) -> Self {
        let mut s = SparseMat::zeros(self.cols, self.rows);
        for (&(r, c), &v) in &self.vals {
            s.vals.insert((c, r), v);
        }
        s
    }

    /// Element-wise combine — used by `+`/`-`. A 1x1 operand broadcasts
    /// (f(v, 0) may still be nonzero — e.g. `S + s` on zeros).
    fn zip(&self, b: &SparseMat, f: impl Fn(f64, f64) -> f64) -> Result<Self, MatError> {
        if self.shape() != b.shape() {
            // A 1x1 operand broadcasts its stored value across the other's
            // nonzero pattern; a stored nonzero scalar would densify the
            // result (f(0,k) != 0), which sparse can't represent here.
            let (big, k, swap) = if b.numel() == 1 {
                (self, b.get(0, 0), false)
            } else if self.numel() == 1 {
                (b, self.get(0, 0), true)
            } else {
                return Err(MatError::DimensionMismatch { a: self.shape(), b: b.shape() });
            };
            let mut out = SparseMat::zeros(big.rows, big.cols);
            for (&key, &v) in &big.vals {
                let nv = if swap { f(k, v) } else { f(v, k) };
                if nv != 0.0 {
                    out.vals.insert(key, nv);
                }
            }
            return Ok(out);
        }
        // +/- on equal shapes: the result lives on the union of nonzero
        // patterns — f(0,0) == 0 for both ops.
        let mut out = SparseMat::zeros(self.rows, self.cols);
        let keys: std::collections::BTreeSet<(usize, usize)> = self.vals.keys().chain(b.vals.keys()).copied().collect();
        for k in keys {
            let v = f(self.vals.get(&k).copied().unwrap_or(0.0), b.vals.get(&k).copied().unwrap_or(0.0));
            if v != 0.0 {
                out.vals.insert(k, v);
            }
        }
        Ok(out)
    }

    pub fn numel(&self) -> usize {
        self.rows * self.cols
    }

    /// `S + T`, `S - T`, `S .* T` (sparse ⊙ sparse stays sparse).
    pub fn add(&self, b: &SparseMat) -> Result<Self, MatError> {
        self.zip(b, |a, c| a + c)
    }

    pub fn sub(&self, b: &SparseMat) -> Result<Self, MatError> {
        self.zip(b, |a, c| a - c)
    }

    /// `S .* T` — the result lives on the intersection of nonzero
    /// patterns (0·anything = 0), so only stored entries need scanning.
    pub fn mul_elem(&self, b: &SparseMat) -> Result<Self, MatError> {
        if self.numel() == 1 {
            let k = self.get(0, 0);
            return Ok(if k == 0.0 { SparseMat::zeros(b.rows, b.cols) } else { b.map(|v| v * k) });
        }
        if b.numel() == 1 {
            let k = b.get(0, 0);
            return Ok(if k == 0.0 { SparseMat::zeros(self.rows, self.cols) } else { self.map(|v| v * k) });
        }
        if self.shape() != b.shape() {
            return Err(MatError::DimensionMismatch { a: self.shape(), b: b.shape() });
        }
        let (small, big) = if self.vals.len() <= b.vals.len() { (self, b) } else { (b, self) };
        let mut out = SparseMat::zeros(self.rows, self.cols);
        for (&k, &av) in &small.vals {
            let v = av * big.vals.get(&k).copied().unwrap_or(0.0);
            if v != 0.0 {
                out.vals.insert(k, v);
            }
        }
        Ok(out)
    }

    /// `S * T` — sparse matmul; O(nnz_a * nnz_b_per_row).
    pub fn matmul(&self, b: &SparseMat) -> Result<Self, MatError> {
        if self.cols != b.rows {
            return Err(MatError::DimensionMismatch { a: (self.rows, self.cols), b: (b.rows, b.cols) });
        }
        let mut out = SparseMat::zeros(self.rows, b.cols);
        // Bucket b's entries by row for fast column-strip lookup.
        let mut by_row: BTreeMap<usize, Vec<(usize, f64)>> = BTreeMap::new();
        for (&(r, c), &v) in &b.vals {
            by_row.entry(r).or_default().push((c, v));
        }
        for (&(r, k), &av) in &self.vals {
            if let Some(strip) = by_row.get(&k) {
                for &(c, bv) in strip {
                    let e = out.vals.entry((r, c)).or_insert(0.0);
                    *e += av * bv;
                }
            }
        }
        out.vals.retain(|_, v| *v != 0.0);
        Ok(out)
    }

    /// `[S T]` — horizontal concat keeps sparse when both are sparse.
    pub fn hcat(&self, b: &SparseMat) -> Result<Self, MatError> {
        if self.rows != b.rows {
            return Err(MatError::DimensionMismatch { a: self.shape(), b: b.shape() });
        }
        let mut out = SparseMat::zeros(self.rows, self.cols + b.cols);
        for (&(r, c), &v) in &self.vals {
            out.vals.insert((r, c), v);
        }
        for (&(r, c), &v) in &b.vals {
            out.vals.insert((r, c + self.cols), v);
        }
        Ok(out)
    }

    /// `[S; T]` — vertical concat keeps sparse when both are sparse.
    pub fn vcat(&self, b: &SparseMat) -> Result<Self, MatError> {
        if self.cols != b.cols {
            return Err(MatError::DimensionMismatch { a: self.shape(), b: b.shape() });
        }
        let mut out = SparseMat::zeros(self.rows + b.rows, self.cols);
        for (&(r, c), &v) in &self.vals {
            out.vals.insert((r, c), v);
        }
        for (&(r, c), &v) in &b.vals {
            out.vals.insert((r + self.rows, c), v);
        }
        Ok(out)
    }

    /// `-S`, `S .* k`, `S + k` (scalar ops).
    pub fn map(&self, f: impl Fn(f64) -> f64) -> Self {
        // Only valid for functions with f(0) == 0 (negate, scale, …).
        let mut out = SparseMat::zeros(self.rows, self.cols);
        for (&k, &v) in &self.vals {
            let nv = f(v);
            if nv != 0.0 {
                out.vals.insert(k, nv);
            }
        }
        out
    }
}
