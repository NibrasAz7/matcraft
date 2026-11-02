//! ODE integration builtins: `ode45`, `ode15s`, `odeset`, `odeget`.
//!
//! `ode45` is adaptive Dormand–Prince 5(4). `ode15s` is a stiffly stable
//! TR-BDF2 integrator with step-doubling error control and modified Newton
//! solves using a finite-difference Jacobian. Both support scalar/vector
//! tolerances, `MaxStep`, `InitialStep`, `MaxIterations` and `NonNegative`.

#![allow(clippy::needless_range_loop, clippy::type_complexity)]

use super::*;

#[derive(Clone)]
struct OdeOptions {
    rel_tol: Matrix,
    abs_tol: Matrix,
    max_step: Option<f64>,
    initial_step: Option<f64>,
    max_iterations: usize,
    non_negative: Vec<usize>,
}

impl OdeOptions {
    fn defaults() -> Self {
        OdeOptions {
            rel_tol: Matrix::scalar(1e-3),
            abs_tol: Matrix::scalar(1e-6),
            max_step: None,
            initial_step: None,
            max_iterations: 1_000_000,
            non_negative: Vec::new(),
        }
    }

    fn tolerances(&self, n: usize) -> Result<(Vec<f64>, Vec<f64>), LangError> {
        let expand = |m: &Matrix, what: &str| -> Result<Vec<f64>, LangError> {
            if m.numel() == 1 {
                Ok(vec![m.data().first().copied().unwrap_or(1e-6); n])
            } else if m.numel() == n {
                Ok(m.data().to_vec())
            } else {
                Err(LangError::new(format!("{what} must be scalar or match the state length"), 0))
            }
        };
        let rt = expand(&self.rel_tol, "RelTol")?;
        let at = expand(&self.abs_tol, "AbsTol")?;
        if rt.iter().any(|v| v.is_nan() || *v <= 0.0) || at.iter().any(|v| v.is_nan() || *v <= 0.0) {
            return Err(LangError::new("RelTol and AbsTol must be positive", 0));
        }
        Ok((rt, at))
    }
}

fn field_number(s: &StructArray, name: &str) -> Option<f64> {
    s.field_at(0, name).and_then(|v| v.to_matrix().ok()).and_then(|m| m.as_scalar())
}

fn options_from_value(v: Option<&Value>) -> Result<OdeOptions, LangError> {
    let mut o = OdeOptions::defaults();
    let Some(Value::Struct(s)) = v else {
        return Ok(o);
    };
    if let Some(m) = s.field_at(0, "RelTol").and_then(|v| v.to_matrix().ok()) {
        o.rel_tol = m;
    }
    if let Some(m) = s.field_at(0, "AbsTol").and_then(|v| v.to_matrix().ok()) {
        o.abs_tol = m;
    }
    if let Some(v) = field_number(s, "MaxStep") {
        if v.is_finite() && v > 0.0 {
            o.max_step = Some(v);
        }
    }
    if let Some(v) = field_number(s, "InitialStep") {
        if v.is_finite() && v > 0.0 {
            o.initial_step = Some(v);
        }
    }
    if let Some(v) = field_number(s, "MaxIterations") {
        if v >= 1.0 && v.is_finite() {
            o.max_iterations = v as usize;
        }
    }
    if let Some(m) = s.field_at(0, "NonNegative").and_then(|v| v.to_matrix().ok()) {
        o.non_negative = m.data().iter().filter(|v| **v >= 1.0).map(|v| *v as usize).collect();
    }
    Ok(o)
}

fn state_vector(v: &Value, what: &str) -> Result<Vec<f64>, LangError> {
    let m = v.to_numeric()?;
    if m.rows() > 1 && m.cols() > 1 {
        return Err(LangError::new(format!("{what} must be a vector"), 0));
    }
    Ok(m.data().to_vec())
}

fn max_abs(v: &[f64]) -> f64 {
    v.iter().fold(0.0f64, |m, x| m.max(x.abs()))
}

impl Session {
    fn ode_eval(&mut self, target: &Value, t: f64, y: &[f64]) -> Result<Vec<f64>, LangError> {
        let out = self.call_feval_target(target, vec![Value::Num(t), Value::Mat(Matrix::col_vector(y.to_vec()))], 1)?;
        let m = out.first().ok_or_else(|| LangError::new("odefun returned no value", 0))?.to_numeric()?;
        if m.numel() != y.len() {
            return Err(LangError::new(format!("odefun must return {} values, got {}", y.len(), m.numel()), 0));
        }
        Ok(m.data().to_vec())
    }

