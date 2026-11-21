//! `faer`-accelerated kernels — the Phase-5 backend swap.
//!
//! Compiled only with `--features faer`; the pure-Rust paths in
//! [`crate::linalg`] stay the default and the wasm build is untouched.
//! Each public op converts `Matrix` ↔ `faer::Mat` at the boundary, runs
//! the blocked kernel sequentially (`Par::Seq` — the SIMD kernels still
//! apply), and maps faer's error enums onto [`MatError::Other`].

use crate::{CMatrix, CNum, MatError, Matrix, Svd};
use dyn_stack::{MemBuffer, MemStack};
use faer::diag::Diag;
use faer::linalg::evd::{self, ComputeEigenvectors};
use faer::linalg::svd::ComputeSvdVectors;
use faer::linalg::{matmul as fmm, svd as fsvd};
use faer::{Accum, Mat, Par, Spec};

fn to_faer(m: &Matrix) -> Mat<f64> {
    Mat::from_fn(m.rows(), m.cols(), |i, j| m.get(i, j).unwrap_or(0.0))
}

fn from_faer(fm: &Mat<f64>) -> Matrix {
    let (r, c) = (fm.nrows(), fm.ncols());
    let mut d = Vec::with_capacity(r.saturating_mul(c));
    for j in 0..c {
        for i in 0..r {
            d.push(fm[(i, j)]);
        }
    }
    match Matrix::from_col_major(r, c, d) {
        Ok(m) => m,
        Err(_) => Matrix::zeros(r, c),
    }
}

/// A·B through faer's blocked GEMM.
pub fn matmul(a: &Matrix, b: &Matrix) -> Result<Matrix, MatError> {
    if a.cols() != b.rows() {
        return Err(MatError::Other(format!("dimension mismatch: {}x{} vs {}x{}", a.rows(), a.cols(), b.rows(), b.cols())));
    }
    let (fa, fb) = (to_faer(a), to_faer(b));
    let mut out = Mat::zeros(fa.nrows(), fb.ncols());
    fmm::matmul(out.as_mut(), Accum::Replace, fa.as_ref(), fb.as_ref(), 1.0, Par::Seq);
    Ok(from_faer(&out))
}

/// A = U S V' — thin U (m×min(m,n)), min(m,n)×n S, full V (n×n), matching
/// the pure-Rust `svd`'s shapes. faer returns singular values sorted
/// non-increasing already.
pub fn svd(a: &Matrix) -> Result<Svd, MatError> {
    let (m, n) = (a.rows(), a.cols());
    let size = m.min(n);
    let fa = to_faer(a);
    let mut s = Diag::zeros(size);
    let mut u = Mat::zeros(m, size);
    let mut v = Mat::zeros(n, n);
    let params: Spec<fsvd::SvdParams, f64> = Spec::default();
    fsvd::svd(
        fa.as_ref(),
        s.as_mut(),
        Some(u.as_mut()),
        Some(v.as_mut()),
        Par::Seq,
        MemStack::new(&mut MemBuffer::new(fsvd::svd_scratch::<f64>(m, n, ComputeSvdVectors::Thin, ComputeSvdVectors::Full, Par::Seq, params))),
        params,
    )
    .map_err(|e| MatError::Other(format!("svd failed to converge: {e:?}")))?;
    let mut smat = Matrix::zeros(size, n);
    for i in 0..size {
        let _ = smat.set(i, i, s[i]);
    }
    Ok(Svd { u: from_faer(&u), s: smat, v: from_faer(&v) })
}

/// `evd_real` output: real/imag spectrum parts plus the right eigenvector
/// matrix in LAPACK layout (`None` when eigenvectors are not requested).
type EvdOut = (Vec<f64>, Vec<f64>, Option<Mat<f64>>);

