//! Uniform Manifold Approximation and Projection (`umap`).
//!
//! A dependency-free implementation of the UMAP pipeline: k-nearest
//! neighbour graph, per-point bandwidth calibration to `log2(k)`, fuzzy
//! union of the directed graphs, low-dimensional random initialisation and
//! cross-entropy stochastic gradient descent with negative sampling.
//! Seeded through the session RNG (`rng`), so repeated calls are identical.

use super::*;

struct UmapOptions {
    n_neighbors: usize,
    min_dist: f64,
    n_components: usize,
    n_epochs: usize,
    spread: f64,
}

impl UmapOptions {
    fn defaults(n: usize) -> Self {
        UmapOptions { n_neighbors: 15.min(n.saturating_sub(1)).max(1), min_dist: 0.1, n_components: 2, n_epochs: 200, spread: 1.0 }
    }
}

fn parse_options(argv: &[Value], n: usize) -> Result<UmapOptions, LangError> {
    let mut o = UmapOptions::defaults(n);
    let mut i = 1usize;
    while i < argv.len() {
        let key = match &argv[i] {
            Value::Str(s) => s.to_ascii_lowercase().replace('_', ""),
            _ => return Err(LangError::new("umap options must be name-value pairs", 0)),
        };
        let v = argv.get(i + 1).ok_or_else(|| LangError::new(format!("umap: {key} needs a value"), 0))?.scalar()?;
        match key.as_str() {
            "nneighbors" | "numneighbors" => o.n_neighbors = v.max(1.0) as usize,
            "mindist" => o.min_dist = v.max(0.0),
            "ncomponents" | "numcomponents" => o.n_components = v.max(1.0) as usize,
            "nepochs" | "numepochs" => o.n_epochs = v.max(1.0) as usize,
            "spread" => o.spread = v.max(1e-3),
            other => return Err(LangError::new(format!("umap: unknown option '{other}'"), 0)),
        }
        i += 2;
    }
    o.n_neighbors = o.n_neighbors.min(n.saturating_sub(1)).max(1);
    o.n_components = o.n_components.min(64);
    o.n_epochs = o.n_epochs.min(10_000);
    Ok(o)
}

fn gauss(s: &mut Session) -> f64 {
    let u1 = s.next_f64().max(1e-300);
    let u2 = s.next_f64();
    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
}

fn shuffled(order: &mut [usize], s: &mut Session) {
    for i in (1..order.len()).rev() {
        let j = ((s.next_f64() * (i + 1) as f64) as usize) % (i + 1);
        order.swap(i, j);
    }
}

/// Fit `1 / (1 + a x^(2b))` to the UMAP membership curve.
fn fit_ab(min_dist: f64, spread: f64) -> (f64, f64) {
    let count = 300usize;
    let xs: Vec<f64> = (0..count).map(|i| 3.0 * spread * i as f64 / (count.saturating_sub(1)) as f64).collect();
    let target: Vec<f64> = xs.iter().map(|&x| if x <= min_dist { 1.0 } else { (-(x - min_dist) / spread).exp() }).collect();
    let mut best = (1.0f64, 1.0f64);
    let mut best_err = f64::INFINITY;
    let mut xpow = vec![0.0f64; count];
    for bi in 0..=180 {
        let b = 0.2 + 0.01 * bi as f64;
        for (i, &x) in xs.iter().enumerate() {
            if let Some(slot) = xpow.get_mut(i) {
                *slot = x.powf(2.0 * b);
            }
        }
        for ai in 0..=290 {
            let a = 0.1 + 0.01 * ai as f64;
            let mut err = 0.0f64;
            for i in 0..count {
                let f = 1.0 / (1.0 + a * xpow.get(i).copied().unwrap_or(0.0));
                let e = f - target.get(i).copied().unwrap_or(0.0);
                err += e * e;
            }
            if err < best_err {
                best_err = err;
                best = (a, b);
            }
        }
    }
    best
}