    fn ode_jacobian(&mut self, target: &Value, t: f64, y: &[f64], f0: &[f64]) -> Result<Matrix, LangError> {
        let n = y.len();
        let mut j = Matrix::zeros(n, n);
        for c in 0..n {
            let mut yp = y.to_vec();
            let delta = 1e-8 * (1.0 + y.get(c).copied().unwrap_or(0.0).abs());
            if let Some(slot) = yp.get_mut(c) {
                *slot += delta;
            }
            let fp = self.ode_eval(target, t, &yp)?;
            for r in 0..n {
                let d = (fp.get(r).copied().unwrap_or(0.0) - f0.get(r).copied().unwrap_or(0.0)) / delta;
                let _ = j.set(r, c, d);
            }
        }
        Ok(j)
    }

    fn rk45_step(&mut self, target: &Value, t: f64, y: &[f64], h: f64) -> Result<(Vec<f64>, Vec<f64>, Vec<f64>), LangError> {
        let n = y.len();
        let k1 = self.ode_eval(target, t, y)?;
        let stage = |frac: f64, sums: &[(f64, &Vec<f64>)]| -> Result<Vec<f64>, LangError> {
            let mut yy = Vec::with_capacity(n);
            for i in 0..n {
                let mut v = y.get(i).copied().unwrap_or(0.0);
                for (a, k) in sums {
                    v += h * *a * k.get(i).copied().unwrap_or(0.0);
                }
                let _ = frac;
                yy.push(v);
            }
            Ok(yy)
        };
        let k2 = self.ode_eval(target, t + h / 5.0, &stage(0.0, &[(1.0 / 5.0, &k1)])?)?;
        let k3 = self.ode_eval(target, t + 3.0 * h / 10.0, &stage(0.0, &[(3.0 / 40.0, &k1), (9.0 / 40.0, &k2)])?)?;
        let k4 = self.ode_eval(target, t + 4.0 * h / 5.0, &stage(0.0, &[(44.0 / 45.0, &k1), (-56.0 / 15.0, &k2), (32.0 / 9.0, &k3)])?)?;
        let k5 = self.ode_eval(
            target,
            t + 8.0 * h / 9.0,
            &stage(0.0, &[(19372.0 / 6561.0, &k1), (-25360.0 / 2187.0, &k2), (64448.0 / 6561.0, &k3), (-212.0 / 729.0, &k4)])?,
        )?;
        let k6 = self.ode_eval(
            target,
            t + h,
            &stage(0.0, &[(9017.0 / 3168.0, &k1), (-355.0 / 33.0, &k2), (46732.0 / 5247.0, &k3), (49.0 / 176.0, &k4), (-5103.0 / 18656.0, &k5)])?,
        )?;
        let ynew = stage(0.0, &[(35.0 / 384.0, &k1), (500.0 / 1113.0, &k3), (125.0 / 192.0, &k4), (-2187.0 / 6784.0, &k5), (11.0 / 84.0, &k6)])?;
        let fnew = self.ode_eval(target, t + h, &ynew)?;
        let mut err = vec![0.0f64; n];
        for i in 0..n {
            let d = (35.0 / 384.0 - 5179.0 / 57600.0) * k1.get(i).copied().unwrap_or(0.0)
                + (500.0 / 1113.0 - 7571.0 / 16695.0) * k3.get(i).copied().unwrap_or(0.0)
                + (125.0 / 192.0 - 393.0 / 640.0) * k4.get(i).copied().unwrap_or(0.0)
                + (-2187.0 / 6784.0 + 92097.0 / 339200.0) * k5.get(i).copied().unwrap_or(0.0)
                + (11.0 / 84.0 - 187.0 / 2100.0) * k6.get(i).copied().unwrap_or(0.0)
                - (1.0 / 40.0) * fnew.get(i).copied().unwrap_or(0.0);
            err[i] = h * d;
        }
        Ok((ynew, err, fnew))
    }

