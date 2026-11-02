//! Matrix exponential — scaling and squaring with a Taylor series.
//!
//! Used by the control toolbox's zero-order-hold state-space simulation.
//! The implementation is independent: scale `A` by a power of two until its
//! 1-norm is at most 0.5, sum the Taylor series, then square back.

use crate::{MatError, Matrix};

fn one_norm(a: &Matrix) -> f64 {
    let n = a.cols();
    (0..n).map(|c| (0..a.rows()).map(|r| a.get(r, c).unwrap_or(0.0).abs()).sum::<f64>()).fold(0.0f64, f64::max)
}

/// `expm(A)` for a square real matrix.
pub fn expm(a: &Matrix) -> Result<Matrix, MatError> {
    let n = a.rows();
    if n != a.cols() {
        return Err(MatError::DimensionMismatch { a: a.shape(), b: a.shape() });
    }
    if n == 0 {
        return Ok(Matrix::zeros(0, 0));
    }
    if n == 1 {
        return Ok(Matrix::scalar(a.get(0, 0).unwrap_or(0.0).exp()));
    }
    let norm = one_norm(a);
    if !norm.is_finite() {
        return Err(MatError::Other("expm: matrix norm is not finite".into()));
    }
    let s = if norm > 0.5 { (norm.log2().ceil() as usize).min(60) } else { 0 };
    let scale = 2f64.powi(s as i32);
    let x = a.map(|v| v / scale);
    let mut result = Matrix::eye(n, n);
    let mut term = Matrix::eye(n, n);
    for k in 1..=30usize {
        term = term.matmul(&x)?.map(|v| v / k as f64);
        result = result.elemwise(&term, |p, q| p + q)?;
        let t_norm = one_norm(&term);
        let r_norm = one_norm(&result).max(1.0);
        if t_norm <= f64::EPSILON * r_norm {
            break;
        }
    }
    for _ in 0..s {
        result = result.matmul(&result)?;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: &Matrix, b: &Matrix, tol: f64) -> bool {
        a.shape() == b.shape() && a.data().iter().zip(b.data()).all(|(x, y)| (x - y).abs() <= tol)
    }

    #[test]
    fn expm_zero_is_identity() {
        let z = Matrix::zeros(3, 3);
        let e = expm(&z).unwrap();
        assert!(close(&e, &Matrix::eye(3, 3), 1e-15));
    }

    #[test]
    fn expm_nilpotent_shift() {
        // exp([[0,1],[0,0]]) = [[1,1],[0,1]].
        let a = Matrix::from_row_major(2, 2, &[0.0, 1.0, 0.0, 0.0]).unwrap();
        let e = expm(&a).unwrap();
        assert!(close(&e, &Matrix::from_row_major(2, 2, &[1.0, 1.0, 0.0, 1.0]).unwrap(), 1e-14));
    }

    #[test]
    fn expm_diagonal() {
        let a = Matrix::from_row_major(2, 2, &[1.0, 0.0, 0.0, -2.0]).unwrap();
        let e = expm(&a).unwrap();
        assert!((e.get(0, 0).unwrap() - 1f64.exp()).abs() < 1e-12);
        assert!((e.get(1, 1).unwrap() - (-2f64).exp()).abs() < 1e-12);
        assert!(e.get(0, 1).unwrap().abs() < 1e-15);
    }

    #[test]
    fn expm_rotation() {
        // exp([[0,-1],[1,0]]) = [[cos 1, -sin 1],[sin 1, cos 1]].
        let a = Matrix::from_row_major(2, 2, &[0.0, -1.0, 1.0, 0.0]).unwrap();
        let e = expm(&a).unwrap();
        assert!((e.get(0, 0).unwrap() - 1f64.cos()).abs() < 1e-14);
        assert!((e.get(0, 1).unwrap() + 1f64.sin()).abs() < 1e-14);
        assert!((e.get(1, 0).unwrap() - 1f64.sin()).abs() < 1e-14);
        assert!((e.get(1, 1).unwrap() - 1f64.cos()).abs() < 1e-14);
    }

    #[test]
    fn expm_rejects_nonsquare() {
        let a = Matrix::zeros(2, 3);
        assert!(expm(&a).is_err());
    }
}
