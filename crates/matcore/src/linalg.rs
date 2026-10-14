//! Decompositions and derived quantities, all pure Rust.
//!
//! Algorithms are textbook forms (Golub & Van Loan style): Householder QR,
//! Cholesky, Jacobi eigenvalue iteration for symmetric matrices, and SVD
//! built on the eigendecomposition of the normal equations. They are
//! correct and dependable at the matrix sizes MatCraft handles today;
//! a blocked `faer` backend is the Phase-5 performance swap.

use crate::{MatError, Matrix};

const MAX_QR_ITERS: usize = 1000;
const JACOBI_ITERS: usize = 200;

fn idx(n: usize, r: usize, c: usize) -> usize {
    c * n + r
}

/// True when every off-diagonal element is symmetric within tolerance.
fn is_symmetric(m: &Matrix) -> bool {
    if m.rows() != m.cols() {
        return false;
    }
    let n = m.rows();
    for r in 0..n {
        for c in (r + 1)..n {
            let a = m.get(r, c).unwrap_or(0.0);
            let b = m.get(c, r).unwrap_or(0.0);
            if (a - b).abs() > 1e-10 * (1.0 + a.abs().max(b.abs())) {
                return false;
            }
        }
    }
    true
}

/// Householder QR: returns (Q, R) with Q orthogonal n×n, R n×n upper
/// triangular (square inputs). For m×n (m>=n) returns Q m×m, R m×n.
pub fn qr(a: &Matrix) -> Result<(Matrix, Matrix), MatError> {
    let (m, n) = a.shape();
    if m < n {
        return Err(MatError::DimensionMismatch { a: a.shape(), b: (m, m) });
    }
    // Work row-major on a copy of A.
    let w = n;
    let mut r: Vec<f64> = vec![0.0; m.saturating_mul(n)];
    for c in 0..n {
        for row in 0..m {
            if let Some(slot) = r.get_mut(row * w + c) {
                *slot = a.get(row, c).unwrap_or(0.0);
            }
        }
    }
    // Accumulate reflectors into Q (start identity).
    let mut q = Matrix::eye(m, m);
    let get = |v: &[f64], i: usize, j: usize| -> f64 { v.get(i * w + j).copied().unwrap_or(0.0) };
    let set = |v: &mut [f64], i: usize, j: usize, x: f64| {
        if let Some(slot) = v.get_mut(i * w + j) {
            *slot = x;
        }
    };
    for k in 0..n {
        // Householder vector for column k, rows k..m.
        let mut norm = 0.0;
        for i in k..m {
            norm += get(&r, i, k).powi(2);
        }
        norm = norm.sqrt();
        if norm == 0.0 {
            continue;
        }
        let sign = if get(&r, k, k) >= 0.0 { 1.0 } else { -1.0 };
        let alpha = -sign * norm;
        let mut v: Vec<f64> = (0..m).map(|i| if i < k { 0.0 } else { get(&r, i, k) }).collect();
        if let Some(slot) = v.get_mut(k) {
            *slot -= alpha;
        }
        let vnorm2: f64 = v.iter().map(|x| x * x).sum();
        if vnorm2 == 0.0 {
            continue;
        }
        // R := R - 2 v (v' R) / (v' v)
        for j in 0..n {
            let mut dot = 0.0;
            for i in k..m {
                dot += v.get(i).copied().unwrap_or(0.0) * get(&r, i, j);
            }
            let coef = 2.0 * dot / vnorm2;
            for i in k..m {
                let nv = get(&r, i, j) - coef * v.get(i).copied().unwrap_or(0.0);
                set(&mut r, i, j, nv);
            }
        }
        // Apply reflector to Q via Matrix ops: Q = Q * H, H = I - 2vv'/v'v.
        let vmat = Matrix::col_vector(v.clone());
        let h = Matrix::eye(m, m).elemwise(&vmat.matmul(&vmat.transpose())?.map(|x| x * 2.0 / vnorm2), |a, b| a - b)?;
        q = q.matmul(&h)?;
    }
    // Rebuild R as Matrix (m×n, row-major scratch → column-major).
    let mut rmat = Matrix::zeros(m, n);
    for c in 0..n {
        for row in 0..m {
            let _ = rmat.set(row, c, get(&r, row, c));
        }
    }
    // Clean tiny lower-triangular residue.
    for c in 0..n {
        for row in (c + 1)..m {
            if let Some(v) = rmat.get(row, c) {
                if v.abs() < 1e-12 {
                    let _ = rmat.set(row, c, 0.0);
                }
            }
        }
    }
    Ok((q, rmat))
}

