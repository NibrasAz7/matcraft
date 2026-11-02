//! Statistics toolbox builtins.
//!
//! Distribution functions are element-wise over matrices with scalar
//! expansion. Random generators use the session's SplitMix64 stream, so
//! `rng(seed)` makes them reproducible. `iqr`, `corrcoef`, `regress` and
//! `kmeans` cover the statistics helpers used by the local MATLAB corpus.

use super::*;
use matcore::stats;

fn matrix_arg(v: Option<&Value>, what: &str) -> Result<Matrix, LangError> {
    v.ok_or_else(|| LangError::new(format!("{what} is required"), 0))?.to_numeric()
}

fn default_matrix(v: Option<&Value>, value: f64) -> Result<Matrix, LangError> {
    match v {
        None => Ok(Matrix::scalar(value)),
        Some(x) => x.to_numeric(),
    }
}

fn broadcast_dims(ms: &[&Matrix]) -> Result<Vec<usize>, LangError> {
    let mut dims: Option<Vec<usize>> = None;
    for m in ms {
        if m.numel() > 1 {
            match &dims {
                None => dims = Some(m.dims().to_vec()),
                Some(d) if d == m.dims() => {}
                Some(_) => return Err(LangError::new("non-scalar inputs must have the same size", 0)),
            }
        }
    }
    Ok(dims.unwrap_or_else(|| vec![1, 1]))
}

fn at(m: &Matrix, i: usize) -> f64 {
    if m.numel() == 1 {
        m.data().first().copied().unwrap_or(0.0)
    } else {
        m.get_linear(i).unwrap_or(0.0)
    }
}

fn map3(a: &Matrix, b: &Matrix, c: &Matrix, f: impl Fn(f64, f64, f64) -> f64) -> Result<Matrix, LangError> {
    let dims = broadcast_dims(&[a, b, c])?;
    let n = dims.iter().fold(1usize, |acc, &d| acc.saturating_mul(d));
    check_elems(n)?;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        out.push(f(at(a, i), at(b, i), at(c, i)));
    }
    Matrix::from_col_major_nd(&dims, out).map_err(lang)
}

fn sample_dims(argv: &[Value], start: usize, fallback: &[usize]) -> Result<Vec<usize>, LangError> {
    if let Some(a) = argv.get(start) {
        if let Some(b) = argv.get(start.saturating_add(1)) {
            let ma = a.to_numeric()?;
            let mb = b.to_numeric()?;
            if ma.numel() == 1 && mb.numel() == 1 {
                let r = ma.data().first().copied().unwrap_or(0.0).max(0.0) as usize;
                let c = mb.data().first().copied().unwrap_or(0.0).max(0.0) as usize;
                check_elems(r.saturating_mul(c))?;
                return Ok(vec![r, c]);
            }
        }
        let m = a.to_numeric()?;
        if m.numel() > 1 && (m.rows() == 1 || m.cols() == 1) {
            let r = m.data().first().copied().unwrap_or(0.0).max(0.0) as usize;
            let c = m.data().get(1).copied().unwrap_or(0.0).max(0.0) as usize;
            check_elems(r.saturating_mul(c))?;
            return Ok(vec![r, c]);
        }
    }
    Ok(fallback.to_vec())
}

fn normal_sample(s: &mut Session) -> f64 {
    let u1 = s.next_f64().max(1e-300);
    let u2 = s.next_f64();
    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
}

fn poisson_sample(s: &mut Session, lambda: f64) -> f64 {
    if lambda.is_nan() || lambda < 0.0 || !lambda.is_finite() {
        return f64::NAN;
    }
    if lambda == 0.0 {
        return 0.0;
    }
    if lambda < 30.0 {
        let l = (-lambda).exp();
        let mut k = 0.0f64;
        let mut p = 1.0f64;
        loop {
            k += 1.0;
            p *= s.next_f64();
            if p <= l || k > 1e7 {
                return (k - 1.0).max(0.0);
            }
        }
    } else {
        (lambda + lambda.sqrt() * normal_sample(s)).round().max(0.0)
    }
}

fn binom_sample(s: &mut Session, n: f64, p: f64) -> f64 {
    if n.is_nan() || n < 0.0 || n.fract() != 0.0 || !(0.0..=1.0).contains(&p) || n > 1e9 {
        return f64::NAN;
    }
    if n <= 1000.0 {
        let mut k = 0.0f64;
        for _ in 0..(n as usize) {
            if s.next_f64() < p {
                k += 1.0;
            }
        }
        k
    } else {
        let var = n * p * (1.0 - p);
        (n * p + var.sqrt() * normal_sample(s)).round().clamp(0.0, n)
    }
}

fn sample_elem(m: &Matrix, i: usize) -> f64 {
    at(m, i)
}

fn quantile_sorted(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    if sorted.len() == 1 {
        return sorted.first().copied().unwrap_or(f64::NAN);
    }
    let h = (sorted.len().saturating_sub(1)) as f64 * p;
    let lo = h.floor() as usize;
    let hi = h.ceil() as usize;
    let frac = h - lo as f64;
    let a = sorted.get(lo).copied().unwrap_or(f64::NAN);
    let b = sorted.get(hi).copied().unwrap_or(a);
    a * (1.0 - frac) + b * frac
}

fn hcat(a: &Matrix, b: &Matrix) -> Result<Matrix, LangError> {
    if a.rows() != b.rows() {
        return Err(LangError::new("corrcoef inputs must have the same number of rows", 0));
    }
    let mut out = Matrix::zeros(a.rows(), a.cols().saturating_add(b.cols()));
    for r in 0..a.rows() {
        for c in 0..a.cols() {
            let _ = out.set(r, c, a.get(r, c).unwrap_or(0.0));
        }
        for c in 0..b.cols() {
            let _ = out.set(r, a.cols().saturating_add(c), b.get(r, c).unwrap_or(0.0));
        }
    }
    Ok(out)
}

fn iqr_values(values: &[f64]) -> f64 {
    let mut v: Vec<f64> = values.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    quantile_sorted(&v, 0.75) - quantile_sorted(&v, 0.25)
}

