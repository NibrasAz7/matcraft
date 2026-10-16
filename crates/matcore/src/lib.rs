//! MatCraft numerical core.
//!
//! Dense, column-major `f64` matrices — the same memory layout classic
//! numerical codes (BLAS/LAPACK) expect, and the layout M-code semantics
//! assume for linear indexing (`A(4)` walks down columns).
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::fmt;

mod complex;
pub mod dsp;
mod linalg;
mod sparse;

pub use complex::{CMatrix, CNum};
pub use linalg::Svd;
pub use linalg::{chol, cond, eig_symmetric, eigvals, eigvals_c, lu, pinv, qr, rank, rref, singular_values, svd, trace};
pub use sparse::SparseMat;

/// Errors from matrix construction and algebra.
#[derive(Debug, Clone, PartialEq)]
pub enum MatError {
    /// Dimensions don't match for the requested operation.
    DimensionMismatch { a: (usize, usize), b: (usize, usize) },
    /// N-D shapes don't match for the requested operation.
    DimsMismatch { a: Vec<usize>, b: Vec<usize> },
    /// A solve/decomposition needs a square or nonsingular matrix.
    Singular,
    /// Index out of bounds.
    OutOfBounds { index: (usize, usize), shape: (usize, usize) },
    /// Data length doesn't match the declared shape.
    BadData { len: usize, expected: usize },
    /// Anything else (unsupported spectrum, convergence failure, …).
    Other(String),
}