/// Cholesky factor R (upper) with R' R = A for symmetric positive-definite A.
pub fn chol(a: &Matrix) -> Result<Matrix, MatError> {
    if a.rows() != a.cols() {
        return Err(MatError::Singular);
    }
    let n = a.rows();
    let mut r = Matrix::zeros(n, n);
    for i in 0..n {
        for j in i..n {
            let mut s = a.get(i, j).unwrap_or(0.0);
            for k in 0..i {
                s -= r.get(k, i).unwrap_or(0.0) * r.get(k, j).unwrap_or(0.0);
            }
            if i == j {
                if s <= 0.0 {
                    return Err(MatError::Singular);
                }
                let _ = r.set(i, i, s.sqrt());
            } else {
                let d = r.get(i, i).unwrap_or(1.0);
                let _ = r.set(i, j, s / d);
            }
        }
    }
    Ok(r)
}

/// LU with partial pivoting: PA = LU. Returns (L, U, P).
pub fn lu(a: &Matrix) -> Result<(Matrix, Matrix, Matrix), MatError> {
    if a.rows() != a.cols() {
        return Err(MatError::Singular);
    }
    let n = a.rows();
    // Row-major copy of A; P starts as identity permutation.
    let mut u = vec![0.0; n.saturating_mul(n)];
    let mut p = Matrix::eye(n, n);
    let mut l = Matrix::zeros(n, n);
    for r in 0..n {
        for c in 0..n {
            if let Some(slot) = u.get_mut(r * n + c) {
                *slot = a.get(r, c).unwrap_or(0.0);
            }
        }
    }
    for col in 0..n {
        let mut pivot = col;
        let mut best = u.get(col * n + col).map(|v| v.abs()).unwrap_or(0.0);
        for r in (col + 1)..n {
            let v = u.get(r * n + col).map(|v| v.abs()).unwrap_or(0.0);
            if v > best {
                best = v;
                pivot = r;
            }
        }
        if best <= f64::EPSILON {
            return Err(MatError::Singular);
        }
        if pivot != col {
            for c in 0..n {
                u.swap(col * n + c, pivot * n + c);
            }
            // Same row swap in P and in the already-built part of L.
            for c in 0..n {
                let (a_, b_) = (p.get(col, c), p.get(pivot, c));
                let _ = p.set(col, c, b_.unwrap_or(0.0));
                let _ = p.set(pivot, c, a_.unwrap_or(0.0));
            }
            for c in 0..col {
                let (a_, b_) = (l.get(col, c), l.get(pivot, c));
                let _ = l.set(col, c, b_.unwrap_or(0.0));
                let _ = l.set(pivot, c, a_.unwrap_or(0.0));
            }
        }
        let diag = u.get(col * n + col).copied().unwrap_or(1.0);
        for r in (col + 1)..n {
            let f = u.get(r * n + col).copied().unwrap_or(0.0) / diag;
            let _ = l.set(r, col, f);
            for c in col..n {
                let nv = u.get(r * n + c).copied().unwrap_or(0.0) - f * u.get(col * n + c).copied().unwrap_or(0.0);
                if let Some(slot) = u.get_mut(r * n + c) {
                    *slot = nv;
                }
            }
        }
    }
    for i in 0..n {
        let _ = l.set(i, i, 1.0);
    }
    let mut umat = Matrix::zeros(n, n);
    for c in 0..n {
        for r in 0..n {
            let _ = umat.set(r, c, u.get(r * n + c).copied().unwrap_or(0.0));
        }
    }
    Ok((l, umat, p))
}

