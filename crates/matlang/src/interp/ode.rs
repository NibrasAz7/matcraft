//! ODE integration builtins: `ode45`, `ode15s`, `odeset`, `odeget`.
//!
//! `ode45` is adaptive Dormand–Prince 5(4). `ode15s` is a stiffly stable
//! TR-BDF2 integrator with step-doubling error control and modified Newton
//! solves using a finite-difference Jacobian. Both support scalar/vector
//! tolerances, `MaxStep`, `InitialStep`, `MaxIterations` and `NonNegative`.

#![allow(clippy::needless_range_loop, clippy::type_complexity)]

use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum OdeMethod {
    Rk45,
    Rk23,
    Trbdf2,
}

#[derive(Clone)]
struct OdeOptions {
    rel_tol: Matrix,
    abs_tol: Matrix,
    max_step: Option<f64>,
    initial_step: Option<f64>,
    max_iterations: usize,
    non_negative: Vec<usize>,
    mass: Option<Value>,
    events: Option<Value>,
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
            mass: None,
            events: None,
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
    if let Some(v) = s.field_at(0, "Mass") {
        o.mass = Some(v.clone());
    }
    if let Some(v) = s.field_at(0, "Events") {
        o.events = Some(v.clone());
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
    fn ode_mass_apply(&mut self, mass: Option<&Value>, t: f64, y: &[f64], f: Vec<f64>) -> Result<Vec<f64>, LangError> {
        let Some(mass) = mass else { return Ok(f) };
        let n = f.len();
        let m = match mass {
            Value::Mat(m) => m.clone(),
            Value::Num(x) => {
                let mut m = Matrix::eye(n, n);
                for i in 0..n {
                    let _ = m.set(i, i, *x);
                }
                m
            }
            Value::Func(_) => {
                let out = self.call_feval_target(mass, vec![Value::Num(t), Value::Mat(Matrix::col_vector(y.to_vec()))], 1)?;
                out.first().ok_or_else(|| LangError::new("Mass returned no value", 0))?.to_matrix()?
            }
            _ => return Err(LangError::new("Mass must be a matrix or function handle", 0)),
        };
        if m.rows() != n || m.cols() != n {
            return Err(LangError::new("Mass must be an n-by-n matrix", 0));
        }
        let solved = m.solve(&Matrix::col_vector(f)).map_err(|_| LangError::new("Mass matrix is singular", 0))?;
        Ok(solved.data().to_vec())
    }

    fn ode_eval(&mut self, target: &Value, t: f64, y: &[f64], mass: Option<&Value>) -> Result<Vec<f64>, LangError> {
        let out = self.call_feval_target(target, vec![Value::Num(t), Value::Mat(Matrix::col_vector(y.to_vec()))], 1)?;
        let m = out.first().ok_or_else(|| LangError::new("odefun returned no value", 0))?.to_numeric()?;
        if m.numel() != y.len() {
            return Err(LangError::new(format!("odefun must return {} values, got {}", y.len(), m.numel()), 0));
        }
        self.ode_mass_apply(mass, t, y, m.data().to_vec())
    }

    fn ode_jacobian(&mut self, target: &Value, t: f64, y: &[f64], f0: &[f64], mass: Option<&Value>) -> Result<Matrix, LangError> {
        let n = y.len();
        let mut j = Matrix::zeros(n, n);
        for c in 0..n {
            let mut yp = y.to_vec();
            let delta = 1e-8 * (1.0 + y.get(c).copied().unwrap_or(0.0).abs());
            if let Some(slot) = yp.get_mut(c) {
                *slot += delta;
            }
            let fp = self.ode_eval(target, t, &yp, mass)?;
            for r in 0..n {
                let d = (fp.get(r).copied().unwrap_or(0.0) - f0.get(r).copied().unwrap_or(0.0)) / delta;
                let _ = j.set(r, c, d);
            }
        }
        Ok(j)
    }

    fn ode_event_values(&mut self, target: &Value, t: f64, y: &[f64]) -> Result<(Vec<f64>, Vec<f64>, Vec<f64>), LangError> {
        let args = vec![Value::Num(t), Value::Mat(Matrix::col_vector(y.to_vec()))];
        let outs = self.call_feval_target(target, args.clone(), 3).or_else(|_| self.call_feval_target(target, args, 1))?;
        let val = outs.first().ok_or_else(|| LangError::new("Events returned no value", 0))?.to_matrix()?;
        let values = val.data().to_vec();
        let n = values.len();
        let term = outs.get(1).and_then(|x| x.to_matrix().ok()).map(|m| m.data().to_vec()).unwrap_or_else(|| vec![1.0; n]);
        let dir = outs.get(2).and_then(|x| x.to_matrix().ok()).map(|m| m.data().to_vec()).unwrap_or_else(|| vec![0.0; n]);
        Ok((values, term, dir))
    }

    fn rk45_step(&mut self, target: &Value, t: f64, y: &[f64], h: f64, mass: Option<&Value>) -> Result<(Vec<f64>, Vec<f64>, Vec<f64>), LangError> {
        let n = y.len();
        let k1 = self.ode_eval(target, t, y, mass)?;
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
        let k2 = self.ode_eval(target, t + h / 5.0, &stage(0.0, &[(1.0 / 5.0, &k1)])?, mass)?;
        let k3 = self.ode_eval(target, t + 3.0 * h / 10.0, &stage(0.0, &[(3.0 / 40.0, &k1), (9.0 / 40.0, &k2)])?, mass)?;
        let k4 = self.ode_eval(target, t + 4.0 * h / 5.0, &stage(0.0, &[(44.0 / 45.0, &k1), (-56.0 / 15.0, &k2), (32.0 / 9.0, &k3)])?, mass)?;
        let k5 = self.ode_eval(
            target,
            t + 8.0 * h / 9.0,
            &stage(0.0, &[(19372.0 / 6561.0, &k1), (-25360.0 / 2187.0, &k2), (64448.0 / 6561.0, &k3), (-212.0 / 729.0, &k4)])?,
            mass,
        )?;
        let k6 = self.ode_eval(
            target,
            t + h,
            &stage(0.0, &[(9017.0 / 3168.0, &k1), (-355.0 / 33.0, &k2), (46732.0 / 5247.0, &k3), (49.0 / 176.0, &k4), (-5103.0 / 18656.0, &k5)])?,
            mass,
        )?;
        let ynew = stage(0.0, &[(35.0 / 384.0, &k1), (500.0 / 1113.0, &k3), (125.0 / 192.0, &k4), (-2187.0 / 6784.0, &k5), (11.0 / 84.0, &k6)])?;
        let fnew = self.ode_eval(target, t + h, &ynew, mass)?;
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

    /// Bogacki–Shampine 3(2) step used by `ode23`.
    fn rk23_step(&mut self, target: &Value, t: f64, y: &[f64], h: f64, mass: Option<&Value>) -> Result<(Vec<f64>, Vec<f64>, Vec<f64>), LangError> {
        let n = y.len();
        let stage = |sums: &[(f64, &Vec<f64>)]| -> Vec<f64> {
            (0..n)
                .map(|i| {
                    let mut v = y.get(i).copied().unwrap_or(0.0);
                    for (a, k) in sums {
                        v += h * *a * k.get(i).copied().unwrap_or(0.0);
                    }
                    v
                })
                .collect()
        };
        let k1 = self.ode_eval(target, t, y, mass)?;
        let k2 = self.ode_eval(target, t + h / 2.0, &stage(&[(1.0 / 2.0, &k1)]), mass)?;
        let k3 = self.ode_eval(target, t + 3.0 * h / 4.0, &stage(&[(3.0 / 4.0, &k2)]), mass)?;
        let ynew = stage(&[(2.0 / 9.0, &k1), (1.0 / 3.0, &k2), (4.0 / 9.0, &k3)]);
        let fnew = self.ode_eval(target, t + h, &ynew, mass)?;
        let second = stage(&[(7.0 / 24.0, &k1), (1.0 / 4.0, &k2), (1.0 / 3.0, &k3), (1.0 / 8.0, &fnew)]);
        let err = ynew.iter().zip(second.iter()).map(|(a, b)| a - b).collect();
        Ok((ynew, err, fnew))
    }

    fn trbdf2_step(&mut self, target: &Value, t: f64, y: &[f64], h: f64, mass: Option<&Value>) -> Result<(Vec<f64>, Vec<f64>), LangError> {
        let n = y.len();
        let gamma = 2.0 - 2f64.sqrt();
        let f0 = self.ode_eval(target, t, y, mass)?;
        let mut yg = vec![0.0f64; n];
        for i in 0..n {
            yg[i] = y.get(i).copied().unwrap_or(0.0) + gamma * h * f0.get(i).copied().unwrap_or(0.0);
        }
        for _ in 0..10 {
            let fg = self.ode_eval(target, t + gamma * h, &yg, mass)?;
            let mut resid = vec![0.0f64; n];
            for i in 0..n {
                resid[i] = yg.get(i).copied().unwrap_or(0.0)
                    - y.get(i).copied().unwrap_or(0.0)
                    - gamma * h / 2.0 * (f0.get(i).copied().unwrap_or(0.0) + fg.get(i).copied().unwrap_or(0.0));
            }
            if max_abs(&resid) <= 1e-12 * (1.0 + max_abs(&yg)) {
                break;
            }
            let jac = self.ode_jacobian(target, t + gamma * h, &yg, &fg, mass)?;
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
            let fn_ = self.ode_eval(target, t + h, &yn, mass)?;
            let mut resid = vec![0.0f64; n];
            for i in 0..n {
                resid[i] = yn.get(i).copied().unwrap_or(0.0) - base.get(i).copied().unwrap_or(0.0) - c2 * h * fn_.get(i).copied().unwrap_or(0.0);
            }
            if max_abs(&resid) <= 1e-12 * (1.0 + max_abs(&yn)) {
                break;
            }
            let jac = self.ode_jacobian(target, t + h, &yn, &fn_, mass)?;
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
        let fnew = self.ode_eval(target, t + h, &yn, mass)?;
        Ok((yn, fnew))
    }

    fn ode_integrate(
        &mut self,
        target: &Value,
        method: OdeMethod,
        t0: f64,
        tf: f64,
        y0: &[f64],
        opts: &OdeOptions,
    ) -> Result<(Vec<f64>, Vec<Vec<f64>>, Vec<Vec<f64>>, Vec<(usize, f64, Vec<f64>)>), LangError> {
        let n = y0.len();
        let (rt, at) = opts.tolerances(n)?;
        let span = tf - t0;
        if span.is_nan() || span <= 0.0 {
            return Err(LangError::new("tspan must increase", 0));
        }
        let max_step = opts.max_step.unwrap_or(span / 10.0).max(1e-12);
        let mut h = opts.initial_step.unwrap_or(if method == OdeMethod::Trbdf2 { span * 1e-3 } else { span * 0.01 });
        h = h.min(max_step).min(span);
        let mut t = t0;
        let mut y = y0.to_vec();
        let mut ts = vec![t];
        let mut ys = vec![y.clone()];
        let mut fs = vec![self.ode_eval(target, t, &y, opts.mass.as_ref())?];
        let mut attempts = 0usize;
        let mut prev_events = if let Some(ev) = &opts.events { Some(self.ode_event_values(ev, t, &y)?) } else { None };
        let mut events_out: Vec<(usize, f64, Vec<f64>)> = Vec::new();
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
            let (ynew, err) = match method {
                OdeMethod::Trbdf2 => {
                    let (y1, _) = self.trbdf2_step(target, t, &y, h_use, opts.mass.as_ref())?;
                    let (yh, _) = self.trbdf2_step(target, t, &y, h_use / 2.0, opts.mass.as_ref())?;
                    let (y2, _) = self.trbdf2_step(target, t + h_use / 2.0, &yh, h_use / 2.0, opts.mass.as_ref())?;
                    let e: Vec<f64> = y2.iter().zip(y1.iter()).map(|(a, b)| (a - b) / 3.0).collect();
                    (y2, e)
                }
                OdeMethod::Rk45 => {
                    let (yn, e, _) = self.rk45_step(target, t, &y, h_use, opts.mass.as_ref())?;
                    (yn, e)
                }
                OdeMethod::Rk23 => {
                    let (yn, e, _) = self.rk23_step(target, t, &y, h_use, opts.mass.as_ref())?;
                    (yn, e)
                }
            };
            let mut scaled = 0.0f64;
            for i in 0..n {
                let scale = at.get(i).copied().unwrap_or(1e-6)
                    + rt.get(i).copied().unwrap_or(1e-3) * y.get(i).copied().unwrap_or(0.0).abs().max(ynew.get(i).copied().unwrap_or(0.0).abs());
                scaled = scaled.max(err.get(i).copied().unwrap_or(0.0).abs() / scale);
            }
            let order = match method {
                OdeMethod::Rk45 => 4.0,
                OdeMethod::Rk23 | OdeMethod::Trbdf2 => 2.0,
            };
            if scaled <= 1.0 {
                let t_step = (t + h_use).min(tf);
                let mut accepted_t = t_step;
                let mut accepted_y = ynew;
                let mut stop = false;
                let mut new_events = None;
                if let Some(ev) = &opts.events {
                    new_events = Some(self.ode_event_values(ev, t_step, &accepted_y)?);
                }
                if let (Some(prev), Some(cur)) = (&prev_events, &new_events) {
                    let mut crossing: Option<(usize, f64)> = None;
                    for i in 0..prev.0.len().min(cur.0.len()) {
                        let a = prev.0.get(i).copied().unwrap_or(0.0);
                        let b = cur.0.get(i).copied().unwrap_or(0.0);
                        let dir = prev.2.get(i).copied().unwrap_or(0.0) as i32;
                        let hit = match dir {
                            1 => a < 0.0 && b >= 0.0,
                            -1 => a > 0.0 && b <= 0.0,
                            _ => (a < 0.0 && b >= 0.0) || (a > 0.0 && b <= 0.0),
                        };
                        if hit {
                            let frac = if (a - b).abs() > 1e-300 { (a / (a - b)).clamp(0.0, 1.0) } else { 1.0 };
                            crossing = Some((i, frac));
                            break;
                        }
                    }
                    if let Some((i, frac)) = crossing {
                        accepted_t = t + h_use * frac;
                        for k in 0..n {
                            accepted_y[k] = y.get(k).copied().unwrap_or(0.0)
                                + frac * (accepted_y.get(k).copied().unwrap_or(0.0) - y.get(k).copied().unwrap_or(0.0));
                        }
                        stop = prev.1.get(i).copied().unwrap_or(1.0) != 0.0;
                        events_out.push((i + 1, accepted_t, accepted_y.clone()));
                    }
                }
                t = accepted_t;
                y = accepted_y;
                for idx in &opts.non_negative {
                    if *idx >= 1 && *idx <= n {
                        if let Some(slot) = y.get_mut(idx - 1) {
                            *slot = slot.max(0.0);
                        }
                    }
                }
                if let Some(ev) = &opts.events {
                    prev_events = Some(self.ode_event_values(ev, t, &y)?);
                }
                ts.push(t);
                ys.push(y.clone());
                fs.push(self.ode_eval(target, t, &y, opts.mass.as_ref())?);
                if stop {
                    break;
                }
                let fac = if scaled > 1e-12 { 0.9 * scaled.powf(-1.0 / (order + 1.0)) } else { 5.0 };
                h = (h_use * fac.clamp(0.2, 5.0)).min(max_step).max(1e-15 * span);
            } else {
                let fac = (0.9 * scaled.powf(-1.0 / (order + 1.0))).clamp(0.1, 0.9);
                h = (h_use * fac).max(1e-15 * span);
            }
        }
        Ok((ts, ys, fs, events_out))
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
            "deval" => {
                let sol = match argv.first() {
                    Some(Value::Struct(s)) => s,
                    _ => return Err(LangError::new("deval needs a solution struct", 0)),
                };
                let ti = argv.get(1).ok_or_else(|| LangError::new("deval needs query times", 0))?.to_numeric()?;
                let ts: Vec<f64> = sol.field_at(0, "x").and_then(|v| v.to_matrix().ok()).map(|m| m.data().to_vec()).unwrap_or_default();
                let ymat = sol.field_at(0, "y").and_then(|v| v.to_matrix().ok()).ok_or_else(|| LangError::new("deval: solution has no y", 0))?;
                let fmat = sol.field_at(0, "f").and_then(|v| v.to_matrix().ok()).ok_or_else(|| LangError::new("deval: solution has no f", 0))?;
                let states = ymat.rows();
                let ys: Vec<Vec<f64>> = (0..ts.len()).map(|j| (0..states).map(|i| ymat.get(i, j).unwrap_or(0.0)).collect()).collect();
                let fs: Vec<Vec<f64>> = (0..ts.len()).map(|j| (0..states).map(|i| fmat.get(i, j).unwrap_or(0.0)).collect()).collect();
                let tout = ti.data().to_vec();
                let (to, yo) = ode_interpolate(&ts, &ys, &fs, &tout)?;
                let mut out = Matrix::zeros(states, to.len());
                for (j, row) in yo.iter().enumerate() {
                    for (i, v) in row.iter().enumerate() {
                        let _ = out.set(i, j, *v);
                    }
                }
                Ok(vec![Value::Mat(out)])
            }
            "ode45" | "ode15s" | "ode23" | "ode23s" => {
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
                let method = match name {
                    "ode15s" | "ode23s" => OdeMethod::Trbdf2,
                    "ode23" => OdeMethod::Rk23,
                    _ => OdeMethod::Rk45,
                };
                let (ts, ys, fs, events) =
                    self.ode_integrate(&target, method, span.first().copied().unwrap_or(0.0), *span.last().unwrap_or(&0.0), &y0, &opts)?;
                let ie = Matrix::col_vector(events.iter().map(|(i, _, _)| *i as f64).collect());
                let te = Matrix::col_vector(events.iter().map(|(_, t, _)| *t).collect());
                let mut ye = Matrix::zeros(events.len(), y0.len());
                for (r, (_, _, yy)) in events.iter().enumerate() {
                    for (c, v) in yy.iter().enumerate() {
                        let _ = ye.set(r, c, *v);
                    }
                }
                if nout == 1 {
                    let states = y0.len();
                    let mut ymat = Matrix::zeros(states, ts.len());
                    let mut fmat = Matrix::zeros(states, ts.len());
                    for (j, row) in ys.iter().enumerate() {
                        for (i, v) in row.iter().enumerate() {
                            let _ = ymat.set(i, j, *v);
                            let _ = fmat.set(i, j, fs.get(j).and_then(|f| f.get(i)).copied().unwrap_or(0.0));
                        }
                    }
                    let mut sol = StructArray::new();
                    sol.set_field(0, "x", Value::Mat(Matrix::col_vector(ts)))?;
                    sol.set_field(0, "y", Value::Mat(ymat))?;
                    sol.set_field(0, "f", Value::Mat(fmat))?;
                    sol.set_field(0, "ie", Value::Mat(ie))?;
                    sol.set_field(0, "te", Value::Mat(te))?;
                    sol.set_field(0, "ye", Value::Mat(ye))?;
                    sol.set_field(0, "solver", Value::Str(name.to_string()))?;
                    return Ok(vec![Value::Struct(sol)]);
                }
                if span.len() > 2 {
                    let (to, yo) = ode_interpolate(&ts, &ys, &fs, &span)?;
                    let mut mat = Matrix::zeros(yo.len(), y0.len());
                    for (r, row) in yo.iter().enumerate() {
                        for (c, v) in row.iter().enumerate() {
                            let _ = mat.set(r, c, *v);
                        }
                    }
                    Ok(truncate_ode(vec![Value::Mat(Matrix::col_vector(to)), Value::Mat(mat), Value::Mat(ie), Value::Mat(te), Value::Mat(ye)], nout))
                } else {
                    let mut mat = Matrix::zeros(ys.len(), y0.len());
                    for (r, row) in ys.iter().enumerate() {
                        for (c, v) in row.iter().enumerate() {
                            let _ = mat.set(r, c, *v);
                        }
                    }
                    Ok(truncate_ode(vec![Value::Mat(Matrix::col_vector(ts)), Value::Mat(mat), Value::Mat(ie), Value::Mat(te), Value::Mat(ye)], nout))
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
