//! t-SNE embedding (`tsne`).
//!
//! Gaussian input affinities are calibrated to the requested perplexity,
//! symmetrized, and matched with Student-t affinities in the low-dimensional
//! space by momentum gradient descent. The implementation follows the
//! published t-SNE objective and uses only the session PRNG, so `rng(seed)`
//! makes runs reproducible.

#![allow(clippy::needless_range_loop)]

use super::*;

struct TsneOptions {
    dims: usize,
    perplexity: f64,
    pca_components: Option<usize>,
    standardize: bool,
    learn_rate: f64,
    iterations: usize,
    exaggeration: f64,
    initial_y: Option<Matrix>,
    distance: String,
}

impl TsneOptions {
    fn defaults(n: usize) -> Self {
        TsneOptions {
            dims: 2,
            perplexity: (30.0f64).min(((n.saturating_sub(1)) as f64 * 0.5).max(1.0)),
            pca_components: None,
            standardize: false,
            learn_rate: 200.0,
            iterations: 1000,
            exaggeration: 4.0,
            initial_y: None,
            distance: "euclidean".into(),
        }
    }
}

fn parse_options(argv: &[Value], n: usize) -> Result<TsneOptions, LangError> {
    let mut o = TsneOptions::defaults(n);
    let mut i = 1usize;
    while i < argv.len() {
        let key = match &argv[i] {
            Value::Str(s) => s.to_ascii_lowercase(),
            _ => return Err(LangError::new("tsne options must be name/value pairs", 0)),
        };
        let value = argv.get(i + 1).ok_or_else(|| LangError::new(format!("tsne: missing value for '{key}'"), 0))?;
        match key.as_str() {
            "numdimensions" | "numdims" => {
                let v = value.scalar()?.round();
                if !v.is_finite() || !(1.0..=3.0).contains(&v) {
                    return Err(LangError::new("tsne: NumDimensions must be 1, 2 or 3", 0));
                }
                o.dims = v as usize;
            }
            "perplexity" => {
                let v = value.scalar()?;
                if !v.is_finite() || v <= 0.0 {
                    return Err(LangError::new("tsne: Perplexity must be positive", 0));
                }
                o.perplexity = v.min((n.saturating_sub(1)) as f64).max(1.0);
            }
            "numpcacomponents" | "numpca" => {
                let v = value.scalar()?.round();
                if !v.is_finite() || v < 1.0 {
                    return Err(LangError::new("tsne: NumPCAComponents must be positive", 0));
                }
                o.pca_components = Some(v as usize);
            }
            "standardize" => o.standardize = value.scalar()? != 0.0,
            "learnrate" => {
                let v = value.scalar()?;
                if !v.is_finite() || v <= 0.0 {
                    return Err(LangError::new("tsne: LearnRate must be positive", 0));
                }
                o.learn_rate = v;
            }
            "numiterations" | "maxiter" | "maxiterations" => {
                let v = value.scalar()?.round();
                if !v.is_finite() || v < 1.0 || v > 100_000.0 {
                    return Err(LangError::new("tsne: NumIterations must be between 1 and 100000", 0));
                }
                o.iterations = v as usize;
            }
            "exaggeration" => {
                let v = value.scalar()?;
                if !v.is_finite() || v <= 0.0 {
                    return Err(LangError::new("tsne: Exaggeration must be positive", 0));
                }
                o.exaggeration = v;
            }
            "initialy" => {
                let m = value.to_numeric()?;
                if m.rows() != n {
                    return Err(LangError::new(format!("tsne: InitialY must have {n} rows"), 0));
                }
                o.initial_y = Some(m);
            }
            "distance" => match value {
                Value::Str(s) if s.eq_ignore_ascii_case("euclidean") => o.distance = "euclidean".into(),
                Value::Str(s) => return Err(LangError::new(format!("tsne: unsupported distance '{s}' (use 'euclidean')"), 0)),
                _ => return Err(LangError::new("tsne: Distance must be a string", 0)),
            },
            other => return Err(LangError::new(format!("tsne: unknown option '{other}'"), 0)),
        }
        i += 2;
    }
    Ok(o)
}

fn standardize(mut x: Matrix) -> Result<Matrix, LangError> {
    let (n, d) = x.shape();
    for c in 0..d {
        let mean = (0..n).map(|r| x.get(r, c).unwrap_or(0.0)).sum::<f64>() / n as f64;
        let var = (0..n).map(|r| (x.get(r, c).unwrap_or(0.0) - mean).powi(2)).sum::<f64>() / n.max(1) as f64;
        let sd = var.sqrt();
        if sd > 1e-12 {
            for r in 0..n {
                let _ = x.set(r, c, (x.get(r, c).unwrap_or(0.0) - mean) / sd);
            }
        }
    }
    Ok(x)
}