/// Run `evd_real` and return (s_re, s_im, right eigenvector matrix) —
/// `ur` is `None` when eigenvectors are not requested.
fn evd_real(a: &Matrix, vectors: bool) -> Result<EvdOut, MatError> {
    if a.rows() != a.cols() {
        return Err(MatError::Singular);
    }
    let n = a.rows();
    let fa = to_faer(a);
    let mut s_re = Diag::zeros(n);
    let mut s_im = Diag::zeros(n);
    let mut ur = Mat::zeros(n, n);
    let params: Spec<evd::EvdParams, f64> = Spec::default();
    let compute = if vectors { ComputeEigenvectors::Yes } else { ComputeEigenvectors::No };
    evd::evd_real(
        fa.as_ref(),
        s_re.as_mut(),
        s_im.as_mut(),
        None,
        vectors.then(|| ur.as_mut()),
        Par::Seq,
        MemStack::new(&mut MemBuffer::new(evd::evd_scratch::<f64>(n, ComputeEigenvectors::No, compute, Par::Seq, params))),
        params,
    )
    .map_err(|e| MatError::Other(format!("eig failed to converge: {e:?}")))?;
    let w_re = (0..n).map(|i| s_re[i]).collect();
    let w_im = (0..n).map(|i| s_im[i]).collect();
    Ok((w_re, w_im, vectors.then_some(ur)))
}

/// Indices of `w` in the same order the pure-Rust [`crate::eigvals_c`]
/// emits: real part ascending, imaginary part descending.
fn spectrum_order(w: &[CNum]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..w.len()).collect();
    order.sort_by(|&i, &j| {
        let (x, y) = (w.get(i).copied().unwrap_or(CNum::real(0.0)), w.get(j).copied().unwrap_or(CNum::real(0.0)));
        x.re.partial_cmp(&y.re).unwrap_or(std::cmp::Ordering::Equal).then(y.im.partial_cmp(&x.im).unwrap_or(std::cmp::Ordering::Equal))
    });
    order
}

/// Complex spectrum via `evd_real` — the blocked Schur path replaces
/// the pure-Rust Hessenberg+QR when the feature is on. Ordering matches
/// [`crate::eigvals_c`].
pub fn eigvals_c(a: &Matrix) -> Result<Vec<CNum>, MatError> {
    if a.rows() != a.cols() {
        return Err(MatError::Singular);
    }
    let (re, im, _) = evd_real(a, false)?;
    let w: Vec<CNum> = re.iter().zip(im.iter()).map(|(&r, &i)| CNum::new(r, i)).collect();
    Ok(spectrum_order(&w).iter().map(|&i| w.get(i).copied().unwrap_or(CNum::real(0.0))).collect())
}

/// Right eigendecomposition via `evd_real`. Complex pairs come back in
/// LAPACK layout — columns j, j+1 of `ur` hold Re/Im of the eigenvector
/// for λ = s_re[j] ± i·s_im[j] — and are converted to `CMatrix` columns.
/// Columns are permuted into [`crate::eigvals_c`] order.
pub fn eig_c(a: &Matrix) -> Result<(Vec<CNum>, CMatrix), MatError> {
    let n = a.rows();
    let (re, im, ur) = evd_real(a, true)?;
    let ur = ur.ok_or_else(|| MatError::Other("eig: missing eigenvectors".into()))?;
    let w: Vec<CNum> = re.iter().zip(im.iter()).map(|(&r, &i)| CNum::new(r, i)).collect();
    let order = spectrum_order(&w);
    let mut cols: Vec<CNum> = vec![CNum::real(0.0); n * n];
    // LAPACK layout: for a pair at source cols (j, j+1), s_im[j] > 0 and
    // v_j = ur[:,j] + i·ur[:,j+1]; v_{j+1} = conj(v_j). Sorted positions
    // are not guaranteed adjacent, so resolve each column's source pair.
    for (dst, &src) in order.iter().enumerate() {
        let im_src = im.get(src).copied().unwrap_or(0.0);
        let (re_c, im_c) = if im_src > 0.0 && src + 1 < n {
            (src, src + 1)
        } else if im_src < 0.0 && src > 0 {
            (src - 1, src)
        } else {
            (src, src)
        };
        for r in 0..n {
            let v = if im_src == 0.0 { CNum::real(ur[(r, re_c)]) } else { CNum::new(ur[(r, re_c)], ur[(r, im_c)] * im_src.signum()) };
            if let Some(slot) = cols.get_mut(r + dst * n) {
                *slot = v;
            }
        }
    }
    let w_sorted: Vec<CNum> = order.iter().map(|&i| w.get(i).copied().unwrap_or(CNum::real(0.0))).collect();
    Ok((w_sorted, CMatrix::from_col_major(n, n, cols)?))
}