/// Jacobi eigendecomposition of a symmetric matrix: returns eigenvalues
/// (ascending) and eigenvectors as columns of V (V' A V = diag).
pub fn eig_symmetric(a: &Matrix) -> Result<(Vec<f64>, Matrix), MatError> {
    if a.rows() != a.cols() {
        return Err(MatError::Singular);
    }
    let n = a.rows();
    if !is_symmetric(a) {
        return Err(MatError::Other("eig of a non-symmetric matrix with real eigenvalues needs the QR path".into()));
    }
    // Work on a Vec in column-major order.
    let mut w = a.data().to_vec();
    let mut v = Matrix::eye(n, n);
    for _ in 0..JACOBI_ITERS {
        // Largest off-diagonal element.
        let mut p = 0;
        let mut q = 1.min(n.saturating_sub(1));
        let mut best = 0.0f64;
        for c in 0..n {
            for r in 0..c {
                let x = w.get(idx(n, r, c)).copied().unwrap_or(0.0).abs();
                if x > best {
                    best = x;
                    p = r;
                    q = c;
                }
            }
        }
        if best < 1e-12 * (1.0 + w.iter().map(|x| x * x).sum::<f64>().sqrt()) {
            break;
        }
        let app = w.get(idx(n, p, p)).copied().unwrap_or(0.0);
        let aqq = w.get(idx(n, q, q)).copied().unwrap_or(0.0);
        let apq = w.get(idx(n, p, q)).copied().unwrap_or(0.0);
        let theta = (aqq - app) / (2.0 * apq);
        let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
        let c = 1.0 / (t * t + 1.0).sqrt();
        let s = t * c;
        for k in 0..n {
            let akp = w.get(idx(n, k, p)).copied().unwrap_or(0.0);
            let akq = w.get(idx(n, k, q)).copied().unwrap_or(0.0);
            if let Some(x) = w.get_mut(idx(n, k, p)) {
                *x = c * akp - s * akq;
            }
            if let Some(x) = w.get_mut(idx(n, k, q)) {
                *x = s * akp + c * akq;
            }
        }
        for k in 0..n {
            let apk = w.get(idx(n, p, k)).copied().unwrap_or(0.0);
            let aqk = w.get(idx(n, q, k)).copied().unwrap_or(0.0);
            if let Some(x) = w.get_mut(idx(n, p, k)) {
                *x = c * apk - s * aqk;
            }
            if let Some(x) = w.get_mut(idx(n, q, k)) {
                *x = s * apk + c * aqk;
            }
        }
        // Accumulate eigenvectors: V = V * J.
        for k in 0..n {
            let vkp = v.get(k, p).unwrap_or(0.0);
            let vkq = v.get(k, q).unwrap_or(0.0);
            let _ = v.set(k, p, c * vkp - s * vkq);
            let _ = v.set(k, q, s * vkp + c * vkq);
        }
    }
    let mut vals: Vec<f64> = (0..n).map(|i| w.get(idx(n, i, i)).copied().unwrap_or(0.0)).collect();
    // Sort ascending, permuting eigenvector columns to match.
    let mut order: Vec<usize> = (0..n).collect();
    order
        .sort_by(|&a, &b| vals.get(a).copied().unwrap_or(0.0).partial_cmp(&vals.get(b).copied().unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal));
    let mut vsorted = Matrix::zeros(n, n);
    for (newc, &oldc) in order.iter().enumerate() {
        for r in 0..n {
            let _ = vsorted.set(r, newc, v.get(r, oldc).unwrap_or(0.0));
        }
    }
    vals = order.iter().map(|&i| vals.get(i).copied().unwrap_or(0.0)).collect();
    Ok((vals, vsorted))
}

