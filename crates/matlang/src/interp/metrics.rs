#![allow(clippy::needless_range_loop)]

//! Distances, hierarchical clustering, cluster validation and set metrics.
//!
//! The distance kernels cover the common MATLAB `pdist`/`pdist2` metrics;
//! linkage trees use the existing `linkage` implementation, and the cluster
//! helpers turn a tree into labels, cophenetic distances or silhouettes.

use std::collections::{BTreeMap, BTreeSet};

use super::*;

#[derive(Clone, Debug)]
struct DistOpts {
    metric: String,
    p: f64,
    scale: Option<Vec<f64>>,
    cov: Option<Matrix>,
}

fn parse_dist_opts(argv: &[Value], start: usize, x: &Matrix) -> Result<DistOpts, LangError> {
    let mut o = DistOpts { metric: "euclidean".into(), p: 2.0, scale: None, cov: None };
    let mut i = start;
    while i < argv.len() {
        match &argv[i] {
            Value::Str(s) => {
                let key = s.to_ascii_lowercase();
                if matches!(
                    key.as_str(),
                    "euclidean"
                        | "squaredeuclidean"
                        | "cityblock"
                        | "manhattan"
                        | "chebychev"
                        | "chebyshev"
                        | "cosine"
                        | "correlation"
                        | "spearman"
                        | "hamming"
                        | "jaccard"
                        | "dice"
                        | "matching"
                        | "canberra"
                        | "braycurtis"
                        | "seuclidean"
                        | "mahalanobis"
                        | "minkowski"
                ) {
                    o.metric = key;
                    i += 1;
                } else if key == "scale" || key == "seuclidean" {
                    if let Some(v) = argv.get(i + 1) {
                        let m = v.to_numeric()?;
                        o.scale = Some(m.data().to_vec());
                        i += 2;
                    } else {
                        return Err(LangError::new("pdist: Scale needs a vector", 0));
                    }
                } else if key == "cov" || key == "mahalanobis" {
                    if let Some(v) = argv.get(i + 1) {
                        o.metric = "mahalanobis".into();
                        o.cov = Some(v.to_numeric()?);
                        i += 2;
                    } else {
                        return Err(LangError::new("pdist: Cov needs a matrix", 0));
                    }
                } else {
                    return Err(LangError::new(format!("pdist: unknown metric/option '{s}'"), 0));
                }
            }
            v => {
                o.p = v.scalar()?;
                if !o.p.is_finite() || o.p <= 0.0 {
                    return Err(LangError::new("pdist: Minkowski p must be positive", 0));
                }
                o.metric = "minkowski".into();
                i += 1;
            }
        }
    }
    if o.metric == "seuclidean" && o.scale.is_none() {
        let mut s = vec![0.0f64; x.cols()];
        let n = x.rows().max(1) as f64;
        for c in 0..x.cols() {
            let mean = (0..x.rows()).map(|r| x.get(r, c).unwrap_or(0.0)).sum::<f64>() / n;
            let var = (0..x.rows()).map(|r| (x.get(r, c).unwrap_or(0.0) - mean).powi(2)).sum::<f64>() / n;
            if let Some(slot) = s.get_mut(c) {
                *slot = var.sqrt().max(1e-12);
            }
        }
        o.scale = Some(s);
    }
    if o.metric == "mahalanobis" && o.cov.is_none() {
        o.cov = Some(covariance_inverse(x)?);
    }
    Ok(o)
}

fn covariance_inverse(x: &Matrix) -> Result<Matrix, LangError> {
    let n = x.rows();
    let p = x.cols();
    if n < 2 || p == 0 {
        return Err(LangError::new("mahalanobis needs at least two observations", 0));
    }
    let mut mean = vec![0.0f64; p];
    for r in 0..n {
        for (c, m) in mean.iter_mut().enumerate() {
            *m += x.get(r, c).unwrap_or(0.0);
        }
    }
    for m in &mut mean {
        *m /= n as f64;
    }
    let mut cov = Matrix::zeros(p, p);
    for r in 0..n {
        for i in 0..p {
            for j in 0..p {
                let v = (x.get(r, i).unwrap_or(0.0) - mean.get(i).copied().unwrap_or(0.0))
                    * (x.get(r, j).unwrap_or(0.0) - mean.get(j).copied().unwrap_or(0.0));
                let _ = cov.set(i, j, cov.get(i, j).unwrap_or(0.0) + v);
            }
        }
    }
    cov = cov.map(|v| v / (n - 1) as f64);
    cov.inverse().map_err(lang)
}