impl Session {
    pub(super) fn stats_builtin(&mut self, name: &str, argv: Vec<Value>, nout: usize) -> Result<Vec<Value>, LangError> {
        match name {
            "normpdf" | "normcdf" | "norminv" => {
                let x = matrix_arg(argv.first(), "x")?;
                let mu = default_matrix(argv.get(1), 0.0)?;
                let sigma = default_matrix(argv.get(2), 1.0)?;
                let f: fn(f64, f64, f64) -> f64 = match name {
                    "normpdf" => stats::norm_pdf,
                    "normcdf" => stats::norm_cdf,
                    _ => stats::norm_inv,
                };
                Ok(vec![mat_or_num(map3(&x, &mu, &sigma, f)?)])
            }
            "exppdf" | "expcdf" | "expinv" => {
                let x = matrix_arg(argv.first(), "x")?;
                let mu = default_matrix(argv.get(1), 1.0)?;
                let f: fn(f64, f64, f64) -> f64 = match name {
                    "exppdf" => |x, mu, _| stats::exp_pdf(x, mu),
                    "expcdf" => |x, mu, _| stats::exp_cdf(x, mu),
                    _ => |x, mu, _| stats::exp_inv(x, mu),
                };
                Ok(vec![mat_or_num(map3(&x, &mu, &Matrix::scalar(0.0), f)?)])
            }
            "unifpdf" | "unifcdf" | "unifinv" => {
                let x = matrix_arg(argv.first(), "x")?;
                let a = default_matrix(argv.get(1), 0.0)?;
                let b = default_matrix(argv.get(2), 1.0)?;
                let f: fn(f64, f64, f64) -> f64 = match name {
                    "unifpdf" => stats::unif_pdf,
                    "unifcdf" => stats::unif_cdf,
                    _ => stats::unif_inv,
                };
                Ok(vec![mat_or_num(map3(&x, &a, &b, f)?)])
            }
            "poisspdf" | "poisscdf" | "poissinv" => {
                let x = matrix_arg(argv.first(), "k")?;
                let lambda = matrix_arg(argv.get(1), "lambda")?;
                let f: fn(f64, f64, f64) -> f64 = match name {
                    "poisspdf" => |k, l, _| stats::poisson_pdf(k, l),
                    "poisscdf" => |k, l, _| stats::poisson_cdf(k, l),
                    _ => |p, l, _| stats::poisson_inv(p, l),
                };
                Ok(vec![mat_or_num(map3(&x, &lambda, &Matrix::scalar(0.0), f)?)])
            }
            "binopdf" | "binocdf" | "binoinv" => {
                let x = matrix_arg(argv.first(), "k")?;
                let n = matrix_arg(argv.get(1), "n")?;
                let p = matrix_arg(argv.get(2), "p")?;
                let f: fn(f64, f64, f64) -> f64 = match name {
                    "binopdf" => stats::binom_pdf,
                    "binocdf" => stats::binom_cdf,
                    _ => stats::binom_inv,
                };
                Ok(vec![mat_or_num(map3(&x, &n, &p, f)?)])
            }
            "gampdf" | "gamcdf" | "gaminv" => {
                let x = matrix_arg(argv.first(), "x")?;
                let a = matrix_arg(argv.get(1), "a")?;
                let b = matrix_arg(argv.get(2), "b")?;
                let f: fn(f64, f64, f64) -> f64 = match name {
                    "gampdf" => stats::gamma_pdf,
                    "gamcdf" => stats::gamma_cdf,
                    _ => stats::gamma_inv,
                };
                Ok(vec![mat_or_num(map3(&x, &a, &b, f)?)])
            }
            "betapdf" | "betacdf" | "betainv" => {
                let x = matrix_arg(argv.first(), "x")?;
                let a = matrix_arg(argv.get(1), "a")?;
                let b = matrix_arg(argv.get(2), "b")?;
                let f: fn(f64, f64, f64) -> f64 = match name {
                    "betapdf" => stats::beta_pdf,
                    "betacdf" => stats::beta_cdf,
                    _ => stats::beta_inv,
                };
                Ok(vec![mat_or_num(map3(&x, &a, &b, f)?)])
            }
            "chi2pdf" | "chi2cdf" | "chi2inv" => {
                let x = matrix_arg(argv.first(), "x")?;
                let nu = matrix_arg(argv.get(1), "nu")?;
                let f: fn(f64, f64, f64) -> f64 = match name {
                    "chi2pdf" => |x, nu, _| stats::chi2_pdf(x, nu),
                    "chi2cdf" => |x, nu, _| stats::chi2_cdf(x, nu),
                    _ => |p, nu, _| stats::chi2_inv(p, nu),
                };
                Ok(vec![mat_or_num(map3(&x, &nu, &Matrix::scalar(0.0), f)?)])
            }
            "tpdf" | "tcdf" | "tinv" => {
                let x = matrix_arg(argv.first(), "x")?;
                let nu = matrix_arg(argv.get(1), "nu")?;
                let f: fn(f64, f64, f64) -> f64 = match name {
                    "tpdf" => |x, nu, _| stats::t_pdf(x, nu),
                    "tcdf" => |x, nu, _| stats::t_cdf(x, nu),
                    _ => |p, nu, _| stats::t_inv(p, nu),
                };
                Ok(vec![mat_or_num(map3(&x, &nu, &Matrix::scalar(0.0), f)?)])
            }
            "fpdf" | "fcdf" | "finv" => {
                let x = matrix_arg(argv.first(), "x")?;
                let d1 = matrix_arg(argv.get(1), "d1")?;
                let d2 = matrix_arg(argv.get(2), "d2")?;
                let f: fn(f64, f64, f64) -> f64 = match name {
                    "fpdf" => stats::f_pdf,
                    "fcdf" => stats::f_cdf,
                    _ => stats::f_inv,
                };
                Ok(vec![mat_or_num(map3(&x, &d1, &d2, f)?)])
            }
            "exprnd" => {
                let mu = matrix_arg(argv.first(), "mu")?;
                let dims = sample_dims(&argv, 1, mu.dims())?;
                let n = dims.iter().fold(1usize, |a, &d| a.saturating_mul(d));
                check_elems(n)?;
                let mut out = Vec::with_capacity(n);
                for i in 0..n {
                    let u = self.next_f64();
                    out.push(-sample_elem(&mu, i) * (1.0 - u).max(1e-300).ln());
                }
                Ok(vec![mat_or_num(Matrix::from_col_major_nd(&dims, out).map_err(lang)?)])
            }
            "unifrnd" => {
                let a = matrix_arg(argv.first(), "a")?;
                let b = matrix_arg(argv.get(1), "b")?;
                let fallback = broadcast_dims(&[&a, &b])?;
                let dims = sample_dims(&argv, 2, &fallback)?;
                let n = dims.iter().fold(1usize, |x, &d| x.saturating_mul(d));
                check_elems(n)?;
                let mut out = Vec::with_capacity(n);
                for i in 0..n {
                    let u = self.next_f64();
                    let lo = sample_elem(&a, i);
                    let hi = sample_elem(&b, i);
                    out.push(lo + (hi - lo) * u);
                }
                Ok(vec![mat_or_num(Matrix::from_col_major_nd(&dims, out).map_err(lang)?)])
            }
            "poissrnd" => {
                let lambda = matrix_arg(argv.first(), "lambda")?;
                let dims = sample_dims(&argv, 1, lambda.dims())?;
                let n = dims.iter().fold(1usize, |a, &d| a.saturating_mul(d));
                check_elems(n)?;
                let mut out = Vec::with_capacity(n);
                for i in 0..n {
                    out.push(poisson_sample(self, sample_elem(&lambda, i)));
                }
                Ok(vec![mat_or_num(Matrix::from_col_major_nd(&dims, out).map_err(lang)?)])
            }
            "binornd" => {
                let nval = matrix_arg(argv.first(), "n")?;
                let p = matrix_arg(argv.get(1), "p")?;
                let fallback = broadcast_dims(&[&nval, &p])?;
                let dims = sample_dims(&argv, 2, &fallback)?;
                let n = dims.iter().fold(1usize, |a, &d| a.saturating_mul(d));
                check_elems(n)?;
                let mut out = Vec::with_capacity(n);
                for i in 0..n {
                    out.push(binom_sample(self, sample_elem(&nval, i), sample_elem(&p, i)));
                }
                Ok(vec![mat_or_num(Matrix::from_col_major_nd(&dims, out).map_err(lang)?)])
            }
            "iqr" => {
                let x = matrix_arg(argv.first(), "X")?;
                if x.rows() <= 1 || x.cols() <= 1 {
                    let v = iqr_values(x.data());
                    return Ok(vec![Value::Num(v)]);
                }
                let dim = argv.get(1).map(|v| v.scalar()).transpose()?.unwrap_or(1.0) as usize;
                if dim == 1 {
                    let mut out = Vec::with_capacity(x.cols());
                    for c in 0..x.cols() {
                        let col: Vec<f64> = (0..x.rows()).map(|r| x.get(r, c).unwrap_or(0.0)).collect();
                        out.push(iqr_values(&col));
                    }
                    Ok(vec![Value::Mat(Matrix::row_vector(out))])
                } else if dim == 2 {
                    let mut out = Vec::with_capacity(x.rows());
                    for r in 0..x.rows() {
                        let rowv: Vec<f64> = (0..x.cols()).map(|c| x.get(r, c).unwrap_or(0.0)).collect();
                        out.push(iqr_values(&rowv));
                    }
                    Ok(vec![Value::Mat(Matrix::col_vector(out))])
                } else {
                    Err(LangError::new("iqr: dim must be 1 or 2", 0))
                }
            }
            "corrcoef" => {
                let mut x = matrix_arg(argv.first(), "X")?;
                let mut ymat = argv.get(1).map(|v| v.to_numeric()).transpose()?;
                if x.rows() == 1 && x.cols() > 1 && (ymat.is_none() || ymat.as_ref().map(|y| y.rows() == 1).unwrap_or(false)) {
                    x = x.transpose();
                    if let Some(y) = &ymat {
                        ymat = Some(y.transpose());
                    }
                }
                let data = match ymat {
                    Some(y) => hcat(&x, &y)?,
                    None => x,
                };
                let (n, p) = (data.rows(), data.cols());
                let mut means = vec![0.0; p];
                for (c, mean) in means.iter_mut().enumerate().take(p) {
                    for r in 0..n {
                        *mean += data.get(r, c).unwrap_or(0.0);
                    }
                    *mean /= n.max(1) as f64;
                }
                let mut ss = vec![0.0; p];
                for c in 0..p {
                    for r in 0..n {
                        let d = data.get(r, c).unwrap_or(0.0) - means[c];
                        ss[c] += d * d;
                    }
                }
                let mut rmat = Matrix::zeros(p, p);
                for i in 0..p {
                    for j in 0..p {
                        let mut cov = 0.0;
                        for r in 0..n {
                            cov += (data.get(r, i).unwrap_or(0.0) - means[i]) * (data.get(r, j).unwrap_or(0.0) - means[j]);
                        }
                        let den = (ss[i] * ss[j]).sqrt();
                        let v = if den > 0.0 && n > 1 { cov / den } else { f64::NAN };
                        let _ = rmat.set(i, j, v);
                    }
                }
                let mut out = vec![Value::Mat(rmat.clone())];
                if nout >= 2 {
                    let mut pmat = Matrix::zeros(p, p);
                    for i in 0..p {
                        for j in 0..p {
                            let r = rmat.get(i, j).unwrap_or(0.0);
                            let v = if i == j || (r.abs() - 1.0).abs() < 1e-15 {
                                0.0
                            } else if n > 2 {
                                let t = r * ((n - 2) as f64 / (1.0 - r * r)).sqrt();
                                2.0 * (1.0 - stats::t_cdf(t.abs(), (n - 2) as f64))
                            } else {
                                f64::NAN
                            };
                            let _ = pmat.set(i, j, v.clamp(0.0, 1.0));
                        }
                    }
                    out.push(Value::Mat(pmat));
                }
                Ok(out)
            }
            "regress" => {
                let y = matrix_arg(argv.first(), "y")?;
                let x = matrix_arg(argv.get(1), "X")?;
                let n = x.rows();
                if y.numel() != n {
                    return Err(LangError::new("regress: y and X must have the same number of rows", 0));
                }
                let p = x.cols();
                let yv = y.data().to_vec();
                let ycol = Matrix::col_vector(yv.clone());
                let pinv = matcore::pinv(&x).map_err(lang)?;
                let b = pinv.matmul(&ycol).map_err(lang)?;
                let pred = x.matmul(&b).map_err(lang)?;
                let resid: Vec<f64> = (0..n).map(|i| yv.get(i).copied().unwrap_or(0.0) - pred.get(i, 0).unwrap_or(0.0)).collect();
                let mean = yv.iter().sum::<f64>() / n.max(1) as f64;
                let sse: f64 = resid.iter().map(|r| r * r).sum();
                let sst: f64 = yv.iter().map(|v| (v - mean) * (v - mean)).sum();
                let r2 = if sst > 0.0 { 1.0 - sse / sst } else { 1.0 };
                let df = n.saturating_sub(p);
                let mse = if df > 0 { sse / df as f64 } else { f64::NAN };
                let (fstat, fp) = if p > 1 && df > 0 && sse.is_finite() {
                    if sse <= 1e-12 * sst.max(1.0) {
                        (f64::INFINITY, 0.0)
                    } else {
                        let f = (r2 / (p - 1) as f64) / ((1.0 - r2) / df as f64);
                        let fp = if f.is_finite() && f >= 0.0 { 1.0 - stats::f_cdf(f, (p - 1) as f64, df as f64) } else { f64::NAN };
                        (f, fp)
                    }
                } else {
                    (0.0, f64::NAN)
                };
                let mut bint = Matrix::zeros(p, 2);
                let mut rint = Matrix::zeros(n, 2);
                for i in 0..p {
                    let _ = bint.set(i, 0, f64::NAN);
                    let _ = bint.set(i, 1, f64::NAN);
                }
                for i in 0..n {
                    let _ = rint.set(i, 0, f64::NAN);
                    let _ = rint.set(i, 1, f64::NAN);
                }
                let stats_row = Matrix::row_vector(vec![r2, fstat, fp, mse]);
                Ok(vec![Value::Mat(b), Value::Mat(bint), Value::Mat(Matrix::col_vector(resid)), Value::Mat(rint), Value::Mat(stats_row)])
            }
            "kmeans" => self.kmeans_call(&argv),
            "fitdist" => self.fitdist_call(&argv),
            "pdf" | "cdf" | "icdf" | "random" => self.prob_call(name, &argv),
            "cmdscale" | "pcoa" => self.pcoa_call(&argv, nout),
            "bonferroni" | "holm" | "sidak" | "benjamini_hochberg" | "mafdr" => self.p_adjust_call(name, &argv, nout),
            "signrank" | "ranksum" | "kruskalwallis" => self.rank_test_call(name, &argv, nout),
            "ttest" => self.ttest_call(&argv, nout),
            "ttest2" => self.ttest2_call(&argv, nout),
            "kstest" => self.kstest_call(&argv, nout),
            "adtest" => self.adtest_call(&argv, nout),
            "pca" => self.pca_call(&argv, nout),
            _ => Err(LangError::new(format!("{name}: not a statistics builtin"), 0)),
        }
    }

