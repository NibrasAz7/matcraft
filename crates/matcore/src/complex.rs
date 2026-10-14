//! Complex scalars and dense column-major complex matrices (`CNum`,
//! `CMatrix`). Mirrors `Matrix`'s API so the interpreter can promote
//! `Value::Mat` to `Value::Cplx` the moment an imaginary part appears —
//! exactly the lazy promotion M-code uses (`A(1) = 3+4i` makes `A` complex).

use crate::{MatError, Matrix};
// Method syntax (`a.add(b)`) needs the traits in scope.
use std::ops::{Add, Div, Mul, Sub};

/// Complex scalar `re + im·i`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CNum {
    pub re: f64,
    pub im: f64,
}

impl CNum {
    pub const I: CNum = CNum { re: 0.0, im: 1.0 };

    pub fn new(re: f64, im: f64) -> Self {
        CNum { re, im }
    }

    pub fn real(re: f64) -> Self {
        CNum { re, im: 0.0 }
    }

    /// True when the imaginary part is exactly zero.
    pub fn is_real(&self) -> bool {
        self.im == 0.0
    }

    pub fn conj(self) -> Self {
        CNum { re: self.re, im: -self.im }
    }

    /// Magnitude `|z|` — `hypot` for overflow safety.
    pub fn abs(self) -> f64 {
        self.re.hypot(self.im)
    }

    /// Squared magnitude.
    pub fn abs2(self) -> f64 {
        self.re * self.re + self.im * self.im
    }

    /// Phase in radians, `atan2(im, re)`.
    pub fn arg(self) -> f64 {
        self.im.atan2(self.re)
    }

    /// `z^w` via polar form; `z^0 = 1`.
    pub fn powc(self, w: CNum) -> Self {
        if self.re == 0.0 && self.im == 0.0 {
            return CNum::new(0.0, 0.0);
        }
        // z^w = exp(w * ln z); ln z = ln|z| + i·arg z.
        let lr = CNum { re: self.abs().ln(), im: self.arg() };
        (w.mul(lr)).exp()
    }

    /// `z^k` for an integer exponent — exact for small |k|.
    pub fn powi(self, k: i32) -> Self {
        let mut acc = CNum::real(1.0);
        let mut base = self;
        let mut n = k.unsigned_abs();
        while n > 0 {
            if n & 1 == 1 {
                acc = acc.mul(base);
            }
            base = base.mul(base);
            n >>= 1;
        }
        if k < 0 {
            CNum::real(1.0).div(acc)
        } else {
            acc
        }
    }

    /// `e^z`.
    pub fn exp(self) -> Self {
        let r = self.re.exp();
        CNum { re: r * self.im.cos(), im: r * self.im.sin() }
    }

    /// Principal `ln z = ln|z| + i·arg z`.
    pub fn ln(self) -> Self {
        CNum { re: self.abs().ln(), im: self.arg() }
    }

    /// Principal square root.
    pub fn sqrt(self) -> Self {
        let r = self.abs();
        let re = ((r + self.re) / 2.0).sqrt();
        let im = ((r - self.re) / 2.0).sqrt().copysign(self.im);
        CNum { re, im }
    }

    /// `sin z = sin(x)cosh(y) + i cos(x)sinh(y)`.
    pub fn sin(self) -> Self {
        CNum { re: self.re.sin() * self.im.cosh(), im: self.re.cos() * self.im.sinh() }
    }

    /// `cos z = cos(x)cosh(y) − i sin(x)sinh(y)`.
    pub fn cos(self) -> Self {
        CNum { re: self.re.cos() * self.im.cosh(), im: -self.re.sin() * self.im.sinh() }
    }
}

