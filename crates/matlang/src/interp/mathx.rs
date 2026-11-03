//! Small numerical helpers that do not belong to a single toolbox module.

use super::*;

fn combinations(n: usize, k: usize) -> Vec<Vec<usize>> {
    fn rec(start: usize, n: usize, k: usize, cur: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        if cur.len() == k {
            out.push(cur.clone());
            return;
        }
        if out.len() > MAX_ELEMS {
            return;
        }
        let remaining = k - cur.len();
        for i in start..=n.saturating_sub(remaining) {
            cur.push(i);
            rec(i + 1, n, k, cur, out);
            cur.pop();
            if out.len() > MAX_ELEMS {
                return;
            }
        }
    }
    let mut out = Vec::new();
    if k <= n {
        rec(0, n, k, &mut Vec::new(), &mut out);
    }
    out
}

fn corr_matrix(x: &Matrix) -> Result<Matrix, LangError> {
    let n = x.rows();
    let p = x.cols();
    if n < 2 || p == 0 {
        return Ok(Matrix::zeros(p, p));
    }
    let means: Vec<f64> = (0..p).map(|c| (0..n).map(|r| x.get(r, c).unwrap_or(0.0)).sum::<f64>() / n as f64).collect();
    let mut out = Matrix::zeros(p, p);
    for i in 0..p {
        for j in 0..p {
            let mut num = 0.0;
            let mut di = 0.0;
            let mut dj = 0.0;
            for r in 0..n {
                let a = x.get(r, i).unwrap_or(0.0) - means[i];
                let b = x.get(r, j).unwrap_or(0.0) - means[j];
                num += a * b;
                di += a * a;
                dj += b * b;
            }
            let v = if di > 0.0 && dj > 0.0 { num / (di * dj).sqrt() } else { f64::NAN };
            let _ = out.set(i, j, v);
        }
    }
    Ok(out)
}

fn pdist_rows(x: &Matrix, metric: &str) -> Vec<f64> {
    let n = x.rows();
    let p = x.cols();
    let mut out = Vec::new();
    for i in 0..n {
        for j in i + 1..n {
            let mut d = 0.0;
            let (mut dot, mut ni, mut nj) = (0.0, 0.0, 0.0);
            let (mut mi, mut mj) = (0.0, 0.0);
            for c in 0..p {
                let a = x.get(i, c).unwrap_or(0.0);
                let b = x.get(j, c).unwrap_or(0.0);
                dot += a * b;
                ni += a * a;
                nj += b * b;
                mi += a;
                mj += b;
                match metric {
                    "cityblock" | "manhattan" => d += (a - b).abs(),
                    "chebychev" | "chebyshev" | "max" => d = d.max((a - b).abs()),
                    _ => d += (a - b) * (a - b),
                }
            }
            let value = match metric {
                "cityblock" | "manhattan" | "chebychev" | "chebyshev" | "max" => d,
                "cosine" => {
                    if ni > 0.0 && nj > 0.0 {
                        1.0 - dot / (ni.sqrt() * nj.sqrt())
                    } else {
                        f64::NAN
                    }
                }
                "correlation" => {
                    let den = ((ni - mi * mi / p as f64) * (nj - mj * mj / p as f64)).sqrt();
                    if den > 0.0 {
                        1.0 - (dot - mi * mj / p as f64) / den
                    } else {
                        f64::NAN
                    }
                }
                _ => d.sqrt(),
            };
            out.push(value);
        }
    }
    out
}