    fn kmeans_call(&mut self, argv: &[Value]) -> Result<Vec<Value>, LangError> {
        let x = matrix_arg(argv.first(), "X")?;
        let n = x.rows();
        let d = x.cols();
        let k = argv.get(1).map(|v| v.scalar()).transpose()?.unwrap_or(0.0).max(1.0) as usize;
        if k > n {
            return Err(LangError::new("kmeans: k must not exceed the number of observations", 0));
        }
        let mut replicates = 1usize;
        let mut max_iter = 100usize;
        let mut metric = KMetric::SqEuclidean;
        let mut start = KStart::Plus;
        let mut singleton = true;
        let mut i = 2usize;
        while i.saturating_add(1) < argv.len() {
            if let Value::Str(key) = &argv[i] {
                let key = key.to_ascii_lowercase();
                let val = argv.get(i.saturating_add(1));
                match key.as_str() {
                    "distance" => {
                        let s = match val {
                            Some(Value::Str(s)) => s.to_ascii_lowercase(),
                            _ => String::new(),
                        };
                        metric = match s.as_str() {
                            "sqeuclidean" => KMetric::SqEuclidean,
                            "euclidean" => KMetric::Euclidean,
                            "cityblock" => KMetric::Cityblock,
                            "cosine" => KMetric::Cosine,
                            _ => return Err(LangError::new("kmeans: unsupported Distance", 0)),
                        };
                    }
                    "replicates" => {
                        replicates = val.map(|v| v.scalar()).transpose()?.unwrap_or(1.0).max(1.0) as usize;
                    }
                    "maxiter" => {
                        max_iter = val.map(|v| v.scalar()).transpose()?.unwrap_or(100.0).max(1.0) as usize;
                    }
                    "start" => match val {
                        Some(Value::Str(s)) => match s.to_ascii_lowercase().as_str() {
                            "plus" => start = KStart::Plus,
                            "sample" => start = KStart::Sample,
                            _ => start = KStart::Plus,
                        },
                        Some(v) => {
                            let m = v.to_numeric()?;
                            if m.rows() != k || m.cols() != d {
                                return Err(LangError::new("kmeans: Start matrix must be k-by-d", 0));
                            }
                            start = KStart::Matrix(m);
                        }
                        None => {}
                    },
                    "emptyaction" => {
                        singleton = !matches!(val, Some(Value::Str(s)) if s.eq_ignore_ascii_case("drop"));
                    }
                    _ => {}
                }
            }
            i = i.saturating_add(2);
        }
        let mut best: Option<KResult> = None;
        for _ in 0..replicates.max(1) {
            let centers = match &start {
                KStart::Matrix(m) => (0..k).map(|r| (0..d).map(|c| m.get(r, c).unwrap_or(0.0)).collect()).collect(),
                KStart::Sample => self.kmeans_sample_start(&x, k),
                KStart::Plus => self.kmeans_plus_start(&x, k, metric),
            };
            let res = lloyd_kmeans(&x, centers, max_iter, metric, singleton);
            let total: f64 = res.sumd.iter().sum();
            if best.as_ref().map(|b| total < b.total).unwrap_or(true) {
                best = Some(KResult { total, ..res });
            }
        }
        let res = best.ok_or_else(|| LangError::new("kmeans: no solution", 0))?;
        let idx: Vec<f64> = res.idx.iter().map(|&v| (v + 1) as f64).collect();
        let mut centers = Matrix::zeros(k, d);
        for r in 0..k {
            for c in 0..d {
                let _ = centers.set(r, c, res.centers.get(r).and_then(|row| row.get(c)).copied().unwrap_or(0.0));
            }
        }
        let mut dist = Matrix::zeros(n, k);
        for r in 0..n {
            for c in 0..k {
                let _ = dist.set(r, c, res.dist.get(r).and_then(|row| row.get(c)).copied().unwrap_or(0.0));
            }
        }
        Ok(vec![Value::Mat(Matrix::col_vector(idx)), Value::Mat(centers), Value::Mat(Matrix::col_vector(res.sumd)), Value::Mat(dist)])
    }

    fn kmeans_sample_start(&mut self, x: &Matrix, k: usize) -> Vec<Vec<f64>> {
        let n = x.rows();
        let d = x.cols();
        let mut picked: Vec<usize> = Vec::new();
        while picked.len() < k {
            let i = (self.next_f64() * n as f64) as usize % n.max(1);
            if !picked.contains(&i) {
                picked.push(i);
            }
            if picked.len() >= n {
                break;
            }
        }
        picked.iter().map(|&r| (0..d).map(|c| x.get(r, c).unwrap_or(0.0)).collect()).collect()
    }

    fn kmeans_plus_start(&mut self, x: &Matrix, k: usize, metric: KMetric) -> Vec<Vec<f64>> {
        let n = x.rows();
        let d = x.cols();
        let mut centers: Vec<Vec<f64>> = Vec::new();
        let first = (self.next_f64() * n as f64) as usize % n.max(1);
        centers.push((0..d).map(|c| x.get(first, c).unwrap_or(0.0)).collect());
        while centers.len() < k {
            let dists: Vec<f64> = (0..n)
                .map(|r| {
                    let p: Vec<f64> = (0..d).map(|c| x.get(r, c).unwrap_or(0.0)).collect();
                    centers.iter().map(|c| kdist(&p, c, metric)).fold(f64::INFINITY, f64::min)
                })
                .collect();
            let total: f64 = dists.iter().map(|v| v * v).sum();
            let pick = if total > 0.0 && total.is_finite() {
                let mut target = self.next_f64() * total;
                let mut chosen = 0usize;
                for (i, d) in dists.iter().enumerate() {
                    target -= d * d;
                    if target <= 0.0 {
                        chosen = i;
                        break;
                    }
                }
                chosen
            } else {
                (self.next_f64() * n as f64) as usize % n.max(1)
            };
            centers.push((0..d).map(|c| x.get(pick, c).unwrap_or(0.0)).collect());
        }
        centers
    }

    fn fitdist_call(&mut self, argv: &[Value]) -> Result<Vec<Value>, LangError> {
        let x = vector_values(argv.first().ok_or_else(|| LangError::new("fitdist needs data", 0))?, "x")?;
        let name = match argv.get(1) {
            Some(Value::Str(s)) => s.to_ascii_lowercase(),
            _ => return Err(LangError::new("fitdist needs a distribution name", 0)),
        };
        if x.len() < 2 {
            return Err(LangError::new("fitdist needs at least two observations", 0));
        }
        let n = x.len() as f64;
        let mean = x.iter().sum::<f64>() / n;
        match name.as_str() {
            "normal" | "norm" => {
                let var = x.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / n;
                let sigma = var.sqrt();
                if sigma.is_nan() || sigma <= 0.0 {
                    return Err(LangError::new("fitdist: Normal data has zero variance", 0));
                }
                Ok(vec![make_dist("prob.NormalDistribution", "Normal", &[("mu", mean), ("sigma", sigma)])?])
            }
            "exponential" | "exp" => {
                if mean.is_nan() || mean <= 0.0 || x.iter().any(|v| *v < 0.0) {
                    return Err(LangError::new("fitdist: Exponential data must be nonnegative", 0));
                }
                Ok(vec![make_dist("prob.ExponentialDistribution", "Exponential", &[("mu", mean)])?])
            }
            "gamma" | "gam" => {
                if x.iter().any(|v| *v <= 0.0) {
                    return Err(LangError::new("fitdist: Gamma data must be positive", 0));
                }
                let (a, b) = gamma_fit(&x)?;
                Ok(vec![make_dist("prob.GammaDistribution", "Gamma", &[("a", a), ("b", b)])?])
            }
            "beta" => {
                if x.iter().any(|v| !(*v > 0.0 && *v < 1.0)) {
                    return Err(LangError::new("fitdist: Beta data must lie strictly between 0 and 1", 0));
                }
                let (a, b) = beta_fit(&x)?;
                Ok(vec![make_dist("prob.BetaDistribution", "Beta", &[("a", a), ("b", b)])?])
            }
            "poisson" | "pois" => {
                if x.iter().any(|v| *v < 0.0 || v.fract() != 0.0) {
                    return Err(LangError::new("fitdist: Poisson data must be nonnegative integers", 0));
                }
                Ok(vec![make_dist("prob.PoissonDistribution", "Poisson", &[("lambda", mean)])?])
            }
            other => Err(LangError::new(format!("fitdist: unsupported distribution '{other}'"), 0)),
        }
    }