// Arithmetic lives in the operator traits (`a + b`, `x.div(y)` need the
// trait in scope — `use std::ops::{Add, …}` at call sites).
impl std::ops::Add for CNum {
    type Output = CNum;
    fn add(self, o: CNum) -> CNum {
        CNum { re: self.re + o.re, im: self.im + o.im }
    }
}
impl std::ops::Sub for CNum {
    type Output = CNum;
    fn sub(self, o: CNum) -> CNum {
        CNum { re: self.re - o.re, im: self.im - o.im }
    }
}
impl std::ops::Mul for CNum {
    type Output = CNum;
    fn mul(self, o: CNum) -> CNum {
        CNum { re: self.re * o.re - self.im * o.im, im: self.re * o.im + self.im * o.re }
    }
}
impl std::ops::Div for CNum {
    type Output = CNum;
    /// Complex division (Smith's algorithm — scale the smaller half).
    fn div(self, o: CNum) -> CNum {
        if o.re.abs() >= o.im.abs() {
            let r = o.im / o.re;
            let d = o.re + o.im * r;
            CNum { re: (self.re + self.im * r) / d, im: (self.im - self.re * r) / d }
        } else {
            let r = o.re / o.im;
            let d = o.re * r + o.im;
            CNum { re: (self.re * r + self.im) / d, im: (self.im * r - self.re) / d }
        }
    }
}
impl std::ops::Neg for CNum {
    type Output = CNum;
    fn neg(self) -> CNum {
        CNum { re: -self.re, im: -self.im }
    }
}

/// Dense column-major complex matrix — `data[col * rows + row]`, same as
/// `Matrix`, so linear indexing matches M-code semantics.
#[derive(Debug, Clone, PartialEq)]
pub struct CMatrix {
    rows: usize,
    cols: usize,
    data: Vec<CNum>,
}

impl CMatrix {
    pub fn zeros(rows: usize, cols: usize) -> Self {
        CMatrix { rows, cols, data: vec![CNum::real(0.0); rows.saturating_mul(cols)] }
    }

    /// Scalar `1×1`.
    pub fn scalar(v: CNum) -> Self {
        CMatrix { rows: 1, cols: 1, data: vec![v] }
    }

    /// Promote a real matrix (imaginary parts zero).
    pub fn from_real(m: &Matrix) -> Self {
        CMatrix { rows: m.rows(), cols: m.cols(), data: m.data().iter().map(|&x| CNum::real(x)).collect() }
    }

    /// `complex(a, b)` — pair re/im elementwise; either side may be scalar.
    pub fn from_parts(re: &Matrix, im: &Matrix) -> Result<Self, MatError> {
        if re.numel() == 1 {
            let r = re.as_scalar().unwrap_or(0.0);
            return Ok(CMatrix { rows: im.rows(), cols: im.cols(), data: im.data().iter().map(|&i| CNum::new(r, i)).collect() });
        }
        if im.numel() == 1 {
            let i0 = im.as_scalar().unwrap_or(0.0);
            return Ok(CMatrix { rows: re.rows(), cols: re.cols(), data: re.data().iter().map(|&r| CNum::new(r, i0)).collect() });
        }
        if re.shape() != im.shape() {
            return Err(MatError::DimensionMismatch { a: re.shape(), b: im.shape() });
        }
        Ok(CMatrix { rows: re.rows(), cols: re.cols(), data: re.data().iter().zip(im.data().iter()).map(|(&r, &i)| CNum::new(r, i)).collect() })
    }