fn distance(a: &[f64], b: &[f64], o: &DistOpts) -> Result<f64, LangError> {
    let n = a.len().min(b.len());
    let binary = |v: f64| v != 0.0;
    let d = match o.metric.as_str() {
        "euclidean" => a.iter().zip(b).take(n).map(|(x, y)| (x - y) * (x - y)).sum::<f64>().sqrt(),
        "squaredeuclidean" => a.iter().zip(b).take(n).map(|(x, y)| (x - y) * (x - y)).sum::<f64>(),
        "cityblock" | "manhattan" => a.iter().zip(b).take(n).map(|(x, y)| (x - y).abs()).sum::<f64>(),
        "chebychev" | "chebyshev" => a.iter().zip(b).take(n).map(|(x, y)| (x - y).abs()).fold(0.0f64, f64::max),
        "minkowski" => a.iter().zip(b).take(n).map(|(x, y)| (x - y).abs().powf(o.p)).sum::<f64>().powf(1.0 / o.p),
        "cosine" => {
            let dot = a.iter().zip(b).take(n).map(|(x, y)| x * y).sum::<f64>();
            let na = a.iter().take(n).map(|x| x * x).sum::<f64>().sqrt();
            let nb = b.iter().take(n).map(|x| x * x).sum::<f64>().sqrt();
            if na > 0.0 && nb > 0.0 {
                1.0 - dot / (na * nb)
            } else {
                f64::NAN
            }
        }
        "correlation" => {
            let mean_a = a.iter().take(n).sum::<f64>() / n.max(1) as f64;
            let mean_b = b.iter().take(n).sum::<f64>() / n.max(1) as f64;
            let dot = a.iter().zip(b).take(n).map(|(x, y)| (x - mean_a) * (y - mean_b)).sum::<f64>();
            let va = a.iter().take(n).map(|x| (x - mean_a).powi(2)).sum::<f64>();
            let vb = b.iter().take(n).map(|x| (x - mean_b).powi(2)).sum::<f64>();
            if va > 0.0 && vb > 0.0 {
                1.0 - dot / (va * vb).sqrt()
            } else {
                f64::NAN
            }
        }
        "spearman" => {
            let rank = |v: &[f64]| -> Vec<f64> {
                let mut idx: Vec<usize> = (0..v.len()).collect();
                idx.sort_by(|&i, &j| v.get(i).unwrap_or(&0.0).total_cmp(v.get(j).unwrap_or(&0.0)));
                let mut r = vec![0.0; v.len()];
                let mut k = 0;
                while k < idx.len() {
                    let mut end = k + 1;
                    while end < idx.len() && (v.get(idx[end]).copied().unwrap_or(0.0) - v.get(idx[k]).copied().unwrap_or(0.0)).abs() < 1e-12 {
                        end += 1;
                    }
                    let avg = (k + end + 1) as f64 / 2.0;
                    for &i in idx.get(k..end).unwrap_or(&[]) {
                        if let Some(slot) = r.get_mut(i) {
                            *slot = avg;
                        }
                    }
                    k = end;
                }
                r
            };
            let ra = rank(&a[..n]);
            let rb = rank(&b[..n]);
            let ma = ra.iter().sum::<f64>() / n.max(1) as f64;
            let mb = rb.iter().sum::<f64>() / n.max(1) as f64;
            let dot = ra.iter().zip(&rb).map(|(x, y)| (x - ma) * (y - mb)).sum::<f64>();
            let va = ra.iter().map(|x| (x - ma).powi(2)).sum::<f64>();
            let vb = rb.iter().map(|x| (x - mb).powi(2)).sum::<f64>();
            if va > 0.0 && vb > 0.0 {
                1.0 - dot / (va * vb).sqrt()
            } else {
                f64::NAN
            }
        }
        "hamming" => a.iter().zip(b).take(n).filter(|(x, y)| (**x - **y).abs() > 1e-12).count() as f64 / n.max(1) as f64,
        "jaccard" => {
            let (mut both, mut either) = (0usize, 0usize);
            for (x, y) in a.iter().zip(b).take(n) {
                let (x, y) = (binary(*x), binary(*y));
                if x && y {
                    both += 1;
                }
                if x || y {
                    either += 1;
                }
            }
            if either == 0 {
                0.0
            } else {
                1.0 - both as f64 / either as f64
            }
        }
        "dice" => {
            let (mut both, mut total) = (0usize, 0usize);
            for (x, y) in a.iter().zip(b).take(n) {
                if binary(*x) && binary(*y) {
                    both += 1;
                }
                total += binary(*x) as usize + binary(*y) as usize;
            }
            if total == 0 {
                0.0
            } else {
                1.0 - 2.0 * both as f64 / total as f64
            }
        }
        "matching" => {
            let (mut same, mut diff) = (0usize, 0usize);
            for (x, y) in a.iter().zip(b).take(n) {
                if binary(*x) == binary(*y) {
                    same += 1;
                } else {
                    diff += 1;
                }
            }
            let _ = same;
            diff as f64 / n.max(1) as f64
        }
        "canberra" => a.iter().zip(b).take(n).map(|(x, y)| (x - y).abs() / (x.abs() + y.abs()).max(1e-12)).sum::<f64>(),
        "braycurtis" => {
            let num: f64 = a.iter().zip(b).take(n).map(|(x, y)| (x - y).abs()).sum();
            let den: f64 = a.iter().zip(b).take(n).map(|(x, y)| (x + y).abs()).sum();
            if den > 0.0 {
                num / den
            } else {
                0.0
            }
        }
        "seuclidean" => {
            let scale = o.scale.as_ref().ok_or_else(|| LangError::new("seuclidean needs Scale", 0))?;
            a.iter()
                .zip(b)
                .take(n)
                .enumerate()
                .map(|(i, (x, y))| ((x - y) / scale.get(i).copied().unwrap_or(1.0).max(1e-12)).powi(2))
                .sum::<f64>()
                .sqrt()
        }
        "mahalanobis" => {
            let cov = o.cov.as_ref().ok_or_else(|| LangError::new("mahalanobis needs Cov", 0))?;
            let mut s = 0.0;
            for i in 0..n {
                for j in 0..n {
                    s += (a.get(i).copied().unwrap_or(0.0) - b.get(i).copied().unwrap_or(0.0))
                        * cov.get(i, j).unwrap_or(0.0)
                        * (a.get(j).copied().unwrap_or(0.0) - b.get(j).copied().unwrap_or(0.0));
                }
            }
            s.max(0.0).sqrt()
        }
        other => return Err(LangError::new(format!("pdist: unsupported metric '{other}'"), 0)),
    };
    Ok(d)
}