    fn prob_call(&mut self, name: &str, argv: &[Value]) -> Result<Vec<Value>, LangError> {
        let o = match argv.first() {
            Some(Value::Object(o)) if o.class_name.starts_with("prob.") => o,
            _ => return Err(LangError::new(format!("{name}: first argument must be a probability distribution object"), 0)),
        };
        let params = ProbParams::from(o)?;
        if name == "random" {
            let (rows, cols) = match (argv.get(1), argv.get(2)) {
                (None, _) => (1, 1),
                (Some(v), None) => {
                    let m = v.to_numeric()?;
                    if m.numel() == 2 {
                        (m.data().first().copied().unwrap_or(1.0).max(0.0) as usize, m.data().get(1).copied().unwrap_or(1.0).max(0.0) as usize)
                    } else {
                        let d = m.data().first().copied().unwrap_or(1.0).max(0.0) as usize;
                        (d, d)
                    }
                }
                (Some(r), Some(c)) => (r.scalar()?.max(0.0) as usize, c.scalar()?.max(0.0) as usize),
            };
            let count = rows.saturating_mul(cols);
            check_elems(count)?;
            let mut data = Vec::with_capacity(count);
            for _ in 0..count {
                data.push(params.sample(self));
            }
            return Ok(vec![mat_or_num(Matrix::from_col_major(rows, cols, data).map_err(lang)?)]);
        }
        let x = matrix_arg(argv.get(1), "x")?;
        let out = x.map(|v| match name {
            "pdf" => params.pdf(v),
            "cdf" => params.cdf(v),
            "icdf" => params.inv(v),
            _ => f64::NAN,
        });
        Ok(vec![mat_or_num(out)])
    }

    pub(super) fn prob_moment(&self, name: &str, o: &MObject) -> Result<Value, LangError> {
        let p = ProbParams::from(o)?;
        let v = match name {
            "mean" => p.mean(),
            "var" => p.variance(),
            "std" => p.variance().sqrt(),
            "median" => p.inv(0.5),
            other => return Err(LangError::new(format!("{other}: unsupported distribution moment"), 0)),
        };
        Ok(Value::Num(v))
    }

    fn pcoa_call(&mut self, argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
        let d = argv.first().ok_or_else(|| LangError::new("cmdscale needs a distance matrix", 0))?.to_numeric()?;
        let n = d.rows();
        if n < 2 || d.cols() != n {
            return Err(LangError::new("cmdscale needs a square distance matrix", 0));
        }
        let mut d2 = Matrix::zeros(n, n);
        let mut row_means = vec![0.0f64; n];
        let mut grand = 0.0f64;
        for (i, rm) in row_means.iter_mut().enumerate() {
            for j in 0..n {
                let v = d.get(i, j).unwrap_or(f64::NAN);
                if v.is_nan() || v < -1e-12 {
                    return Err(LangError::new("cmdscale distances must be nonnegative", 0));
                }
                let sq = v * v;
                let _ = d2.set(i, j, sq);
                *rm += sq;
                grand += sq;
            }
        }
        for m in &mut row_means {
            *m /= n as f64;
        }
        grand /= (n * n) as f64;
        let mut b = Matrix::zeros(n, n);
        for i in 0..n {
            for j in 0..n {
                let _ = b.set(i, j, -0.5 * (d2.get(i, j).unwrap_or(0.0) - row_means[i] - row_means[j] + grand));
            }
        }
        let (vals, vecs) = matcore::eig_symmetric(&b).map_err(lang)?;
        let mut eig: Vec<f64> = vals.into_iter().rev().collect();
        let max_abs = eig.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        let tol = 1e-10 * max_abs.max(1.0);
        let positive = eig.iter().filter(|v| **v > tol).count();
        let k = match argv.get(1) {
            Some(v) => {
                let k = v.scalar()?.max(1.0) as usize;
                if k > n {
                    return Err(LangError::new("cmdscale dimension exceeds the matrix order", 0));
                }
                k
            }
            None => positive.max(1).min(n),
        };
        let total: f64 = eig.iter().map(|v| v.max(0.0)).sum();
        let explained: Vec<f64> = eig.iter().map(|v| if total > 0.0 { v.max(0.0) / total * 100.0 } else { 0.0 }).collect();
        let mut coords = Matrix::zeros(n, k);
        for j in 0..k {
            let src = j;
            let scale = eig.get(j).copied().unwrap_or(0.0).max(0.0).sqrt();
            let mut max_row = 0usize;
            let mut max_abs_v = 0.0f64;
            for r in 0..n {
                let v = vecs.get(r, n - 1 - src).unwrap_or(0.0).abs();
                if v > max_abs_v {
                    max_abs_v = v;
                    max_row = r;
                }
            }
            let sign = if vecs.get(max_row, n - 1 - src).unwrap_or(0.0) < 0.0 { -1.0 } else { 1.0 };
            for r in 0..n {
                let _ = coords.set(r, j, sign * scale * vecs.get(r, n - 1 - src).unwrap_or(0.0));
            }
        }
        let all_eig = eig.clone();
        eig.truncate(k);
        Ok(truncate_outputs(
            vec![
                Value::Mat(coords),
                Value::Mat(Matrix::col_vector(all_eig)),
                Value::Mat(Matrix::col_vector(explained.into_iter().take(k).collect())),
            ],
            nout,
        ))
    }

    fn p_adjust_call(&mut self, name: &str, argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
        let p = argv.first().ok_or_else(|| LangError::new(format!("{name} needs p-values"), 0))?.to_numeric()?;
        if p.data().iter().any(|v| v.is_nan() || *v < 0.0 || *v > 1.0) {
            return Err(LangError::new("p-values must lie in [0,1]", 0));
        }
        let m = p.numel();
        if m == 0 {
            return Ok(vec![mat_or_num(p)]);
        }
        let values = p.data();
        let method = if name == "mafdr" { "benjamini_hochberg" } else { name };
        let mut q = vec![0.0f64; m];
        match method {
            "bonferroni" => {
                for (i, &v) in values.iter().enumerate() {
                    q[i] = (v * m as f64).min(1.0);
                }
            }
            "sidak" => {
                for (i, &v) in values.iter().enumerate() {
                    q[i] = (1.0 - (1.0 - v).powi(m as i32)).min(1.0);
                }
            }
            "holm" => {
                let mut order: Vec<usize> = (0..m).collect();
                order.sort_by(|&a, &b| values[a].total_cmp(&values[b]));
                let mut running = 0.0f64;
                for (rank, &idx) in order.iter().enumerate() {
                    let adj = (values[idx] * (m - rank) as f64).min(1.0);
                    running = running.max(adj);
                    q[idx] = running;
                }
            }
            _ => {
                let mut order: Vec<usize> = (0..m).collect();
                order.sort_by(|&a, &b| values[a].total_cmp(&values[b]));
                let mut running = 1.0f64;
                for rank in (0..m).rev() {
                    let idx = order[rank];
                    let adj = (values[idx] * m as f64 / (rank + 1) as f64).min(1.0);
                    running = running.min(adj);
                    q[idx] = running;
                }
            }
        }
        let out = Matrix::from_col_major(p.rows(), p.cols(), q).map_err(lang)?;
        let alpha = match argv.get(1) {
            Some(Value::Num(a)) => *a,
            _ => 0.05,
        };
        let h: Vec<f64> = out.data().iter().map(|v| if *v < alpha { 1.0 } else { 0.0 }).collect();
        let outs = vec![mat_or_num(out), mat_or_num(Matrix::from_col_major(p.rows(), p.cols(), h).map_err(lang)?)];
        Ok(truncate_outputs(outs, nout))
    }

    fn rank_test_call(&mut self, name: &str, argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
        match name {
            "signrank" => {
                let x = vector_values(argv.first().ok_or_else(|| LangError::new("signrank needs a sample", 0))?, "x")?;
                let mut start = 1;
                let mut target = vec![0.0; x.len()];
                if let Some(v) = argv.get(1) {
                    let m = v.to_numeric()?;
                    if m.numel() == 1 {
                        let t = m.as_scalar().unwrap_or(0.0);
                        target = vec![t; x.len()];
                        start = 2;
                    } else if m.numel() == x.len() {
                        if m.rows() > 1 && m.cols() > 1 {
                            return Err(LangError::new("signrank second sample must be a vector", 0));
                        }
                        target = m.data().to_vec();
                        start = 2;
                    } else {
                        return Err(LangError::new("signrank needs a scalar median or a matching sample", 0));
                    }
                }
                let o = test_options(argv, start)?;
                let diffs: Vec<f64> = x.iter().zip(target.iter()).map(|(a, b)| a - b).filter(|d| d.abs() > 1e-15).collect();
                let n = diffs.len();
                if n == 0 {
                    return Err(LangError::new("signrank: all differences are zero", 0));
                }
                let abs: Vec<f64> = diffs.iter().map(|d| d.abs()).collect();
                let ranks = rank_values(&abs);
                let w_plus: f64 = diffs.iter().zip(ranks.iter()).filter(|(d, _)| **d > 0.0).map(|(_, r)| *r).sum();
                let has_ties = tie_groups(&abs) > 0;
                let p = if !has_ties && n <= 50 {
                    signed_rank_exact_p(w_plus, n, o.tail)
                } else {
                    let mean = (n * (n + 1)) as f64 / 4.0;
                    let var = (n * (n + 1) * (2 * n + 1)) as f64 / 24.0 - tie_sum(&abs) / 48.0;
                    let sd = var.max(1e-12).sqrt();
                    let z = (w_plus - mean - 0.5 * (w_plus - mean).signum()) / sd;
                    tail_normal_p(z, o.tail)
                };
                let (h, p) = decision(p, o.alpha);
                let mut st = StructArray::new();
                st.set_field(0, "signedrank", Value::Num(w_plus))?;
                Ok(truncate_outputs(vec![Value::Num(p), Value::Num(h), Value::Struct(st)], nout))
            }
            "ranksum" => {
                let x = vector_values(argv.first().ok_or_else(|| LangError::new("ranksum needs x", 0))?, "x")?;
                let y = vector_values(argv.get(1).ok_or_else(|| LangError::new("ranksum needs y", 0))?, "y")?;
                let o = test_options(argv, 2)?;
                let n1 = x.len();
                let n2 = y.len();
                if n1 == 0 || n2 == 0 {
                    return Err(LangError::new("ranksum needs two nonempty samples", 0));
                }
                let mut combined = x.clone();
                combined.extend_from_slice(&y);
                let ranks = rank_values(&combined);
                let w: f64 = ranks.iter().take(n1).sum();
                let u = w - (n1 * (n1 + 1)) as f64 / 2.0;
                let n = n1 + n2;
                let mean = (n1 * n2) as f64 / 2.0;
                let tie = tie_sum(&combined);
                let var = (n1 * n2) as f64 / 12.0 * ((n + 1) as f64 - tie / (n * (n - 1)) as f64);
                let sd = var.max(1e-12).sqrt();
                let z = (u - mean - 0.5 * (u - mean).signum()) / sd;
                let p = tail_normal_p(z, o.tail);
                let (h, p) = decision(p, o.alpha);
                let mut st = StructArray::new();
                st.set_field(0, "ranksum", Value::Num(w))?;
                Ok(truncate_outputs(vec![Value::Num(p), Value::Num(h), Value::Struct(st)], nout))
            }
            _ => {
                let groups = kruskal_groups(argv)?;
                let k = groups.len();
                let n: usize = groups.iter().map(Vec::len).sum();
                if k < 2 || n < 3 {
                    return Err(LangError::new("kruskalwallis needs at least two groups", 0));
                }
                let mut combined = Vec::with_capacity(n);
                for g in &groups {
                    combined.extend_from_slice(g);
                }
                let ranks = rank_values(&combined);
                let mut offset = 0usize;
                let mut meanranks = Vec::with_capacity(k);
                let mut h = 0.0f64;
                for g in &groups {
                    let ng = g.len();
                    let mr = ranks.iter().skip(offset).take(ng).sum::<f64>() / ng as f64;
                    meanranks.push(mr);
                    h += ng as f64 * (mr - (n + 1) as f64 / 2.0).powi(2);
                    offset += ng;
                }
                h *= 12.0 / (n * (n + 1)) as f64;
                let tie = tie_sum(&combined);
                if tie > 0.0 {
                    h /= 1.0 - tie / ((n * n * n - n) as f64);
                }
                let df = (k - 1) as f64;
                let p = (1.0 - stats::chi2_cdf(h, df)).clamp(0.0, 1.0);
                let mut st = StructArray::new();
                st.set_field(0, "chi2", Value::Num(h))?;
                st.set_field(0, "df", Value::Num(df))?;
                st.set_field(0, "meanranks", Value::Mat(Matrix::row_vector(meanranks)))?;
                st.set_field(0, "n", Value::Mat(Matrix::row_vector(groups.iter().map(|g| g.len() as f64).collect())))?;
                Ok(truncate_outputs(vec![Value::Num(p), Value::Struct(st)], nout))
            }
        }
    }