impl Session {
    #[allow(clippy::needless_range_loop)]
    pub(super) fn mathx_builtin(&mut self, name: &str, argv: Vec<Value>) -> Result<Vec<Value>, LangError> {
        match name {
            "nchoosek" => {
                let a = argv.first().ok_or_else(|| LangError::new("nchoosek needs n", 0))?;
                let k = argv.get(1).ok_or_else(|| LangError::new("nchoosek needs k", 0))?.scalar()?.max(0.0) as usize;
                let av = a.to_matrix()?;
                if av.numel() == 1 {
                    let n = a.scalar()?.max(0.0) as usize;
                    if k > n {
                        return Ok(vec![Value::Num(0.0)]);
                    }
                    let k = k.min(n - k);
                    let mut out = 1.0f64;
                    for i in 0..k {
                        out = out * (n - i) as f64 / (i + 1) as f64;
                    }
                    Ok(vec![Value::Num(out.round())])
                } else {
                    let n = av.numel();
                    let combos = combinations(n, k);
                    if combos.len() > MAX_ELEMS {
                        return Err(LangError::new("nchoosek: result exceeds the element limit", 0));
                    }
                    let mut out = Matrix::zeros(combos.len(), k);
                    for (r, c) in combos.iter().enumerate() {
                        for (j, &idx) in c.iter().enumerate() {
                            let _ = out.set(r, j, av.get_linear(idx).unwrap_or(f64::NAN));
                        }
                    }
                    Ok(vec![mat_or_num(out)])
                }
            }
            "gammaln" => {
                let m = argv.first().ok_or_else(|| LangError::new("gammaln needs an argument", 0))?.to_matrix()?;
                let mut out = Matrix::zeros(m.rows(), m.cols());
                for i in 0..m.numel() {
                    out.set_linear(i, matcore::stats::gammaln(m.get_linear(i).unwrap_or(f64::NAN)));
                }
                Ok(vec![mat_or_num(out)])
            }
            "hygecdf" => {
                let x = argv.first().ok_or_else(|| LangError::new("hygecdf needs x", 0))?.to_matrix()?;
                let m = argv.get(1).ok_or_else(|| LangError::new("hygecdf needs M", 0))?.scalar()?;
                let k = argv.get(2).ok_or_else(|| LangError::new("hygecdf needs K", 0))?.scalar()?;
                let n = argv.get(3).ok_or_else(|| LangError::new("hygecdf needs N", 0))?.scalar()?;
                let mut out = Matrix::zeros(x.rows(), x.cols());
                let log_den = matcore::stats::gammaln(m + 1.0) - matcore::stats::gammaln(n + 1.0) - matcore::stats::gammaln(m - n + 1.0);
                for i in 0..x.numel() {
                    let xi = x.get_linear(i).unwrap_or(f64::NAN).floor();
                    if !xi.is_finite() || xi < 0.0 {
                        out.set_linear(i, 0.0);
                        continue;
                    }
                    let mut cdf = 0.0;
                    let hi = xi.min(n).min(k) as i64;
                    let lo = (n - (m - k)).max(0.0) as i64;
                    for j in lo.max(0)..=hi {
                        let jf = j as f64;
                        if jf > n || jf > k || n - jf > m - k {
                            continue;
                        }
                        let lp = matcore::stats::gammaln(k + 1.0) - matcore::stats::gammaln(jf + 1.0) - matcore::stats::gammaln(k - jf + 1.0)
                            + matcore::stats::gammaln(m - k + 1.0)
                            - matcore::stats::gammaln(n - jf + 1.0)
                            - matcore::stats::gammaln(m - k - (n - jf) + 1.0)
                            - log_den;
                        cdf += lp.exp();
                    }
                    out.set_linear(i, cdf.min(1.0));
                }
                Ok(vec![mat_or_num(out)])
            }
            "tabulate" => {
                let a = argv.first().ok_or_else(|| LangError::new("tabulate needs data", 0))?;
                match a {
                    Value::Mat(m) => {
                        let mut vals: Vec<f64> = m.data().iter().copied().filter(|x| x.is_finite()).collect();
                        vals.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
                        let mut uniq: Vec<(f64, usize)> = Vec::new();
                        for v in vals {
                            match uniq.last_mut() {
                                Some((u, n)) if *u == v => *n += 1,
                                _ => uniq.push((v, 1)),
                            }
                        }
                        let total: usize = uniq.iter().map(|(_, n)| *n).sum();
                        let mut out = Matrix::zeros(uniq.len(), 3);
                        for (r, (v, n)) in uniq.iter().enumerate() {
                            let _ = out.set(r, 0, *v);
                            let _ = out.set(r, 1, *n as f64);
                            let _ = out.set(r, 2, *n as f64 / total.max(1) as f64 * 100.0);
                        }
                        Ok(vec![Value::Mat(out)])
                    }
                    Value::Cell(c) => {
                        let mut uniq: Vec<(String, usize)> = Vec::new();
                        for item in &c.items {
                            if let Value::Str(s) = item {
                                match uniq.iter_mut().find(|(u, _)| u == s) {
                                    Some((_, n)) => *n += 1,
                                    None => uniq.push((s.clone(), 1)),
                                }
                            }
                        }
                        let total: usize = uniq.iter().map(|(_, n)| *n).sum();
                        let mut out = CellArray::new(uniq.len(), 3);
                        for (r, (s, n)) in uniq.iter().enumerate() {
                            out.set(r, 0, Value::Str(s.clone()));
                            out.set(r, 1, Value::Num(*n as f64));
                            out.set(r, 2, Value::Num(*n as f64 / total.max(1) as f64 * 100.0));
                        }
                        Ok(vec![Value::Cell(out)])
                    }
                    _ => Err(LangError::new("tabulate needs a numeric vector or cellstr", 0)),
                }
            }
            "corr" => {
                let x = argv.first().ok_or_else(|| LangError::new("corr needs data", 0))?.to_matrix()?;
                if let Some(y) = argv.get(1) {
                    let y = y.to_matrix()?;
                    let a = corr_matrix(&x)?;
                    let b = corr_matrix(&y)?;
                    // Cross-correlation: standardise columns and take X'Y/(n-1).
                    let n = x.rows().min(y.rows()).max(1);
                    let mut out = Matrix::zeros(x.cols(), y.cols());
                    for i in 0..x.cols() {
                        for j in 0..y.cols() {
                            let (mut sx, mut sy) = (0.0, 0.0);
                            for r in 0..n {
                                sx += x.get(r, i).unwrap_or(0.0);
                                sy += y.get(r, j).unwrap_or(0.0);
                            }
                            let (mx, my) = (sx / n as f64, sy / n as f64);
                            let (mut num, mut dx, mut dy) = (0.0, 0.0, 0.0);
                            for r in 0..n {
                                let a = x.get(r, i).unwrap_or(0.0) - mx;
                                let b = y.get(r, j).unwrap_or(0.0) - my;
                                num += a * b;
                                dx += a * a;
                                dy += b * b;
                            }
                            let _ = out.set(i, j, if dx > 0.0 && dy > 0.0 { num / (dx * dy).sqrt() } else { f64::NAN });
                        }
                    }
                    let _ = (a, b);
                    Ok(vec![Value::Mat(out)])
                } else {
                    Ok(vec![Value::Mat(corr_matrix(&x)?)])
                }
            }
            "pdist" => {
                let x = argv.first().ok_or_else(|| LangError::new("pdist needs data", 0))?.to_matrix()?;
                let metric = match argv.get(1) {
                    Some(Value::Str(s)) => s.to_ascii_lowercase(),
                    _ => "euclidean".into(),
                };
                Ok(vec![Value::Mat(Matrix::row_vector(pdist_rows(&x, &metric)))])
            }
            "squareform" => {
                let m = argv.first().ok_or_else(|| LangError::new("squareform needs data", 0))?.to_matrix()?;
                if m.rows() == 1 || m.cols() == 1 {
                    let n = m.numel();
                    let size = ((1.0 + (1.0 + 8.0 * n as f64).sqrt()) / 2.0).round() as usize;
                    if size * size.saturating_sub(1) / 2 != n {
                        return Err(LangError::new("squareform: vector length is not n*(n-1)/2", 0));
                    }
                    let mut out = Matrix::zeros(size, size);
                    let mut k = 0;
                    for i in 0..size {
                        for j in i + 1..size {
                            let v = m.get_linear(k).unwrap_or(0.0);
                            let _ = out.set(i, j, v);
                            let _ = out.set(j, i, v);
                            k += 1;
                        }
                    }
                    Ok(vec![Value::Mat(out)])
                } else if m.rows() == m.cols() {
                    let mut out = Vec::new();
                    for i in 0..m.rows() {
                        for j in i + 1..m.cols() {
                            out.push(m.get(i, j).unwrap_or(0.0));
                        }
                    }
                    Ok(vec![Value::Mat(Matrix::row_vector(out))])
                } else {
                    Err(LangError::new("squareform needs a vector or square matrix", 0))
                }
            }
            "vecnorm" => {
                let m = argv.first().ok_or_else(|| LangError::new("vecnorm needs data", 0))?.to_matrix()?;
                let p = argv.get(1).map(|x| x.scalar()).transpose()?.unwrap_or(2.0);
                let dim = argv.get(2).map(|x| x.scalar()).transpose()?.unwrap_or(1.0).max(1.0) as usize;
                let norm = |vals: Vec<f64>| -> f64 {
                    if p == f64::INFINITY {
                        vals.iter().fold(0.0f64, |a, &b| a.max(b.abs()))
                    } else if p == 1.0 {
                        vals.iter().map(|x| x.abs()).sum()
                    } else {
                        vals.iter().map(|x| x.abs().powf(p)).sum::<f64>().powf(1.0 / p)
                    }
                };
                if dim == 1 {
                    let mut out = Matrix::zeros(1, m.cols());
                    for c in 0..m.cols() {
                        let vals: Vec<f64> = (0..m.rows()).map(|r| m.get(r, c).unwrap_or(0.0)).collect();
                        let _ = out.set(0, c, norm(vals));
                    }
                    Ok(vec![Value::Mat(out)])
                } else {
                    let mut out = Matrix::zeros(m.rows(), 1);
                    for r in 0..m.rows() {
                        let vals: Vec<f64> = (0..m.cols()).map(|c| m.get(r, c).unwrap_or(0.0)).collect();
                        let _ = out.set(r, 0, norm(vals));
                    }
                    Ok(vec![Value::Mat(out)])
                }
            }
            "blkdiag" => {
                let mats: Vec<Matrix> = argv.iter().map(|x| x.to_matrix()).collect::<Result<_, _>>()?;
                let rows: usize = mats.iter().map(|m| m.rows()).sum();
                let cols: usize = mats.iter().map(|m| m.cols()).sum();
                check_elems(rows.saturating_mul(cols))?;
                let mut out = Matrix::zeros(rows, cols);
                let (mut r0, mut c0) = (0, 0);
                for m in &mats {
                    for r in 0..m.rows() {
                        for c in 0..m.cols() {
                            let _ = out.set(r0 + r, c0 + c, m.get(r, c).unwrap_or(0.0));
                        }
                    }
                    r0 += m.rows();
                    c0 += m.cols();
                }
                Ok(vec![mat_or_num(out)])
            }
            "mat2cell" => {
                let a = argv.first().ok_or_else(|| LangError::new("mat2cell needs a matrix", 0))?.to_matrix()?;
                let row_d = argv.get(1).ok_or_else(|| LangError::new("mat2cell needs row sizes", 0))?.to_matrix()?;
                let col_d = argv.get(2).map(|x| x.to_matrix()).transpose()?.unwrap_or_else(|| Matrix::row_vector(vec![a.cols() as f64]));
                let rd: Vec<usize> = row_d.data().iter().map(|&x| x.max(0.0) as usize).collect();
                let cd: Vec<usize> = col_d.data().iter().map(|&x| x.max(0.0) as usize).collect();
                if rd.iter().sum::<usize>() != a.rows() || cd.iter().sum::<usize>() != a.cols() {
                    return Err(LangError::new("mat2cell: block sizes must sum to the matrix size", 0));
                }
                let mut out = CellArray::new(rd.len(), cd.len());
                let mut r0 = 0;
                for (i, &rh) in rd.iter().enumerate() {
                    let mut c0 = 0;
                    for (j, &cw) in cd.iter().enumerate() {
                        let mut block = Matrix::zeros(rh, cw);
                        for r in 0..rh {
                            for c in 0..cw {
                                let _ = block.set(r, c, a.get(r0 + r, c0 + c).unwrap_or(0.0));
                            }
                        }
                        out.set(i, j, mat_or_num(block));
                        c0 += cw;
                    }
                    r0 += rh;
                }
                Ok(vec![Value::Cell(out)])
            }
            "gradient" => {
                let m = argv.first().ok_or_else(|| LangError::new("gradient needs a matrix", 0))?.to_matrix()?;
                let h = argv.get(1).map(|x| x.scalar()).transpose()?.unwrap_or(1.0);
                let mut out = Matrix::zeros(m.rows(), m.cols());
                for r in 0..m.rows() {
                    for c in 0..m.cols() {
                        let v = if c == 0 {
                            (m.get(r, 1.min(m.cols().saturating_sub(1))).unwrap_or(0.0) - m.get(r, 0).unwrap_or(0.0)) / h
                        } else if c + 1 == m.cols() {
                            (m.get(r, c).unwrap_or(0.0) - m.get(r, c.saturating_sub(1)).unwrap_or(0.0)) / h
                        } else {
                            (m.get(r, c + 1).unwrap_or(0.0) - m.get(r, c.saturating_sub(1)).unwrap_or(0.0)) / (2.0 * h)
                        };
                        let _ = out.set(r, c, v);
                    }
                }
                Ok(vec![Value::Mat(out)])
            }
            "expm" => {
                let m = argv.first().ok_or_else(|| LangError::new("expm needs a matrix", 0))?.to_matrix()?;
                Ok(vec![mat_or_num(matcore::expm(&m).map_err(lang)?)])
            }
            "fitlm" => {
                let (x, y) = match argv.len() {
                    1 => {
                        let y = argv[0].to_matrix()?;
                        let n = y.numel();
                        let mut x = Matrix::zeros(n, 1);
                        for i in 0..n {
                            let _ = x.set(i, 0, (i + 1) as f64);
                        }
                        (x, y.data().to_vec())
                    }
                    _ => {
                        let x = argv[0].to_matrix()?;
                        let y = argv[1].to_matrix()?;
                        (x, y.data().to_vec())
                    }
                };
                let n = x.rows();
                if n < 2 || y.len() != n {
                    return Err(LangError::new("fitlm: X and y must have the same number of rows", 0));
                }
                let p = x.cols() + 1;
                let mut design = Matrix::zeros(n, p);
                for r in 0..n {
                    let _ = design.set(r, 0, 1.0);
                    for c in 0..x.cols() {
                        let _ = design.set(r, c + 1, x.get(r, c).unwrap_or(0.0));
                    }
                }
                let yv = Matrix::col_vector(y.clone());
                let xt = design.transpose();
                let xtx = xt.matmul(&design).map_err(lang)?;
                let xty = xt.matmul(&yv).map_err(lang)?;
                let beta = xtx.solve(&xty).map_err(lang)?;
                let fitted = design.matmul(&beta).map_err(lang)?;
                let resid = {
                    let mut m = Matrix::zeros(n, 1);
                    for i in 0..n {
                        let _ = m.set(i, 0, y.get(i).copied().unwrap_or(0.0) - fitted.get(i, 0).unwrap_or(0.0));
                    }
                    m
                };
                let sse: f64 = resid.data().iter().map(|v| v * v).sum();
                let mean = y.iter().sum::<f64>() / n as f64;
                let sst: f64 = y.iter().map(|v| (v - mean) * (v - mean)).sum();
                let r2 = if sst > 0.0 { 1.0 - sse / sst } else { f64::NAN };
                let adj = if n > p && sst > 0.0 { 1.0 - (1.0 - r2) * (n - 1) as f64 / (n - p) as f64 } else { f64::NAN };
                let inv = xtx.inverse().map_err(lang)?;
                let sigma2 = sse / (n.saturating_sub(p)).max(1) as f64;
                let mut coefs = StructArray::new();
                let mut se = Matrix::zeros(p, 1);
                let mut tstat = Matrix::zeros(p, 1);
                let mut pval = Matrix::zeros(p, 1);
                for i in 0..p {
                    let s = (sigma2 * inv.get(i, i).unwrap_or(0.0)).max(0.0).sqrt();
                    let t = beta.get(i, 0).unwrap_or(0.0) / s;
                    let pv = if s > 0.0 { 2.0 * (1.0 - matcore::stats::t_cdf(t.abs(), (n - p) as f64)) } else { f64::NAN };
                    let _ = se.set(i, 0, s);
                    let _ = tstat.set(i, 0, t);
                    let _ = pval.set(i, 0, pv);
                }
                coefs.set_field(0, "Estimate", Value::Mat(beta))?;
                coefs.set_field(0, "SE", Value::Mat(se))?;
                coefs.set_field(0, "tStat", Value::Mat(tstat))?;
                coefs.set_field(0, "pValue", Value::Mat(pval))?;
                let mut rsq = StructArray::new();
                rsq.set_field(0, "Ordinary", Value::Num(r2))?;
                rsq.set_field(0, "Adjusted", Value::Num(adj))?;
                let mut props = StructArray::new();
                props.set_field(0, "Coefficients", Value::Struct(coefs))?;
                props.set_field(0, "Rsquared", Value::Struct(rsq))?;
                props.set_field(0, "Fitted", Value::Mat(fitted))?;
                props.set_field(0, "Residuals", Value::Mat(resid))?;
                props.set_field(0, "NumObservations", Value::Num(n as f64))?;
                props.set_field(0, "NumPredictors", Value::Num(x.cols() as f64))?;
                props.set_field(0, "Formula", Value::Str("y ~ 1 + x".into()))?;
                Ok(vec![Value::Object(MObject::owned("LinearModel".into(), props))])
            }
            "optimalleaforder" => {
                let a = argv.first().ok_or_else(|| LangError::new("optimalleaforder needs a linkage", 0))?;
                let n = match a {
                    Value::Mat(m) if m.rows() > 0 && m.cols() >= 2 => (m.data().iter().copied().fold(0.0f64, f64::max) as usize).max(1),
                    _ => 1,
                };
                Ok(vec![Value::Mat(Matrix::col_vector((1..=n).map(|i| i as f64).collect()))])
            }
            "zscore" => {
                let x = argv.first().ok_or_else(|| LangError::new("zscore needs data", 0))?.to_numeric()?;
                if x.numel() == 0 {
                    return Ok(vec![Value::Mat(x)]);
                }
                let flag = argv.get(1).and_then(|v| v.scalar().ok()).unwrap_or(0.0);
                let mut out = Matrix::zeros(x.rows(), x.cols());
                if x.rows() == 1 {
                    let n = x.numel() as f64;
                    let mean = x.data().iter().sum::<f64>() / n;
                    let denom = if flag == 1.0 { n } else { (n - 1.0).max(1.0) };
                    let var = x.data().iter().map(|v| (v - mean).powi(2)).sum::<f64>() / denom;
                    let sd = var.sqrt().max(1e-15);
                    for (i, v) in x.data().iter().enumerate() {
                        out.set_linear(i, (v - mean) / sd);
                    }
                } else {
                    let n = x.rows() as f64;
                    let denom = if flag == 1.0 { n } else { (n - 1.0).max(1.0) };
                    for c in 0..x.cols() {
                        let mean = (0..x.rows()).map(|r| x.get(r, c).unwrap_or(0.0)).sum::<f64>() / n;
                        let var = (0..x.rows()).map(|r| (x.get(r, c).unwrap_or(0.0) - mean).powi(2)).sum::<f64>() / denom;
                        let sd = var.sqrt().max(1e-15);
                        for r in 0..x.rows() {
                            let _ = out.set(r, c, (x.get(r, c).unwrap_or(0.0) - mean) / sd);
                        }
                    }
                }
                Ok(vec![Value::Mat(out)])
            }
            "isbetween" => {
                let x = argv.first().ok_or_else(|| LangError::new("isbetween needs data", 0))?.to_matrix()?;
                let lo = argv.get(1).ok_or_else(|| LangError::new("isbetween needs a lower bound", 0))?.to_matrix()?;
                let hi = argv.get(2).ok_or_else(|| LangError::new("isbetween needs an upper bound", 0))?.to_matrix()?;
                let lo_b = lo.as_scalar().unwrap_or(f64::NAN);
                let hi_b = hi.as_scalar().unwrap_or(f64::NAN);
                let mut out = Matrix::zeros(x.rows(), x.cols());
                for r in 0..x.rows() {
                    for c in 0..x.cols() {
                        let l = lo.get(r, c).or_else(|| lo.get(0, 0)).unwrap_or(lo_b);
                        let h = hi.get(r, c).or_else(|| hi.get(0, 0)).unwrap_or(hi_b);
                        let v = x.get(r, c).unwrap_or(f64::NAN);
                        let _ = out.set(r, c, if v >= l && v <= h { 1.0 } else { 0.0 });
                    }
                }
                Ok(vec![Value::Mat(out)])
            }
            "isoutlier" => {
                let x = argv.first().ok_or_else(|| LangError::new("isoutlier needs data", 0))?.to_matrix()?;
                let method = match argv.get(1) {
                    Some(Value::Str(s)) => s.to_ascii_lowercase(),
                    _ => "median".into(),
                };
                let mut out = Matrix::zeros(x.rows(), x.cols());
                // A row vector is one sample set; a matrix is column-wise.
                let columns: Vec<Vec<f64>> = if x.rows() == 1 {
                    vec![x.data().iter().copied().filter(|v| v.is_finite()).collect()]
                } else {
                    (0..x.cols()).map(|c| (0..x.rows()).filter_map(|r| x.get(r, c).filter(|v| v.is_finite())).collect()).collect()
                };
                for (c, mut vals) in columns.into_iter().enumerate() {
                    if vals.is_empty() {
                        continue;
                    }
                    vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                    let median = vals[vals.len() / 2];
                    let (center, spread) = match method.as_str() {
                        "mean" => {
                            let mean = vals.iter().sum::<f64>() / vals.len() as f64;
                            let var = vals.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / vals.len().max(1) as f64;
                            (mean, var.sqrt())
                        }
                        "quartiles" => {
                            let q1 = vals[vals.len() / 4];
                            let q3 = vals[(3 * vals.len()) / 4];
                            (median, (q3 - q1).abs())
                        }
                        _ => {
                            let dev: Vec<f64> = vals.iter().map(|v| (v - median).abs()).collect();
                            let mut dev = dev;
                            dev.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                            let mad = dev[dev.len() / 2];
                            (median, 1.4826 * mad)
                        }
                    };
                    let threshold = if method == "quartiles" { 1.5 } else { 3.0 };
                    let positions: Vec<(usize, usize)> =
                        if x.rows() == 1 { (0..x.cols()).map(|cc| (0, cc)).collect() } else { (0..x.rows()).map(|r| (r, c)).collect() };
                    for (r, cc) in positions {
                        if let Some(v) = x.get(r, cc) {
                            let is_out = spread > 0.0 && (v - center).abs() > threshold * spread;
                            let _ = out.set(r, cc, if is_out { 1.0 } else { 0.0 });
                        }
                    }
                }
                Ok(vec![Value::Mat(out)])
            }
            "partialcorr" => {
                let x = argv.first().ok_or_else(|| LangError::new("partialcorr needs data", 0))?.to_matrix()?;
                if x.cols() < 2 || x.rows() < 2 {
                    return Ok(vec![Value::Mat(Matrix::zeros(x.cols(), x.cols()))]);
                }
                let n = x.rows();
                let p = x.cols();
                let means: Vec<f64> = (0..p).map(|c| (0..n).map(|r| x.get(r, c).unwrap_or(0.0)).sum::<f64>() / n as f64).collect();
                let mut cov = Matrix::zeros(p, p);
                for i in 0..p {
                    for j in 0..p {
                        let mut s = 0.0;
                        for r in 0..n {
                            s += (x.get(r, i).unwrap_or(0.0) - means[i]) * (x.get(r, j).unwrap_or(0.0) - means[j]);
                        }
                        let _ = cov.set(i, j, s / (n - 1).max(1) as f64);
                    }
                }
                let inv = cov.inverse().map_err(lang)?;
                let mut out = Matrix::zeros(p, p);
                for i in 0..p {
                    for j in 0..p {
                        let den = (inv.get(i, i).unwrap_or(0.0) * inv.get(j, j).unwrap_or(0.0)).sqrt();
                        let v = if den > 0.0 { -inv.get(i, j).unwrap_or(0.0) / den } else { f64::NAN };
                        let _ = out.set(i, j, v);
                    }
                }
                Ok(vec![Value::Mat(out)])
            }
            "linkage" => {
                let a = argv.first().ok_or_else(|| LangError::new("linkage needs distances", 0))?.to_matrix()?;
                let method = match argv.get(1) {
                    Some(Value::Str(s)) => s.to_ascii_lowercase(),
                    _ => "single".into(),
                };
                let n = if a.rows() == a.cols() { a.rows() } else { ((1.0 + (1.0 + 8.0 * a.numel() as f64).sqrt()) / 2.0).round() as usize };
                if n < 2 {
                    return Ok(vec![Value::Mat(Matrix::zeros(0, 3))]);
                }
                let mut dist = vec![vec![0.0f64; n]; n];
                if a.rows() == a.cols() {
                    for i in 0..n {
                        for j in 0..n {
                            dist[i][j] = a.get(i, j).unwrap_or(0.0);
                        }
                    }
                } else {
                    let mut k = 0;
                    for i in 0..n {
                        for j in i + 1..n {
                            let v = a.get_linear(k).unwrap_or(0.0);
                            dist[i][j] = v;
                            dist[j][i] = v;
                            k += 1;
                        }
                    }
                }
                let mut clusters: Vec<Vec<usize>> = (0..n).map(|i| vec![i]).collect();
                let mut ids: Vec<usize> = (1..=n).collect();
                let mut active: Vec<bool> = vec![true; n];
                let mut out = Matrix::zeros(n - 1, 3);
                for step in 0..n - 1 {
                    let mut best = (0usize, 1usize);
                    let mut bd = f64::INFINITY;
                    for i in 0..n {
                        if !active[i] {
                            continue;
                        }
                        for j in i + 1..n {
                            if active[j] && dist[i][j] < bd {
                                bd = dist[i][j];
                                best = (i, j);
                            }
                        }
                    }
                    let (i, j) = best;
                    let _ = out.set(step, 0, ids[i] as f64);
                    let _ = out.set(step, 1, ids[j] as f64);
                    let _ = out.set(step, 2, bd);
                    let ci = clusters[i].len() as f64;
                    let cj = clusters[j].len() as f64;
                    for k in 0..n {
                        if !active[k] || k == i || k == j {
                            continue;
                        }
                        let dij = dist[i][j];
                        let d = match method.as_str() {
                            "complete" => dist[i][k].max(dist[j][k]),
                            "average" | "upgma" => (dist[i][k] * ci + dist[j][k] * cj) / (ci + cj),
                            "weighted" | "wpgma" => (dist[i][k] + dist[j][k]) / 2.0,
                            "ward" | "ward.d2" => {
                                let nk = clusters[k].len() as f64;
                                (((ci + nk) * dist[i][k] * dist[i][k] + (cj + nk) * dist[j][k] * dist[j][k] - nk * dij * dij) / (ci + cj + nk))
                                    .max(0.0)
                                    .sqrt()
                            }
                            "centroid" | "upgmc" => (((ci * dist[i][k] * dist[i][k] + cj * dist[j][k] * dist[j][k]) / (ci + cj))
                                - ci * cj * dij * dij / ((ci + cj) * (ci + cj)))
                                .max(0.0)
                                .sqrt(),
                            "median" | "wpgmc" => (0.5 * dist[i][k] * dist[i][k] + 0.5 * dist[j][k] * dist[j][k] - 0.25 * dij * dij).max(0.0).sqrt(),
                            _ => dist[i][k].min(dist[j][k]),
                        };
                        dist[i][k] = d;
                        dist[k][i] = d;
                    }
                    let moved = clusters[j].clone();
                    clusters[i].extend(moved);
                    ids[i] = n + step + 1;
                    active[j] = false;
                }
                Ok(vec![Value::Mat(out)])
            }
            _ => Err(LangError::new(format!("{name}: not implemented"), 0)),
        }
    }
}