fn pdist_matrix(x: &Matrix, o: &DistOpts) -> Result<Vec<f64>, LangError> {
    let n = x.rows();
    let a: Vec<Vec<f64>> = (0..n).map(|r| (0..x.cols()).map(|c| x.get(r, c).unwrap_or(0.0)).collect()).collect();
    let mut out = Vec::new();
    for i in 0..n {
        for j in i + 1..n {
            out.push(distance(a.get(i).map(Vec::as_slice).unwrap_or(&[]), a.get(j).map(Vec::as_slice).unwrap_or(&[]), o)?);
        }
    }
    Ok(out)
}

pub(super) fn set_of(v: &Value) -> Result<BTreeSet<String>, LangError> {
    match v {
        Value::Num(_) | Value::Mat(_) => {
            let m = v.to_numeric()?;
            Ok(m.data().iter().map(|x| fmt_num(*x, false)).collect())
        }
        Value::Str(s) => Ok([s.clone()].into_iter().collect()),
        Value::Cell(c) => {
            let mut out = BTreeSet::new();
            for i in 0..c.numel() {
                match c.get_linear(i) {
                    Some(Value::Str(s)) => {
                        out.insert(s.clone());
                    }
                    Some(other) => {
                        if let Ok(m) = other.to_numeric() {
                            for x in m.data() {
                                out.insert(fmt_num(*x, false));
                            }
                        }
                    }
                    None => {}
                }
            }
            Ok(out)
        }
        _ => Err(LangError::new("expected a numeric array or cellstr", 0)),
    }
}