    fn ttest_call(&mut self, argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
        let xv = argv.first().ok_or_else(|| LangError::new("ttest needs a sample", 0))?;
        let x = vector_values(xv, "x")?;
        let mut start = 1;
        let mut mu = 0.0;
        if let Some(v) = argv.get(1) {
            if let Ok(m) = v.to_matrix() {
                if let Some(s) = m.as_scalar() {
                    mu = s;
                    start = 2;
                }
            }
        }
        let o = test_options(argv, start)?;
        let (mean, sd, n) = sample_stats(&x)?;
        let df = (n - 1) as f64;
        let se = sd / (n as f64).sqrt();
        if se.is_nan() || se <= 0.0 {
            return Err(LangError::new("ttest: the sample standard deviation is zero", 0));
        }
        let t = (mean - mu) / se;
        let p = tail_pvalue(t, df, o.tail);
        let tc = match o.tail {
            Tail::Both => stats::t_inv(1.0 - o.alpha / 2.0, df),
            _ => stats::t_inv(1.0 - o.alpha, df),
        };
        let (lo, hi) = match o.tail {
            Tail::Both => (mean - tc * se, mean + tc * se),
            Tail::Right => (mean - tc * se, f64::INFINITY),
            Tail::Left => (f64::NEG_INFINITY, mean + tc * se),
        };
        let (h, p) = decision(p, o.alpha);
        let mut st = StructArray::new();
        st.set_field(0, "tstat", Value::Num(t))?;
        st.set_field(0, "df", Value::Num(df))?;
        st.set_field(0, "sd", Value::Num(sd))?;
        Ok(truncate_outputs(vec![Value::Num(h), Value::Num(p), Value::Mat(Matrix::row_vector(vec![lo, hi])), Value::Struct(st)], nout))
    }

    fn ttest2_call(&mut self, argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
        let x = vector_values(argv.first().ok_or_else(|| LangError::new("ttest2 needs x", 0))?, "x")?;
        let y = vector_values(argv.get(1).ok_or_else(|| LangError::new("ttest2 needs y", 0))?, "y")?;
        let o = test_options(argv, 2)?;
        let (mx, sx, nx) = sample_stats(&x)?;
        let (my, sy, ny) = sample_stats(&y)?;
        if nx < 2 || ny < 2 {
            return Err(LangError::new("ttest2: each sample needs at least two values", 0));
        }
        let vx = sx * sx / nx as f64;
        let vy = sy * sy / ny as f64;
        let (t, df, sd) = if o.unequal {
            let se = (vx + vy).sqrt();
            if se.is_nan() || se <= 0.0 {
                return Err(LangError::new("ttest2: the standard error is zero", 0));
            }
            let df = (vx + vy) * (vx + vy) / ((vx * vx) / (nx - 1) as f64 + (vy * vy) / (ny - 1) as f64);
            ((mx - my) / se, df, se)
        } else {
            let pooled = (((nx - 1) as f64 * sx * sx + (ny - 1) as f64 * sy * sy) / (nx + ny - 2) as f64).sqrt();
            let se = pooled * (1.0 / nx as f64 + 1.0 / ny as f64).sqrt();
            if se.is_nan() || se <= 0.0 {
                return Err(LangError::new("ttest2: the pooled standard error is zero", 0));
            }
            ((mx - my) / se, (nx + ny - 2) as f64, pooled)
        };
        let diff = mx - my;
        let se = if o.unequal { sd } else { sd * (1.0 / nx as f64 + 1.0 / ny as f64).sqrt() };
        let p = tail_pvalue(t, df, o.tail);
        let tc = match o.tail {
            Tail::Both => stats::t_inv(1.0 - o.alpha / 2.0, df),
            _ => stats::t_inv(1.0 - o.alpha, df),
        };
        let (lo, hi) = match o.tail {
            Tail::Both => (diff - tc * se, diff + tc * se),
            Tail::Right => (diff - tc * se, f64::INFINITY),
            Tail::Left => (f64::NEG_INFINITY, diff + tc * se),
        };
        let (h, p) = decision(p, o.alpha);
        let mut st = StructArray::new();
        st.set_field(0, "tstat", Value::Num(t))?;
        st.set_field(0, "df", Value::Num(df))?;
        st.set_field(0, "sd", Value::Num(sd))?;
        Ok(truncate_outputs(vec![Value::Num(h), Value::Num(p), Value::Mat(Matrix::row_vector(vec![lo, hi])), Value::Struct(st)], nout))
    }

    fn kstest_call(&mut self, argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
        let x = vector_values(argv.first().ok_or_else(|| LangError::new("kstest needs a sample", 0))?, "x")?;
        if x.len() < 2 {
            return Err(LangError::new("kstest: at least two observations are required", 0));
        }
        let mut cdf = None;
        let mut start = 1;
        if let Some(v) = argv.get(1) {
            match v {
                Value::Func(_) => {
                    cdf = Some(v.clone());
                    start = 2;
                }
                Value::Str(s) if s.eq_ignore_ascii_case("norm") => {
                    start = 2;
                }
                Value::Mat(m) if m.cols() == 2 => {
                    cdf = Some(v.clone());
                    start = 2;
                }
                _ => {}
            }
        }
        let o = test_options(argv, start)?;
        let mut sorted = x;
        sorted.sort_by(|a, b| a.total_cmp(b));
        let n = sorted.len();
        let f = self.evaluate_cdf(cdf.as_ref(), &sorted)?;
        let mut d = 0.0f64;
        for (i, (&xi, &fi)) in sorted.iter().zip(f.iter()).enumerate() {
            let _ = xi;
            let lo = i as f64 / n as f64;
            let hi = (i + 1) as f64 / n as f64;
            let fi = fi.clamp(0.0, 1.0);
            d = d.max((fi - lo).abs()).max((hi - fi).abs());
        }
        let lambda = (n as f64).sqrt() * d;
        let p = kolmogorov_q(lambda).clamp(0.0, 1.0);
        let cv = kolmogorov_critical(o.alpha, n);
        let (h, p) = decision(p, o.alpha);
        Ok(truncate_outputs(vec![Value::Num(h), Value::Num(p), Value::Num(d), Value::Num(cv)], nout))
    }

    fn adtest_call(&mut self, argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
        let x = vector_values(argv.first().ok_or_else(|| LangError::new("adtest needs a sample", 0))?, "x")?;
        if x.len() < 8 {
            return Err(LangError::new("adtest: at least eight observations are required", 0));
        }
        let o = test_options(argv, 1)?;
        let (mean, sd, n) = sample_stats(&x)?;
        if sd.is_nan() || sd <= 0.0 {
            return Err(LangError::new("adtest: the sample standard deviation is zero", 0));
        }
        let mut z: Vec<f64> = x.iter().map(|v| (v - mean) / sd).collect();
        z.sort_by(|a, b| a.total_cmp(b));
        let mut sum = 0.0f64;
        for (i, &zi) in z.iter().enumerate() {
            let fi = stats::norm_cdf(zi, 0.0, 1.0).clamp(1e-15, 1.0 - 1e-15);
            let fn_ = stats::norm_cdf(z[n - 1 - i], 0.0, 1.0).clamp(1e-15, 1.0 - 1e-15);
            sum += (2 * i + 1) as f64 * (fi.ln() + (1.0 - fn_).ln());
        }
        let ad = -(n as f64) - sum / n as f64;
        let adj = ad * (1.0 + 0.75 / n as f64 + 2.25 / (n * n) as f64);
        let p = anderson_darling_p(adj).clamp(0.0, 1.0);
        let cv = anderson_darling_critical(o.alpha);
        let (h, p) = decision(p, o.alpha);
        Ok(truncate_outputs(vec![Value::Num(h), Value::Num(p), Value::Num(ad), Value::Num(cv)], nout))
    }