    fn trbdf2_step(&mut self, target: &Value, t: f64, y: &[f64], h: f64) -> Result<(Vec<f64>, Vec<f64>), LangError> {
        let n = y.len();
        let gamma = 2.0 - 2f64.sqrt();
        let f0 = self.ode_eval(target, t, y)?;
        let mut yg = vec![0.0f64; n];
        for i in 0..n {
            yg[i] = y.get(i).copied().unwrap_or(0.0) + gamma * h * f0.get(i).copied().unwrap_or(0.0);
        }
        for _ in 0..10 {
            let fg = self.ode_eval(target, t + gamma * h, &yg)?;
            let mut resid = vec![0.0f64; n];
            for i in 0..n {
                resid[i] = yg.get(i).copied().unwrap_or(0.0)
                    - y.get(i).copied().unwrap_or(0.0)
                    - gamma * h / 2.0 * (f0.get(i).copied().unwrap_or(0.0) + fg.get(i).copied().unwrap_or(0.0));
            }
            if max_abs(&resid) <= 1e-12 * (1.0 + max_abs(&yg)) {
                break;
            }
            let jac = self.ode_jacobian(target, t + gamma * h, &yg, &fg)?;
            let mut a = Matrix::eye(n, n);
            for r in 0..n {
                for c in 0..n {
                    let v = -gamma * h / 2.0 * jac.get(r, c).unwrap_or(0.0);
                    let _ = a.set(r, c, a.get(r, c).unwrap_or(0.0) + v);
                }
            }
            let rhs: Vec<f64> = resid.iter().map(|v| -v).collect();
            let delta = a.solve(&Matrix::col_vector(rhs)).map_err(|_| LangError::new("ode15s: singular Newton matrix", 0))?;
            for i in 0..n {
                yg[i] += delta.get(i, 0).unwrap_or(0.0);
            }
        }
        let c0 = 1.0 / (gamma * (2.0 - gamma));
        let c1 = (1.0 - gamma) * (1.0 - gamma) / (gamma * (2.0 - gamma));
        let c2 = (1.0 - gamma) / (2.0 - gamma);
        let mut base = vec![0.0f64; n];
        for i in 0..n {
            base[i] = c0 * yg.get(i).copied().unwrap_or(0.0) - c1 * y.get(i).copied().unwrap_or(0.0);
        }
        let mut yn = yg.clone();
        for _ in 0..10 {
            let fn_ = self.ode_eval(target, t + h, &yn)?;
            let mut resid = vec![0.0f64; n];
            for i in 0..n {
                resid[i] = yn.get(i).copied().unwrap_or(0.0) - base.get(i).copied().unwrap_or(0.0) - c2 * h * fn_.get(i).copied().unwrap_or(0.0);
            }
            if max_abs(&resid) <= 1e-12 * (1.0 + max_abs(&yn)) {
                break;
            }
            let jac = self.ode_jacobian(target, t + h, &yn, &fn_)?;
            let mut a = Matrix::eye(n, n);
            for r in 0..n {
                for c in 0..n {
                    let v = -c2 * h * jac.get(r, c).unwrap_or(0.0);
                    let _ = a.set(r, c, a.get(r, c).unwrap_or(0.0) + v);
                }
            }
            let rhs: Vec<f64> = resid.iter().map(|v| -v).collect();
            let delta = a.solve(&Matrix::col_vector(rhs)).map_err(|_| LangError::new("ode15s: singular Newton matrix", 0))?;
            for i in 0..n {
                yn[i] += delta.get(i, 0).unwrap_or(0.0);
            }
        }
        let fnew = self.ode_eval(target, t + h, &yn)?;
        Ok((yn, fnew))
    }