    pub fn from_col_major(rows: usize, cols: usize, data: Vec<CNum>) -> Result<Self, MatError> {
        if data.len() != rows.saturating_mul(cols) {
            return Err(MatError::BadData { len: data.len(), expected: rows.saturating_mul(cols) });
        }
        Ok(CMatrix { rows, cols, data })
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

    pub fn data(&self) -> &[CNum] {
        &self.data
    }

    pub fn get(&self, row: usize, col: usize) -> Option<CNum> {
        if row < self.rows && col < self.cols {
            self.data.get(col * self.rows + row).copied()
        } else {
            None
        }
    }

    pub fn set(&mut self, row: usize, col: usize, v: CNum) -> Result<(), MatError> {
        if row < self.rows && col < self.cols {
            if let Some(slot) = self.data.get_mut(col * self.rows + row) {
                *slot = v;
            }
            Ok(())
        } else {
            Err(MatError::OutOfBounds { index: (row, col), shape: self.shape() })
        }
    }

    pub fn get_linear(&self, i: usize) -> Option<CNum> {
        self.data.get(i).copied()
    }

    pub fn set_linear(&mut self, i: usize, v: CNum) -> Result<(), MatError> {
        if let Some(slot) = self.data.get_mut(i) {
            *slot = v;
            Ok(())
        } else {
            Err(MatError::OutOfBounds { index: (i, 0), shape: self.shape() })
        }
    }

    pub fn is_scalar(&self) -> bool {
        self.data.len() == 1
    }

    pub fn as_scalar(&self) -> Option<CNum> {
        if self.is_scalar() {
            self.data.first().copied()
        } else {
            None
        }
    }

    /// Every imaginary part is zero → the value is really real.
    pub fn is_real(&self) -> bool {
        self.data.iter().all(|z| z.is_real())
    }

    /// Real part as a real matrix.
    pub fn re(&self) -> Matrix {
        Matrix::from_col_major(self.rows, self.cols, self.data.iter().map(|z| z.re).collect()).unwrap_or_else(|_| Matrix::zeros(0, 0))
    }

    /// Imaginary part as a real matrix.
    pub fn im(&self) -> Matrix {
        Matrix::from_col_major(self.rows, self.cols, self.data.iter().map(|z| z.im).collect()).unwrap_or_else(|_| Matrix::zeros(0, 0))
    }

    /// Drop the imaginary part when it is entirely zero.
    pub fn real_if_real(&self) -> Option<Matrix> {
        self.is_real().then(|| self.re())
    }

    /// Elementwise magnitudes.
    pub fn abs(&self) -> Matrix {
        Matrix::from_col_major(self.rows, self.cols, self.data.iter().map(|z| z.abs()).collect()).unwrap_or_else(|_| Matrix::zeros(0, 0))
    }

    /// Elementwise phases.
    pub fn angle(&self) -> Matrix {
        Matrix::from_col_major(self.rows, self.cols, self.data.iter().map(|z| z.arg()).collect()).unwrap_or_else(|_| Matrix::zeros(0, 0))
    }

    /// Elementwise map.
    pub fn map(&self, f: impl Fn(CNum) -> CNum) -> Self {
        CMatrix { rows: self.rows, cols: self.cols, data: self.data.iter().map(|&z| f(z)).collect() }
    }

    /// Elementwise binary op; shapes must match exactly.
    pub fn elemwise(&self, other: &CMatrix, f: impl Fn(CNum, CNum) -> CNum) -> Result<Self, MatError> {
        if self.shape() != other.shape() {
            return Err(MatError::DimensionMismatch { a: self.shape(), b: other.shape() });
        }
        let data = self.data.iter().zip(other.data.iter()).map(|(&a, &b)| f(a, b)).collect();
        Ok(CMatrix { rows: self.rows, cols: self.cols, data })
    }

    /// Broadcast `other` if scalar, else require equal shapes.
    pub fn broadcast(&self, other: &CMatrix, f: impl Fn(CNum, CNum) -> CNum) -> Result<Self, MatError> {
        if let Some(s) = other.as_scalar() {
            return Ok(self.map(|a| f(a, s)));
        }
        if let Some(s) = self.as_scalar() {
            return Ok(other.map(|b| f(s, b)));
        }
        self.elemwise(other, f)
    }

    /// Complex matrix multiply: (m×k)·(k×n); scalars broadcast.
    pub fn matmul(&self, other: &CMatrix) -> Result<Self, MatError> {
        if let Some(s) = self.as_scalar() {
            return Ok(other.map(|b| s.mul(b)));
        }
        if let Some(s) = other.as_scalar() {
            return Ok(self.map(|a| a.mul(s)));
        }
        if self.cols != other.rows {
            return Err(MatError::DimensionMismatch { a: self.shape(), b: other.shape() });
        }
        let (m, k, n) = (self.rows, self.cols, other.cols);
        let mut out = CMatrix::zeros(m, n);
        for c in 0..n {
            for i in 0..k {
                let b = other.get(i, c).unwrap_or(CNum::real(0.0));
                if b.re == 0.0 && b.im == 0.0 {
                    continue;
                }
                for r in 0..m {
                    let a = self.get(r, i).unwrap_or(CNum::real(0.0));
                    if let Some(slot) = out.data.get_mut(c * m + r) {
                        *slot = slot.add(a.mul(b));
                    }
                }
            }
        }
        Ok(out)
    }

    /// Plain transpose (no conjugation) — the `.'` operator.
    pub fn transpose(&self) -> Self {
        let mut out = CMatrix::zeros(self.cols, self.rows);
        for r in 0..self.rows {
            for c in 0..self.cols {
                if let (Some(v), Some(slot)) = (self.get(r, c), out.data.get_mut(r * self.cols + c)) {
                    *slot = v;
                }
            }
        }
        out
    }

    /// Conjugate transpose — the `'` operator on complex data.
    pub fn ctranspose(&self) -> Self {
        self.transpose().map(|z| z.conj())
    }

    /// Elementwise conjugate.
    pub fn conj(&self) -> Self {
        self.map(|z| z.conj())
    }

    /// LU solve `self · x = b` with partial pivoting (complex arithmetic).
    pub fn solve(&self, b: &CMatrix) -> Result<CMatrix, MatError> {
        if self.rows != self.cols || b.rows != self.rows {
            return Err(MatError::DimensionMismatch { a: self.shape(), b: b.shape() });
        }
        let n = self.rows;
        let width = n + b.cols;
        // Row-major augmented system.
        let mut a = vec![CNum::real(0.0); n.saturating_mul(width)];
        for r in 0..n {
            for c in 0..n {
                if let Some(slot) = a.get_mut(r * width + c) {
                    *slot = self.get(r, c).unwrap_or(CNum::real(0.0));
                }
            }
            for c in 0..b.cols {
                if let Some(slot) = a.get_mut(r * width + n + c) {
                    *slot = b.get(r, c).unwrap_or(CNum::real(0.0));
                }
            }
        }
        for col in 0..n {
            let mut pivot = col;
            let mut best = a.get(col * width + col).map(|z| z.abs2()).unwrap_or(0.0);
            for r in (col + 1)..n {
                let v = a.get(r * width + col).map(|z| z.abs2()).unwrap_or(0.0);
                if v > best {
                    best = v;
                    pivot = r;
                }
            }
            if best <= 0.0 {
                return Err(MatError::Singular);
            }
            if pivot != col {
                for c in 0..width {
                    a.swap(col * width + c, pivot * width + c);
                }
            }
            let diag = a.get(col * width + col).copied().unwrap_or(CNum::real(1.0));
            for r in (col + 1)..n {
                let factor = a.get(r * width + col).copied().unwrap_or(CNum::real(0.0)).div(diag);
                if factor.re == 0.0 && factor.im == 0.0 {
                    continue;
                }
                for c in col..width {
                    let sub = factor.mul(a.get(col * width + c).copied().unwrap_or(CNum::real(0.0)));
                    if let Some(slot) = a.get_mut(r * width + c) {
                        *slot = slot.sub(sub);
                    }
                }
            }
        }
        let mut x = CMatrix::zeros(n, b.cols);
        for col in 0..b.cols {
            for r in (0..n).rev() {
                let mut acc = a.get(r * width + n + col).copied().unwrap_or(CNum::real(0.0));
                for c in (r + 1)..n {
                    acc = acc.sub(a.get(r * width + c).copied().unwrap_or(CNum::real(0.0)).mul(x.get(c, col).unwrap_or(CNum::real(0.0))));
                }
                let diag = a.get(r * width + r).copied().unwrap_or(CNum::real(1.0));
                if let Some(slot) = x.data.get_mut(col * n + r) {
                    *slot = acc.div(diag);
                }
            }
        }
        Ok(x)
    }

    /// Reshape preserving column-major element order.
    pub fn reshape(&self, rows: usize, cols: usize) -> Result<CMatrix, MatError> {
        if rows.saturating_mul(cols) != self.data.len() {
            return Err(MatError::BadData { len: self.data.len(), expected: rows.saturating_mul(cols) });
        }
        Ok(CMatrix { rows, cols, data: self.data.clone() })
    }

    /// Grow to at least (rows, cols), padding with zeros.
    pub fn grow(&mut self, rows: usize, cols: usize) {
        if rows <= self.rows && cols <= self.cols {
            return;
        }
        let (nr, nc) = (rows.max(self.rows), cols.max(self.cols));
        let mut out = CMatrix::zeros(nr, nc);
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
    pub fn hcat(a: &CMatrix, b: &CMatrix) -> Result<CMatrix, MatError> {
        if a.rows != b.rows {
            return Err(MatError::DimensionMismatch { a: a.shape(), b: b.shape() });
        }
        let mut out = CMatrix::zeros(a.rows, a.cols + b.cols);
        for c in 0..a.cols + b.cols {
            let (src, sc) = if c < a.cols { (a, c) } else { (b, c - a.cols) };
            for r in 0..a.rows {
                if let (Some(v), Some(slot)) = (src.get(r, sc), out.data.get_mut(c * a.rows + r)) {
                    *slot = v;
                }
            }
        }
        Ok(out)
    }

    /// Vertical concatenation `[a; b]`.
    pub fn vcat(a: &CMatrix, b: &CMatrix) -> Result<CMatrix, MatError> {
        if a.cols != b.cols {
            return Err(MatError::DimensionMismatch { a: a.shape(), b: b.shape() });
        }
        let mut data = Vec::with_capacity(a.data.len() + b.data.len());
        for c in 0..a.cols {
            for r in 0..a.rows {
                data.push(a.get(r, c).unwrap_or(CNum::real(0.0)));
            }
            for r in 0..b.rows {
                data.push(b.get(r, c).unwrap_or(CNum::real(0.0)));
            }
        }
        Ok(CMatrix { rows: a.rows + b.rows, cols: a.cols, data })
    }

    /// Frobenius norm `sqrt(Σ|z|²)`.
    pub fn norm(&self) -> f64 {
        self.data.iter().map(|z| z.abs2()).sum::<f64>().sqrt()
    }

    /// Determinant via LU with partial pivoting — product of the U diagonal
    /// times the pivot-sign parity.
    pub fn det(&self) -> Result<CNum, MatError> {
        if self.rows != self.cols {
            return Err(MatError::DimensionMismatch { a: self.shape(), b: (0, 0) });
        }
        let n = self.rows;
        let mut a = self.data.clone();
        let mut sign = 1.0f64;
        for col in 0..n {
            let mut pivot = col;
            let mut best = a.get(col * n + col).map(|z| z.abs2()).unwrap_or(0.0);
            for r in (col + 1)..n {
                let v = a.get(r * n + col).map(|z| z.abs2()).unwrap_or(0.0);
                if v > best {
                    best = v;
                    pivot = r;
                }
            }
            if best <= 0.0 {
                return Ok(CNum::real(0.0));
            }
            if pivot != col {
                for c in 0..n {
                    a.swap(col * n + c, pivot * n + c);
                }
                sign = -sign;
            }
            let diag = a.get(col * n + col).copied().unwrap_or(CNum::real(1.0));
            for r in (col + 1)..n {
                let factor = a.get(r * n + col).copied().unwrap_or(CNum::real(0.0)).div(diag);
                if factor.re == 0.0 && factor.im == 0.0 {
                    continue;
                }
                for c in col..n {
                    let sub = factor.mul(a.get(col * n + c).copied().unwrap_or(CNum::real(0.0)));
                    if let Some(slot) = a.get_mut(r * n + c) {
                        *slot = slot.sub(sub);
                    }
                }
            }
        }
        let mut d = CNum::real(sign);
        for i in 0..n {
            d = d.mul(a.get(i * n + i).copied().unwrap_or(CNum::real(1.0)));
        }
        Ok(d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cm(rows: usize, cols: usize, re: &[f64], im: &[f64]) -> CMatrix {
        CMatrix::from_parts(&Matrix::from_row_major(rows, cols, re).unwrap(), &Matrix::from_row_major(rows, cols, im).unwrap()).unwrap()
    }

    #[test]
    fn cnum_arithmetic() {
        let a = CNum::new(1.0, 2.0);
        let b = CNum::new(3.0, -4.0);
        assert_eq!(a.mul(b), CNum::new(11.0, 2.0));
        // (1+2i)/(3-4i) = (11+2i)/25·… → (-5+10i)/25 = -0.2+0.4i
        let q = a.div(b);
        assert!((q.re - (-0.2)).abs() < 1e-12 && (q.im - 0.4).abs() < 1e-12);
        assert_eq!(a.mul(a.conj()).im, 0.0);
        assert!((a.mul(a.conj()).re - 5.0).abs() < 1e-12);
    }

    #[test]
    fn powi_and_powc() {
        let i = CNum::I;
        assert_eq!(i.powi(2), CNum::new(-1.0, 0.0));
        assert!((i.powi(4).re - 1.0).abs() < 1e-12);
        // (-1)^(1/2) = i
        let s = CNum::new(-1.0, 0.0).powc(CNum::new(0.5, 0.0));
        assert!((s.re).abs() < 1e-9 && (s.im - 1.0).abs() < 1e-9);
    }

    #[test]
    fn ctranspose_conjugates() {
        let m = cm(1, 2, &[1.0, 2.0], &[3.0, -4.0]);
        let t = m.ctranspose();
        assert_eq!(t.shape(), (2, 1));
        assert_eq!(t.get(0, 0), Some(CNum::new(1.0, -3.0)));
        assert_eq!(t.get(1, 0), Some(CNum::new(2.0, 4.0)));
    }

    #[test]
    fn matmul_complex() {
        // [i] * [i] = -1
        let i = CMatrix::scalar(CNum::I);
        let out = i.matmul(&i).unwrap();
        assert_eq!(out.as_scalar(), Some(CNum::new(-1.0, 0.0)));
        let a = cm(2, 2, &[1.0, 0.0, 0.0, 1.0], &[1.0, 0.0, 0.0, -1.0]);
        // (i·I)·(i·I)'... just A*A with complex entries.
        let sq = a.matmul(&a).unwrap();
        // (1+i)^2 = 2i, (1-i)^2 = -2i
        assert_eq!(sq.get(0, 0), Some(CNum::new(0.0, 2.0)));
        assert_eq!(sq.get(1, 1), Some(CNum::new(0.0, -2.0)));
    }

    #[test]
    fn complex_solve() {
        // (1+i) x = 2  →  x = 2/(1+i) = 1 - i
        let a = CMatrix::scalar(CNum::new(1.0, 1.0));
        let b = CMatrix::scalar(CNum::new(2.0, 0.0));
        let x = a.solve(&b).unwrap();
        let s = x.as_scalar().unwrap_or(CNum::real(0.0));
        assert!((s.re - 1.0).abs() < 1e-12 && (s.im + 1.0).abs() < 1e-12);
    }
}