impl fmt::Display for MatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MatError::DimensionMismatch { a, b } => {
                write!(f, "dimension mismatch: {}x{} vs {}x{}", a.0, a.1, b.0, b.1)
            }
            MatError::DimsMismatch { a, b } => {
                let dims = |d: &[usize]| d.iter().map(usize::to_string).collect::<Vec<_>>().join("x");
                write!(f, "dimension mismatch: {} vs {}", dims(a), dims(b))
            }
            MatError::Singular => write!(f, "matrix is singular or not square"),
            MatError::OutOfBounds { index, shape } => write!(f, "index ({},{}) out of bounds for {}x{} matrix", index.0, index.1, shape.0, shape.1),
            MatError::BadData { len, expected } => {
                write!(f, "got {} elements for a {}-element matrix", len, expected)
            }
            MatError::Other(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for MatError {}

/// Dense column-major array of f64 with two or more dims — a plain matrix
/// when `dims.len() == 2`, an N-D array otherwise. `data` is column-major,
/// so element `(i, j, k, …)` (0-based) sits at `i + j*d0 + k*d0*d1 + …`
/// and a single linear index always walks the whole array.
#[derive(Debug, Clone, PartialEq)]
pub struct Matrix {
    /// Extent of each dimension; `dims.len() >= 2` always — `[r, c]` is the
    /// classic matrix case. Trailing dims of extent 1 are kept: `size`
    /// reports exactly this vector.
    dims: Vec<usize>,
    /// `data.len() == product(dims)`; layout described above.
    data: Vec<f64>,
}

fn dim_product(dims: &[usize]) -> usize {
    dims.iter().fold(1usize, |acc, &d| acc.saturating_mul(d))
}

impl Matrix {
    /// Zero matrix.
    pub fn zeros(rows: usize, cols: usize) -> Self {
        Matrix { dims: vec![rows, cols], data: vec![0.0; rows.saturating_mul(cols)] }
    }

    /// Zero N-D array (`dims` must have at least two entries).
    pub fn zeros_nd(dims: &[usize]) -> Self {
        let dims = if dims.len() >= 2 { dims.to_vec() } else { vec![0, 0] };
        Matrix { data: vec![0.0; dim_product(&dims)], dims }
    }

    /// Matrix of ones.
    pub fn ones(rows: usize, cols: usize) -> Self {
        Matrix { dims: vec![rows, cols], data: vec![1.0; rows.saturating_mul(cols)] }
    }

    /// N-D array of ones (`dims` must have at least two entries).
    pub fn ones_nd(dims: &[usize]) -> Self {
        let dims = if dims.len() >= 2 { dims.to_vec() } else { vec![0, 0] };
        Matrix { data: vec![1.0; dim_product(&dims)], dims }
    }

    /// Identity matrix (max(rows, cols) diagonal of ones).
    pub fn eye(rows: usize, cols: usize) -> Self {
        let mut m = Matrix::zeros(rows, cols);
        for i in 0..rows.min(cols) {
            if let Some(slot) = m.data.get_mut(i * rows + i) {
                *slot = 1.0;
            }
        }
        m
    }

    /// Build from column-major storage.
    pub fn from_col_major(rows: usize, cols: usize, data: Vec<f64>) -> Result<Self, MatError> {
        if data.len() != rows.saturating_mul(cols) {
            return Err(MatError::BadData { len: data.len(), expected: rows.saturating_mul(cols) });
        }
        Ok(Matrix { dims: vec![rows, cols], data })
    }

    /// Build an N-D array from column-major storage (`dims.len() >= 2`).
    pub fn from_col_major_nd(dims: &[usize], data: Vec<f64>) -> Result<Self, MatError> {
        if dims.len() < 2 {
            return Err(MatError::BadData { len: data.len(), expected: 0 });
        }
        if data.len() != dim_product(dims) {
            return Err(MatError::BadData { len: data.len(), expected: dim_product(dims) });
        }
        Ok(Matrix { dims: dims.to_vec(), data })
    }

    /// Build from row-major storage (the natural way to write literals).
    pub fn from_row_major(rows: usize, cols: usize, data: &[f64]) -> Result<Self, MatError> {
        if data.len() != rows.saturating_mul(cols) {
            return Err(MatError::BadData { len: data.len(), expected: rows.saturating_mul(cols) });
        }
        let mut m = Matrix::zeros(rows, cols);
        for r in 0..rows {
            for c in 0..cols {
                let v = data.get(r * cols + c).copied().unwrap_or(0.0);
                if let Some(slot) = m.data.get_mut(c * rows + r) {
                    *slot = v;
                }
            }
        }
        Ok(m)
    }

    /// 1x1 scalar matrix.
    pub fn scalar(v: f64) -> Self {
        Matrix { dims: vec![1, 1], data: vec![v] }
    }

    /// Row vector `1 x values.len()`.
    pub fn row_vector(values: Vec<f64>) -> Self {
        Matrix { dims: vec![1, values.len()], data: values }
    }

    /// Column vector `values.len() x 1`.
    pub fn col_vector(values: Vec<f64>) -> Self {
        Matrix { dims: vec![values.len(), 1], data: values }
    }

    pub fn rows(&self) -> usize {
        self.dims.first().copied().unwrap_or(0)
    }

    pub fn cols(&self) -> usize {
        self.dims.get(1).copied().unwrap_or(0)
    }

    /// First two dims as a `(rows, cols)` pair — the classic matrix view.
    pub fn shape(&self) -> (usize, usize) {
        (self.rows(), self.cols())
    }

    /// Extent of every stored dimension (`len() >= 2`).
    pub fn dims(&self) -> &[usize] {
        &self.dims
    }

    /// Number of stored dimensions (`size` reports this many).
    pub fn ndims(&self) -> usize {
        self.dims.len()
    }

    /// Whether the array is effectively a 2-D matrix: every dim past the
    /// second has extent 1. Matrix-only ops (`*`, `\`, `det`, …) use this
    /// so `x(:,:,1)` results still work.
    pub fn is_matrix(&self) -> bool {
        self.dims.iter().skip(2).all(|&d| d <= 1)
    }

    pub fn numel(&self) -> usize {
        self.data.len()
    }

    /// Column-major element access.
    pub fn data(&self) -> &[f64] {
        &self.data
    }

    /// Element at (row, col), 0-based — the (row, col, 0, …) element of an
    /// N-D array.
    pub fn get(&self, row: usize, col: usize) -> Option<f64> {
        if row < self.rows() && col < self.cols() {
            self.data.get(col * self.rows() + row).copied()
        } else {
            None
        }
    }

    /// Element at 0-based subscripts `subs` (one per dim). Missing trailing
    /// subscripts read as 0.
    pub fn get_nd(&self, subs: &[usize]) -> Option<f64> {
        self.offset(subs).and_then(|i| self.data.get(i).copied())
    }

    /// Column-major offset of 0-based subscripts, or `None` when any
    /// subscript is out of bounds or `subs` has too many entries.
    pub fn offset(&self, subs: &[usize]) -> Option<usize> {
        if subs.len() > self.dims.len() {
            return None;
        }
        let mut off = 0usize;
        let mut stride = 1usize;
        for (i, &d) in self.dims.iter().enumerate() {
            let s = subs.get(i).copied().unwrap_or(0);
            if s >= d {
                return None;
            }
            off = off.saturating_add(s.saturating_mul(stride));
            stride = stride.saturating_mul(d);
        }
        Some(off)
    }

    /// Set element at (row, col), 0-based.
    pub fn set(&mut self, row: usize, col: usize, v: f64) -> Result<(), MatError> {
        let (rows, cols) = self.shape();
        if row < rows && col < cols {
            if let Some(slot) = self.data.get_mut(col * rows + row) {
                *slot = v;
            }
            Ok(())
        } else {
            Err(MatError::OutOfBounds { index: (row, col), shape: (rows, cols) })
        }
    }

    /// Set element at 0-based subscripts `subs`.
    pub fn set_nd(&mut self, subs: &[usize], v: f64) -> Result<(), MatError> {
        match self.offset(subs) {
            Some(i) => {
                if let Some(slot) = self.data.get_mut(i) {
                    *slot = v;
                }
                Ok(())
            }
            None => Err(MatError::Other(format!("subscript index out of bounds for dims {:?}", self.dims))),
        }
    }

    /// Linear index (0-based, column-major order).
    pub fn get_linear(&self, i: usize) -> Option<f64> {
        self.data.get(i).copied()
    }

    /// Write by linear index (0-based, column-major order); out-of-range
    /// writes are ignored.
    pub fn set_linear(&mut self, i: usize, v: f64) {
        if let Some(slot) = self.data.get_mut(i) {
            *slot = v;
        }
    }

    pub fn is_scalar(&self) -> bool {
        self.data.len() == 1
    }

    /// Scalar value of a 1x1 matrix.
    pub fn as_scalar(&self) -> Option<f64> {
        if self.is_scalar() {
            self.data.first().copied()
        } else {
            None
        }
    }

    /// Elementwise map.
    pub fn map(&self, f: impl Fn(f64) -> f64) -> Self {
        Matrix { dims: self.dims.clone(), data: self.data.iter().map(|&v| f(v)).collect() }
    }

    /// Do two dims vectors describe the same shape, counting implicit
    /// trailing singleton dims as equal? `[2 3]` ≡ `[2 3 1]`.
    pub fn same_shape(a: &[usize], b: &[usize]) -> bool {
        let n = a.len().max(b.len());
        (0..n).all(|i| a.get(i).copied().unwrap_or(1) == b.get(i).copied().unwrap_or(1))
    }

    /// Elementwise binary op; shapes must match exactly (implicit trailing
    /// singleton dims aside). The result keeps the longer dims vector.
    pub fn elemwise(&self, other: &Matrix, f: impl Fn(f64, f64) -> f64) -> Result<Self, MatError> {
        if self.numel() != other.numel() || !Self::same_shape(&self.dims, &other.dims) {
            return Err(MatError::DimsMismatch { a: self.dims.clone(), b: other.dims.clone() });
        }
        let dims = if other.dims.len() > self.dims.len() { other.dims.clone() } else { self.dims.clone() };
        let data = self.data.iter().zip(other.data.iter()).map(|(&a, &b)| f(a, b)).collect();
        Ok(Matrix { dims, data })
    }

    /// Broadcast `other` if it is a scalar, else require equal shapes.
    pub fn broadcast(&self, other: &Matrix, f: impl Fn(f64, f64) -> f64) -> Result<Self, MatError> {
        if let Some(s) = other.as_scalar() {
            return Ok(self.map(|a| f(a, s)));
        }
        if let Some(s) = self.as_scalar() {
            return Ok(other.map(|b| f(s, b)));
        }
        self.elemwise(other, f)
    }

    /// Matrix multiply: (m x k) * (k x n).
    /// Scalar operands broadcast to elementwise scaling first — `2 * A` must
    /// work regardless of A's shape.
    pub fn matmul(&self, other: &Matrix) -> Result<Self, MatError> {
        if let Some(s) = self.as_scalar() {
            return Ok(other.map(|b| s * b));
        }
        if let Some(s) = other.as_scalar() {
            return Ok(self.map(|a| a * s));
        }
        if !self.is_matrix() || !other.is_matrix() {
            return Err(MatError::Other("matrix multiply requires 2-D operands".into()));
        }
        if self.cols() != other.rows() {
            return Err(MatError::DimensionMismatch { a: self.shape(), b: other.shape() });
        }
        let (m, k, n) = (self.rows(), self.cols(), other.cols());
        let mut out = Matrix::zeros(m, n);
        for c in 0..n {
            for i in 0..k {
                let b = other.get(i, c).unwrap_or(0.0);
                if b == 0.0 {
                    continue;
                }
                for r in 0..m {
                    let a = self.get(r, i).unwrap_or(0.0);
                    if let Some(slot) = out.data.get_mut(c * m + r) {
                        *slot += a * b;
                    }
                }
            }
        }
        Ok(out)
    }

    /// Transpose — defined on the 2-D view; callers must check
    /// [`Matrix::is_matrix`] first (an N-D transpose is not defined).
    pub fn transpose(&self) -> Self {
        let mut out = Matrix::zeros(self.cols(), self.rows());
        for r in 0..self.rows() {
            for c in 0..self.cols() {
                if let (Some(v), Some(slot)) = (self.get(r, c), out.data.get_mut(r * self.cols() + c)) {
                    *slot = v;
                }
            }
        }
        out
    }

    /// LU solve `self * x = b` with partial pivoting.
    pub fn solve(&self, b: &Matrix) -> Result<Matrix, MatError> {
        if !self.is_matrix() || !b.is_matrix() {
            return Err(MatError::Other("solve requires 2-D operands".into()));
        }
        if self.rows() != self.cols() || b.rows() != self.rows() {
            return Err(MatError::DimensionMismatch { a: self.shape(), b: b.shape() });
        }
        let n = self.rows();
        // Augmented copy, row-major for cache-friendly elimination.
        let mut a = vec![0.0; n.saturating_mul(n + b.cols())];
        let width = n + b.cols();
        for r in 0..n {
            for c in 0..n {
                if let Some(dst) = a.get_mut(r * width + c) {
                    *dst = self.get(r, c).unwrap_or(0.0);
                }
            }
            for c in 0..b.cols() {
                if let Some(dst) = a.get_mut(r * width + n + c) {
                    *dst = b.get(r, c).unwrap_or(0.0);
                }
            }
        }
        // Forward elimination with partial pivoting.
        for col in 0..n {
            let mut pivot = col;
            let mut best = a.get(col * width + col).map(|v| v.abs()).unwrap_or(0.0);
            for r in (col + 1)..n {
                let v = a.get(r * width + col).map(|v| v.abs()).unwrap_or(0.0);
                if v > best {
                    best = v;
                    pivot = r;
                }
            }
            if best <= f64::EPSILON {
                return Err(MatError::Singular);
            }
            if pivot != col {
                for c in 0..width {
                    a.swap(col * width + c, pivot * width + c);
                }
            }
            let diag = a.get(col * width + col).copied().unwrap_or(1.0);
            for r in (col + 1)..n {
                let factor = a.get(r * width + col).copied().unwrap_or(0.0) / diag;
                if factor == 0.0 {
                    continue;
                }
                for c in col..width {
                    let sub = factor * a.get(col * width + c).copied().unwrap_or(0.0);
                    if let Some(slot) = a.get_mut(r * width + c) {
                        *slot -= sub;
                    }
                }
            }
        }
        // Back substitution.
        let mut x = Matrix::zeros(n, b.cols());
        for col in 0..b.cols() {
            for r in (0..n).rev() {
                let mut acc = a.get(r * width + n + col).copied().unwrap_or(0.0);
                for c in (r + 1)..n {
                    acc -= a.get(r * width + c).copied().unwrap_or(0.0) * x.get(c, col).unwrap_or(0.0);
                }
                let diag = a.get(r * width + r).copied().unwrap_or(1.0);
                if let Some(slot) = x.data.get_mut(col * n + r) {
                    *slot = acc / diag;
                }
            }
        }
        Ok(x)
    }

    /// Inverse via repeated solves (columns of the identity).
    pub fn inverse(&self) -> Result<Matrix, MatError> {
        if !self.is_matrix() {
            return Err(MatError::Other("inverse requires a 2-D operand".into()));
        }
        if self.rows() != self.cols() {
            return Err(MatError::Singular);
        }
        self.solve(&Matrix::eye(self.rows(), self.cols()))
    }

    /// Determinant via LU elimination (sign tracked on pivot swaps).
    pub fn det(&self) -> Result<f64, MatError> {
        if !self.is_matrix() {
            return Err(MatError::Other("det requires a 2-D operand".into()));
        }
        if self.rows() != self.cols() {
            return Err(MatError::Singular);
        }
        let n = self.rows();
        let mut a = self.data.clone(); // column-major
        let mut sign = 1.0;
        let mut det = 1.0;
        for col in 0..n {
            let mut pivot = col;
            let mut best = a.get(col * n + col).map(|v| v.abs()).unwrap_or(0.0);
            for r in (col + 1)..n {
                // Element (row r, col): a[col * n + r] in column-major order.
                let v = a.get(col * n + r).map(|v| v.abs()).unwrap_or(0.0);
                if v > best {
                    best = v;
                    pivot = r;
                }
            }
            if best <= f64::EPSILON {
                return Ok(0.0);
            }
            if pivot != col {
                // Swap rows r=pivot and r=col across all columns.
                for c in 0..n {
                    a.swap(c * n + col, c * n + pivot);
                }
                sign = -sign;
            }
            let diag = a.get(col * n + col).copied().unwrap_or(1.0);
            det *= diag;
            for r in (col + 1)..n {
                let factor = a.get(col * n + r).copied().unwrap_or(0.0) / diag;
                if factor == 0.0 {
                    continue;
                }
                for c in (col + 1)..n {
                    let sub = factor * a.get(c * n + col).copied().unwrap_or(0.0);
                    if let Some(slot) = a.get_mut(c * n + r) {
                        *slot -= sub;
                    }
                }
            }
        }
        Ok(det * sign)
    }

    /// 2-norm (Frobenius for matrices, Euclidean for vectors).
    pub fn norm(&self) -> f64 {
        self.data.iter().map(|v| v * v).sum::<f64>().sqrt()
    }

    /// Reshape preserving column-major element order.
    pub fn reshape(&self, rows: usize, cols: usize) -> Result<Matrix, MatError> {
        self.reshape_nd(&[rows, cols])
    }

    /// Reshape to any dims (`len >= 2`) preserving column-major order.
    pub fn reshape_nd(&self, dims: &[usize]) -> Result<Matrix, MatError> {
        if dims.len() < 2 {
            return Err(MatError::BadData { len: self.data.len(), expected: 0 });
        }
        if dim_product(dims) != self.data.len() {
            return Err(MatError::BadData { len: self.data.len(), expected: dim_product(dims) });
        }
        Ok(Matrix { dims: dims.to_vec(), data: self.data.clone() })
    }

    /// Drop every extent-1 dim, keeping at least two — `squeeze`.
    /// (`squeeze` of a row vector gives a column, like M-code.)
    pub fn squeezed(&self) -> Matrix {
        let mut dims: Vec<usize> = self.dims.iter().copied().filter(|&d| d != 1).collect();
        while dims.len() < 2 {
            dims.push(1);
        }
        Matrix { dims, data: self.data.clone() }
    }

    /// Grow to at least (rows, cols), padding with zeros.
    pub fn grow(&mut self, rows: usize, cols: usize) {
        self.grow_nd(&[rows, cols]);
    }

    /// Grow to at least `want` dims (element-wise max per dim), padding
    /// with zeros. `want` may have more entries than `self.dims` — the
    /// dims vector extends (new dims start at extent 1), which is how
    /// `x(1,1,1) = v` on a 2-D array makes it 3-D.
    pub fn grow_nd(&mut self, want: &[usize]) {
        let nd = self.dims.len().max(want.len()).max(2);
        let mut new_dims = vec![1usize; nd];
        for (i, d) in new_dims.iter_mut().enumerate() {
            *d = self.dims.get(i).copied().unwrap_or(1).max(want.get(i).copied().unwrap_or(1));
        }
        if new_dims == self.dims {
            return;
        }
        if dim_product(&new_dims) == self.data.len() {
            // Only trailing dims of extent 1 were appended — same storage.
            self.dims = new_dims;
            return;
        }
        let mut out = vec![0.0; dim_product(&new_dims)];
        let old_dims = std::mem::replace(&mut self.dims, new_dims);
        for (i, &v) in self.data.iter().enumerate() {
            // Decompose the old linear index into old subscripts…
            let mut rem = i;
            let mut off = 0usize;
            let mut stride = 1usize;
            for (d, &od) in old_dims.iter().enumerate() {
                let s = rem % od.max(1);
                rem /= od.max(1);
                // …and re-accumulate under the new strides.
                off = off.saturating_add(s.saturating_mul(stride));
                stride = stride.saturating_mul(self.dims.get(d).copied().unwrap_or(1));
            }
            if let Some(slot) = out.get_mut(off) {
                *slot = v;
            }
        }
        self.data = out;
    }

    /// Concatenate along 0-based dimension `dim` (`cat(dim+1, a, b)`).
    /// Every other dim must match after padding missing dims with 1s.
    pub fn cat(dim: usize, a: &Matrix, b: &Matrix) -> Result<Matrix, MatError> {
        let nd = a.dims.len().max(b.dims.len()).max(dim + 1).max(2);
        let mut rd = vec![1usize; nd];
        for (i, d) in rd.iter_mut().enumerate() {
            let (da, db) = (a.dims.get(i).copied().unwrap_or(1), b.dims.get(i).copied().unwrap_or(1));
            if i == dim {
                *d = da.saturating_add(db);
            } else if da != db {
                return Err(MatError::DimsMismatch { a: a.dims.clone(), b: b.dims.clone() });
            } else {
                *d = da;
            }
        }
        let mut out = vec![0.0; dim_product(&rd)];
        // Copy `src` into `out` with its cat-dim subscript offset by `shift`.
        let copy = |src: &Matrix, shift: usize, out: &mut [f64]| {
            for (i, &v) in src.data().iter().enumerate() {
                let mut rem = i;
                let mut off = 0usize;
                let mut stride = 1usize;
                for (d, &od) in rd.iter().enumerate() {
                    let sd = src.dims.get(d).copied().unwrap_or(1).max(1);
                    let s = rem % sd;
                    rem /= sd;
                    off = off.saturating_add((s + if d == dim { shift } else { 0 }).saturating_mul(stride));
                    stride = stride.saturating_mul(od);
                }
                if let Some(slot) = out.get_mut(off) {
                    *slot = v;
                }
            }
        };
        copy(a, 0, &mut out);
        copy(b, a.dims.get(dim).copied().unwrap_or(1), &mut out);
        Ok(Matrix { dims: rd, data: out })
    }

    /// Horizontal concatenation `[a b]` — `cat` along dim 2.
    pub fn hcat(a: &Matrix, b: &Matrix) -> Result<Matrix, MatError> {
        Matrix::cat(1, a, b)
    }

    /// Vertical concatenation `[a; b]` — `cat` along dim 1.
    pub fn vcat(a: &Matrix, b: &Matrix) -> Result<Matrix, MatError> {
        Matrix::cat(0, a, b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(rows: usize, cols: usize, data: &[f64]) -> Matrix {
        Matrix::from_row_major(rows, cols, data).unwrap()
    }

    #[test]
    fn matmul_identity() {
        let a = m(2, 2, &[1.0, 2.0, 3.0, 4.0]);
        let i = Matrix::eye(2, 2);
        let out = a.matmul(&i).unwrap();
        assert_eq!(out, a);
    }

    #[test]
    fn solve_linear_system() {
        // 2x + y = 5 ; x + 3y = 5  ->  x = 2, y = 1
        let a = m(2, 2, &[2.0, 1.0, 1.0, 3.0]);
        let b = Matrix::col_vector(vec![5.0, 5.0]);
        let x = a.solve(&b).unwrap();
        assert!((x.get(0, 0).unwrap() - 2.0).abs() < 1e-9);
        assert!((x.get(1, 0).unwrap() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn det_known_value() {
        let a = m(3, 3, &[6.0, 1.0, 1.0, 4.0, -2.0, 5.0, 2.0, 8.0, 7.0]);
        assert!((a.det().unwrap() - (-306.0)).abs() < 1e-9);
    }

    #[test]
    fn transpose_swaps_shape() {
        let a = m(2, 3, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        let t = a.transpose();
        assert_eq!(t.shape(), (3, 2));
        assert_eq!(t.get(2, 1), Some(6.0));
    }

    #[test]
    fn grow_pads_zeros() {
        let mut a = Matrix::scalar(7.0);
        a.grow(2, 2);
        assert_eq!(a.shape(), (2, 2));
        assert_eq!(a.get(0, 0), Some(7.0));
        assert_eq!(a.get(1, 1), Some(0.0));
    }

    #[test]
    fn singular_solve_errors() {
        let a = m(2, 2, &[1.0, 2.0, 2.0, 4.0]);
        let b = Matrix::col_vector(vec![1.0, 2.0]);
        assert_eq!(a.solve(&b), Err(MatError::Singular));
    }

    #[test]
    fn nd_basics() {
        let mut m = Matrix::zeros_nd(&[2, 3, 4]);
        assert_eq!(m.dims(), &[2, 3, 4]);
        assert_eq!(m.ndims(), 3);
        assert_eq!(m.numel(), 24);
        assert!(!m.is_matrix());
        // (1,2,3) 0-based → linear 1 + 2*2 + 3*6 = 23.
        assert_eq!(m.offset(&[1, 2, 3]), Some(23));
        m.set_nd(&[1, 2, 3], 9.0).unwrap();
        assert_eq!(m.get_nd(&[1, 2, 3]), Some(9.0));
        assert_eq!(m.get_linear(23), Some(9.0));
        assert!(m.get_nd(&[2, 0, 0]).is_none());
        assert!(m.set_nd(&[0, 0, 4], 1.0).is_err());
        // `get`/`set` see the first-page (row, col) view.
        assert_eq!(m.get(1, 2), Some(0.0));
        let _ = m.set(1, 2, 7.0);
        assert_eq!(m.get_nd(&[1, 2, 0]), Some(7.0));
    }

    #[test]
    fn nd_grow_extends_dims() {
        // x(1,1,1) = 5 growth: appending an extent-1 dim keeps storage.
        let mut m = Matrix::scalar(5.0);
        m.grow_nd(&[1, 1, 1]);
        assert_eq!(m.dims(), &[1, 1, 1]);
        assert_eq!(m.get_nd(&[0, 0, 0]), Some(5.0));
        // Growing a real dim pads zeros and preserves old elements.
        let mut m = Matrix::zeros(2, 2);
        m.set(0, 0, 1.0).unwrap();
        m.set(1, 1, 2.0).unwrap();
        m.grow_nd(&[2, 2, 3]);
        assert_eq!(m.dims(), &[2, 2, 3]);
        assert_eq!(m.numel(), 12);
        assert_eq!(m.get_nd(&[0, 0, 0]), Some(1.0));
        assert_eq!(m.get_nd(&[1, 1, 0]), Some(2.0));
        assert_eq!(m.get_nd(&[1, 1, 2]), Some(0.0));
    }

    #[test]
    fn nd_cat_pages() {
        let a = Matrix::ones(2, 2);
        let b = Matrix::zeros(2, 2);
        let c = Matrix::cat(2, &a, &b).unwrap();
        assert_eq!(c.dims(), &[2, 2, 2]);
        assert_eq!(c.get_nd(&[0, 0, 0]), Some(1.0));
        assert_eq!(c.get_nd(&[1, 1, 1]), Some(0.0));
        // Non-cat dims must match.
        assert!(Matrix::cat(2, &a, &Matrix::ones(2, 3)).is_err());
        // cat along dim 1/2 still concatenates N-D operands.
        let h = Matrix::cat(1, &c, &c).unwrap();
        assert_eq!(h.dims(), &[2, 4, 2]);
        let v = Matrix::cat(0, &c, &c).unwrap();
        assert_eq!(v.dims(), &[4, 2, 2]);
    }

    #[test]
    fn nd_elemwise_and_2d_guards() {
        let a = Matrix::ones_nd(&[2, 2, 2]);
        let b = Matrix::ones_nd(&[2, 2, 2]);
        assert_eq!(a.broadcast(&b, |x, y| x + y).unwrap().dims(), &[2, 2, 2]);
        // Implicit trailing singleton dims are shape-compatible:
        // [2 2] ≡ [2 2 1], but not [2 2 2].
        let page = Matrix::ones_nd(&[2, 2, 1]);
        assert_eq!(page.elemwise(&Matrix::ones(2, 2), |x, y| x * y).unwrap().dims(), &[2, 2, 1]);
        // Mismatched N-D shapes error.
        assert!(a.elemwise(&Matrix::ones_nd(&[2, 2, 3]), |x, y| x + y).is_err());
        assert!(a.elemwise(&Matrix::ones(2, 2), |x, y| x + y).is_err());
        // Matrix ops refuse N-D operands.
        assert!(a.matmul(&b).is_err());
        assert!(a.det().is_err());
        assert!(a.solve(&b).is_err());
        assert!(a.inverse().is_err());
        // …but a trailing-singleton 3-D view still counts as a matrix.
        assert!(page.is_matrix());
        assert!(page.det().is_ok());
    }

    #[test]
    fn nd_reshape_and_squeeze() {
        let m = Matrix::ones_nd(&[1, 4, 1, 3]);
        let s = m.squeezed();
        assert_eq!(s.dims(), &[4, 3]);
        // squeeze of a row vector is a column.
        assert_eq!(Matrix::row_vector(vec![1.0; 4]).squeezed().dims(), &[4, 1]);
        let r = s.reshape_nd(&[2, 3, 2]).unwrap();
        assert_eq!(r.dims(), &[2, 3, 2]);
        assert_eq!(r.numel(), 12);
        assert!(r.reshape_nd(&[2, 2]).is_err());
        assert!(r.reshape_nd(&[5]).is_err());
    }
}