/// Eigenvalues of a general real matrix — real entries only.
/// Thin wrapper over [`eigvals_c`]; errors when the spectrum has
/// a complex pair.
pub fn eigvals(a: &Matrix) -> Result<Vec<f64>, MatError> {
    let w = eigvals_c(a)?;
    let mut out = Vec::with_capacity(w.len());
    for z in &w {
        if !z.is_real() {
            return Err(MatError::Other("complex eigenvalues need eigvals_c / Value::Cplx".into()));
        }
        out.push(z.re);
    }
    Ok(out)
}

/// Eigenvalues of a general real matrix as complex numbers.
/// Hessenberg + shifted QR; complex pairs are read off the 2×2
/// blocks of the quasi-triangular Schur form (the quadratic formula
/// on `[[a,b],[c,d]]`: λ = (a+d)/2 ± √((a−d)²/4 + bc)·i-ish).
pub fn eigvals_c(a: &Matrix) -> Result<Vec<crate::CNum>, MatError> {
    if a.rows() != a.cols() {
        return Err(MatError::Singular);
    }
    if is_symmetric(a) {
        return Ok(eig_symmetric(a)?.0.into_iter().map(crate::CNum::real).collect());
    }
    let n = a.rows();
    // Hessenberg reduction (Householder).
    let mut h = a.clone();
    for k in 0..n.saturating_sub(2) {
        let mut v: Vec<f64> = (0..n).map(|i| if i <= k { 0.0 } else { h.get(i, k).unwrap_or(0.0) }).collect();
        let norm = v.iter().map(|x| x * x).sum::<f64>().sqrt();
        if norm == 0.0 {
            continue;
        }
        let sign = if v.get(k + 1).copied().unwrap_or(0.0) >= 0.0 { 1.0 } else { -1.0 };
        if let Some(slot) = v.get_mut(k + 1) {
            *slot += sign * norm;
        }
        let vnorm2: f64 = v.iter().map(|x| x * x).sum();
        if vnorm2 == 0.0 {
            continue;
        }
        // H = H - 2 (H v) v' / v'v ; then H = H - 2 v (v' H) / v'v
        for j in 0..n {
            let mut dot = 0.0;
            for i in 0..n {
                dot += h.get(i, j).unwrap_or(0.0) * v.get(i).copied().unwrap_or(0.0);
            }
            let coef = 2.0 * dot / vnorm2;
            for i in 0..n {
                let nv = h.get(i, j).unwrap_or(0.0) - coef * v.get(i).copied().unwrap_or(0.0);
                let _ = h.set(i, j, nv);
            }
        }
        for i in 0..n {
            let mut dot = 0.0;
            for j in 0..n {
                dot += v.get(j).copied().unwrap_or(0.0) * h.get(j, i).unwrap_or(0.0);
            }
            let coef = 2.0 * dot / vnorm2;
            for j in 0..n {
                let nv = h.get(j, i).unwrap_or(0.0) - coef * v.get(j).copied().unwrap_or(0.0);
                let _ = h.set(j, i, nv);
            }
        }
    }
    // Unshifted QR iteration on the Hessenberg form.
    let mut a_k = h;
    for _ in 0..MAX_QR_ITERS {
        // Convergence check on subdiagonals.
        let mut converged = true;
        for i in 0..n.saturating_sub(1) {
            if a_k.get(i + 1, i).unwrap_or(0.0).abs() > 1e-10 {
                converged = false;
            }
        }
        if converged {
            break;
        }
        let (q, r) = qr(&a_k)?;
        a_k = r.matmul(&q)?;
    }
    let mut out = Vec::with_capacity(n);
    let mut i = 0;
    while i < n {
        if i + 1 < n && a_k.get(i + 1, i).unwrap_or(0.0).abs() > 1e-8 {
            // Stalled 2x2 block [[a,b],[c,d]] — eigenvalues solve
            // λ² − (a+d)λ + (ad−bc) = 0.
            let (x11, x12) = (a_k.get(i, i).unwrap_or(0.0), a_k.get(i, i + 1).unwrap_or(0.0));
            let (x21, x22) = (a_k.get(i + 1, i).unwrap_or(0.0), a_k.get(i + 1, i + 1).unwrap_or(0.0));
            let tr = x11 + x22;
            let disc = (x11 - x22) * (x11 - x22) / 4.0 + x12 * x21;
            if disc < 0.0 {
                let im = (-disc).sqrt();
                out.push(crate::CNum::new(tr / 2.0, im));
                out.push(crate::CNum::new(tr / 2.0, -im));
            } else {
                let s = disc.sqrt();
                out.push(crate::CNum::real(tr / 2.0 + s));
                out.push(crate::CNum::real(tr / 2.0 - s));
            }
            i += 2;
        } else {
            out.push(crate::CNum::real(a_k.get(i, i).unwrap_or(0.0)));
            i += 1;
        }
    }
    out.sort_by(|x, y| {
        x.re.partial_cmp(&y.re).unwrap_or(std::cmp::Ordering::Equal).then(y.im.partial_cmp(&x.im).unwrap_or(std::cmp::Ordering::Equal))
    });
    Ok(out)
}