/// Project rows onto the leading principal components.
fn pca_reduce(x: &Matrix, k: usize) -> Result<Matrix, LangError> {
    let (n, d) = x.shape();
    let k = k.min(d).max(1);
    let mut mu = vec![0.0f64; d];
    for r in 0..n {
        for (c, m) in mu.iter_mut().enumerate() {
            *m += x.get(r, c).unwrap_or(0.0);
        }
    }
    for m in &mut mu {
        *m /= n as f64;
    }
    let mut centered = Matrix::zeros(n, d);
    for r in 0..n {
        for c in 0..d {
            let _ = centered.set(r, c, x.get(r, c).unwrap_or(0.0) - mu.get(c).copied().unwrap_or(0.0));
        }
    }
    let cov = centered.transpose().matmul(&centered).map_err(lang)?;
    let cov = cov.map(|v| v / (n.saturating_sub(1).max(1)) as f64);
    let (vals, vecs) = matcore::eig_symmetric(&cov).map_err(lang)?;
    let mut out = Matrix::zeros(n, k);
    for j in 0..k {
        let src = d.saturating_sub(1).saturating_sub(j);
        for r in 0..n {
            let mut v = 0.0;
            for c in 0..d {
                v += centered.get(r, c).unwrap_or(0.0) * vecs.get(c, src).unwrap_or(0.0);
            }
            let scale = vals.get(src).copied().unwrap_or(0.0).max(0.0).sqrt().max(1e-12);
            let _ = out.set(r, j, v / scale);
        }
    }
    Ok(out)
}

fn pairwise_sq(x: &Matrix) -> Vec<Vec<f64>> {
    let (n, d) = x.shape();
    let mut dist = vec![vec![0.0f64; n]; n];
    for i in 0..n {
        for j in (i + 1)..n {
            let mut s = 0.0f64;
            for c in 0..d {
                let diff = x.get(i, c).unwrap_or(0.0) - x.get(j, c).unwrap_or(0.0);
                s += diff * diff;
            }
            if let Some(row) = dist.get_mut(i) {
                if let Some(slot) = row.get_mut(j) {
                    *slot = s;
                }
            }
            if let Some(row) = dist.get_mut(j) {
                if let Some(slot) = row.get_mut(i) {
                    *slot = s;
                }
            }
        }
    }
    dist
}

/// Symmetric input affinities with a per-point Gaussian bandwidth chosen so
/// the conditional distribution has the requested perplexity.
fn input_affinities(dist: &[Vec<f64>], perplexity: f64) -> Vec<Vec<f64>> {
    let n = dist.len();
    let target = perplexity.ln();
    let mut p = vec![vec![0.0f64; n]; n];
    for i in 0..n {
        let mut beta = 1.0f64;
        let mut lo = 0.0f64;
        let mut hi = f64::INFINITY;
        for _ in 0..80 {
            let mut sum = 0.0f64;
            let mut weighted = 0.0f64;
            for j in 0..n {
                if i == j {
                    continue;
                }
                let w = (-dist[i][j] * beta).exp();
                sum += w;
                weighted += dist[i][j] * w;
            }
            if sum <= 1e-300 || !sum.is_finite() {
                beta = (beta * 0.5).max(1e-12);
                hi = beta;
                continue;
            }
            let entropy = sum.ln() + beta * weighted / sum;
            if entropy > target {
                lo = beta;
                beta = if hi.is_finite() { 0.5 * (beta + hi) } else { beta * 2.0 };
            } else {
                hi = beta;
                beta = 0.5 * (beta + lo);
            }
        }
        let mut sum = 0.0f64;
        for j in 0..n {
            if i != j {
                let w = (-dist[i][j] * beta).exp();
                if let Some(row) = p.get_mut(i) {
                    if let Some(slot) = row.get_mut(j) {
                        *slot = w;
                    }
                }
                sum += w;
            }
        }
        if sum > 0.0 {
            for j in 0..n {
                if let Some(row) = p.get_mut(i) {
                    if let Some(slot) = row.get_mut(j) {
                        *slot /= sum;
                    }
                }
            }
        }
    }
    // Symmetrize and normalize.
    let mut total = 0.0f64;
    for i in 0..n {
        for j in 0..n {
            let v = 0.5 * (p[i][j] + p[j][i]);
            p[i][j] = v.max(1e-12);
            total += p[i][j];
        }
    }
    if total > 0.0 {
        for row in &mut p {
            for v in row {
                *v /= total;
            }
        }
    }
    p
}