    fn pca_call(&mut self, argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
        let raw = argv.first().ok_or_else(|| LangError::new("pca needs a data matrix", 0))?.to_numeric()?;
        if raw.rows() < 2 || raw.cols() < 1 {
            return Err(LangError::new("pca needs at least two observations", 0));
        }
        let mut num_components = None;
        let mut i = 1;
        while i < argv.len() {
            if let Value::Str(s) = &argv[i] {
                let key = s.to_ascii_lowercase();
                if key == "numcomponents" {
                    let v = argv.get(i + 1).ok_or_else(|| LangError::new("pca: NumComponents needs a value", 0))?;
                    num_components = Some(v.scalar()?.max(1.0) as usize);
                    i += 2;
                    continue;
                }
                return Err(LangError::new(format!("pca: unknown option '{s}'"), 0));
            }
            return Err(LangError::new("pca: options must be name-value pairs", 0));
        }
        let p = raw.cols();
        let mut rows: Vec<usize> = (0..raw.rows()).filter(|&r| (0..p).all(|c| raw.get(r, c).map(|v| v.is_finite()).unwrap_or(false))).collect();
        if rows.len() < 2 {
            return Err(LangError::new("pca: not enough complete rows", 0));
        }
        rows.sort_unstable();
        let n = rows.len();
        let mut mu = vec![0.0f64; p];
        for &r in &rows {
            for (c, m) in mu.iter_mut().enumerate() {
                *m += raw.get(r, c).unwrap_or(0.0);
            }
        }
        for m in &mut mu {
            *m /= n as f64;
        }
        let mut centered = Matrix::zeros(n, p);
        for (ri, &r) in rows.iter().enumerate() {
            for c in 0..p {
                let _ = centered.set(ri, c, raw.get(r, c).unwrap_or(0.0) - mu.get(c).copied().unwrap_or(0.0));
            }
        }
        let cov = centered.transpose().matmul(&centered).map_err(lang)?;
        let cov = cov.map(|v| v / (n - 1) as f64);
        let (vals, vecs) = matcore::eig_symmetric(&cov).map_err(lang)?;
        let k = num_components.unwrap_or(p).min(p);
        let mut latent = Vec::with_capacity(k);
        let mut coeff = Matrix::zeros(p, k);
        let total: f64 = vals.iter().map(|v| v.max(0.0)).sum();
        for j in 0..k {
            let src = p.saturating_sub(1).saturating_sub(j);
            let l = vals.get(src).copied().unwrap_or(0.0).max(0.0);
            latent.push(l);
            let mut max_row = 0usize;
            let mut max_abs = 0.0f64;
            for r in 0..p {
                let v = vecs.get(r, src).unwrap_or(0.0).abs();
                if v > max_abs {
                    max_abs = v;
                    max_row = r;
                }
            }
            let sign = if vecs.get(max_row, src).unwrap_or(0.0) < 0.0 { -1.0 } else { 1.0 };
            for r in 0..p {
                let _ = coeff.set(r, j, sign * vecs.get(r, src).unwrap_or(0.0));
            }
        }
        let score = centered.matmul(&coeff).map_err(lang)?;
        let mut tsq = Vec::with_capacity(n);
        for r in 0..n {
            let mut s = 0.0;
            for j in 0..k {
                let l = latent.get(j).copied().unwrap_or(0.0);
                let v = score.get(r, j).unwrap_or(0.0);
                if l > 1e-12 {
                    s += v * v / l;
                }
            }
            tsq.push(s);
        }
        let explained: Vec<f64> = latent.iter().map(|v| if total > 0.0 { v / total * 100.0 } else { 0.0 }).collect();
        Ok(truncate_outputs(
            vec![
                Value::Mat(coeff),
                Value::Mat(score),
                Value::Mat(Matrix::col_vector(latent)),
                Value::Mat(Matrix::col_vector(tsq)),
                Value::Mat(Matrix::col_vector(explained)),
                Value::Mat(Matrix::row_vector(mu)),
            ],
            nout,
        ))
    }

