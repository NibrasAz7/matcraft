//! Streamline integration for 2-D vector fields.

use super::*;

fn axis_values(m: &Matrix) -> Vec<f64> {
    if m.rows() == 1 || m.cols() == 1 {
        m.data().to_vec()
    } else if m.rows() > 0 && m.cols() > 0 {
        (0..m.cols()).map(|c| m.get(0, c).unwrap_or(0.0)).collect()
    } else {
        Vec::new()
    }
}

fn axis_values_y(m: &Matrix) -> Vec<f64> {
    if m.rows() == 1 || m.cols() == 1 {
        m.data().to_vec()
    } else if m.rows() > 0 && m.cols() > 0 {
        (0..m.rows()).map(|r| m.get(r, 0).unwrap_or(0.0)).collect()
    } else {
        Vec::new()
    }
}

fn bracket(xs: &[f64], v: f64) -> Option<(usize, f64)> {
    if xs.len() < 2 {
        return None;
    }
    let first = xs.first().copied().unwrap_or(0.0);
    let last = xs.last().copied().unwrap_or(0.0);
    let lo = first.min(last);
    let hi = first.max(last);
    if v < lo || v > hi {
        return None;
    }
    for i in 0..xs.len() - 1 {
        let a = xs.get(i).copied().unwrap_or(0.0);
        let b = xs.get(i + 1).copied().unwrap_or(0.0);
        let (a, b) = (a.min(b), a.max(b));
        if v >= a && v <= b {
            let den = b - a;
            let t = if den.abs() < 1e-15 { 0.0 } else { (v - a) / den };
            return Some((i, t.clamp(0.0, 1.0)));
        }
    }
    None
}

fn interp_component(xs: &[f64], ys: &[f64], f: &Matrix, x: f64, y: f64) -> Option<f64> {
    let cols = xs.len();
    let rows = ys.len();
    if cols == 0 || rows == 0 {
        return None;
    }
    let (c, tx) = if cols == 1 { (0usize, 0.0) } else { bracket(xs, x)? };
    let (r, ty) = if rows == 1 { (0usize, 0.0) } else { bracket(ys, y)? };
    let c1 = if cols == 1 { c } else { (c + 1).min(cols - 1) };
    let r1 = if rows == 1 { r } else { (r + 1).min(rows - 1) };
    let f00 = f.get(r, c).unwrap_or(0.0);
    let f10 = f.get(r, c1).unwrap_or(f00);
    let f01 = f.get(r1, c).unwrap_or(f00);
    let f11 = f.get(r1, c1).unwrap_or(f10);
    let a = f00 * (1.0 - tx) + f10 * tx;
    let b = f01 * (1.0 - tx) + f11 * tx;
    Some(a * (1.0 - ty) + b * ty)
}

fn field(xs: &[f64], ys: &[f64], u: &Matrix, v: &Matrix, x: f64, y: f64) -> Option<(f64, f64)> {
    Some((interp_component(xs, ys, u, x, y)?, interp_component(xs, ys, v, x, y)?))
}

/// Integrates streamlines from seed points. Returns one polyline per seed.
pub(super) fn compute(x: &Matrix, y: &Matrix, u: &Matrix, v: &Matrix, seeds: &[(f64, f64)], grid_seeds: bool) -> Vec<Vec<(f64, f64)>> {
    let mut xs = if x.numel() == 0 { (1..=u.cols()).map(|i| i as f64).collect() } else { axis_values(x) };
    let mut ys = if y.numel() == 0 { (1..=u.rows()).map(|i| i as f64).collect() } else { axis_values_y(y) };
    if xs.is_empty() {
        xs = vec![1.0];
    }
    if ys.is_empty() {
        ys = vec![1.0];
    }
    let xmin = xs.iter().copied().fold(f64::INFINITY, f64::min);
    let xmax = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let ymin = ys.iter().copied().fold(f64::INFINITY, f64::min);
    let ymax = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if !(xmin.is_finite() && xmax.is_finite() && ymin.is_finite() && ymax.is_finite()) {
        return Vec::new();
    }
    let span = (xmax - xmin).abs().max((ymax - ymin).abs());
    if span <= 1e-12 {
        return Vec::new();
    }
    let h = span * 0.02;
    let mut starts = seeds.to_vec();
    if grid_seeds {
        starts.clear();
        for i in 0..5 {
            for j in 0..5 {
                let fx = 0.1 + 0.8 * i as f64 / 4.0;
                let fy = 0.1 + 0.8 * j as f64 / 4.0;
                starts.push((xmin + fx * (xmax - xmin), ymin + fy * (ymax - ymin)));
            }
        }
    }
    let mut lines = Vec::new();
    for (sx, sy) in starts {
        if sx < xmin || sx > xmax || sy < ymin || sy > ymax {
            continue;
        }
        let mut px = sx;
        let mut py = sy;
        let mut line = vec![(px, py)];
        for _ in 0..400 {
            let Some((ux, uy)) = field(&xs, &ys, u, v, px, py) else { break };
            let speed = (ux * ux + uy * uy).sqrt();
            if !speed.is_finite() || speed < 1e-12 {
                break;
            }
            let dx = ux / speed * h;
            let dy = uy / speed * h;
            let Some((mx, my)) = field(&xs, &ys, u, v, px + dx * 0.5, py + dy * 0.5) else { break };
            let ms = (mx * mx + my * my).sqrt();
            if !ms.is_finite() || ms < 1e-12 {
                break;
            }
            px += mx / ms * h;
            py += my / ms * h;
            if px < xmin || px > xmax || py < ymin || py > ymax {
                break;
            }
            line.push((px, py));
        }
        if line.len() > 1 {
            lines.push(line);
        }
    }
    lines
}
