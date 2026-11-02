//! Analysis-of-variance builtins: `anova1`, `anova2`, `multcompare`,
//! `friedman`.
//!
//! `anova1` is one-way ANOVA over matrix columns, a cell array of groups,
//! or a vector plus group labels. `anova2` is two-way ANOVA with or without
//! replication. `multcompare` performs pairwise comparisons using LSD,
//! Bonferroni, Sidak, Holm or a Tukey–Kramer studentized-range
//! approximation. `friedman` is the nonparametric two-way rank test.

#![allow(clippy::needless_range_loop, clippy::type_complexity)]

use super::*;
use matcore::stats;

fn vector_values(v: &Value, what: &str) -> Result<Vec<f64>, LangError> {
    let m = v.to_numeric()?;
    if m.rows() > 1 && m.cols() > 1 {
        return Err(LangError::new(format!("{what} must be a vector"), 0));
    }
    Ok(m.data().iter().copied().filter(|x| x.is_finite()).collect())
}

fn cell_strings(v: &Value) -> Vec<String> {
    match v {
        Value::Cell(c) => c
            .items
            .iter()
            .filter_map(|x| match x {
                Value::Str(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        Value::Str(s) => vec![s.clone()],
        _ => Vec::new(),
    }
}

fn anova1_groups(x: &Value, group: Option<&Value>) -> Result<(Vec<Vec<f64>>, Vec<String>), LangError> {
    if let Value::Cell(c) = x {
        let mut groups = Vec::new();
        for item in &c.items {
            groups.push(vector_values(item, "group")?);
        }
        let names =
            group.map(cell_strings).filter(|n| n.len() == groups.len()).unwrap_or_else(|| (1..=groups.len()).map(|i| format!("Group{i}")).collect());
        return Ok((groups, names));
    }
    let m = x.to_numeric()?;
    if let Some(g) = group {
        if let Value::Cell(names) = g {
            if names.items.len() == m.cols() && m.cols() > 1 {
                let groups: Vec<Vec<f64>> =
                    (0..m.cols()).map(|c| (0..m.rows()).map(|r| m.get(r, c).unwrap_or(f64::NAN)).filter(|v| v.is_finite()).collect()).collect();
                return Ok((groups, cell_strings(g)));
            }
        }
        let labels = vector_values(g, "group")?;
        if labels.len() != m.numel() {
            return Err(LangError::new("group labels must match the data length", 0));
        }
        let mut unique: Vec<f64> = Vec::new();
        for &l in &labels {
            if !unique.contains(&l) {
                unique.push(l);
            }
        }
        let mut groups = vec![Vec::new(); unique.len()];
        for (i, &l) in labels.iter().enumerate() {
            if let Some(pos) = unique.iter().position(|u| *u == l) {
                if let Some(slot) = groups.get_mut(pos) {
                    if let Some(v) = m.get_linear(i) {
                        if v.is_finite() {
                            slot.push(v);
                        }
                    }
                }
            }
        }
        let names = unique.iter().map(|v| format!("{v}")).collect();
        return Ok((groups, names));
    }
    if m.cols() > 1 {
        let groups: Vec<Vec<f64>> =
            (0..m.cols()).map(|c| (0..m.rows()).map(|r| m.get(r, c).unwrap_or(f64::NAN)).filter(|v| v.is_finite()).collect()).collect();
        let names = (1..=m.cols()).map(|i| format!("Column{i}")).collect();
        Ok((groups, names))
    } else {
        Err(LangError::new("anova1 needs a matrix of groups or a vector plus group labels", 0))
    }
}

fn anova_table(rows: &[(&str, f64, f64, Option<f64>, Option<f64>, Option<f64>)]) -> Result<Value, LangError> {
    let mut c = CellArray::new(rows.len() + 1, 6);
    let headers = ["Source", "SS", "df", "MS", "F", "Prob>F"];
    for (i, h) in headers.iter().enumerate() {
        c.set(0, i, Value::Str((*h).to_string()));
    }
    for (r, (name, ss, df, ms, f, p)) in rows.iter().enumerate() {
        c.set(r + 1, 0, Value::Str((*name).to_string()));
        c.set(r + 1, 1, Value::Num(*ss));
        c.set(r + 1, 2, Value::Num(*df));
        c.set(r + 1, 3, ms.map(Value::Num).unwrap_or_else(|| Value::Mat(Matrix::zeros(0, 0))));
        c.set(r + 1, 4, f.map(Value::Num).unwrap_or_else(|| Value::Mat(Matrix::zeros(0, 0))));
        c.set(r + 1, 5, p.map(Value::Num).unwrap_or_else(|| Value::Mat(Matrix::zeros(0, 0))));
    }
    Ok(Value::Cell(c))
}

fn f_pvalue(f: f64, d1: f64, d2: f64) -> f64 {
    if !f.is_finite() || f < 0.0 {
        return f64::NAN;
    }
    (1.0 - stats::f_cdf(f, d1, d2)).clamp(0.0, 1.0)
}

fn rank_values(values: &[f64]) -> Vec<f64> {
    let mut order: Vec<usize> = (0..values.len()).collect();
    order.sort_by(|&a, &b| values[a].total_cmp(&values[b]));
    let mut ranks = vec![0.0f64; values.len()];
    let mut i = 0usize;
    while i < values.len() {
        let mut j = i;
        while j + 1 < values.len() && values[order[j + 1]] == values[order[i]] {
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

fn tie_sum(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
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

/// Infinite-df studentized-range CDF:
/// `k ∫ φ(z) [Φ(z) − Φ(z−q)]^(k−1) dz`.
fn range_cdf_inf(q: f64, k: usize) -> f64 {
    if q <= 0.0 {
        return 0.0;
    }
    if q >= 20.0 {
        return 1.0;
    }
    let n = 400usize;
    let a = -8.0f64;
    let b = 8.0f64;
    let h = (b - a) / n as f64;
    let mut sum = 0.0f64;
    for i in 0..=n {
        let z = a + i as f64 * h;
        let w = if i == 0 || i == n {
            1.0
        } else if i % 2 == 1 {
            4.0
        } else {
            2.0
        };
        let d = stats::norm_cdf(z, 0.0, 1.0) - stats::norm_cdf(z - q, 0.0, 1.0);
        sum += w * stats::norm_pdf(z, 0.0, 1.0) * d.powi(k.saturating_sub(1) as i32);
    }
    (k as f64 * sum * h / 3.0).clamp(0.0, 1.0)
}

/// Studentized-range CDF with finite error degrees of freedom. The range
/// statistic is `R / sqrt(V/df)` with `V ~ chi2(df)`; integrate over the
/// chi-square quantile scale. For `k = 2` this is exactly `sqrt(2)·|t_df|`,
/// which is used as the regression check.
fn studentized_range_cdf(q: f64, k: usize, df: f64) -> f64 {
    if q <= 0.0 {
        return 0.0;
    }
    if k <= 2 {
        return (2.0 * stats::t_cdf(q / 2f64.sqrt(), df) - 1.0).clamp(0.0, 1.0);
    }
    if !df.is_finite() || df > 1e7 {
        return range_cdf_inf(q, k);
    }
    let n = 60usize;
    let eps = 1e-6f64;
    let span = 1.0 - 2.0 * eps;
    let mut sum = 0.0f64;
    for i in 0..=n {
        let p = eps + span * i as f64 / n as f64;
        let v = stats::chi2_inv(p, df);
        if !v.is_finite() || v <= 0.0 {
            continue;
        }
        let f = range_cdf_inf(q * (v / df).sqrt(), k);
        let w = if i == 0 || i == n {
            1.0
        } else if i % 2 == 1 {
            4.0
        } else {
            2.0
        };
        sum += w * f;
    }
    (sum * span / (3.0 * n as f64)).clamp(0.0, 1.0)
}

fn studentized_range_inv(p: f64, k: usize, df: f64) -> f64 {
    let mut lo = 0.0f64;
    let mut hi = 30.0f64;
    for _ in 0..80 {
        let mid = 0.5 * (lo + hi);
        if studentized_range_cdf(mid, k, df) < p {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

#[derive(Clone, Copy, PartialEq)]
enum CType {
    Tukey,
    Lsd,
    Bonferroni,
    Sidak,
    Holm,
}

fn ctype(v: Option<&Value>) -> CType {
    match v {
        Some(Value::Str(s)) => match s.to_ascii_lowercase().replace('_', "-").as_str() {
            "lsd" => CType::Lsd,
            "bonferroni" | "bonf" => CType::Bonferroni,
            "sidak" => CType::Sidak,
            "holm" => CType::Holm,
            _ => CType::Tukey,
        },
        _ => CType::Tukey,
    }
}

impl Session {
    fn anova1_call(&mut self, argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
        let x = argv.first().ok_or_else(|| LangError::new("anova1 needs data", 0))?;
        let (groups, names) = anova1_groups(x, argv.get(1))?;
        let k = groups.len();
        if k < 2 || groups.iter().any(Vec::is_empty) {
            return Err(LangError::new("anova1 needs at least two nonempty groups", 0));
        }
        let total: usize = groups.iter().map(Vec::len).sum();
        let dfw = total.saturating_sub(k);
        if dfw == 0 {
            return Err(LangError::new("anova1 needs within-group degrees of freedom", 0));
        }
        let grand = groups.iter().flat_map(|g| g.iter()).sum::<f64>() / total as f64;
        let mut ssb = 0.0f64;
        let mut ssw = 0.0f64;
        let mut means = Vec::with_capacity(k);
        let mut counts = Vec::with_capacity(k);
        for g in &groups {
            let m = g.iter().sum::<f64>() / g.len() as f64;
            means.push(m);
            counts.push(g.len() as f64);
            ssb += g.len() as f64 * (m - grand) * (m - grand);
            ssw += g.iter().map(|v| (v - m) * (v - m)).sum::<f64>();
        }
        let dfb = (k - 1) as f64;
        let msb = ssb / dfb;
        let mse = ssw / dfw as f64;
        let f = if mse > 0.0 { msb / mse } else { f64::INFINITY };
        let p = f_pvalue(f, dfb, dfw as f64);
        let tbl = anova_table(&[
            ("Groups", ssb, dfb, Some(msb), Some(f), Some(p)),
            ("Error", ssw, dfw as f64, Some(mse), None, None),
            ("Total", ssb + ssw, (total - 1) as f64, None, None, None),
        ])?;
        let mut st = StructArray::new();
        st.set_field(0, "source", Value::Str("anova1".into()))?;
        st.set_field(0, "means", Value::Mat(Matrix::col_vector(means)))?;
        st.set_field(0, "n", Value::Mat(Matrix::row_vector(counts)))?;
        st.set_field(0, "df", Value::Num(dfw as f64))?;
        st.set_field(0, "s", Value::Num(mse.sqrt()))?;
        st.set_field(0, "gnames", Value::Cell(CellArray::row(names.into_iter().map(Value::Str).collect())))?;
        Ok(truncate_anova(vec![Value::Num(p), tbl, Value::Struct(st)], nout))
    }

    fn anova2_call(&mut self, argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
        let x = argv.first().ok_or_else(|| LangError::new("anova2 needs a data matrix", 0))?.to_numeric()?;
        let reps = argv.get(1).map(|v| v.scalar()).transpose()?.unwrap_or(1.0);
        if reps < 1.0 || reps.fract() != 0.0 {
            return Err(LangError::new("anova2 reps must be a positive integer", 0));
        }
        let reps = reps as usize;
        let rows = x.rows();
        let cols = x.cols();
        if cols < 2 || rows < 2 {
            return Err(LangError::new("anova2 needs at least two rows and two columns", 0));
        }
        if rows % reps != 0 {
            return Err(LangError::new("anova2 row count must be a multiple of reps", 0));
        }
        let a = rows / reps;
        let b = cols;
        if a < 2 {
            return Err(LangError::new("anova2 needs at least two row levels", 0));
        }
        let grand = x.data().iter().sum::<f64>() / x.numel() as f64;
        let mut row_means = vec![0.0f64; a];
        let mut col_means = vec![0.0f64; b];
        let mut cell_means = vec![vec![0.0f64; b]; a];
        let mut cell_counts = vec![vec![0usize; b]; a];
        for r in 0..rows {
            let level = r / reps;
            for c in 0..cols {
                let v = x.get(r, c).unwrap_or(0.0);
                row_means[level] += v;
                col_means[c] += v;
                cell_means[level][c] += v;
                cell_counts[level][c] += 1;
            }
        }
        for m in row_means.iter_mut() {
            *m /= (reps * b) as f64;
        }
        for m in &mut col_means {
            *m /= (a * reps) as f64;
        }
        for i in 0..a {
            for j in 0..b {
                let n = cell_counts[i][j].max(1) as f64;
                cell_means[i][j] /= n;
            }
        }
        let ss_a = (reps * b) as f64 * row_means.iter().map(|m| (m - grand) * (m - grand)).sum::<f64>();
        let ss_b = (a * reps) as f64 * col_means.iter().map(|m| (m - grand) * (m - grand)).sum::<f64>();
        let ss_ab = reps as f64
            * cell_means
                .iter()
                .enumerate()
                .map(|(i, row)| {
                    row.iter()
                        .enumerate()
                        .map(|(j, cm)| {
                            let v = cm - row_means.get(i).copied().unwrap_or(0.0) - col_means.get(j).copied().unwrap_or(0.0) + grand;
                            v * v
                        })
                        .sum::<f64>()
                })
                .sum::<f64>();
        let mut sst = 0.0f64;
        let mut sse_within = 0.0f64;
        for r in 0..rows {
            let i = r / reps;
            for c in 0..cols {
                let v = x.get(r, c).unwrap_or(0.0);
                sst += (v - grand) * (v - grand);
                sse_within += (v - cell_means[i][c]) * (v - cell_means[i][c]);
            }
        }
        // Without replication the interaction and residual are the same
        // quantity; with replication the residual is the within-cell error.
        let sse = if reps > 1 { sse_within } else { (sst - ss_a - ss_b).max(0.0) };
        let df_a = (a - 1) as f64;
        let df_b = (b - 1) as f64;
        let df_ab = ((a - 1) * (b - 1)) as f64;
        let df_e = if reps > 1 { (a * b * (reps - 1)) as f64 } else { df_ab };
        let ms_a = ss_a / df_a;
        let ms_b = ss_b / df_b;
        let ms_ab = if reps > 1 { ss_ab / df_ab } else { 0.0 };
        let ms_e = if df_e > 0.0 { sse / df_e } else { f64::NAN };
        let fa = if ms_e > 0.0 { ms_a / ms_e } else { f64::INFINITY };
        let fb = if ms_e > 0.0 { ms_b / ms_e } else { f64::INFINITY };
        let fab = if reps > 1 && ms_e > 0.0 { ms_ab / ms_e } else { f64::NAN };
        let pa = f_pvalue(fa, df_a, df_e);
        let pb = f_pvalue(fb, df_b, df_e);
        let pab = if reps > 1 { f_pvalue(fab, df_ab, df_e) } else { f64::NAN };
        let mut table_rows: Vec<(&str, f64, f64, Option<f64>, Option<f64>, Option<f64>)> =
            vec![("Columns", ss_b, df_b, Some(ms_b), Some(fb), Some(pb)), ("Rows", ss_a, df_a, Some(ms_a), Some(fa), Some(pa))];
        if reps > 1 {
            table_rows.push(("Interaction", ss_ab, df_ab, Some(ms_ab), Some(fab), Some(pab)));
        }
        table_rows.push(("Error", sse, df_e, Some(ms_e), None, None));
        table_rows.push(("Total", sst, (rows * cols - 1) as f64, None, None, None));
        let tbl = anova_table(&table_rows)?;
        let mut st = StructArray::new();
        st.set_field(0, "source", Value::Str("anova2".into()))?;
        st.set_field(0, "means", Value::Mat(Matrix::col_vector(col_means.clone())))?;
        st.set_field(0, "n", Value::Mat(Matrix::col_vector(vec![(a * reps) as f64; b])))?;
        st.set_field(0, "df", Value::Num(df_e))?;
        st.set_field(0, "s", Value::Num(if ms_e.is_finite() { ms_e.sqrt() } else { f64::NAN }))?;
        st.set_field(0, "gnames", Value::Cell(CellArray::row((1..=b).map(|i| Value::Str(format!("Column{i}"))).collect())))?;
        st.set_field(0, "rowmeans", Value::Mat(Matrix::col_vector(row_means)))?;
        st.set_field(0, "rown", Value::Mat(Matrix::col_vector(vec![(b * reps) as f64; a])))?;
        Ok(truncate_anova(vec![Value::Num(pb), tbl, Value::Struct(st)], nout))
    }

    fn multcompare_call(&mut self, argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
        let st = match argv.first() {
            Some(Value::Struct(s)) => s,
            _ => return Err(LangError::new("multcompare needs an ANOVA stats struct", 0)),
        };
        let means =
            st.field_at(0, "means").and_then(|v| v.to_matrix().ok()).ok_or_else(|| LangError::new("stats.means is missing", 0))?.data().to_vec();
        let nvals = st.field_at(0, "n").and_then(|v| v.to_matrix().ok()).ok_or_else(|| LangError::new("stats.n is missing", 0))?.data().to_vec();
        let s = st.field_at(0, "s").and_then(|v| v.scalar().ok()).ok_or_else(|| LangError::new("stats.s is missing", 0))?;
        let df = st.field_at(0, "df").and_then(|v| v.scalar().ok()).ok_or_else(|| LangError::new("stats.df is missing", 0))?;
        let k = means.len();
        if nvals.len() != k || k < 2 {
            return Err(LangError::new("stats means/n are inconsistent", 0));
        }
        let mut alpha = 0.05f64;
        let mut method = CType::Tukey;
        let mut i = 1usize;
        while i < argv.len() {
            if let Value::Str(key) = &argv[i] {
                let v = argv.get(i + 1);
                match key.to_ascii_lowercase().as_str() {
                    "alpha" => {
                        alpha = v.map(|x| x.scalar()).transpose()?.unwrap_or(0.05);
                    }
                    "ctype" => method = ctype(v),
                    _ => {}
                }
                i += 2;
            } else {
                i += 1;
            }
        }
        if !(alpha > 0.0 && alpha < 1.0) {
            return Err(LangError::new("multcompare alpha must be between 0 and 1", 0));
        }
        let gnames =
            st.field_at(0, "gnames").map(cell_strings).filter(|g| g.len() == k).unwrap_or_else(|| (1..=k).map(|i| format!("Group{i}")).collect());
        let pairs = k * (k - 1) / 2;
        let mut raw = Vec::with_capacity(pairs);
        let mut diffs = Vec::with_capacity(pairs);
        let mut ses = Vec::with_capacity(pairs);
        let mut idx = Vec::with_capacity(pairs);
        for a in 0..k {
            for b in (a + 1)..k {
                let diff = means[a] - means[b];
                let se = s * (1.0 / nvals[a].max(1.0) + 1.0 / nvals[b].max(1.0)).sqrt();
                let t = if se > 0.0 { diff / se } else { f64::NAN };
                let p = if t.is_finite() { (2.0 * (1.0 - stats::t_cdf(t.abs(), df))).clamp(0.0, 1.0) } else { f64::NAN };
                raw.push(p);
                diffs.push(diff);
                ses.push(se);
                idx.push((a, b));
            }
        }
        let mut adjusted = raw.clone();
        match method {
            CType::Tukey => {
                for (p, &(a, b)) in adjusted.iter_mut().zip(idx.iter()) {
                    let se_q = s * (0.5 * (1.0 / nvals[a].max(1.0) + 1.0 / nvals[b].max(1.0))).sqrt();
                    let q = if se_q > 0.0 { (means[a] - means[b]).abs() / se_q } else { f64::NAN };
                    *p = if q.is_finite() { (1.0 - studentized_range_cdf(q, k, df)).clamp(0.0, 1.0) } else { f64::NAN };
                }
            }
            CType::Bonferroni => {
                for p in &mut adjusted {
                    *p = (*p * pairs as f64).min(1.0);
                }
            }
            CType::Sidak => {
                for p in &mut adjusted {
                    *p = (1.0 - (1.0 - *p).powi(pairs as i32)).min(1.0);
                }
            }
            CType::Holm => {
                let mut order: Vec<usize> = (0..pairs).collect();
                order.sort_by(|&a, &b| raw[a].total_cmp(&raw[b]));
                let mut running = 0.0f64;
                for (rank, &p) in order.iter().enumerate() {
                    let adj = (raw[p] * (pairs - rank) as f64).min(1.0);
                    running = running.max(adj);
                    if let Some(slot) = adjusted.get_mut(p) {
                        *slot = running;
                    }
                }
            }
            CType::Lsd => {}
        }
        let mut c = Matrix::zeros(pairs, 6);
        let tcrit = match method {
            CType::Bonferroni | CType::Holm => stats::t_inv(1.0 - alpha / (2.0 * pairs as f64), df),
            CType::Sidak => {
                let a2 = 1.0 - (1.0 - alpha).powf(1.0 / pairs as f64);
                stats::t_inv(1.0 - a2 / 2.0, df)
            }
            CType::Lsd => stats::t_inv(1.0 - alpha / 2.0, df),
            CType::Tukey => studentized_range_inv(1.0 - alpha, k, df),
        };
        for (row, &(a, b)) in idx.iter().enumerate() {
            let se = ses[row];
            let critical = match method {
                CType::Tukey => tcrit * s * (0.5 * (1.0 / nvals[a].max(1.0) + 1.0 / nvals[b].max(1.0))).sqrt(),
                _ => tcrit * se,
            };
            let diff = diffs[row];
            let _ = c.set(row, 0, (a + 1) as f64);
            let _ = c.set(row, 1, (b + 1) as f64);
            let _ = c.set(row, 2, diff - critical);
            let _ = c.set(row, 3, diff);
            let _ = c.set(row, 4, diff + critical);
            let _ = c.set(row, 5, adjusted[row]);
        }
        let mut m = Matrix::zeros(k, 2);
        for i in 0..k {
            let _ = m.set(i, 0, means[i]);
            let _ = m.set(i, 1, s / nvals[i].max(1.0).sqrt());
        }
        let outs = vec![Value::Mat(c), Value::Mat(m), Value::Num(0.0), Value::Cell(CellArray::row(gnames.into_iter().map(Value::Str).collect()))];
        Ok(truncate_anova(outs, nout))
    }

    pub(super) fn anova_builtin(&mut self, name: &str, argv: Vec<Value>, nout: usize) -> Result<Vec<Value>, LangError> {
        match name {
            "anova1" => self.anova1_call(&argv, nout),
            "anova2" => self.anova2_call(&argv, nout),
            "multcompare" => self.multcompare_call(&argv, nout),
            "friedman" => self.friedman_call(&argv, nout),
            _ => Err(LangError::new(format!("{name}: not an ANOVA builtin"), 0)),
        }
    }

    fn friedman_call(&mut self, argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
        let x = argv.first().ok_or_else(|| LangError::new("friedman needs a data matrix", 0))?.to_numeric()?;
        let reps = argv.get(1).map(|v| v.scalar()).transpose()?.unwrap_or(1.0);
        if reps < 1.0 || reps.fract() != 0.0 {
            return Err(LangError::new("friedman reps must be a positive integer", 0));
        }
        let reps = reps as usize;
        let rows = x.rows();
        let cols = x.cols();
        if cols < 2 || rows < 2 {
            return Err(LangError::new("friedman needs at least two treatments and two blocks", 0));
        }
        if rows % reps != 0 {
            return Err(LangError::new("friedman row count must be a multiple of reps", 0));
        }
        let blocks = rows / reps;
        let per_block = cols * reps;
        let mut ranks = vec![0.0f64; rows * cols];
        let mut tie = 0.0f64;
        for b in 0..blocks {
            let mut vals = Vec::with_capacity(per_block);
            let mut positions = Vec::with_capacity(per_block);
            for r in 0..reps {
                let row = b * reps + r;
                for c in 0..cols {
                    vals.push(x.get(row, c).unwrap_or(0.0));
                    positions.push((row, c));
                }
            }
            let ranked = rank_values(&vals);
            tie += tie_sum(&vals);
            for (k, &(row, col)) in positions.iter().enumerate() {
                if let Some(slot) = ranks.get_mut(row * cols + col) {
                    *slot = ranked.get(k).copied().unwrap_or(0.0);
                }
            }
        }
        let mut rank_sums = vec![0.0f64; cols];
        for r in 0..rows {
            for c in 0..cols {
                rank_sums[c] += ranks.get(r * cols + c).copied().unwrap_or(0.0);
            }
        }
        let n = (blocks * reps) as f64;
        let m = per_block as f64;
        let mut chi2 = 0.0f64;
        for r in &rank_sums {
            chi2 += r * r;
        }
        chi2 = 12.0 / (blocks * reps * cols) as f64 / (m + 1.0) * chi2 - 3.0 * n * (m + 1.0);
        let max_tie = (blocks * reps * cols * (per_block * per_block - 1)) as f64;
        if tie > 0.0 && max_tie > 0.0 {
            chi2 /= 1.0 - tie / max_tie;
        }
        let df = (cols - 1) as f64;
        let p = (1.0 - stats::chi2_cdf(chi2, df)).clamp(0.0, 1.0);
        let mut tbl = CellArray::new(2, 6);
        for (i, h) in ["Source", "SS", "df", "MS", "Chi-sq", "Prob>Chi-sq"].iter().enumerate() {
            tbl.set(0, i, Value::Str((*h).to_string()));
        }
        let row = [
            Value::Str("Columns".into()),
            Value::Mat(Matrix::zeros(0, 0)),
            Value::Num(df),
            Value::Mat(Matrix::zeros(0, 0)),
            Value::Num(chi2),
            Value::Num(p),
        ];
        for (k, v) in row.into_iter().enumerate() {
            tbl.set(1, k, v);
        }
        let means: Vec<f64> = rank_sums.iter().map(|r| r / n).collect();
        let mut st = StructArray::new();
        st.set_field(0, "source", Value::Str("friedman".into()))?;
        st.set_field(0, "n", Value::Num(n))?;
        st.set_field(0, "meanranks", Value::Mat(Matrix::row_vector(means)))?;
        st.set_field(0, "chi2", Value::Num(chi2))?;
        st.set_field(0, "df", Value::Num(df))?;
        Ok(truncate_anova(vec![Value::Num(p), Value::Cell(tbl), Value::Struct(st)], nout))
    }
}

fn truncate_anova(mut out: Vec<Value>, nout: usize) -> Vec<Value> {
    out.truncate(nout.max(1).min(out.len()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn studentized_range_k2_matches_t() {
        for &q in &[1.0, 2.5, 4.0] {
            let cdf = studentized_range_cdf(q, 2, 6.0);
            let exact = (2.0 * stats::t_cdf(q / 2f64.sqrt(), 6.0) - 1.0).clamp(0.0, 1.0);
            assert!((cdf - exact).abs() < 1e-10, "q={q}: {cdf} vs {exact}");
        }
    }

    #[test]
    fn studentized_range_grows_with_df() {
        let low = studentized_range_cdf(3.0, 3, 4.0);
        let high = studentized_range_cdf(3.0, 3, 100.0);
        assert!(low < high, "low={low} high={high}");
        assert!(high <= 1.0);
    }
}