    fn evaluate_cdf(&mut self, cdf: Option<&Value>, sorted: &[f64]) -> Result<Vec<f64>, LangError> {
        match cdf {
            None => Ok(sorted.iter().map(|&x| stats::norm_cdf(x, 0.0, 1.0)).collect()),
            Some(Value::Str(s)) if s.eq_ignore_ascii_case("norm") => Ok(sorted.iter().map(|&x| stats::norm_cdf(x, 0.0, 1.0)).collect()),
            Some(v @ Value::Func(_)) => {
                let arg = Matrix::col_vector(sorted.to_vec());
                let outs = self.call_feval_target(v, vec![Value::Mat(arg)], 1)?;
                let m = outs.first().ok_or_else(|| LangError::new("kstest: the CDF returned no value", 0))?.to_numeric()?;
                if m.numel() == sorted.len() {
                    Ok(m.data().to_vec())
                } else if let Some(s) = m.as_scalar() {
                    Ok(vec![s; sorted.len()])
                } else {
                    Err(LangError::new("kstest: the CDF must return one value per observation", 0))
                }
            }
            Some(Value::Mat(m)) if m.cols() == 2 => {
                let mut xs: Vec<(f64, f64)> = (0..m.rows()).map(|r| (m.get(r, 0).unwrap_or(0.0), m.get(r, 1).unwrap_or(0.0))).collect();
                xs.sort_by(|a, b| a.0.total_cmp(&b.0));
                Ok(sorted
                    .iter()
                    .map(|&x| {
                        let mut f = 0.0;
                        for &(xv, fv) in &xs {
                            if xv <= x {
                                f = fv;
                            } else {
                                break;
                            }
                        }
                        f
                    })
                    .collect())
            }
            _ => Err(LangError::new("kstest: the CDF must be a function handle, 'norm', or an m-by-2 matrix", 0)),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Tail {
    Both,
    Right,
    Left,
}

struct TestOptions {
    alpha: f64,
    tail: Tail,
    unequal: bool,
}

fn test_options(argv: &[Value], start: usize) -> Result<TestOptions, LangError> {
    let mut o = TestOptions { alpha: 0.05, tail: Tail::Both, unequal: true };
    let mut i = start;
    while i < argv.len() {
        match &argv[i] {
            Value::Str(s) => {
                let key = s.to_ascii_lowercase();
                let tail = match key.as_str() {
                    "both" => Some(Tail::Both),
                    "right" => Some(Tail::Right),
                    "left" => Some(Tail::Left),
                    "equal" => {
                        o.unequal = false;
                        None
                    }
                    "unequal" => {
                        o.unequal = true;
                        None
                    }
                    _ => None,
                };
                if let Some(t) = tail {
                    o.tail = t;
                    i += 1;
                    continue;
                }
                if matches!(key.as_str(), "alpha" | "tail" | "vartype") {
                    let v = argv.get(i + 1).ok_or_else(|| LangError::new(format!("{key} needs a value"), 0))?;
                    match key.as_str() {
                        "alpha" => o.alpha = v.scalar()?,
                        "tail" => {
                            o.tail = match &v {
                                Value::Str(t) if t.eq_ignore_ascii_case("right") => Tail::Right,
                                Value::Str(t) if t.eq_ignore_ascii_case("left") => Tail::Left,
                                Value::Str(t) if t.eq_ignore_ascii_case("both") => Tail::Both,
                                _ => return Err(LangError::new("tail must be 'both', 'right' or 'left'", 0)),
                            }
                        }
                        _ => {
                            o.unequal = match &v {
                                Value::Str(t) if t.eq_ignore_ascii_case("equal") => false,
                                Value::Str(t) if t.eq_ignore_ascii_case("unequal") => true,
                                _ => return Err(LangError::new("Vartype must be 'equal' or 'unequal'", 0)),
                            }
                        }
                    }
                    i += 2;
                    continue;
                }
                return Err(LangError::new(format!("unknown option '{s}'"), 0));
            }
            v => {
                if let Ok(m) = v.to_matrix() {
                    if let Some(a) = m.as_scalar() {
                        o.alpha = a;
                        i += 1;
                        continue;
                    }
                }
                return Err(LangError::new("options must be scalars or name-value pairs", 0));
            }
        }
    }
    if !(o.alpha > 0.0 && o.alpha < 1.0) {
        return Err(LangError::new("alpha must be between 0 and 1", 0));
    }
    Ok(o)
}

fn vector_values(v: &Value, what: &str) -> Result<Vec<f64>, LangError> {
    let m = v.to_numeric()?;
    if m.rows() > 1 && m.cols() > 1 {
        return Err(LangError::new(format!("{what} must be a vector"), 0));
    }
    Ok(m.data().to_vec())
}

fn sample_stats(x: &[f64]) -> Result<(f64, f64, usize), LangError> {
    let n = x.len();
    if n < 2 {
        return Err(LangError::new("at least two observations are required", 0));
    }
    let mean = x.iter().sum::<f64>() / n as f64;
    let var = x.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / (n - 1) as f64;
    Ok((mean, var.sqrt(), n))
}

fn tail_pvalue(t: f64, df: f64, tail: Tail) -> f64 {
    let p = match tail {
        Tail::Both => 2.0 * (1.0 - stats::t_cdf(t.abs(), df)),
        Tail::Right => 1.0 - stats::t_cdf(t, df),
        Tail::Left => stats::t_cdf(t, df),
    };
    if p.is_finite() {
        p.clamp(0.0, 1.0)
    } else {
        f64::NAN
    }
}

fn decision(p: f64, alpha: f64) -> (f64, f64) {
    (if p < alpha { 1.0 } else { 0.0 }, p)
}

fn truncate_outputs(mut out: Vec<Value>, nout: usize) -> Vec<Value> {
    out.truncate(nout.max(1).min(out.len()));
    out
}

fn kolmogorov_q(lambda: f64) -> f64 {
    if lambda.is_nan() || lambda <= 0.0 {
        return 1.0;
    }
    if lambda < 0.5 {
        let mut s = 0.0f64;
        for k in 1..=100 {
            let a = (2 * k - 1) as f64;
            s += (-(a * a * std::f64::consts::PI * std::f64::consts::PI) / (8.0 * lambda * lambda)).exp();
        }
        1.0 - (2.0 * std::f64::consts::PI).sqrt() / lambda * s
    } else {
        let mut s = 0.0f64;
        for k in 1..=100 {
            let sign = if k % 2 == 1 { 1.0 } else { -1.0 };
            s += sign * (-2.0 * (k * k) as f64 * lambda * lambda).exp();
        }
        2.0 * s
    }
    .clamp(0.0, 1.0)
}

fn kolmogorov_critical(alpha: f64, n: usize) -> f64 {
    let mut lo = 0.0f64;
    let mut hi = 2.0f64;
    for _ in 0..100 {
        let mid = 0.5 * (lo + hi);
        if kolmogorov_q((n as f64).sqrt() * mid) > alpha {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

fn anderson_darling_p(ad: f64) -> f64 {
    if ad < 0.2 {
        1.0 - (-13.436 + 101.14 * ad - 223.73 * ad * ad).exp()
    } else if ad < 0.34 {
        1.0 - (-8.318 + 42.796 * ad - 59.938 * ad * ad).exp()
    } else if ad < 0.6 {
        (0.9177 - 4.279 * ad - 1.38 * ad * ad).exp()
    } else {
        (1.2937 - 5.709 * ad + 0.0186 * ad * ad).exp()
    }
}

fn anderson_darling_critical(alpha: f64) -> f64 {
    const ALPHAS: [f64; 5] = [0.15, 0.10, 0.05, 0.025, 0.01];
    const CVS: [f64; 5] = [0.561, 0.631, 0.752, 0.873, 1.035];
    if alpha >= ALPHAS[0] {
        return CVS[0];
    }
    for i in 1..ALPHAS.len() {
        if alpha >= ALPHAS[i] {
            let a0 = ALPHAS[i - 1];
            let a1 = ALPHAS[i];
            let c0 = CVS[i - 1];
            let c1 = CVS[i];
            return c0 + (alpha - a0) / (a1 - a0) * (c1 - c0);
        }
    }
    CVS[CVS.len() - 1]
}

fn rank_values(values: &[f64]) -> Vec<f64> {
    let n = values.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| values[a].total_cmp(&values[b]));
    let mut ranks = vec![0.0f64; n];
    let mut i = 0usize;
    while i < n {
        let mut j = i;
        while j + 1 < n && values[order[j + 1]] == values[order[i]] {
            j += 1;
        }
        let rank = (i + j + 2) as f64 / 2.0;
        for k in i..=j {
            ranks[order[k]] = rank;
        }
        i = j + 1;
    }
    ranks
}

fn tie_groups(values: &[f64]) -> usize {
    let mut sorted: Vec<f64> = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let mut count = 0usize;
    let mut i = 0usize;
    while i < sorted.len() {
        let mut j = i;
        while j + 1 < sorted.len() && sorted[j + 1] == sorted[i] {
            j += 1;
        }
        if j > i {
            count += 1;
        }
        i = j + 1;
    }
    count
}

fn tie_sum(values: &[f64]) -> f64 {
    let mut sorted: Vec<f64> = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let mut sum = 0.0f64;
    let mut i = 0usize;
    while i < sorted.len() {
        let mut j = i;
        while j + 1 < sorted.len() && sorted[j + 1] == sorted[i] {
            j += 1;
        }
        let t = (j - i + 1) as f64;
        if t > 1.0 {
            sum += t * t * t - t;
        }
        i = j + 1;
    }
    sum
}

fn tail_normal_p(z: f64, tail: Tail) -> f64 {
    let p = match tail {
        Tail::Both => 2.0 * (1.0 - stats::norm_cdf(z.abs(), 0.0, 1.0)),
        Tail::Right => 1.0 - stats::norm_cdf(z, 0.0, 1.0),
        Tail::Left => stats::norm_cdf(z, 0.0, 1.0),
    };
    p.clamp(0.0, 1.0)
}

fn signed_rank_exact_p(w: f64, n: usize, tail: Tail) -> f64 {
    let max = n * (n + 1) / 2;
    let mut dp = vec![0u64; max + 1];
    dp[0] = 1;
    for r in 1..=n {
        for s in (r..=max).rev() {
            dp[s] = dp[s].saturating_add(dp[s - r]);
        }
    }
    let total = 1u64.checked_shl(n as u32).unwrap_or(u64::MAX);
    let w = w.round().clamp(0.0, max as f64) as usize;
    let left: u64 = dp.iter().take(w + 1).sum();
    let right: u64 = dp.iter().skip(w).sum();
    let p = match tail {
        Tail::Both => 2.0 * left.min(right) as f64 / total as f64,
        Tail::Right => right as f64 / total as f64,
        Tail::Left => left as f64 / total as f64,
    };
    p.clamp(0.0, 1.0)
}

fn kruskal_groups(argv: &[Value]) -> Result<Vec<Vec<f64>>, LangError> {
    match argv.first() {
        Some(Value::Cell(c)) => c.items.iter().map(|v| vector_values(v, "group")).collect(),
        Some(v) => {
            let m = v.to_numeric()?;
            if let Some(g) = argv.get(1) {
                let gv = g.to_numeric()?;
                if gv.numel() != m.numel() {
                    return Err(LangError::new("kruskalwallis group labels must match the data length", 0));
                }
                let mut labels: Vec<f64> = Vec::new();
                for &x in gv.data() {
                    if !labels.contains(&x) {
                        labels.push(x);
                    }
                }
                let mut groups = vec![Vec::new(); labels.len()];
                for (i, &label) in gv.data().iter().enumerate() {
                    if let Some(pos) = labels.iter().position(|x| *x == label) {
                        if let Some(slot) = groups.get_mut(pos) {
                            slot.push(m.data().get(i).copied().unwrap_or(f64::NAN));
                        }
                    }
                }
                Ok(groups)
            } else if m.cols() > 1 {
                Ok((0..m.cols()).map(|c| (0..m.rows()).map(|r| m.get(r, c).unwrap_or(f64::NAN)).collect()).collect())
            } else {
                Err(LangError::new("kruskalwallis needs a data matrix or a group vector", 0))
            }
        }
        None => Err(LangError::new("kruskalwallis needs data", 0)),
    }
}

fn make_dist(class: &str, name: &str, fields: &[(&str, f64)]) -> Result<Value, LangError> {
    let mut s = StructArray::new();
    s.set_field(0, "DistributionName", Value::Str(name.to_string()))?;
    for (field, v) in fields {
        s.set_field(0, field, Value::Num(*v))?;
    }
    Ok(Value::Object(MObject::owned(class.to_string(), s)))
}

enum ProbParams {
    Normal { mu: f64, sigma: f64 },
    Exponential { mu: f64 },
    Gamma { a: f64, b: f64 },
    Beta { a: f64, b: f64 },
    Poisson { lambda: f64 },
}

impl ProbParams {
    fn from(o: &MObject) -> Result<Self, LangError> {
        let num = |field: &str| -> Result<f64, LangError> {
            o.prop(field).ok_or_else(|| LangError::new(format!("distribution object has no '{field}' property"), 0))?.scalar()
        };
        match o.class_name.as_str() {
            "prob.NormalDistribution" => {
                let (mu, sigma) = (num("mu")?, num("sigma")?);
                if sigma.is_nan() || sigma <= 0.0 {
                    return Err(LangError::new("Normal sigma must be positive", 0));
                }
                Ok(ProbParams::Normal { mu, sigma })
            }
            "prob.ExponentialDistribution" => {
                let mu = num("mu")?;
                if mu.is_nan() || mu <= 0.0 {
                    return Err(LangError::new("Exponential mu must be positive", 0));
                }
                Ok(ProbParams::Exponential { mu })
            }
            "prob.GammaDistribution" => Ok(ProbParams::Gamma { a: num("a")?, b: num("b")? }),
            "prob.BetaDistribution" => Ok(ProbParams::Beta { a: num("a")?, b: num("b")? }),
            "prob.PoissonDistribution" => Ok(ProbParams::Poisson { lambda: num("lambda")? }),
            other => Err(LangError::new(format!("unsupported distribution class '{other}'"), 0)),
        }
    }

    fn pdf(&self, x: f64) -> f64 {
        match *self {
            ProbParams::Normal { mu, sigma } => stats::norm_pdf(x, mu, sigma),
            ProbParams::Exponential { mu } => stats::exp_pdf(x, mu),
            ProbParams::Gamma { a, b } => stats::gamma_pdf(x, a, b),
            ProbParams::Beta { a, b } => stats::beta_pdf(x, a, b),
            ProbParams::Poisson { lambda } => stats::poisson_pdf(x, lambda),
        }
    }

    fn cdf(&self, x: f64) -> f64 {
        match *self {
            ProbParams::Normal { mu, sigma } => stats::norm_cdf(x, mu, sigma),
            ProbParams::Exponential { mu } => stats::exp_cdf(x, mu),
            ProbParams::Gamma { a, b } => stats::gamma_cdf(x, a, b),
            ProbParams::Beta { a, b } => stats::beta_cdf(x, a, b),
            ProbParams::Poisson { lambda } => stats::poisson_cdf(x, lambda),
        }
    }

    fn inv(&self, p: f64) -> f64 {
        match *self {
            ProbParams::Normal { mu, sigma } => stats::norm_inv(p, mu, sigma),
            ProbParams::Exponential { mu } => stats::exp_inv(p, mu),
            ProbParams::Gamma { a, b } => stats::gamma_inv(p, a, b),
            ProbParams::Beta { a, b } => stats::beta_inv(p, a, b),
            ProbParams::Poisson { lambda } => stats::poisson_inv(p, lambda),
        }
    }

    fn mean(&self) -> f64 {
        match *self {
            ProbParams::Normal { mu, .. } => mu,
            ProbParams::Exponential { mu } => mu,
            ProbParams::Gamma { a, b } => a * b,
            ProbParams::Beta { a, b } => a / (a + b),
            ProbParams::Poisson { lambda } => lambda,
        }
    }

    fn variance(&self) -> f64 {
        match *self {
            ProbParams::Normal { sigma, .. } => sigma * sigma,
            ProbParams::Exponential { mu } => mu * mu,
            ProbParams::Gamma { a, b } => a * b * b,
            ProbParams::Beta { a, b } => a * b / ((a + b) * (a + b) * (a + b + 1.0)),
            ProbParams::Poisson { lambda } => lambda,
        }
    }

    fn sample(&self, s: &mut Session) -> f64 {
        match *self {
            ProbParams::Normal { mu, sigma } => mu + sigma * normal_sample(s),
            ProbParams::Exponential { mu } => -mu * (1.0 - s.next_f64()).max(1e-300).ln(),
            ProbParams::Gamma { a, b } => gamma_sample(s, a, b),
            ProbParams::Beta { a, b } => {
                let x = gamma_sample(s, a, 1.0);
                let y = gamma_sample(s, b, 1.0);
                if x + y > 0.0 {
                    x / (x + y)
                } else {
                    0.5
                }
            }
            ProbParams::Poisson { lambda } => poisson_sample(s, lambda),
        }
    }
}

fn gamma_sample(s: &mut Session, a: f64, b: f64) -> f64 {
    if a.is_nan() || a <= 0.0 || b.is_nan() || b <= 0.0 {
        return f64::NAN;
    }
    if a < 1.0 {
        let u = s.next_f64().max(1e-300);
        return gamma_sample(s, a + 1.0, b) * u.powf(1.0 / a);
    }
    let d = a - 1.0 / 3.0;
    let c = 1.0 / (9.0 * d).sqrt();
    for _ in 0..1000 {
        let x = normal_sample(s);
        let v = 1.0 + c * x;
        if v <= 0.0 {
            continue;
        }
        let v3 = v * v * v;
        let u = s.next_f64();
        if u < 1.0 - 0.0331 * x * x * x * x || u.ln() < 0.5 * x * x + d * (1.0 - v3 + v3.ln()) {
            return d * v3 * b;
        }
    }
    a * b
}

fn gamma_fit(x: &[f64]) -> Result<(f64, f64), LangError> {
    let n = x.len() as f64;
    let mean = x.iter().sum::<f64>() / n;
    if mean.is_nan() || mean <= 0.0 {
        return Err(LangError::new("fitdist: Gamma data must have a positive mean", 0));
    }
    let mean_ln = x.iter().map(|v| v.ln()).sum::<f64>() / n;
    let s = mean.ln() - mean_ln;
    let mut a = if s < 1e-12 { 1e6 } else { (3.0 - s + ((s - 3.0) * (s - 3.0) + 24.0 * s).sqrt()) / (12.0 * s) };
    if a.is_nan() || a <= 0.0 {
        a = 1.0;
    }
    for _ in 0..200 {
        let f = a.ln() - stats::digamma(a) - s;
        let fp = 1.0 / a - stats::trigamma(a);
        if fp.abs() < 1e-15 {
            break;
        }
        let next = a - f / fp;
        if next.is_nan() || next <= 0.0 {
            a *= 0.5;
        } else if (next - a).abs() < 1e-12 * a {
            a = next;
            break;
        } else {
            a = next;
        }
    }
    Ok((a, mean / a))
}

fn beta_fit(x: &[f64]) -> Result<(f64, f64), LangError> {
    let n = x.len() as f64;
    let mean = x.iter().sum::<f64>() / n;
    let var = x.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / n;
    if var.is_nan() || var <= 0.0 || mean.is_nan() || mean <= 0.0 || mean >= 1.0 {
        return Err(LangError::new("fitdist: Beta data needs nonzero variance inside (0,1)", 0));
    }
    let common = mean * (1.0 - mean) / var - 1.0;
    let mut a = mean * common;
    let mut b = (1.0 - mean) * common;
    if a.is_nan() || a <= 0.0 || b.is_nan() || b <= 0.0 {
        a = 1.0;
        b = 1.0;
    }
    let sum_ln = x.iter().map(|v| v.ln()).sum::<f64>();
    let sum_ln1 = x.iter().map(|v| (1.0 - v).ln()).sum::<f64>();
    for _ in 0..200 {
        let ga = n * (stats::digamma(a + b) - stats::digamma(a)) + sum_ln;
        let gb = n * (stats::digamma(a + b) - stats::digamma(b)) + sum_ln1;
        let haa = n * (stats::trigamma(a + b) - stats::trigamma(a));
        let hbb = n * (stats::trigamma(a + b) - stats::trigamma(b));
        let hab = n * stats::trigamma(a + b);
        let det = haa * hbb - hab * hab;
        if det.abs() < 1e-18 {
            break;
        }
        let da = (hbb * ga - hab * gb) / det;
        let db = (haa * gb - hab * ga) / det;
        if da.abs().max(db.abs()) < 1e-12 {
            break;
        }
        let mut scale = 1.0;
        let (mut na, mut nb) = (a - da, b - db);
        while (na <= 1e-8 || nb <= 1e-8) && scale > 1e-8 {
            scale *= 0.5;
            na = a - da * scale;
            nb = b - db * scale;
        }
        if na <= 1e-8 || nb <= 1e-8 {
            break;
        }
        a = na;
        b = nb;
    }
    Ok((a, b))
}

#[derive(Clone, Copy)]
enum KMetric {
    SqEuclidean,
    Euclidean,
    Cityblock,
    Cosine,
}

enum KStart {
    Plus,
    Sample,
    Matrix(Matrix),
}

struct KResult {
    idx: Vec<usize>,
    centers: Vec<Vec<f64>>,
    sumd: Vec<f64>,
    dist: Vec<Vec<f64>>,
    total: f64,
}

fn kdist(a: &[f64], b: &[f64], metric: KMetric) -> f64 {
    match metric {
        KMetric::SqEuclidean => a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum(),
        KMetric::Euclidean => a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f64>().sqrt(),
        KMetric::Cityblock => a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum(),
        KMetric::Cosine => {
            let dot = a.iter().zip(b).map(|(x, y)| x * y).sum::<f64>();
            let na = a.iter().map(|x| x * x).sum::<f64>().sqrt();
            let nb = b.iter().map(|x| x * x).sum::<f64>().sqrt();
            if na > 0.0 && nb > 0.0 {
                1.0 - dot / (na * nb)
            } else {
                1.0
            }
        }
    }
}

fn lloyd_kmeans(x: &Matrix, mut centers: Vec<Vec<f64>>, max_iter: usize, metric: KMetric, singleton: bool) -> KResult {
    let n = x.rows();
    let d = x.cols();
    let k = centers.len();
    let mut idx = vec![0usize; n];
    let mut dist = vec![vec![0.0f64; k]; n];
    for _ in 0..max_iter {
        for r in 0..n {
            let p: Vec<f64> = (0..d).map(|c| x.get(r, c).unwrap_or(0.0)).collect();
            let mut best = 0usize;
            let mut bd = f64::INFINITY;
            for c in 0..k {
                let v = kdist(&p, &centers[c], metric);
                dist[r][c] = v;
                if v < bd {
                    bd = v;
                    best = c;
                }
            }
            idx[r] = best;
        }
        let mut counts = vec![0usize; k];
        let mut sums = vec![vec![0.0f64; d]; k];
        for (r, &c) in idx.iter().enumerate() {
            if let Some(cnt) = counts.get_mut(c) {
                *cnt += 1;
            }
            for (j, slot) in sums[c].iter_mut().enumerate().take(d) {
                *slot += x.get(r, j).unwrap_or(0.0);
            }
        }
        if singleton {
            for c in 0..k {
                if counts[c] == 0 {
                    let mut far = 0usize;
                    let mut fd = f64::NEG_INFINITY;
                    for r in 0..n {
                        let v = dist[r][idx[r]];
                        if v > fd {
                            fd = v;
                            far = r;
                        }
                    }
                    let old = idx.get(far).copied().unwrap_or(0);
                    idx[far] = c;
                    counts[c] = 1;
                    if old != c {
                        counts[old] = counts[old].saturating_sub(1);
                    }
                    for (j, s) in sums[c].iter_mut().enumerate() {
                        *s = x.get(far, j).unwrap_or(0.0);
                    }
                }
            }
        }
        let mut moved = 0.0f64;
        for c in 0..k {
            if counts[c] > 0 {
                let mut next = vec![0.0; d];
                for j in 0..d {
                    next[j] = sums[c][j] / counts[c] as f64;
                }
                if matches!(metric, KMetric::Cosine) {
                    let norm = next.iter().map(|v| v * v).sum::<f64>().sqrt();
                    if norm > 0.0 {
                        for v in &mut next {
                            *v /= norm;
                        }
                    }
                }
                for j in 0..d {
                    moved = moved.max((next[j] - centers[c][j]).abs());
                }
                centers[c] = next;
            }
        }
        if moved <= 1e-12 {
            break;
        }
    }
    let mut sumd = vec![0.0; k];
    for r in 0..n {
        let p: Vec<f64> = (0..d).map(|c| x.get(r, c).unwrap_or(0.0)).collect();
        let mut bd = f64::INFINITY;
        for c in 0..k {
            let v = kdist(&p, &centers[c], metric);
            dist[r][c] = v;
            if v < bd {
                bd = v;
                idx[r] = c;
            }
        }
        sumd[idx[r]] += bd;
    }
    KResult { idx, centers, sumd, dist, total: 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distribution_values() {
        assert!((stats::norm_pdf(0.0, 0.0, 1.0) - 0.398_942_280_401_432_7).abs() < 1e-12);
        assert!((stats::norm_cdf(0.0, 0.0, 1.0) - 0.5).abs() < 1e-8);
        assert!((stats::norm_inv(0.975, 0.0, 1.0) - 1.959_963_984_540_054).abs() < 1e-7);
        assert!((stats::exp_cdf(1.0, 1.0) - (1.0 - (-1f64).exp())).abs() < 1e-12);
        assert!((stats::unif_cdf(0.5, 0.0, 1.0) - 0.5).abs() < 1e-12);
        assert!((stats::poisson_cdf(3.0, 2.0) - 0.857_123_460_498_547).abs() < 1e-12);
        assert!((stats::binom_cdf(3.0, 10.0, 0.5) - 0.171_875).abs() < 1e-12);
    }

    #[test]
    fn iqr_quantiles() {
        assert!((iqr_values(&[1.0, 2.0, 3.0, 4.0]) - 1.5).abs() < 1e-12);
        assert!((iqr_values(&[1.0, 2.0, 3.0]) - 1.0).abs() < 1e-12);
    }
}