/// Singular value decomposition A = U S V' (economy or full, m×n).
/// Built on the symmetric eigendecomposition of A'A — adequate for the
/// dense small/medium problems MatCraft targets today.
pub struct Svd {
    pub u: Matrix,
    pub s: Matrix,
    pub v: Matrix,
}

pub fn svd(a: &Matrix) -> Result<Svd, MatError> {
    let (m, n) = a.shape();
    let ata = a.transpose().matmul(a)?;
    let (w2, v) = eig_symmetric(&ata)?;
    // Eigenvalues ascending from Jacobi; singular values descending.
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&i, &j| w2.get(j).copied().unwrap_or(0.0).partial_cmp(&w2.get(i).copied().unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal));
    let mut svals: Vec<f64> = Vec::with_capacity(n);
    let mut vsort = Matrix::zeros(n, n);
    for (newc, &oldc) in order.iter().enumerate() {
        svals.push(w2.get(oldc).copied().unwrap_or(0.0).max(0.0).sqrt());
        for r in 0..n {
            let _ = vsort.set(r, newc, v.get(r, oldc).unwrap_or(0.0));
        }
    }
    // U columns: A v_i / s_i; near-zero singulars get orthonormalised
    // complements via Gram–Schmidt against the columns so far.
    let mut u = Matrix::zeros(m, n.min(m));
    for c in 0..n.min(m) {
        let s = svals.get(c).copied().unwrap_or(0.0);
        let mut col: Vec<f64> = Vec::with_capacity(m);
        for r in 0..m {
            let mut acc = 0.0;
            for k in 0..n {
                acc += a.get(r, k).unwrap_or(0.0) * vsort.get(k, c).unwrap_or(0.0);
            }
            col.push(if s > 1e-12 { acc / s } else { 0.0 });
        }
        if s <= 1e-12 {
            // Fill with a unit vector orthogonal to previous columns.
            for k in 0..m {
                let mut e: Vec<f64> = vec![0.0; m];
                if let Some(slot) = e.get_mut(k) {
                    *slot = 1.0;
                }
                for p in 0..c {
                    let d: f64 = (0..m).map(|r| e.get(r).copied().unwrap_or(0.0) * u.get(r, p).unwrap_or(0.0)).sum();
                    for r in 0..m {
                        if let Some(x) = e.get_mut(r) {
                            *x -= d * u.get(r, p).unwrap_or(0.0);
                        }
                    }
                }
                let en: f64 = e.iter().map(|x| x * x).sum::<f64>().sqrt();
                if en > 1e-9 {
                    col = e.iter().map(|x| x / en).collect();
                    break;
                }
            }
        } else {
            // Re-orthogonalise against previous columns (one pass suffices
            // at these sizes).
            for p in 0..c {
                let d: f64 = (0..m).map(|r| col.get(r).copied().unwrap_or(0.0) * u.get(r, p).unwrap_or(0.0)).sum();
                for r in 0..m {
                    if let Some(x) = col.get_mut(r) {
                        *x -= d * u.get(r, p).unwrap_or(0.0);
                    }
                }
            }
            let cn: f64 = col.iter().map(|x| x * x).sum::<f64>().sqrt();
            if cn > 0.0 {
                for x in col.iter_mut() {
                    *x /= cn;
                }
            }
        }
        for r in 0..m {
            let _ = u.set(r, c, col.get(r).copied().unwrap_or(0.0));
        }
    }
    let mut smat = Matrix::zeros(m.min(n), n);
    for (i, s) in svals.iter().take(m.min(n)).enumerate() {
        let _ = smat.set(i, i, *s);
    }
    Ok(Svd { u, s: smat, v: vsort })
}