fn label_map(v: &Value) -> Result<Vec<String>, LangError> {
    match v {
        Value::Num(_) | Value::Mat(_) => {
            let m = v.to_numeric()?;
            Ok(m.data().iter().map(|x| fmt_num(*x, false)).collect())
        }
        Value::Cell(c) => {
            let mut out = Vec::with_capacity(c.numel());
            for i in 0..c.numel() {
                match c.get_linear(i) {
                    Some(Value::Str(s)) => out.push(s.clone()),
                    Some(other) => {
                        let m = other.to_numeric()?;
                        out.push(fmt_num(m.get_linear(0).unwrap_or(0.0), false));
                    }
                    None => out.push(String::new()),
                }
            }
            Ok(out)
        }
        Value::Str(s) => Ok(vec![s.clone()]),
        _ => Err(LangError::new("labels must be numeric or cellstr", 0)),
    }
}

type LinkageMerges = Vec<(usize, usize, f64)>;

fn parse_linkage(z: &Matrix) -> Result<(usize, LinkageMerges), LangError> {
    if z.cols() != 3 {
        return Err(LangError::new("linkage matrix must have three columns", 0));
    }
    let merges = z.rows();
    let n = merges + 1;
    let mut out = Vec::with_capacity(merges);
    for r in 0..merges {
        let a = z.get(r, 0).unwrap_or(0.0).round();
        let b = z.get(r, 1).unwrap_or(0.0).round();
        let h = z.get(r, 2).unwrap_or(0.0);
        if a < 1.0 || b < 1.0 || a > (n + r) as f64 || b > (n + r) as f64 {
            return Err(LangError::new("linkage matrix contains invalid cluster ids", 0));
        }
        out.push((a as usize, b as usize, h));
    }
    Ok((n, out))
}

pub(super) fn cluster_from_linkage(z: &Matrix, cutoff: f64, maxclust: Option<usize>) -> Result<Vec<f64>, LangError> {
    let (n, merges) = parse_linkage(z)?;
    let mut parent: Vec<usize> = (0..=(2 * n)).collect();
    let find = |parent: &mut Vec<usize>, mut x: usize| -> usize {
        while parent.get(x).copied().unwrap_or(x) != x {
            let next = parent.get(x).copied().unwrap_or(x);
            parent[x] = parent.get(next).copied().unwrap_or(next);
            x = next;
        }
        x
    };
    let mut count = n;
    for (i, (a, b, h)) in merges.iter().enumerate() {
        if let Some(k) = maxclust {
            if count <= k {
                break;
            }
        } else if *h > cutoff {
            break;
        }
        let ra = find(&mut parent, *a);
        let rb = find(&mut parent, *b);
        let new = n + i + 1;
        parent[ra] = new;
        parent[rb] = new;
        parent[new] = new;
        count = count.saturating_sub(1);
    }
    let mut roots: Vec<usize> = (1..=n).map(|i| find(&mut parent, i)).collect();
    let mut map: BTreeMap<usize, usize> = BTreeMap::new();
    for r in &mut roots {
        let next = map.len() + 1;
        let id = *map.entry(*r).or_insert(next);
        *r = id;
    }
    Ok(roots.into_iter().map(|x| x as f64).collect())
}

fn cophenetic_distances(z: &Matrix, n: usize) -> Result<Vec<f64>, LangError> {
    let (_, merges) = parse_linkage(z)?;
    let mut parent: Vec<Option<usize>> = vec![None; 2 * n + 2];
    let mut height = vec![0.0f64; 2 * n + 2];
    for (i, (a, b, h)) in merges.iter().enumerate() {
        let new = n + i + 1;
        parent[*a] = Some(new);
        parent[*b] = Some(new);
        height[new] = *h;
    }
    let lca_height = |mut a: usize, mut b: usize| -> f64 {
        let mut seen = BTreeSet::new();
        while let Some(p) = parent.get(a).copied().flatten() {
            seen.insert(p);
            a = p;
        }
        while let Some(p) = parent.get(b).copied().flatten() {
            if seen.contains(&p) {
                return height[p];
            }
            b = p;
        }
        height[b]
    };
    let mut out = Vec::new();
    for i in 1..=n {
        for j in i + 1..=n {
            out.push(lca_height(i, j));
        }
    }
    Ok(out)
}