impl Session {
    pub(super) fn umap_call(&mut self, argv: &[Value], _nout: usize) -> Result<Vec<Value>, LangError> {
        let x = argv.first().ok_or_else(|| LangError::new("umap needs a data matrix", 0))?.to_numeric()?;
        let (n, d) = x.shape();
        if n < 4 || d < 1 {
            return Err(LangError::new("umap needs at least four observations and one variable", 0));
        }
        check_elems(n.saturating_mul(n))?;
        let opts = parse_options(argv, n)?;
        let k = opts.n_neighbors;

        // Pairwise Euclidean distances.
        let mut dist = vec![vec![0.0f64; n]; n];
        for i in 0..n {
            for j in (i + 1)..n {
                let mut s = 0.0f64;
                for c in 0..d {
                    let diff = x.get(i, c).unwrap_or(0.0) - x.get(j, c).unwrap_or(0.0);
                    s += diff * diff;
                }
                let s = s.sqrt();
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

        // k-nearest neighbours (self excluded).
        let mut neighbors: Vec<Vec<(usize, f64)>> = Vec::with_capacity(n);
        for i in 0..n {
            let mut cand: Vec<(usize, f64)> =
                (0..n).filter(|&j| j != i).map(|j| (j, dist.get(i).and_then(|r| r.get(j)).copied().unwrap_or(0.0))).collect();
            cand.sort_by(|a, b| a.1.total_cmp(&b.1));
            cand.truncate(k);
            neighbors.push(cand);
        }

        // Per-point bandwidth: sum exp(-(d-rho)/sigma) = log2(k).
        let target = (k as f64).ln();
        let mut rho = vec![0.0f64; n];
        let mut sigma = vec![1.0f64; n];
        for i in 0..n {
            let nearest = neighbors.get(i).and_then(|v| v.first()).map(|p| p.1).unwrap_or(0.0);
            if let Some(slot) = rho.get_mut(i) {
                *slot = nearest;
            }
            let sum_at =
                |s: f64| -> f64 { neighbors.get(i).map(|v| v.iter().map(|&(_, dij)| (-((dij - nearest).max(0.0)) / s).exp()).sum()).unwrap_or(0.0) };
            let mut lo = 0.0f64;
            let mut hi = 1.0f64;
            while sum_at(hi) < target && hi < 1e6 {
                hi *= 2.0;
            }
            for _ in 0..64 {
                let mid = 0.5 * (lo + hi);
                if sum_at(mid) < target {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            if let Some(slot) = sigma.get_mut(i) {
                *slot = (0.5 * (lo + hi)).max(1e-3);
            }
        }

        // Directed fuzzy weights, then the fuzzy union.
        let mut directed = vec![vec![0.0f64; n]; n];
        for i in 0..n {
            for &(j, dij) in neighbors.get(i).map(|v| v.as_slice()).unwrap_or(&[]) {
                let w = (-((dij - rho.get(i).copied().unwrap_or(0.0)).max(0.0)) / sigma.get(i).copied().unwrap_or(1.0)).exp();
                if let Some(row) = directed.get_mut(i) {
                    if let Some(slot) = row.get_mut(j) {
                        *slot = w;
                    }
                }
            }
        }
        let mut edges: Vec<(usize, usize, f64)> = Vec::new();
        for i in 0..n {
            for j in (i + 1)..n {
                let a = directed.get(i).and_then(|r| r.get(j)).copied().unwrap_or(0.0);
                let b = directed.get(j).and_then(|r| r.get(i)).copied().unwrap_or(0.0);
                let w = a + b - a * b;
                if w > 1e-6 {
                    edges.push((i, j, w));
                }
            }
        }
        if edges.is_empty() {
            return Err(LangError::new("umap: the neighbour graph is empty", 0));
        }

        let (a, b) = fit_ab(opts.min_dist, opts.spread);
        let dim = opts.n_components;
        let mut y: Vec<Vec<f64>> = (0..n).map(|_| (0..dim).map(|_| gauss(self) * 10.0).collect()).collect();

        // Cross-entropy SGD with negative sampling.
        let neg_rate = 5usize;
        let epochs = opts.n_epochs;
        let mut order: Vec<usize> = (0..edges.len()).collect();
        for epoch in 0..epochs {
            shuffled(&mut order, self);
            let progress = epoch as f64 / epochs.max(1) as f64;
            let lr = 0.01 + 0.99 * (1.0 - progress);
            for &ei in &order {
                let (i, j, w) = edges.get(ei).copied().unwrap_or((0, 0, 0.0));
                let mut dist_sq = 0.0f64;
                for c in 0..dim {
                    let diff = y.get(i).and_then(|r| r.get(c)).copied().unwrap_or(0.0) - y.get(j).and_then(|r| r.get(c)).copied().unwrap_or(0.0);
                    dist_sq += diff * diff;
                }
                if dist_sq > 1e-12 {
                    let coeff = -2.0 * a * b * dist_sq.powf(b - 1.0) / (a * dist_sq.powf(b) + 1.0);
                    for c in 0..dim {
                        let diff = y.get(i).and_then(|r| r.get(c)).copied().unwrap_or(0.0) - y.get(j).and_then(|r| r.get(c)).copied().unwrap_or(0.0);
                        let g = (w * coeff * diff).clamp(-4.0, 4.0);
                        if let Some(row) = y.get_mut(i) {
                            if let Some(slot) = row.get_mut(c) {
                                *slot += lr * g;
                            }
                        }
                        if let Some(row) = y.get_mut(j) {
                            if let Some(slot) = row.get_mut(c) {
                                *slot -= lr * g;
                            }
                        }
                    }
                }
                for _ in 0..neg_rate {
                    let mut neg = ((self.next_f64() * n as f64) as usize) % n;
                    if neg == i {
                        neg = (neg + 1) % n;
                    }
                    if neg == i || neg == j {
                        continue;
                    }
                    let mut nd = 0.0f64;
                    for c in 0..dim {
                        let diff =
                            y.get(i).and_then(|r| r.get(c)).copied().unwrap_or(0.0) - y.get(neg).and_then(|r| r.get(c)).copied().unwrap_or(0.0);
                        nd += diff * diff;
                    }
                    let coeff = 2.0 * b / ((0.001 + nd) * (a * nd.powf(b) + 1.0));
                    for c in 0..dim {
                        let diff =
                            y.get(i).and_then(|r| r.get(c)).copied().unwrap_or(0.0) - y.get(neg).and_then(|r| r.get(c)).copied().unwrap_or(0.0);
                        let g = (coeff * diff).clamp(-4.0, 4.0);
                        if let Some(row) = y.get_mut(i) {
                            if let Some(slot) = row.get_mut(c) {
                                *slot += lr * g;
                            }
                        }
                    }
                }
            }
        }

        let mut out = Matrix::zeros(n, dim);
        for i in 0..n {
            for c in 0..dim {
                let _ = out.set(i, c, y.get(i).and_then(|r| r.get(c)).copied().unwrap_or(0.0));
            }
        }
        Ok(vec![Value::Mat(out)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ab_curve_is_monotone() {
        let (a, b) = fit_ab(0.1, 1.0);
        assert!(a > 0.0 && b > 0.0);
        let f0 = 1.0 / (1.0 + a * 0.0f64.powf(2.0 * b));
        let f1 = 1.0 / (1.0 + a * 1.0f64.powf(2.0 * b));
        assert!((f0 - 1.0).abs() < 1e-12);
        assert!(f1 < 1.0);
    }
}