/// Singular values only.
pub fn singular_values(a: &Matrix) -> Result<Vec<f64>, MatError> {
    let s = svd(a)?;
    let mut out = Vec::new();
    for i in 0..s.s.rows().min(s.s.cols()) {
        out.push(s.s.get(i, i).unwrap_or(0.0));
    }
    Ok(out)
}

/// Rank: number of singular values above the standard tolerance.
pub fn rank(a: &Matrix) -> Result<usize, MatError> {
    let sv = singular_values(a)?;
    let smax = sv.iter().copied().fold(0.0f64, f64::max);
    let tol = smax * (a.rows().max(a.cols()) as f64) * f64::EPSILON * 32.0;
    Ok(sv.iter().filter(|&&s| s > tol).count())
}

/// 2-norm condition number σmax/σmin (∞ for singular).
pub fn cond(a: &Matrix) -> Result<f64, MatError> {
    let sv = singular_values(a)?;
    let smax = sv.iter().copied().fold(0.0f64, f64::max);
    let smin = sv.iter().copied().fold(f64::INFINITY, f64::min);
    if smin == 0.0 {
        return Ok(f64::INFINITY);
    }
    Ok(smax / smin)
}

/// Moore–Penrose pseudoinverse via SVD.
pub fn pinv(a: &Matrix) -> Result<Matrix, MatError> {
    let s = svd(a)?;
    let smax = {
        let mut m = 0.0f64;
        for i in 0..s.s.rows().min(s.s.cols()) {
            m = m.max(s.s.get(i, i).unwrap_or(0.0));
        }
        m
    };
    let tol = smax * (a.rows().max(a.cols()) as f64) * f64::EPSILON * 32.0;
    let mut sp = Matrix::zeros(s.v.cols(), s.u.cols());
    for i in 0..s.s.rows().min(s.s.cols()) {
        let sv = s.s.get(i, i).unwrap_or(0.0);
        if sv > tol {
            let _ = sp.set(i, i, 1.0 / sv);
        }
    }
    // pinv = V * S+ * U'
    s.v.matmul(&sp)?.matmul(&s.u.transpose())
}

/// Trace (sum of the main diagonal).
pub fn trace(a: &Matrix) -> f64 {
    (0..a.rows().min(a.cols())).map(|i| a.get(i, i).unwrap_or(0.0)).sum()
}