fn corrcoef(x: &[f64], y: &[f64]) -> f64 {
    let n = x.len().min(y.len());
    if n < 2 {
        return f64::NAN;
    }
    let mx = x.iter().take(n).sum::<f64>() / n as f64;
    let my = y.iter().take(n).sum::<f64>() / n as f64;
    let dx: f64 = x.iter().take(n).map(|v| (v - mx).powi(2)).sum();
    let dy: f64 = y.iter().take(n).map(|v| (v - my).powi(2)).sum();
    let dxy: f64 = x.iter().take(n).zip(y).map(|(a, b)| (a - mx) * (b - my)).sum();
    if dx <= 0.0 || dy <= 0.0 {
        f64::NAN
    } else {
        dxy / (dx * dy).sqrt()
    }
}

fn cluster_scores(true_labels: &[String], pred_labels: &[String]) -> (f64, f64, f64) {
    let n = true_labels.len().min(pred_labels.len());
    if n == 0 {
        return (f64::NAN, f64::NAN, f64::NAN);
    }
    let mut tmap: BTreeMap<&str, usize> = BTreeMap::new();
    let mut pmap: BTreeMap<&str, usize> = BTreeMap::new();
    let mut tv = Vec::with_capacity(n);
    let mut pv = Vec::with_capacity(n);
    for i in 0..n {
        let t = true_labels.get(i).map(String::as_str).unwrap_or("");
        let p = pred_labels.get(i).map(String::as_str).unwrap_or("");
        let next_t = tmap.len();
        let ti = *tmap.entry(t).or_insert(next_t);
        let next_p = pmap.len();
        let pi = *pmap.entry(p).or_insert(next_p);
        tv.push(ti);
        pv.push(pi);
    }
    let kt = tmap.len();
    let kp = pmap.len();
    let mut c = vec![vec![0usize; kp]; kt];
    let mut a = vec![0usize; kt];
    let mut b = vec![0usize; kp];
    for i in 0..n {
        let ti = tv.get(i).copied().unwrap_or(0);
        let pi = pv.get(i).copied().unwrap_or(0);
        if let Some(row) = c.get_mut(ti) {
            if let Some(slot) = row.get_mut(pi) {
                *slot += 1;
            }
        }
        if let Some(slot) = a.get_mut(ti) {
            *slot += 1;
        }
        if let Some(slot) = b.get_mut(pi) {
            *slot += 1;
        }
    }
    let comb2 = |x: usize| (x as f64) * (x as f64 - 1.0) / 2.0;
    let sum_ij: f64 = c.iter().flatten().map(|v| comb2(*v)).sum();
    let sum_a: f64 = a.iter().map(|v| comb2(*v)).sum();
    let sum_b: f64 = b.iter().map(|v| comb2(*v)).sum();
    let total = comb2(n);
    let expected = if total > 0.0 { sum_a * sum_b / total } else { 0.0 };
    let max = 0.5 * (sum_a + sum_b);
    let ari = if (max - expected).abs() > 1e-15 { (sum_ij - expected) / (max - expected) } else { 1.0 };
    let purity = c.iter().map(|row| row.iter().copied().max().unwrap_or(0)).sum::<usize>() as f64 / n as f64;
    let mut mi = 0.0;
    for (i, row) in c.iter().enumerate() {
        for (j, &v) in row.iter().enumerate() {
            if v == 0 {
                continue;
            }
            let pij = v as f64 / n as f64;
            let pi = a.get(i).copied().unwrap_or(0) as f64 / n as f64;
            let pj = b.get(j).copied().unwrap_or(0) as f64 / n as f64;
            if pi > 0.0 && pj > 0.0 {
                mi += pij * (pij / (pi * pj)).ln();
            }
        }
    }
    let h = |v: &[usize]| -> f64 {
        v.iter()
            .filter(|x| **x > 0)
            .map(|x| {
                let p = *x as f64 / n as f64;
                -p * p.ln()
            })
            .sum()
    };
    let ht = h(&a);
    let hp = h(&b);
    let nmi = if ht > 0.0 && hp > 0.0 { mi / (ht * hp).sqrt() } else { 0.0 };
    (ari, nmi, purity)
}