    fn ode_integrate(
        &mut self,
        target: &Value,
        stiff: bool,
        t0: f64,
        tf: f64,
        y0: &[f64],
        opts: &OdeOptions,
    ) -> Result<(Vec<f64>, Vec<Vec<f64>>, Vec<Vec<f64>>), LangError> {
        let n = y0.len();
        let (rt, at) = opts.tolerances(n)?;
        let span = tf - t0;
        if span.is_nan() || span <= 0.0 {
            return Err(LangError::new("tspan must increase", 0));
        }
        let max_step = opts.max_step.unwrap_or(span / 10.0).max(1e-12);
        let mut h = opts.initial_step.unwrap_or(if stiff { span * 1e-3 } else { span * 0.01 });
        h = h.min(max_step).min(span);
        let mut t = t0;
        let mut y = y0.to_vec();
        let mut ts = vec![t];
        let mut ys = vec![y.clone()];
        let mut fs = vec![self.ode_eval(target, t, &y)?];
        let mut attempts = 0usize;
        let tol_time = 1e-14 * span.max(1.0);
        while t < tf - tol_time {
            attempts = attempts.saturating_add(1);
            if attempts > opts.max_iterations {
                return Err(LangError::new("ODE integration exceeded MaxIterations", 0));
            }
            let h_use = h.min(max_step).min(tf - t);
            if h_use <= 1e-15 * span.max(1.0) {
                return Err(LangError::new("ODE step size underflow", 0));
            }
            let (ynew, err) = if stiff {
                let (y1, _) = self.trbdf2_step(target, t, &y, h_use)?;
                let (yh, _) = self.trbdf2_step(target, t, &y, h_use / 2.0)?;
                let (y2, _) = self.trbdf2_step(target, t + h_use / 2.0, &yh, h_use / 2.0)?;
                let e: Vec<f64> = y2.iter().zip(y1.iter()).map(|(a, b)| (a - b) / 3.0).collect();
                (y2, e)
            } else {
                let (yn, e, _) = self.rk45_step(target, t, &y, h_use)?;
                (yn, e)
            };
            let mut scaled = 0.0f64;
            for i in 0..n {
                let scale = at.get(i).copied().unwrap_or(1e-6)
                    + rt.get(i).copied().unwrap_or(1e-3) * y.get(i).copied().unwrap_or(0.0).abs().max(ynew.get(i).copied().unwrap_or(0.0).abs());
                scaled = scaled.max(err.get(i).copied().unwrap_or(0.0).abs() / scale);
            }
            let order = if stiff { 2.0 } else { 4.0 };
            if scaled <= 1.0 {
                t += h_use;
                if t > tf - tol_time {
                    t = tf;
                }
                y = ynew;
                for idx in &opts.non_negative {
                    if *idx >= 1 && *idx <= n {
                        if let Some(slot) = y.get_mut(idx - 1) {
                            *slot = slot.max(0.0);
                        }
                    }
                }
                ts.push(t);
                ys.push(y.clone());
                fs.push(self.ode_eval(target, t, &y)?);
                let fac = if scaled > 1e-12 { 0.9 * scaled.powf(-1.0 / (order + 1.0)) } else { 5.0 };
                h = (h_use * fac.clamp(0.2, 5.0)).min(max_step).max(1e-15 * span);
            } else {
                let fac = (0.9 * scaled.powf(-1.0 / (order + 1.0))).clamp(0.1, 0.9);
                h = (h_use * fac).max(1e-15 * span);
            }
        }
        Ok((ts, ys, fs))
    }

    pub(super) fn ode_builtin(&mut self, name: &str, argv: Vec<Value>, nout: usize) -> Result<Vec<Value>, LangError> {
        match name {
            "odeset" => {
                let mut s = match argv.first() {
                    Some(Value::Struct(x)) => x.clone(),
                    _ => {
                        let mut d = StructArray::new();
                        d.set_field(0, "RelTol", Value::Num(1e-3))?;
                        d.set_field(0, "AbsTol", Value::Num(1e-6))?;
                        d.set_field(0, "MaxIterations", Value::Num(1_000_000.0))?;
                        d
                    }
                };
                let start = if matches!(argv.first(), Some(Value::Struct(_))) { 1 } else { 0 };
                let mut i = start;
                while i < argv.len() {
                    let key = match argv.get(i) {
                        Some(Value::Str(k)) => k.clone(),
                        _ => return Err(LangError::new("odeset needs name-value pairs", 0)),
                    };
                    let v = argv.get(i + 1).cloned().ok_or_else(|| LangError::new(format!("odeset: {key} needs a value"), 0))?;
                    s.set_field(0, &key, v)?;
                    i += 2;
                }
                Ok(vec![Value::Struct(s)])
            }
            "odeget" => {
                let s = match argv.first() {
                    Some(Value::Struct(s)) => s,
                    _ => return Err(LangError::new("odeget needs an options struct", 0)),
                };
                let key = match argv.get(1) {
                    Some(Value::Str(k)) => k.clone(),
                    _ => return Err(LangError::new("odeget needs an option name", 0)),
                };
                match s.field_at(0, &key).cloned() {
                    Some(v) => Ok(vec![v]),
                    None => match argv.get(2).cloned() {
                        Some(v) => Ok(vec![v]),
                        None => Err(LangError::new(format!("odeget: option '{key}' is not set"), 0)),
                    },
                }
            }
            "ode45" | "ode15s" => {
                let target = argv.first().ok_or_else(|| LangError::new(format!("{name} needs an odefun"), 0))?.clone();
                let span_m = argv.get(1).ok_or_else(|| LangError::new(format!("{name} needs tspan"), 0))?.to_numeric()?;
                if span_m.numel() < 2 {
                    return Err(LangError::new("tspan needs at least two times", 0));
                }
                let span = span_m.data().to_vec();
                if span.windows(2).any(|w| w.get(1).copied().unwrap_or(0.0) <= w.first().copied().unwrap_or(0.0)) {
                    return Err(LangError::new("tspan must be strictly increasing", 0));
                }
                let y0 = state_vector(argv.get(2).ok_or_else(|| LangError::new(format!("{name} needs y0"), 0))?, "y0")?;
                if y0.is_empty() {
                    return Err(LangError::new("y0 must not be empty", 0));
                }
                check_elems(y0.len().saturating_mul(y0.len()).saturating_mul(4))?;
                let opts = options_from_value(argv.get(3))?;
                let (ts, ys, fs) =
                    self.ode_integrate(&target, name == "ode15s", span.first().copied().unwrap_or(0.0), *span.last().unwrap_or(&0.0), &y0, &opts)?;
                if span.len() > 2 {
                    let (to, yo) = ode_interpolate(&ts, &ys, &fs, &span)?;
                    let mut mat = Matrix::zeros(yo.len(), y0.len());
                    for (r, row) in yo.iter().enumerate() {
                        for (c, v) in row.iter().enumerate() {
                            let _ = mat.set(r, c, *v);
                        }
                    }
                    Ok(truncate_ode(vec![Value::Mat(Matrix::col_vector(to)), Value::Mat(mat)], nout))
                } else {
                    let mut mat = Matrix::zeros(ys.len(), y0.len());
                    for (r, row) in ys.iter().enumerate() {
                        for (c, v) in row.iter().enumerate() {
                            let _ = mat.set(r, c, *v);
                        }
                    }
                    Ok(truncate_ode(vec![Value::Mat(Matrix::col_vector(ts)), Value::Mat(mat)], nout))
                }
            }
            _ => Err(LangError::new(format!("{name}: not an ODE builtin"), 0)),
        }
    }
}