/// Reduced row echelon form (Gauss–Jordan with partial pivoting).
pub fn rref(a: &Matrix) -> Result<Matrix, MatError> {
    let (m, n) = a.shape();
    let mut r = vec![0.0; m.saturating_mul(n)];
    for c in 0..n {
        for row in 0..m {
            if let Some(slot) = r.get_mut(row * n + c) {
                *slot = a.get(row, c).unwrap_or(0.0);
            }
        }
    }
    let mut lead = 0usize;
    let tol = 1e-12;
    for row in 0..m {
        if lead >= n {
            break;
        }
        let mut i = row;
        while r.get(i * n + lead).map(|v| v.abs()).unwrap_or(0.0) <= tol {
            i += 1;
            if i == m {
                i = row;
                lead += 1;
                if lead == n {
                    // Rebuild and return.
                    let mut out = Matrix::zeros(m, n);
                    for c in 0..n {
                        for rr in 0..m {
                            let _ = out.set(rr, c, r.get(rr * n + c).copied().unwrap_or(0.0));
                        }
                    }
                    return Ok(out);
                }
            }
        }
        if i != row {
            for c in 0..n {
                r.swap(row * n + c, i * n + c);
            }
        }
        let div = r.get(row * n + lead).copied().unwrap_or(1.0);
        for c in 0..n {
            if let Some(slot) = r.get_mut(row * n + c) {
                *slot /= div;
            }
        }
        for i in 0..m {
            if i != row {
                let f = r.get(i * n + lead).copied().unwrap_or(0.0);
                if f.abs() > tol {
                    for c in 0..n {
                        let nv = r.get(i * n + c).copied().unwrap_or(0.0) - f * r.get(row * n + c).copied().unwrap_or(0.0);
                        if let Some(slot) = r.get_mut(i * n + c) {
                            *slot = nv;
                        }
                    }
                }
            }
        }
        lead += 1;
    }
    let mut out = Matrix::zeros(m, n);
    for c in 0..n {
        for rr in 0..m {
            let _ = out.set(rr, c, r.get(rr * n + c).copied().unwrap_or(0.0));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Matrix as M;

    fn m(rows: usize, cols: usize, data: &[f64]) -> M {
        M::from_row_major(rows, cols, data).unwrap()
    }

    #[test]
    fn qr_reconstructs() {
        let a = m(3, 3, &[2.0, 1.0, 0.0, 1.0, 3.0, 1.0, 0.0, 1.0, 2.0]);
        let (q, r) = qr(&a).unwrap();
        let back = q.matmul(&r).unwrap();
        for i in 0..9 {
            let (row, c) = (i % 3, i / 3);
            assert!((back.get(row, c).unwrap() - a.get(row, c).unwrap()).abs() < 1e-9);
        }
        // Q orthogonal.
        let qq = q.transpose().matmul(&q).unwrap();
        for i in 0..3 {
            assert!((qq.get(i, i).unwrap() - 1.0).abs() < 1e-9);
        }
    }

    #[test]
    fn chol_of_spd() {
        let a = m(2, 2, &[4.0, 2.0, 2.0, 3.0]);
        let r = chol(&a).unwrap();
        let back = r.transpose().matmul(&r).unwrap();
        assert!((back.get(0, 0).unwrap() - 4.0).abs() < 1e-9);
        assert!((back.get(1, 1).unwrap() - 3.0).abs() < 1e-9);
    }

    #[test]
    fn lu_pivot_reconstructs() {
        let a = m(3, 3, &[0.0, 2.0, 1.0, 1.0, 0.0, 0.0, 3.0, 1.0, 1.0]);
        let (l, u, p) = lu(&a).unwrap();
        let pa = p.matmul(&a).unwrap();
        let luu = l.matmul(&u).unwrap();
        for i in 0..9 {
            let (r, c) = (i % 3, i / 3);
            assert!((pa.get(r, c).unwrap() - luu.get(r, c).unwrap()).abs() < 1e-9);
        }
    }

    #[test]
    fn eig_symmetric_known() {
        // Eigenvalues of [[2,1],[1,2]] are 1 and 3.
        let a = m(2, 2, &[2.0, 1.0, 1.0, 2.0]);
        let (w, v) = eig_symmetric(&a).unwrap();
        assert!((w[0] - 1.0).abs() < 1e-9);
        assert!((w[1] - 3.0).abs() < 1e-9);
        // V' A V ≈ diag(w).
        let d = v.transpose().matmul(&a).unwrap().matmul(&v).unwrap();
        assert!((d.get(0, 1).unwrap()).abs() < 1e-9);
    }

    #[test]
    fn eigvals_upper_triangular() {
        // Eigenvalues are the diagonal of a triangular matrix.
        let a = m(3, 3, &[2.0, 5.0, -1.0, 0.0, 3.0, 4.0, 0.0, 0.0, -2.0]);
        let w = eigvals(&a).unwrap();
        assert!((w[0] - (-2.0)).abs() < 1e-8);
        assert!((w[1] - 2.0).abs() < 1e-8);
        assert!((w[2] - 3.0).abs() < 1e-8);
    }

    #[test]
    fn svd_reconstructs_and_orders() {
        let a = m(3, 2, &[3.0, 0.0, 4.0, 5.0, 0.0, 0.0]);
        let s = svd(&a).unwrap();
        let back = s.u.matmul(&s.s).unwrap().matmul(&s.v.transpose()).unwrap();
        for i in 0..6 {
            let (r, c) = (i % 3, i / 3);
            assert!((back.get(r, c).unwrap() - a.get(r, c).unwrap()).abs() < 1e-8, "{r},{c}");
        }
        // Singular values descending.
        assert!(s.s.get(0, 0).unwrap() >= s.s.get(1, 1).unwrap());
    }

    #[test]
    fn rank_cond_pinv() {
        let a = m(2, 2, &[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(rank(&a).unwrap(), 2);
        assert!(cond(&a).unwrap() > 1.0);
        let p = pinv(&a).unwrap();
        // A * pinv(A) * A ≈ A
        let back = a.matmul(&p).unwrap().matmul(&a).unwrap();
        assert!((back.get(0, 0).unwrap() - 1.0).abs() < 1e-8);
        assert!((back.get(1, 1).unwrap() - 4.0).abs() < 1e-8);
        let sing = m(2, 2, &[1.0, 2.0, 2.0, 4.0]);
        assert_eq!(rank(&sing).unwrap(), 1);
        assert_eq!(cond(&sing).unwrap(), f64::INFINITY);
    }

    #[test]
    fn rref_known() {
        // rref([1 2 3; 4 5 6]) = [1 0 -1; 0 1 2]
        let a = m(2, 3, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        let r = rref(&a).unwrap();
        assert!((r.get(0, 2).unwrap() - (-1.0)).abs() < 1e-9);
        assert!((r.get(1, 2).unwrap() - 2.0).abs() < 1e-9);
    }

    #[test]
    fn complex_pair_errors_not_panics() {
        // Rotation matrix: eigenvalues ±i.
        let a = m(2, 2, &[0.0, -1.0, 1.0, 0.0]);
        assert!(eigvals(&a).is_err());
    }
}

#[cfg(test)]
mod cplx_tests {
    use super::*;

    #[test]
    fn eigvals_c_rotation_pair() {
        // Rotation by 90°: eigenvalues ±i.
        let a = Matrix::from_row_major(2, 2, &[0.0, -1.0, 1.0, 0.0]).unwrap();
        let w = eigvals_c(&a).unwrap();
        assert_eq!(w.len(), 2);
        let ims: Vec<f64> = w.iter().map(|z| z.im).collect();
        assert!(ims.iter().any(|&i| (i - 1.0).abs() < 1e-6), "{w:?}");
        assert!(ims.iter().any(|&i| (i + 1.0).abs() < 1e-6), "{w:?}");
        assert!(w.iter().all(|z| z.re.abs() < 1e-6), "{w:?}");
    }

    #[test]
    fn eigvals_c_real_matrix_matches() {
        // Real spectrum stays real; agrees with eigvals.
        let a = Matrix::from_row_major(3, 3, &[2.0, 1.0, 0.0, 0.0, 3.0, 0.0, 0.0, 0.0, 5.0]).unwrap();
        let w = eigvals_c(&a).unwrap();
        assert!(w.iter().all(|z| z.is_real()));
        assert_eq!(w.iter().map(|z| z.re).collect::<Vec<_>>(), vec![2.0, 3.0, 5.0]);
    }
}