impl Session {
    pub(super) fn tsne_call(&mut self, argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
        let raw = argv.first().ok_or_else(|| LangError::new("tsne needs a data matrix", 0))?.to_numeric()?;
        let (n, d) = raw.shape();
        if n < 4 || d < 1 {
            return Err(LangError::new("tsne needs at least four observations and one variable", 0));
        }
        check_elems(n.saturating_mul(n))?;
        let opts = parse_options(argv, n)?;
        let mut x = raw;
        if opts.standardize {
            x = standardize(x)?;
        }
        if let Some(k) = opts.pca_components {
            if k < d {
                x = pca_reduce(&x, k)?;
            }
        }
        let dist = pairwise_sq(&x);
        let p = input_affinities(&dist, opts.perplexity);
        let dims = opts.dims;
        let mut y = if let Some(init) = &opts.initial_y {
            if init.cols() != dims {
                return Err(LangError::new(format!("tsne: InitialY must have {dims} columns"), 0));
            }
            init.clone()
        } else {
            let mut m = Matrix::zeros(n, dims);
            for r in 0..n {
                for c in 0..dims {
                    let _ = m.set(r, c, (self.next_f64() - 0.5) * 1e-4);
                }
            }
            m
        };
        let mut velocity = Matrix::zeros(n, dims);
        let early = opts.iterations.min(250);
        let mut loss = f64::NAN;
        for iter in 0..opts.iterations {
            let exaggeration = if iter < early { opts.exaggeration } else { 1.0 };
            let mut num = vec![vec![0.0f64; n]; n];
            let mut z = 0.0f64;
            for i in 0..n {
                for j in (i + 1)..n {
                    let mut sq = 0.0f64;
                    for c in 0..dims {
                        let diff = y.get(i, c).unwrap_or(0.0) - y.get(j, c).unwrap_or(0.0);
                        sq += diff * diff;
                    }
                    let w = 1.0 / (1.0 + sq.max(0.0));
                    if let Some(row) = num.get_mut(i) {
                        if let Some(slot) = row.get_mut(j) {
                            *slot = w;
                        }
                    }
                    if let Some(row) = num.get_mut(j) {
                        if let Some(slot) = row.get_mut(i) {
                            *slot = w;
                        }
                    }
                    z += 2.0 * w;
                }
            }
            let z = z.max(1e-12);
            let mut grad = Matrix::zeros(n, dims);
            for i in 0..n {
                for j in 0..n {
                    if i == j {
                        continue;
                    }
                    let q = num[i][j] / z;
                    let force = 4.0 * (p[i][j] * exaggeration - q) * num[i][j];
                    for c in 0..dims {
                        let diff = y.get(i, c).unwrap_or(0.0) - y.get(j, c).unwrap_or(0.0);
                        let g = grad.get(i, c).unwrap_or(0.0) + force * diff;
                        let _ = grad.set(i, c, g.clamp(-100.0, 100.0));
                    }
                }
            }
            let momentum = if iter < 250 { 0.5 } else { 0.8 };
            let lr = if iter < early { opts.learn_rate * 0.5 } else { opts.learn_rate };
            for r in 0..n {
                for c in 0..dims {
                    let v = momentum * velocity.get(r, c).unwrap_or(0.0) - lr * grad.get(r, c).unwrap_or(0.0);
                    let _ = velocity.set(r, c, v);
                    let _ = y.set(r, c, y.get(r, c).unwrap_or(0.0) + v);
                }
            }
            // Re-centre the embedding after each step.
            for c in 0..dims {
                let mean = (0..n).map(|r| y.get(r, c).unwrap_or(0.0)).sum::<f64>() / n as f64;
                for r in 0..n {
                    let _ = y.set(r, c, y.get(r, c).unwrap_or(0.0) - mean);
                }
            }
            if iter + 1 == opts.iterations {
                // KL(P || Q), omitting the constant log Z terms.
                let mut kl = 0.0f64;
                for i in 0..n {
                    for j in 0..n {
                        if i != j && p[i][j] > 0.0 {
                            let q = num[i][j] / z;
                            kl += p[i][j] * (p[i][j] / q.max(1e-12)).ln();
                        }
                    }
                }
                loss = kl;
            }
        }
        Ok(vec![Value::Mat(y), Value::Num(loss)].into_iter().take(nout.max(1)).collect())
    }
}