impl Session {
    pub(super) fn metrics_builtin(&mut self, name: &str, argv: Vec<Value>, nout: usize) -> Result<Vec<Value>, LangError> {
        match name {
            "pdist" => {
                let x = argv.first().ok_or_else(|| LangError::new("pdist needs data", 0))?.to_numeric()?;
                let o = parse_dist_opts(&argv, 1, &x)?;
                Ok(vec![Value::Mat(Matrix::row_vector(pdist_matrix(&x, &o)?))])
            }
            "pdist2" => {
                let x = argv.first().ok_or_else(|| LangError::new("pdist2 needs X", 0))?.to_numeric()?;
                let y = argv.get(1).ok_or_else(|| LangError::new("pdist2 needs Y", 0))?.to_numeric()?;
                let mut opts = parse_dist_opts(&argv, 2, &x)?;
                if opts.metric == "seuclidean" && opts.scale.is_none() {
                    opts.scale = Some(
                        (0..x.cols())
                            .map(|c| {
                                let mean = (0..x.rows()).map(|r| x.get(r, c).unwrap_or(0.0)).sum::<f64>() / x.rows().max(1) as f64;
                                ((0..x.rows()).map(|r| (x.get(r, c).unwrap_or(0.0) - mean).powi(2)).sum::<f64>() / x.rows().max(1) as f64)
                                    .sqrt()
                                    .max(1e-12)
                            })
                            .collect(),
                    );
                }
                let mut out = Matrix::zeros(x.rows(), y.rows());
                for i in 0..x.rows() {
                    let a: Vec<f64> = (0..x.cols()).map(|c| x.get(i, c).unwrap_or(0.0)).collect();
                    for j in 0..y.rows() {
                        let b: Vec<f64> = (0..y.cols()).map(|c| y.get(j, c).unwrap_or(0.0)).collect();
                        let _ = out.set(i, j, distance(&a, &b, &opts)?);
                    }
                }
                Ok(vec![Value::Mat(out)])
            }
            "jaccard" | "jaccarddistance" => {
                let av = argv.first().ok_or_else(|| LangError::new("jaccard needs A", 0))?;
                let bv = argv.get(1).ok_or_else(|| LangError::new("jaccard needs B", 0))?;
                let j = if let (Ok(am), Ok(bm)) = (av.to_numeric(), bv.to_numeric()) {
                    if am.numel() == bm.numel() && am.numel() > 0 && am.data().iter().chain(bm.data()).all(|v| *v == 0.0 || *v == 1.0) {
                        let mut both = 0usize;
                        let mut either = 0usize;
                        for (x, y) in am.data().iter().zip(bm.data()) {
                            if *x != 0.0 && *y != 0.0 {
                                both += 1;
                            }
                            if *x != 0.0 || *y != 0.0 {
                                either += 1;
                            }
                        }
                        if either == 0 {
                            1.0
                        } else {
                            both as f64 / either as f64
                        }
                    } else {
                        let a = set_of(av)?;
                        let b = set_of(bv)?;
                        let inter = a.intersection(&b).count() as f64;
                        let union = a.union(&b).count() as f64;
                        if union > 0.0 {
                            inter / union
                        } else {
                            1.0
                        }
                    }
                } else {
                    let a = set_of(av)?;
                    let b = set_of(bv)?;
                    let inter = a.intersection(&b).count() as f64;
                    let union = a.union(&b).count() as f64;
                    if union > 0.0 {
                        inter / union
                    } else {
                        1.0
                    }
                };
                Ok(vec![Value::Num(if name == "jaccarddistance" { 1.0 - j } else { j })])
            }
            "setmetrics" => {
                let a = set_of(argv.first().ok_or_else(|| LangError::new("setmetrics needs reference", 0))?)?;
                let b = set_of(argv.get(1).ok_or_else(|| LangError::new("setmetrics needs candidate", 0))?)?;
                let tp = a.intersection(&b).count() as f64;
                let fp = b.difference(&a).count() as f64;
                let fneg = a.difference(&b).count() as f64;
                let precision = if tp + fp > 0.0 { tp / (tp + fp) } else { 0.0 };
                let recall = if tp + fneg > 0.0 { tp / (tp + fneg) } else { 0.0 };
                let f1 = if precision + recall > 0.0 { 2.0 * precision * recall / (precision + recall) } else { 0.0 };
                let union = a.union(&b).count() as f64;
                let j = if union > 0.0 { tp / union } else { 1.0 };
                let mut s = StructArray::new();
                s.set_field(0, "TP", Value::Num(tp))?;
                s.set_field(0, "FP", Value::Num(fp))?;
                s.set_field(0, "FN", Value::Num(fneg))?;
                s.set_field(0, "Precision", Value::Num(precision))?;
                s.set_field(0, "Recall", Value::Num(recall))?;
                s.set_field(0, "F1", Value::Num(f1))?;
                s.set_field(0, "Jaccard", Value::Num(j))?;
                s.set_field(0, "Coverage", Value::Num(recall))?;
                s.set_field(0, "Novelty", Value::Num(if b.is_empty() { 0.0 } else { fp / b.len() as f64 }))?;
                Ok(vec![Value::Struct(s)])
            }
            "confusionmat" => {
                let tv = argv.first().ok_or_else(|| LangError::new("confusionmat needs true labels", 0))?;
                let pv = argv.get(1).ok_or_else(|| LangError::new("confusionmat needs predicted labels", 0))?;
                let numeric = matches!(tv, Value::Num(_) | Value::Mat(_)) && matches!(pv, Value::Num(_) | Value::Mat(_));
                let t = label_map(tv)?;
                let p = label_map(pv)?;
                if t.len() != p.len() {
                    return Err(LangError::new("confusionmat: label vectors must have the same length", 0));
                }
                let mut labels: Vec<String> = t.iter().chain(p.iter()).cloned().collect();
                if numeric {
                    labels.sort_by(|a, b| a.parse::<f64>().unwrap_or(0.0).total_cmp(&b.parse::<f64>().unwrap_or(0.0)));
                } else {
                    labels.sort();
                }
                labels.dedup();
                let mut c = Matrix::zeros(labels.len(), labels.len());
                for (a, b) in t.iter().zip(p.iter()) {
                    let i = labels.iter().position(|x| x == a).unwrap_or(0);
                    let j = labels.iter().position(|x| x == b).unwrap_or(0);
                    let _ = c.set(i, j, c.get(i, j).unwrap_or(0.0) + 1.0);
                }
                Ok(vec![Value::Mat(c), Value::Cell(CellArray::row(labels.into_iter().map(Value::Str).collect()))])
            }
            "clustermetrics" => {
                let t = label_map(argv.first().ok_or_else(|| LangError::new("clustermetrics needs true labels", 0))?)?;
                let p = label_map(argv.get(1).ok_or_else(|| LangError::new("clustermetrics needs predicted labels", 0))?)?;
                if t.len() != p.len() {
                    return Err(LangError::new("clustermetrics: label vectors must have the same length", 0));
                }
                let (ari, nmi, purity) = cluster_scores(&t, &p);
                let mut s = StructArray::new();
                s.set_field(0, "ARI", Value::Num(ari))?;
                s.set_field(0, "NMI", Value::Num(nmi))?;
                s.set_field(0, "Purity", Value::Num(purity))?;
                Ok(vec![Value::Struct(s)])
            }
            "cluster" => {
                let z = argv.first().ok_or_else(|| LangError::new("cluster needs a linkage matrix", 0))?.to_numeric()?;
                let mut cutoff = f64::INFINITY;
                let mut maxclust = None;
                if let Some(v) = argv.get(1) {
                    match v {
                        Value::Str(s) if s.eq_ignore_ascii_case("maxclust") => {
                            let k = argv.get(2).ok_or_else(|| LangError::new("cluster: maxclust needs K", 0))?.scalar()?.round();
                            maxclust = Some(k.max(1.0) as usize);
                        }
                        Value::Str(s) if s.eq_ignore_ascii_case("cutoff") => {
                            cutoff = argv.get(2).ok_or_else(|| LangError::new("cluster: cutoff needs a value", 0))?.scalar()?;
                        }
                        _ => cutoff = v.scalar()?,
                    }
                }
                let labels = cluster_from_linkage(&z, cutoff, maxclust)?;
                Ok(vec![Value::Mat(Matrix::col_vector(labels))])
            }
            "clusterdata" => {
                let x = argv.first().ok_or_else(|| LangError::new("clusterdata needs data", 0))?.to_numeric()?;
                let cutoff = argv.get(1).ok_or_else(|| LangError::new("clusterdata needs a cutoff or K", 0))?.scalar()?;
                let mut metric = "euclidean".to_string();
                let mut method = "single".to_string();
                let mut i = 2;
                while i + 1 < argv.len() {
                    if let Value::Str(k) = &argv[i] {
                        let v = argv
                            .get(i + 1)
                            .and_then(|x| match x {
                                Value::Str(s) => Some(s.clone()),
                                _ => None,
                            })
                            .unwrap_or_default();
                        match k.to_ascii_lowercase().as_str() {
                            "distance" => metric = v.to_ascii_lowercase(),
                            "linkage" => method = v.to_ascii_lowercase(),
                            other => return Err(LangError::new(format!("clusterdata: unknown option '{other}'"), 0)),
                        }
                    }
                    i += 2;
                }
                let opts = DistOpts { metric, p: 2.0, scale: None, cov: None };
                let d = pdist_matrix(&x, &opts)?;
                let lv = self.mathx_builtin("linkage", vec![Value::Mat(Matrix::row_vector(d)), Value::Str(method)])?;
                let z = lv.first().ok_or_else(|| LangError::new("clusterdata: linkage failed", 0))?.to_numeric()?;
                let maxclust = if cutoff.fract() == 0.0 && cutoff >= 1.0 { Some(cutoff as usize) } else { None };
                let labels = cluster_from_linkage(&z, cutoff, maxclust)?;
                Ok(vec![Value::Mat(Matrix::col_vector(labels))])
            }
            "cophenet" => {
                let z = argv.first().ok_or_else(|| LangError::new("cophenet needs a linkage matrix", 0))?.to_numeric()?;
                let (n, _) = parse_linkage(&z)?;
                let c = cophenetic_distances(&z, n)?;
                if let Some(y) = argv.get(1) {
                    let y = y.to_numeric()?;
                    Ok(vec![Value::Num(corrcoef(&c, y.data()))])
                } else {
                    Ok(vec![Value::Mat(Matrix::row_vector(c))])
                }
            }
            "silhouette" => {
                let x = argv.first().ok_or_else(|| LangError::new("silhouette needs data", 0))?.to_numeric()?;
                let labels = label_map(argv.get(1).ok_or_else(|| LangError::new("silhouette needs labels", 0))?)?;
                if labels.len() != x.rows() {
                    return Err(LangError::new("silhouette: labels must match the number of rows", 0));
                }
                let opts = DistOpts { metric: "euclidean".into(), p: 2.0, scale: None, cov: None };
                let n = x.rows();
                let rows: Vec<Vec<f64>> = (0..n).map(|r| (0..x.cols()).map(|c| x.get(r, c).unwrap_or(0.0)).collect()).collect();
                let mut d = vec![vec![0.0f64; n]; n];
                for i in 0..n {
                    for j in i + 1..n {
                        let v = distance(rows.get(i).map(Vec::as_slice).unwrap_or(&[]), rows.get(j).map(Vec::as_slice).unwrap_or(&[]), &opts)?;
                        d[i][j] = v;
                        d[j][i] = v;
                    }
                }
                let mut s = vec![0.0f64; n];
                for i in 0..n {
                    let li = labels.get(i).cloned().unwrap_or_default();
                    let mut groups: BTreeMap<String, (f64, usize)> = BTreeMap::new();
                    for j in 0..n {
                        if i == j {
                            continue;
                        }
                        let key = labels.get(j).cloned().unwrap_or_default();
                        let entry = groups.entry(key).or_insert((0.0, 0));
                        entry.0 += d[i][j];
                        entry.1 += 1;
                    }
                    let a = groups.get(&li).map(|(sum, count)| if *count > 0 { sum / *count as f64 } else { 0.0 }).unwrap_or(0.0);
                    let b = groups
                        .iter()
                        .filter(|(k, _)| **k != li)
                        .map(|(_, (sum, count))| if *count > 0 { sum / *count as f64 } else { f64::INFINITY })
                        .fold(f64::INFINITY, f64::min);
                    s[i] = if groups.get(&li).map(|(_, c)| *c).unwrap_or(0) == 0 || !b.is_finite() { 0.0 } else { (b - a) / a.max(b).max(1e-12) };
                }
                let mean = s.iter().sum::<f64>() / n.max(1) as f64;
                if nout >= 2 {
                    Ok(vec![Value::Mat(Matrix::col_vector(s)), Value::Num(mean)])
                } else {
                    Ok(vec![Value::Mat(Matrix::col_vector(s))])
                }
            }
            _ => Err(LangError::new(format!("{name}: not a metrics builtin"), 0)),
        }
    }
}