fn truncate_ode(mut out: Vec<Value>, nout: usize) -> Vec<Value> {
    out.truncate(nout.max(1).min(out.len()));
    out
}

fn ode_interpolate(ts: &[f64], ys: &[Vec<f64>], fs: &[Vec<f64>], tout: &[f64]) -> Result<(Vec<f64>, Vec<Vec<f64>>), LangError> {
    let n = ys.first().map(Vec::len).unwrap_or(0);
    let mut out = Vec::with_capacity(tout.len());
    for &t in tout {
        if t < ts.first().copied().unwrap_or(0.0) - 1e-12 || t > ts.last().copied().unwrap_or(0.0) + 1e-12 {
            return Err(LangError::new("output time is outside the integration interval", 0));
        }
        let mut idx = 0usize;
        while idx + 1 < ts.len() && ts.get(idx + 1).copied().unwrap_or(0.0) < t {
            idx += 1;
        }
        let t0 = ts.get(idx).copied().unwrap_or(0.0);
        let t1 = ts.get(idx + 1).copied().unwrap_or(t0);
        let y0 = ys.get(idx).cloned().unwrap_or_else(|| vec![0.0; n]);
        let y1 = ys.get(idx + 1).cloned().unwrap_or_else(|| y0.clone());
        if (t - t0).abs() <= 1e-14 * (1.0 + t0.abs()) {
            out.push(y0);
            continue;
        }
        if (t - t1).abs() <= 1e-14 * (1.0 + t1.abs()) {
            out.push(y1);
            continue;
        }
        let h = t1 - t0;
        if h.abs() < 1e-300 {
            out.push(y1);
            continue;
        }
        let s = (t - t0) / h;
        let f0 = fs.get(idx).cloned().unwrap_or_else(|| vec![0.0; n]);
        let f1 = fs.get(idx + 1).cloned().unwrap_or_else(|| vec![0.0; n]);
        let h00 = 2.0 * s * s * s - 3.0 * s * s + 1.0;
        let h10 = s * s * s - 2.0 * s * s + s;
        let h01 = -2.0 * s * s * s + 3.0 * s * s;
        let h11 = s * s * s - s * s;
        let mut row = Vec::with_capacity(n);
        for i in 0..n {
            row.push(
                h00 * y0.get(i).copied().unwrap_or(0.0)
                    + h10 * h * f0.get(i).copied().unwrap_or(0.0)
                    + h01 * y1.get(i).copied().unwrap_or(0.0)
                    + h11 * h * f1.get(i).copied().unwrap_or(0.0),
            );
        }
        out.push(row);
    }
    Ok((tout.to_vec(), out))
}
