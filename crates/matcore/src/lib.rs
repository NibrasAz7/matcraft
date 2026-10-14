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

/// Dense column-major matrix of f64.
#[derive(Debug, Clone, PartialEq)]
pub struct Matrix {
    rows: usize,
    cols: usize,
    /// `data[col * rows + row]`
    data: Vec<f64>,
}

impl Matrix {
    /// Zero matrix.
    pub fn zeros(rows: usize, cols: usize) -> Self {
        Matrix { rows, cols, data: vec![0.0; rows.saturating_mul(cols)] }
    }

    /// Matrix of ones.
    pub fn ones(rows: usize, cols: usize) -> Self {
        Matrix { rows, cols, data: vec![1.0; rows.saturating_mul(cols)] }
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
        Ok(Matrix { rows, cols, data })
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
        Matrix { rows: 1, cols: 1, data: vec![v] }
    }

    /// Row vector `1 x values.len()`.
    pub fn row_vector(values: Vec<f64>) -> Self {
        Matrix { rows: 1, cols: values.len(), data: values }
    }

    /// Column vector `values.len() x 1`.
    pub fn col_vector(values: Vec<f64>) -> Self {
        Matrix { rows: values.len(), cols: 1, data: values }
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

    pub fn numel(&self) -> usize {
        self.data.len()
    }

    /// Column-major element access.
    pub fn data(&self) -> &[f64] {
        &self.data
    }

    /// Element at (row, col), 0-based.
    pub fn get(&self, row: usize, col: usize) -> Option<f64> {
        if row < self.rows && col < self.cols {
            self.data.get(col * self.rows + row).copied()
        } else {
            None
        }
    }

    /// Set element at (row, col), 0-based.
    pub fn set(&mut self, row: usize, col: usize, v: f64) -> Result<(), MatError> {
        if row < self.rows && col < self.cols {
            if let Some(slot) = self.data.get_mut(col * self.rows + row) {
                *slot = v;
            }
            Ok(())
        } else {
            Err(MatError::OutOfBounds { index: (row, col), shape: self.shape() })
        }
    }

    /// Linear index (0-based, column-major order).
    pub fn get_linear(&self, i: usize) -> Option<f64> {
        self.data.get(i).copied()
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
        Matrix { rows: self.rows, cols: self.cols, data: self.data.iter().map(|&v| f(v)).collect() }
    }

    /// Elementwise binary op; shapes must match exactly.
    pub fn elemwise(&self, other: &Matrix, f: impl Fn(f64, f64) -> f64) -> Result<Self, MatError> {
        if self.shape() != other.shape() {
            return Err(MatError::DimensionMismatch { a: self.shape(), b: other.shape() });
        }
        let data = self.data.iter().zip(other.data.iter()).map(|(&a, &b)| f(a, b)).collect();
        Ok(Matrix { rows: self.rows, cols: self.cols, data })
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
        if self.cols != other.rows {
            return Err(MatError::DimensionMismatch { a: self.shape(), b: other.shape() });
        }
        let (m, k, n) = (self.rows, self.cols, other.cols);
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

    /// Transpose.
    pub fn transpose(&self) -> Self {
        let mut out = Matrix::zeros(self.cols, self.rows);
        for r in 0..self.rows {
            for c in 0..self.cols {
                if let (Some(v), Some(slot)) = (self.get(r, c), out.data.get_mut(r * self.cols + c)) {
                    *slot = v;
                }
            }
        }
        out
    }

    /// LU solve `self * x = b` with partial pivoting.
    pub fn solve(&self, b: &Matrix) -> Result<Matrix, MatError> {
        if self.rows != self.cols || b.rows != self.rows {
            return Err(MatError::DimensionMismatch { a: self.shape(), b: b.shape() });
        }
        let n = self.rows;
        // Augmented copy, row-major for cache-friendly elimination.
        let mut a = vec![0.0; n.saturating_mul(n + b.cols)];
        let width = n + b.cols;
        for r in 0..n {
            for c in 0..n {
                if let Some(dst) = a.get_mut(r * width + c) {
                    *dst = self.get(r, c).unwrap_or(0.0);
                }
            }
            for c in 0..b.cols {
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
        let mut x = Matrix::zeros(n, b.cols);
        for col in 0..b.cols {
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
        if self.rows != self.cols {
            return Err(MatError::Singular);
        }
        self.solve(&Matrix::eye(self.rows, self.cols))
    }

    /// Determinant via LU elimination (sign tracked on pivot swaps).
    pub fn det(&self) -> Result<f64, MatError> {
        if self.rows != self.cols {
            return Err(MatError::Singular);
        }
        let n = self.rows;
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
        if rows.saturating_mul(cols) != self.data.len() {
            return Err(MatError::BadData { len: self.data.len(), expected: rows.saturating_mul(cols) });
        }
        Ok(Matrix { rows, cols, data: self.data.clone() })
    }

    /// Grow to at least (rows, cols), padding with zeros.
    pub fn grow(&mut self, rows: usize, cols: usize) {
        if rows <= self.rows && cols <= self.cols {
            return;
        }
        let (nr, nc) = (rows.max(self.rows), cols.max(self.cols));
        let mut out = Matrix::zeros(nr, nc);
        for c in 0..self.cols {
            for r in 0..self.rows {
                if let (Some(v), Some(slot)) = (self.get(r, c), out.data.get_mut(c * nr + r)) {
                    *slot = v;
                }
            }
        }
        *self = out;
    }

    /// Horizontal concatenation `[a b]`.
    pub fn hcat(a: &Matrix, b: &Matrix) -> Result<Matrix, MatError> {
        if a.rows != b.rows {
            return Err(MatError::DimensionMismatch { a: a.shape(), b: b.shape() });
        }
        let mut out = Matrix::zeros(a.rows, a.cols + b.cols);
        for c in 0..a.cols + b.cols {
            let src = if c < a.cols { a } else { b };
            let sc = if c < a.cols { c } else { c - a.cols };
            for r in 0..a.rows {
                if let (Some(v), Some(slot)) = (src.get(r, sc), out.data.get_mut(c * a.rows + r)) {
                    *slot = v;
                }
            }
        }
        Ok(out)
    }

    /// Vertical concatenation `[a; b]`.
    pub fn vcat(a: &Matrix, b: &Matrix) -> Result<Matrix, MatError> {
        if a.cols != b.cols {
            return Err(MatError::DimensionMismatch { a: a.shape(), b: b.shape() });
        }
        let mut data = Vec::with_capacity(a.data.len() + b.data.len());
        for c in 0..a.cols {
            for r in 0..a.rows {
                data.push(a.get(r, c).unwrap_or(0.0));
            }
            for r in 0..b.rows {
                data.push(b.get(r, c).unwrap_or(0.0));
            }
        }
        Ok(Matrix { rows: a.rows + b.rows, cols: a.cols, data })
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
}
