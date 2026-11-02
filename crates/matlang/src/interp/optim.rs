//! Optimization toolbox builtins.
//!
//! Clean-room implementations of the documented MATLAB interfaces for
//! `fminbnd`, `fminsearch`, `fminunc`, `fsolve`, `lsqnonlin` and
//! `lsqcurvefit`:
//!
//! * `fminbnd` — Brent's bounded minimisation (golden section with
//!   parabolic interpolation).
//! * `fminsearch` — Nelder–Mead simplex on N parameters.
//! * `fminunc` — BFGS quasi-Newton with central-difference gradients.
//! * `fsolve`, `lsqnonlin`, `lsqcurvefit` — Levenberg–Marquardt with
//!   central-difference Jacobians and optional box bounds.
//!
//! Options are read from `optimset`/`optimoptions` structs by the usual
//! MATLAB names (`MaxIterations`/`MaxIter`, `MaxFunctionEvaluations`/
//! `MaxFunEvals`, `StepTolerance`/`TolX`, `FunctionTolerance`/`TolFun`,
//! `OptimalityTolerance`, `FiniteDifferenceStepSize`, `Display`).

use super::*;

/// Relative step for central differences: eps^(1/3) balances truncation
/// and round-off for a first derivative.
const FD_BASE: f64 = 6.055_454_452_942_879e-6;

#[derive(Debug, Clone)]
struct Opts {
    max_iter: usize,
    max_fevals: usize,
    tol_x: f64,
    tol_fun: f64,
    tol_grad: f64,
    tol_constr: f64,
    fd_step: Option<f64>,
    display: bool,
}

impl Opts {
    fn for_problem(n: usize) -> Self {
        let n = n.max(1);
        Opts {
            max_iter: 400usize.saturating_mul(n),
            max_fevals: 4000usize.saturating_mul(n),
            tol_x: 1e-8,
            tol_fun: 1e-10,
            tol_grad: 1e-6,
            tol_constr: 1e-6,
            fd_step: None,
            display: false,
        }
    }

    fn from_value(v: Option<&Value>, n: usize) -> Result<Self, LangError> {
        let mut o = Opts::for_problem(n);
        let Some(Value::Struct(s)) = v else {
            return Ok(o);
        };
        if let Some(x) = field_matches(s, &["maxiter", "maxiterations"]).and_then(|x| x.to_matrix().ok()).and_then(|m| m.as_scalar()) {
            if x.is_finite() && x >= 0.0 {
                o.max_iter = x as usize;
            }
        }
        if let Some(x) = field_matches(s, &["maxfunevals", "maxfunctionevaluations"]).and_then(|x| x.to_matrix().ok()).and_then(|m| m.as_scalar()) {
            if x.is_finite() && x >= 0.0 {
                o.max_fevals = x as usize;
            }
        }
        if let Some(x) = field_matches(s, &["tolx", "steptolerance"]).and_then(|x| x.to_matrix().ok()).and_then(|m| m.as_scalar()) {
            if x.is_finite() && x > 0.0 {
                o.tol_x = x;
            }
        }
        if let Some(x) = field_matches(s, &["tolfun", "functiontolerance"]).and_then(|x| x.to_matrix().ok()).and_then(|m| m.as_scalar()) {
            if x.is_finite() && x > 0.0 {
                o.tol_fun = x;
            }
        }
        if let Some(x) = field_matches(s, &["optimalitytolerance", "tolg"]).and_then(|x| x.to_matrix().ok()).and_then(|m| m.as_scalar()) {
            if x.is_finite() && x > 0.0 {
                o.tol_grad = x;
            }
        }
        if let Some(x) =
            field_matches(s, &["constrainttolerance", "tolcon", "tolconstr"]).and_then(|x| x.to_matrix().ok()).and_then(|m| m.as_scalar())
        {
            if x.is_finite() && x > 0.0 {
                o.tol_constr = x;
            }
        }
        if let Some(x) = field_matches(s, &["finitedifferencestepsize", "fdstep"]).and_then(|x| x.to_matrix().ok()).and_then(|m| m.as_scalar()) {
            if x.is_finite() && x > 0.0 {
                o.fd_step = Some(x);
            }
        }
        if let Some(Value::Str(d)) = field_matches(s, &["display"]) {
            o.display = !matches!(d.to_ascii_lowercase().as_str(), "off" | "none");
        }
        Ok(o)
    }
}

/// Case-insensitive option lookup; the aliases are lower-case.
fn field_matches(s: &StructArray, aliases: &[&str]) -> Option<Value> {
    for name in s.field_names() {
        if aliases.contains(&name.to_ascii_lowercase().as_str()) {
            if let Some(v) = s.field_at(0, &name) {
                return Some(v.clone());
            }
        }
    }
    None
}

#[derive(Debug, Clone, Copy)]
struct Stats {
    iters: usize,
    fevals: usize,
    max_fevals: usize,
}

impl Stats {
    fn new(max_fevals: usize) -> Self {
        Stats { iters: 0, fevals: 0, max_fevals }
    }

    fn exhausted(&self) -> bool {
        self.fevals >= self.max_fevals
    }
}

/// A finite-difference step scaled to the magnitude of the parameter.
fn fd_step(xi: f64, opts: &Opts) -> f64 {
    let base = opts.fd_step.unwrap_or(FD_BASE);
    let h = base * (1.0 + xi.abs());
    if h.is_finite() && h > 0.0 {
        h
    } else {
        FD_BASE
    }
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn norm2(a: &[f64]) -> f64 {
    dot(a, a).sqrt()
}

fn norm_inf(a: &[f64]) -> f64 {
    a.iter().fold(0.0f64, |m, &x| m.max(x.abs()))
}

fn mat_vec(h: &[f64], v: &[f64], n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| (0..n).map(|j| h.get(i.saturating_mul(n).saturating_add(j)).copied().unwrap_or(0.0) * v.get(j).copied().unwrap_or(0.0)).sum())
        .collect()
}

/// BFGS inverse-Hessian update; `h` is row-major `n x n`.
fn bfgs_update(h: &mut [f64], s: &[f64], y: &[f64], n: usize) {
    let sy = dot(s, y);
    if sy.is_nan() || sy <= 1e-12 {
        return;
    }
    let rho = 1.0 / sy;
    let hy = mat_vec(h, y, n);
    let yhy = dot(y, &hy);
    let coef = rho * rho * yhy + rho;
    for i in 0..n {
        for j in 0..n {
            let idx = i.saturating_mul(n).saturating_add(j);
            let si = s.get(i).copied().unwrap_or(0.0);
            let sj = s.get(j).copied().unwrap_or(0.0);
            let hyi = hy.get(i).copied().unwrap_or(0.0);
            let hyj = hy.get(j).copied().unwrap_or(0.0);
            if let Some(slot) = h.get_mut(idx) {
                *slot += -rho * (si * hyj + hyi * sj) + coef * si * sj;
            }
        }
    }
}

fn identity(n: usize) -> Vec<f64> {
    let mut h = vec![0.0; n.saturating_mul(n)];
    for i in 0..n {
        if let Some(slot) = h.get_mut(i.saturating_mul(n).saturating_add(i)) {
            *slot = 1.0;
        }
    }
    h
}

fn output_struct(iters: usize, fevals: usize, algorithm: &str, message: &str, firstorderopt: Option<f64>) -> Result<Value, LangError> {
    let mut s = StructArray::new();
    s.set_field(0, "iterations", Value::Num(iters as f64))?;
    s.set_field(0, "funcCount", Value::Num(fevals as f64))?;
    s.set_field(0, "algorithm", Value::Str(algorithm.to_string()))?;
    s.set_field(0, "message", Value::Str(message.to_string()))?;
    if let Some(g) = firstorderopt {
        s.set_field(0, "firstorderopt", Value::Num(g))?;
    }
    Ok(Value::Struct(s))
}

fn target_arg(v: Option<&Value>, name: &str) -> Result<Value, LangError> {
    match v {
        Some(t @ (Value::Func(_) | Value::Str(_))) => Ok(t.clone()),
        Some(_) => Err(LangError::new(format!("{name}: first argument must be a function handle or function name"), 0)),
        None => Err(LangError::new(format!("{name} needs a function handle or function name"), 0)),
    }
}

fn vector_arg(v: Option<&Value>, what: &str) -> Result<Vec<f64>, LangError> {
    let v = v.ok_or_else(|| LangError::new(format!("{what} is required"), 0))?;
    let m = v.to_matrix()?;
    if m.numel() == 0 {
        return Err(LangError::new(format!("{what} must not be empty"), 0));
    }
    Ok(m.data().to_vec())
}

fn bound_vec(v: Option<&Value>, n: usize, default: f64) -> Result<Vec<f64>, LangError> {
    let Some(v) = v else {
        return Ok(vec![default; n]);
    };
    let m = v.to_matrix()?;
    if m.numel() == 0 {
        return Ok(vec![default; n]);
    }
    if m.numel() == 1 {
        let b = m.data().first().copied().unwrap_or(default);
        return Ok(vec![b; n]);
    }
    if m.numel() == n {
        return Ok(m.data().to_vec());
    }
    Err(LangError::new("bounds must be scalar or the same length as x0", 0))
}

fn clamp_step(x: &[f64], dx: &mut [f64], lb: &[f64], ub: &[f64]) {
    for i in 0..x.len() {
        let xi = x.get(i).copied().unwrap_or(0.0);
        let lo = lb.get(i).copied().unwrap_or(f64::NEG_INFINITY);
        let hi = ub.get(i).copied().unwrap_or(f64::INFINITY);
        let step = dx.get(i).copied().unwrap_or(0.0);
        let mut cand = xi + step;
        if cand < lo {
            cand = lo;
        }
        if cand > hi {
            cand = hi;
        }
        if let Some(slot) = dx.get_mut(i) {
            *slot = cand - xi;
        }
    }
}

/// Which residual evaluation a least-squares problem needs.
#[derive(Debug, Clone)]
enum LsqMode {
    Plain,
    Curve { xdata: Value, ydata: Vec<f64> },
}

struct LmResult {
    x: Vec<f64>,
    residual: Vec<f64>,
    jacobian: Matrix,
    iters: usize,
    fevals: usize,
    exitflag: f64,
    message: String,
    firstorderopt: f64,
}

struct BfgsResult {
    x: Vec<f64>,
    fx: f64,
    g: Vec<f64>,
    converged: bool,
    message: String,
}

struct QpDirection {
    dx: Vec<f64>,
    ds: Vec<f64>,
    dlam: Vec<f64>,
    dy: Vec<f64>,
}

impl Session {
    /// Dispatch entry used by `builtin`/`builtin_multi`.
    pub(super) fn optimize_builtin(&mut self, name: &str, argv: Vec<Value>, nout: usize) -> Result<Vec<Value>, LangError> {
        match name {
            "fminbnd" => self.fminbnd_call(&argv, nout),
            "fminsearch" => self.fminsearch_call(&argv, nout),
            "fminunc" => self.fminunc_call(&argv, nout),
            "fsolve" => self.fsolve_call(&argv, nout),
            "lsqnonlin" => self.lsqnonlin_call(&argv, nout),
            "lsqcurvefit" => self.lsqcurvefit_call(&argv, nout),
            "linprog" => self.linprog_call(&argv, nout),
            "quadprog" => self.quadprog_call(&argv, nout),
            "fmincon" => self.fmincon_call(&argv, nout),
            "patternsearch" => self.patternsearch_call(&argv, nout),
            _ => Err(LangError::new(format!("{name}: unknown optimization function"), 0)),
        }
    }

    fn eval_objective(&mut self, target: &Value, x: &[f64], stats: &mut Stats) -> Result<f64, LangError> {
        stats.fevals = stats.fevals.saturating_add(1);
        let arg = if x.len() == 1 { Value::Num(x.first().copied().unwrap_or(0.0)) } else { Value::Mat(Matrix::col_vector(x.to_vec())) };
        let outs = self.call_feval_target(target, vec![arg], 1)?;
        let out = outs.first().ok_or_else(|| LangError::new("objective function returned no value", 0))?;
        let m = out.to_matrix()?;
        m.as_scalar().ok_or_else(|| LangError::new("objective function must return a scalar", 0))
    }

    fn eval_residual(&mut self, target: &Value, x: &[f64], mode: &LsqMode, stats: &mut Stats) -> Result<Vec<f64>, LangError> {
        stats.fevals = stats.fevals.saturating_add(1);
        let xv = Value::Mat(Matrix::col_vector(x.to_vec()));
        let outs = match mode {
            LsqMode::Plain => self.call_feval_target(target, vec![xv], 1)?,
            LsqMode::Curve { xdata, .. } => self.call_feval_target(target, vec![xv, xdata.clone()], 1)?,
        };
        let out = outs.first().ok_or_else(|| LangError::new("residual function returned no value", 0))?;
        let m = out.to_matrix()?;
        let mut r = m.data().to_vec();
        if let LsqMode::Curve { ydata, .. } = mode {
            if r.len() != ydata.len() {
                return Err(LangError::new("the model function and ydata must have the same number of elements", 0));
            }
            for (a, b) in r.iter_mut().zip(ydata.iter()) {
                *a -= *b;
            }
        }
        if r.is_empty() {
            return Err(LangError::new("residual function returned an empty array", 0));
        }
        if r.iter().any(|x| !x.is_finite()) {
            return Err(LangError::new("residual function returned NaN or Inf", 0));
        }
        Ok(r)
    }

    fn gradient(&mut self, target: &Value, x: &[f64], opts: &Opts, stats: &mut Stats) -> Result<Vec<f64>, LangError> {
        let n = x.len();
        let mut g = vec![0.0; n];
        for j in 0..n {
            let xj = x.get(j).copied().unwrap_or(0.0);
            let h = fd_step(xj, opts);
            let mut xp = x.to_vec();
            let mut xm = x.to_vec();
            if let Some(slot) = xp.get_mut(j) {
                *slot = xj + h;
            }
            if let Some(slot) = xm.get_mut(j) {
                *slot = xj - h;
            }
            let fp = self.eval_objective(target, &xp, stats)?;
            let fm = self.eval_objective(target, &xm, stats)?;
            if let Some(slot) = g.get_mut(j) {
                *slot = (fp - fm) / (2.0 * h);
            }
        }
        Ok(g)
    }

    fn residual_jacobian(
        &mut self,
        target: &Value,
        x: &[f64],
        r0: &[f64],
        mode: &LsqMode,
        opts: &Opts,
        stats: &mut Stats,
    ) -> Result<Matrix, LangError> {
        let n = x.len();
        let m = r0.len();
        let mut jac = Matrix::zeros(m, n);
        for col in 0..n {
            let xj = x.get(col).copied().unwrap_or(0.0);
            let h = fd_step(xj, opts);
            let mut xp = x.to_vec();
            let mut xm = x.to_vec();
            if let Some(slot) = xp.get_mut(col) {
                *slot = xj + h;
            }
            if let Some(slot) = xm.get_mut(col) {
                *slot = xj - h;
            }
            let rp = self.eval_residual(target, &xp, mode, stats)?;
            let rm = self.eval_residual(target, &xm, mode, stats)?;
            if rp.len() != m || rm.len() != m {
                return Err(LangError::new("residual function returned a different number of elements", 0));
            }
            for row in 0..m {
                let v = (rp.get(row).copied().unwrap_or(0.0) - rm.get(row).copied().unwrap_or(0.0)) / (2.0 * h);
                let _ = jac.set(row, col, v);
            }
        }
        Ok(jac)
    }

    /// Brent's method on `[a, b]`; returns `(x, f(x), converged, message)`.
    fn brent(&mut self, target: &Value, a0: f64, b0: f64, opts: &Opts, stats: &mut Stats) -> Result<(f64, f64, bool, String), LangError> {
        if !a0.is_finite() || !b0.is_finite() {
            return Err(LangError::new("fminbnd: bounds must be finite", 0));
        }
        let (mut a, mut b) = if a0 <= b0 { (a0, b0) } else { (b0, a0) };
        if a == b {
            let f = self.eval_objective(target, &[a], stats)?;
            return Ok((a, f, true, "Optimization terminated: the bracket is a single point.".to_string()));
        }
        let golden = 0.381_966_011_250_105;
        let mut x = a + golden * (b - a);
        let mut w = x;
        let mut v = x;
        let mut fx = self.eval_objective(target, &[x], stats)?;
        let mut fw = fx;
        let mut fv = fx;
        let mut d: f64 = 0.0;
        let mut e: f64 = 0.0;
        let mut converged = false;
        for iter in 0..opts.max_iter {
            stats.iters = iter.saturating_add(1);
            if stats.exhausted() {
                break;
            }
            let m = 0.5 * (a + b);
            let tol = opts.tol_x.max(1e-12);
            let t2 = 2.0 * tol;
            if (x - m).abs() <= t2 - 0.5 * (b - a) {
                converged = true;
                break;
            }
            let mut use_golden = true;
            if e.abs() > tol {
                let r = (x - w) * (fx - fv);
                let q = (x - v) * (fx - fw);
                let mut p = (x - v) * q - (x - w) * r;
                let q_abs = (2.0 * (q - r)).abs();
                if q - r > 0.0 {
                    p = -p;
                }
                let r_old = e;
                e = d;
                if q_abs > 0.0 && p.abs() < (0.5 * q_abs * r_old).abs() && p > q_abs * (a - x) && p < q_abs * (b - x) {
                    d = p / q_abs;
                    let u = x + d;
                    if (u - a) < t2 || (b - u) < t2 {
                        d = if x < m { tol } else { -tol };
                    }
                    use_golden = false;
                }
            }
            if use_golden {
                e = if x < m { b - x } else { a - x };
                d = golden * e;
            }
            let step = if d.abs() >= tol {
                d
            } else if d > 0.0 {
                tol
            } else {
                -tol
            };
            let u = x + step;
            let fu = self.eval_objective(target, &[u], stats)?;
            if fu <= fx {
                if u < x {
                    b = x;
                } else {
                    a = x;
                }
                v = w;
                fv = fw;
                w = x;
                fw = fx;
                x = u;
                fx = fu;
            } else {
                if u < x {
                    a = u;
                } else {
                    b = u;
                }
                if fu <= fw || w == x {
                    v = w;
                    fv = fw;
                    w = u;
                    fw = fu;
                } else if fu <= fv || v == x || v == w {
                    v = u;
                    fv = fu;
                }
            }
        }
        let message = if converged {
            "Optimization terminated: X satisfies the step tolerance.".to_string()
        } else {
            "Maximum number of iterations or function evaluations exceeded.".to_string()
        };
        Ok((x, fx, converged, message))
    }

    fn fminbnd_call(&mut self, argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
        let target = target_arg(argv.first(), "fminbnd")?;
        let a = argv.get(1).ok_or_else(|| LangError::new("fminbnd needs x1", 0))?.scalar()?;
        let b = argv.get(2).ok_or_else(|| LangError::new("fminbnd needs x2", 0))?.scalar()?;
        let opts = Opts::from_value(argv.get(3), 1)?;
        let mut stats = Stats::new(opts.max_fevals);
        let (x, fx, converged, message) = self.brent(&target, a, b, &opts, &mut stats)?;
        if opts.display {
            self.print_buf.push_str(&format!("{message}\n"));
        }
        let exitflag = if converged { 1.0 } else { 0.0 };
        let out = vec![
            Value::Num(x),
            Value::Num(fx),
            Value::Num(exitflag),
            output_struct(stats.iters, stats.fevals, "golden section search, parabolic interpolation", &message, None)?,
        ];
        Ok(pad_outputs(out, nout))
    }

    /// Nelder–Mead on N parameters.
    fn nelder_mead(&mut self, target: &Value, x0: &[f64], opts: &Opts, stats: &mut Stats) -> Result<(Vec<f64>, f64, bool, String), LangError> {
        let n = x0.len();
        if n == 0 {
            return Err(LangError::new("fminsearch: x0 must not be empty", 0));
        }
        if stats.max_fevals == 0 {
            stats.max_fevals = opts.max_fevals;
        }
        let mut simplex: Vec<Vec<f64>> = Vec::with_capacity(n.saturating_add(1));
        simplex.push(x0.to_vec());
        for i in 0..n {
            let mut p = x0.to_vec();
            let xi = p.get(i).copied().unwrap_or(0.0);
            let step = if xi != 0.0 { 0.05 * xi } else { 0.00025 };
            if let Some(slot) = p.get_mut(i) {
                *slot = xi + step;
            }
            simplex.push(p);
        }
        let mut fvals = Vec::with_capacity(n.saturating_add(1));
        for p in &simplex {
            fvals.push(self.eval_objective(target, p, stats)?);
        }
        let mut converged = false;
        for iter in 0..opts.max_iter {
            stats.iters = iter.saturating_add(1);
            sort_simplex(&mut simplex, &mut fvals);
            let best = fvals.first().copied().unwrap_or(0.0);
            let worst = fvals.last().copied().unwrap_or(0.0);
            let spread = worst - best;
            let mut max_dist = 0.0f64;
            let best_x = simplex.first().cloned().unwrap_or_default();
            for p in simplex.iter().skip(1) {
                for j in 0..n {
                    let d = (p.get(j).copied().unwrap_or(0.0) - best_x.get(j).copied().unwrap_or(0.0)).abs();
                    max_dist = max_dist.max(d);
                }
            }
            if spread <= opts.tol_fun && max_dist <= opts.tol_x {
                converged = true;
                break;
            }
            if stats.exhausted() {
                break;
            }
            // Centroid of every vertex except the worst.
            let mut centroid = vec![0.0; n];
            for p in simplex.iter().take(n) {
                for (j, c) in centroid.iter_mut().enumerate() {
                    *c += p.get(j).copied().unwrap_or(0.0);
                }
            }
            for c in &mut centroid {
                *c /= n as f64;
            }
            let worst_x = simplex.get(n).cloned().unwrap_or_else(|| x0.to_vec());
            let xr: Vec<f64> = (0..n)
                .map(|j| centroid.get(j).copied().unwrap_or(0.0) + (centroid.get(j).copied().unwrap_or(0.0) - worst_x.get(j).copied().unwrap_or(0.0)))
                .collect();
            let fr = self.eval_objective(target, &xr, stats)?;
            let second_worst = fvals.get(n.saturating_sub(1)).copied().unwrap_or(f64::INFINITY);
            if fr < best {
                let xe: Vec<f64> = (0..n)
                    .map(|j| {
                        centroid.get(j).copied().unwrap_or(0.0)
                            + 2.0 * (centroid.get(j).copied().unwrap_or(0.0) - worst_x.get(j).copied().unwrap_or(0.0))
                    })
                    .collect();
                let fe = self.eval_objective(target, &xe, stats)?;
                if fe < fr {
                    replace_vertex(&mut simplex, &mut fvals, n, xe, fe);
                } else {
                    replace_vertex(&mut simplex, &mut fvals, n, xr, fr);
                }
            } else if fr < second_worst {
                replace_vertex(&mut simplex, &mut fvals, n, xr, fr);
            } else {
                let outside = fr < worst;
                let xc: Vec<f64> = (0..n)
                    .map(|j| {
                        let c = centroid.get(j).copied().unwrap_or(0.0);
                        let target = if outside { xr.get(j).copied().unwrap_or(0.0) } else { worst_x.get(j).copied().unwrap_or(0.0) };
                        c + 0.5 * (target - c)
                    })
                    .collect();
                let fc = self.eval_objective(target, &xc, stats)?;
                let compare = if outside { fr } else { worst };
                if fc < compare {
                    replace_vertex(&mut simplex, &mut fvals, n, xc, fc);
                } else {
                    // Shrink every vertex toward the best one.
                    let best_x = simplex.first().cloned().unwrap_or_else(|| x0.to_vec());
                    for i in 1..=n {
                        if let Some(p) = simplex.get_mut(i) {
                            for (j, slot) in p.iter_mut().enumerate() {
                                let bj = best_x.get(j).copied().unwrap_or(0.0);
                                *slot = bj + 0.5 * (*slot - bj);
                            }
                            if let Some(f) = fvals.get_mut(i) {
                                *f = self.eval_objective(target, p, stats)?;
                            }
                        }
                    }
                }
            }
        }
        sort_simplex(&mut simplex, &mut fvals);
        let x = simplex.first().cloned().unwrap_or_else(|| x0.to_vec());
        let fx = fvals.first().copied().unwrap_or(f64::NAN);
        let message = if converged {
            "Optimization terminated: the simplex collapsed to the tolerances.".to_string()
        } else {
            "Maximum number of iterations or function evaluations exceeded.".to_string()
        };
        Ok((x, fx, converged, message))
    }

    fn fminsearch_call(&mut self, argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
        let target = target_arg(argv.first(), "fminsearch")?;
        let x0 = vector_arg(argv.get(1), "x0")?;
        let opts = Opts::from_value(argv.get(2), x0.len())?;
        let mut stats = Stats::new(opts.max_fevals);
        let (x, fx, converged, message) = self.nelder_mead(&target, &x0, &opts, &mut stats)?;
        if opts.display {
            self.print_buf.push_str(&format!("{message}\n"));
        }
        let exitflag = if converged { 1.0 } else { 0.0 };
        let out = vec![
            mat_or_num(Matrix::col_vector(x)),
            Value::Num(fx),
            Value::Num(exitflag),
            output_struct(stats.iters, stats.fevals, "Nelder-Mead simplex", &message, None)?,
        ];
        Ok(pad_outputs(out, nout))
    }

    /// BFGS quasi-Newton with central-difference gradients.
    fn bfgs(&mut self, target: &Value, x0: &[f64], opts: &Opts, stats: &mut Stats) -> Result<BfgsResult, LangError> {
        let n = x0.len();
        if n == 0 {
            return Err(LangError::new("fminunc: x0 must not be empty", 0));
        }
        stats.max_fevals = opts.max_fevals;
        let mut x = x0.to_vec();
        let mut fx = self.eval_objective(target, &x, stats)?;
        let mut g = self.gradient(target, &x, opts, stats)?;
        let mut h = identity(n);
        let mut converged = false;
        let mut message = "Maximum number of iterations or function evaluations exceeded.".to_string();
        for iter in 0..opts.max_iter {
            stats.iters = iter.saturating_add(1);
            let mut gnorm = norm_inf(&g);
            if gnorm <= opts.tol_grad {
                converged = true;
                message = "Optimization terminated: the gradient norm is below the tolerance.".to_string();
                break;
            }
            if stats.exhausted() {
                break;
            }
            let mut p: Vec<f64> = mat_vec(&h, &g, n).into_iter().map(|v| -v).collect();
            let mut gp = dot(&g, &p);
            if gp.is_nan() || gp >= 0.0 {
                p = g.iter().map(|v| -v).collect();
                h = identity(n);
                gp = dot(&g, &p);
            }
            if gp.is_nan() || gp >= 0.0 {
                message = "Optimization stopped: no descent direction.".to_string();
                break;
            }
            let mut alpha = 1.0;
            let mut accepted = false;
            let mut xnew = x.clone();
            let mut fnew = fx;
            for _ in 0..40 {
                xnew = (0..n).map(|i| x.get(i).copied().unwrap_or(0.0) + alpha * p.get(i).copied().unwrap_or(0.0)).collect();
                fnew = self.eval_objective(target, &xnew, stats)?;
                if fnew <= fx + 1e-4 * alpha * gp {
                    accepted = true;
                    break;
                }
                alpha *= 0.5;
                if stats.exhausted() {
                    break;
                }
            }
            if !accepted || alpha < 1e-14 {
                message = "Optimization stopped: line search failed to find a lower point.".to_string();
                break;
            }
            let gnew = self.gradient(target, &xnew, opts, stats)?;
            let s: Vec<f64> = (0..n).map(|i| xnew.get(i).copied().unwrap_or(0.0) - x.get(i).copied().unwrap_or(0.0)).collect();
            let y: Vec<f64> = (0..n).map(|i| gnew.get(i).copied().unwrap_or(0.0) - g.get(i).copied().unwrap_or(0.0)).collect();
            let sy = dot(&s, &y);
            if sy > 1e-12 {
                bfgs_update(&mut h, &s, &y, n);
            }
            let step_norm = norm2(&s);
            let df = (fnew - fx).abs();
            x = xnew;
            fx = fnew;
            g = gnew;
            gnorm = norm_inf(&g);
            if gnorm <= opts.tol_grad || (step_norm <= opts.tol_x * (1.0 + norm2(&x)) && df <= opts.tol_fun) {
                converged = true;
                message = "Optimization terminated: the step and function changes are below the tolerances.".to_string();
                break;
            }
        }
        Ok(BfgsResult { x, fx, g, converged, message })
    }

    fn fminunc_call(&mut self, argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
        let target = target_arg(argv.first(), "fminunc")?;
        let x0 = vector_arg(argv.get(1), "x0")?;
        let opts = Opts::from_value(argv.get(2), x0.len())?;
        let mut stats = Stats::new(opts.max_fevals);
        let res = self.bfgs(&target, &x0, &opts, &mut stats)?;
        if opts.display {
            self.print_buf.push_str(&format!("{}\n", res.message));
        }
        let exitflag = if res.converged { 1.0 } else { 0.0 };
        let gnorm = norm_inf(&res.g);
        let mut out = vec![
            mat_or_num(Matrix::col_vector(res.x.clone())),
            Value::Num(res.fx),
            Value::Num(exitflag),
            output_struct(stats.iters, stats.fevals, "quasi-newton (BFGS)", &res.message, Some(gnorm))?,
            mat_or_num(Matrix::col_vector(res.g)),
        ];
        if nout >= 6 {
            let hess = self.numerical_hessian(&target, &res.x, &opts, &mut stats)?;
            out.push(Value::Mat(hess));
        }
        Ok(pad_outputs(out, nout))
    }

    fn numerical_hessian(&mut self, target: &Value, x: &[f64], opts: &Opts, stats: &mut Stats) -> Result<Matrix, LangError> {
        let n = x.len();
        let mut hess = Matrix::zeros(n, n);
        for col in 0..n {
            let xj = x.get(col).copied().unwrap_or(0.0);
            let h = (opts.fd_step.unwrap_or(1e-4) * (1.0 + xj.abs())).max(1e-8);
            let mut xp = x.to_vec();
            let mut xm = x.to_vec();
            if let Some(slot) = xp.get_mut(col) {
                *slot = xj + h;
            }
            if let Some(slot) = xm.get_mut(col) {
                *slot = xj - h;
            }
            let gp = self.gradient(target, &xp, opts, stats)?;
            let gm = self.gradient(target, &xm, opts, stats)?;
            for row in 0..n {
                let v = (gp.get(row).copied().unwrap_or(0.0) - gm.get(row).copied().unwrap_or(0.0)) / (2.0 * h);
                let _ = hess.set(row, col, v);
                if row != col {
                    let _ = hess.set(col, row, v);
                }
            }
        }
        Ok(hess)
    }

    /// Levenberg–Marquardt on a residual vector.
    fn lm_solve(&mut self, target: &Value, x0: Vec<f64>, lb: Vec<f64>, ub: Vec<f64>, mode: LsqMode, opts: &Opts) -> Result<LmResult, LangError> {
        let n = x0.len();
        if n == 0 {
            return Err(LangError::new("x0 must not be empty", 0));
        }
        for i in 0..n {
            let xi = x0.get(i).copied().unwrap_or(0.0);
            let lo = lb.get(i).copied().unwrap_or(f64::NEG_INFINITY);
            let hi = ub.get(i).copied().unwrap_or(f64::INFINITY);
            if lo > hi {
                return Err(LangError::new("lower bounds must not exceed upper bounds", 0));
            }
            if xi < lo || xi > hi {
                return Err(LangError::new("x0 is outside the bounds", 0));
            }
        }
        let mut stats = Stats::new(opts.max_fevals);
        let mut x = x0;
        let mut r = self.eval_residual(target, &x, &mode, &mut stats)?;
        let m = r.len();
        let mut lambda = 1e-3;
        let mut converged = false;
        let mut message = "Maximum number of iterations or function evaluations exceeded.".to_string();
        for iter in 0..opts.max_iter {
            stats.iters = iter.saturating_add(1);
            if stats.exhausted() {
                break;
            }
            let jac = self.residual_jacobian(target, &x, &r, &mode, opts, &mut stats)?;
            let g = jtj_grad(&jac, &r);
            let gnorm = norm_inf(&g);
            if gnorm <= opts.tol_grad {
                converged = true;
                message = "Optimization terminated: the first-order optimality measure is below the tolerance.".to_string();
                break;
            }
            let cost = dot(&r, &r);
            let jtj = jtj_matrix(&jac);
            let mut accepted = false;
            let mut position_tolerance = false;
            for _ in 0..40 {
                let mut a = jtj.clone();
                for i in 0..n {
                    let diag = jtj.get(i, i).unwrap_or(0.0).abs().max(1.0);
                    let _ = a.set(i, i, jtj.get(i, i).unwrap_or(0.0) + lambda * diag);
                }
                let mut rhs = Matrix::zeros(n, 1);
                for i in 0..n {
                    let mut s = 0.0;
                    for k in 0..m {
                        s += jac.get(k, i).unwrap_or(0.0) * r.get(k).copied().unwrap_or(0.0);
                    }
                    let _ = rhs.set(i, 0, -s);
                }
                let delta = match a.solve(&rhs) {
                    Ok(d) => d,
                    Err(_) => {
                        lambda = (lambda * 10.0).min(1e12);
                        if lambda >= 1e12 {
                            break;
                        }
                        continue;
                    }
                };
                let mut dx = delta.data().to_vec();
                clamp_step(&x, &mut dx, &lb, &ub);
                let step_norm = norm2(&dx);
                if step_norm <= opts.tol_x * (1.0 + norm2(&x)) {
                    position_tolerance = true;
                    break;
                }
                let xnew: Vec<f64> = (0..n).map(|i| x.get(i).copied().unwrap_or(0.0) + dx.get(i).copied().unwrap_or(0.0)).collect();
                let rnew = self.eval_residual(target, &xnew, &mode, &mut stats)?;
                let cost_new = dot(&rnew, &rnew);
                if cost_new.is_finite() && cost_new < cost {
                    x = xnew;
                    r = rnew;
                    lambda = (lambda * 0.3).max(1e-12);
                    accepted = true;
                    break;
                }
                lambda = (lambda * 10.0).min(1e12);
                if lambda >= 1e12 {
                    break;
                }
            }
            if position_tolerance {
                converged = true;
                message = "Optimization terminated: the step is below the step tolerance.".to_string();
                break;
            }
            if !accepted {
                message = "Optimization stopped: no step decreased the residual norm.".to_string();
                break;
            }
            let cost_new = dot(&r, &r);
            if (cost - cost_new).abs() <= opts.tol_fun * (1.0 + cost) {
                converged = true;
                message = "Optimization terminated: the residual norm change is below the function tolerance.".to_string();
                break;
            }
        }
        let jac = self.residual_jacobian(target, &x, &r, &mode, opts, &mut stats)?;
        let g = jtj_grad(&jac, &r);
        if norm_inf(&g) <= opts.tol_grad {
            converged = true;
        }
        let exitflag = if converged { 1.0 } else { 0.0 };
        Ok(LmResult { x, residual: r, jacobian: jac, iters: stats.iters, fevals: stats.fevals, exitflag, message, firstorderopt: norm_inf(&g) })
    }

    fn fsolve_call(&mut self, argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
        let target = target_arg(argv.first(), "fsolve")?;
        let x0 = vector_arg(argv.get(1), "x0")?;
        let n = x0.len();
        let opts = Opts::from_value(argv.get(2), n)?;
        let lb = vec![f64::NEG_INFINITY; n];
        let ub = vec![f64::INFINITY; n];
        let res = self.lm_solve(&target, x0, lb, ub, LsqMode::Plain, &opts)?;
        if opts.display {
            self.print_buf.push_str(&format!("{}\n", res.message));
        }
        let mut out = vec![
            mat_or_num(Matrix::col_vector(res.x)),
            mat_or_num(Matrix::col_vector(res.residual)),
            Value::Num(res.exitflag),
            output_struct(res.iters, res.fevals, "trust-region-dogleg", &res.message, Some(res.firstorderopt))?,
            Value::Mat(res.jacobian),
        ];
        out.truncate(nout.max(1));
        Ok(out)
    }

    fn parse_lsq_bounds(argv: &[Value], options_at: usize) -> Result<(Vec<f64>, Vec<f64>, Opts), LangError> {
        let x0 = argv.get(1).ok_or_else(|| LangError::new("x0 is required", 0))?;
        let n = x0.to_matrix()?.numel();
        let mut lb = vec![f64::NEG_INFINITY; n];
        let mut ub = vec![f64::INFINITY; n];
        let mut opt_val: Option<&Value> = None;
        if let Some(v) = argv.get(options_at) {
            if matches!(v, Value::Struct(_)) {
                opt_val = Some(v);
            } else {
                lb = bound_vec(Some(v), n, f64::NEG_INFINITY)?;
                ub = bound_vec(argv.get(options_at.saturating_add(1)), n, f64::INFINITY)?;
                opt_val = argv.get(options_at.saturating_add(2));
            }
        }
        let opts = Opts::from_value(opt_val, n)?;
        Ok((lb, ub, opts))
    }

    fn lsqnonlin_call(&mut self, argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
        let target = target_arg(argv.first(), "lsqnonlin")?;
        let x0 = vector_arg(argv.get(1), "x0")?;
        let (lb, ub, opts) = Self::parse_lsq_bounds(argv, 2)?;
        let res = self.lm_solve(&target, x0, lb, ub, LsqMode::Plain, &opts)?;
        if opts.display {
            self.print_buf.push_str(&format!("{}\n", res.message));
        }
        self.lsq_outputs(res, nout)
    }

    fn lsqcurvefit_call(&mut self, argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
        let target = target_arg(argv.first(), "lsqcurvefit")?;
        let x0 = vector_arg(argv.get(1), "x0")?;
        let xdata = argv.get(2).cloned().ok_or_else(|| LangError::new("lsqcurvefit needs xdata", 0))?;
        let ydata = argv.get(3).ok_or_else(|| LangError::new("lsqcurvefit needs ydata", 0))?.to_matrix()?;
        let n = x0.len();
        let mut lb = vec![f64::NEG_INFINITY; n];
        let mut ub = vec![f64::INFINITY; n];
        let mut opt_val: Option<&Value> = None;
        if let Some(v) = argv.get(4) {
            if matches!(v, Value::Struct(_)) {
                opt_val = Some(v);
            } else {
                lb = bound_vec(Some(v), n, f64::NEG_INFINITY)?;
                ub = bound_vec(argv.get(5), n, f64::INFINITY)?;
                opt_val = argv.get(6);
            }
        }
        let opts = Opts::from_value(opt_val, n)?;
        let mode = LsqMode::Curve { xdata, ydata: ydata.data().to_vec() };
        let res = self.lm_solve(&target, x0, lb, ub, mode, &opts)?;
        if opts.display {
            self.print_buf.push_str(&format!("{}\n", res.message));
        }
        self.lsq_outputs(res, nout)
    }

    fn lsq_outputs(&self, res: LmResult, nout: usize) -> Result<Vec<Value>, LangError> {
        let n = res.x.len();
        let resnorm = dot(&res.residual, &res.residual);
        let mut out = vec![
            mat_or_num(Matrix::col_vector(res.x)),
            Value::Num(resnorm),
            mat_or_num(Matrix::col_vector(res.residual)),
            Value::Num(res.exitflag),
            output_struct(res.iters, res.fevals, "levenberg-marquardt", &res.message, Some(res.firstorderopt))?,
            Value::Mat(Matrix::zeros(n, 1)),
            Value::Mat(res.jacobian),
        ];
        out.truncate(nout.max(1));
        Ok(out)
    }

    /// `linprog` — linear program with the MATLAB positional or `problem`
    /// struct interface. Bounds are folded into the inequality set, then a
    /// primal-dual interior-point method solves the convex QP form (H = 0).
    fn linprog_call(&mut self, argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
        let spec = if let Some(Value::Struct(p)) = argv.first() {
            linprog_problem(p)?
        } else {
            let f = vector_arg(argv.first(), "f")?;
            let n = f.len();
            let a = matrix_arg(argv.get(1), n, "A")?;
            let b = vec_or_empty(argv.get(2))?;
            check_len(&b, a.rows(), "b")?;
            let aeq = matrix_arg(argv.get(3), n, "Aeq")?;
            let beq = vec_or_empty(argv.get(4))?;
            check_len(&beq, aeq.rows(), "beq")?;
            let lb = bound_vec(argv.get(5), n, f64::NEG_INFINITY)?;
            let ub = bound_vec(argv.get(6), n, f64::INFINITY)?;
            let (x0_val, opt_val) = parse_x0_options(argv, 7, 8);
            QpSpec { h: Matrix::zeros(n, n), f, a, b, aeq, beq, lb, ub, x0: x0_vec(x0_val, n)?, opts: Opts::from_value(opt_val, n)? }
        };
        let res = solve_qp(spec)?;
        self.qp_outputs(res, nout)
    }

    /// `quadprog` — convex quadratic program, same interfaces as `linprog`
    /// with `H` as the first argument (or a `H` field in a problem struct).
    fn quadprog_call(&mut self, argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
        let spec = if let Some(Value::Struct(p)) = argv.first() {
            quadprog_problem(p)?
        } else {
            let h = square_matrix(argv.first(), "H")?;
            let n = h.rows();
            let f = vector_arg(argv.get(1), "f")?;
            check_len(&f, n, "f")?;
            let a = matrix_arg(argv.get(2), n, "A")?;
            let b = vec_or_empty(argv.get(3))?;
            check_len(&b, a.rows(), "b")?;
            let aeq = matrix_arg(argv.get(4), n, "Aeq")?;
            let beq = vec_or_empty(argv.get(5))?;
            check_len(&beq, aeq.rows(), "beq")?;
            let lb = bound_vec(argv.get(6), n, f64::NEG_INFINITY)?;
            let ub = bound_vec(argv.get(7), n, f64::INFINITY)?;
            let (x0_val, opt_val) = parse_x0_options(argv, 8, 9);
            QpSpec { h, f, a, b, aeq, beq, lb, ub, x0: x0_vec(x0_val, n)?, opts: Opts::from_value(opt_val, n)? }
        };
        let res = solve_qp(spec)?;
        self.qp_outputs(res, nout)
    }

    /// `fmincon` — sequential quadratic programming. Each iteration solves
    /// a QP model of the objective and the linearised constraints, then a
    /// backtracking line search on an L1 merit function globalises it. A
    /// BFGS approximation of the Lagrangian Hessian drives the QP model.
    fn fmincon_call(&mut self, argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
        let spec = nlp_spec_from_args(argv, "fmincon")?;
        let res = self.solve_nlp(spec)?;
        let mut out = vec![
            mat_or_num(Matrix::col_vector(res.x)),
            Value::Num(res.fval),
            Value::Num(res.exitflag),
            nlp_output(res.iters, res.fevals, &res.message, res.constrviolation, res.firstorderopt)?,
            res.lambda,
            mat_or_num(Matrix::col_vector(res.grad)),
        ];
        if nout >= 7 {
            out.push(Value::Mat(res.hessian));
        }
        Ok(pad_outputs(out, nout))
    }

    /// `patternsearch` — derivative-free generalized pattern search with
    /// ±coordinate polling. Constraints are handled through the same L1
    /// merit function the SQP line search uses; the step doubles after a
    /// successful poll and halves after a failed one.
    fn patternsearch_call(&mut self, argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
        let spec = nlp_spec_from_args(argv, "patternsearch")?;
        let res = self.solve_patternsearch(spec)?;
        let out = vec![
            mat_or_num(Matrix::col_vector(res.x)),
            Value::Num(res.fval),
            Value::Num(res.exitflag),
            nlp_output(res.iters, res.fevals, &res.message, res.constrviolation, 0.0)?,
        ];
        Ok(pad_outputs(out, nout))
    }

    fn solve_patternsearch(&mut self, spec: NlpSpec) -> Result<PatternResult, LangError> {
        let n = spec.x0.len();
        if n == 0 {
            return Err(LangError::new("patternsearch: x0 must not be empty", 0));
        }
        if spec.a.cols() != n || spec.aeq.cols() != n {
            return Err(LangError::new("patternsearch: constraint matrices must match x0", 0));
        }
        if spec.b.len() != spec.a.rows() || spec.beq.len() != spec.aeq.rows() {
            return Err(LangError::new("patternsearch: constraint right-hand sides have the wrong length", 0));
        }
        let mut stats = Stats::new(spec.opts.max_fevals);
        let mut x = spec.x0.clone();
        for i in 0..n {
            let lo = spec.lb.get(i).copied().unwrap_or(f64::NEG_INFINITY);
            let hi = spec.ub.get(i).copied().unwrap_or(f64::INFINITY);
            if let Some(slot) = x.get_mut(i) {
                *slot = slot.clamp(lo, hi);
            }
        }
        let mut fx = self.eval_objective(&spec.target, &x, &mut stats)?;
        let (mut c, mut ceq) = self.eval_nonlcon(&spec.nonlcon, &x, &mut stats)?;
        let mut viol = nlp_violation(&spec, &x, &c, &ceq);
        let penalty = |f: f64| 1e6 * (1.0 + f.abs());
        let mut merit = fx + penalty(fx) * viol;
        let mut step: Vec<f64> = (0..n)
            .map(|i| {
                let lo = spec.lb.get(i).copied().unwrap_or(f64::NEG_INFINITY);
                let hi = spec.ub.get(i).copied().unwrap_or(f64::INFINITY);
                if lo.is_finite() && hi.is_finite() {
                    ((hi - lo) / 4.0).clamp(1e-6, 1.0)
                } else {
                    1.0
                }
            })
            .collect();
        let mut converged = false;
        let mut message = "Maximum number of iterations or function evaluations exceeded.".to_string();
        for iter in 0..spec.opts.max_iter {
            stats.iters = iter.saturating_add(1);
            if stats.exhausted() {
                break;
            }
            let mut best_x = x.clone();
            let mut best_f = fx;
            let mut best_c = c.clone();
            let mut best_ceq = ceq.clone();
            let mut best_merit = merit;
            let mut candidates: Vec<Vec<f64>> = Vec::new();
            for (i, si) in step.iter().enumerate().take(n) {
                for sign in [-1.0f64, 1.0] {
                    let mut cand = x.clone();
                    if let Some(slot) = cand.get_mut(i) {
                        *slot += sign * si;
                    }
                    candidates.push(cand);
                }
            }
            // Paired diagonal polls let the method slide along constraints
            // whose boundary is not axis-aligned. Capped at n = 12 so the
            // O(n^2) poll set stays bounded.
            if n <= 12 {
                let inv = std::f64::consts::FRAC_1_SQRT_2;
                for i in 0..n {
                    for j in (i + 1)..n {
                        for si in [-1.0f64, 1.0] {
                            for sj in [-1.0f64, 1.0] {
                                let mut cand = x.clone();
                                if let Some(slot) = cand.get_mut(i) {
                                    *slot += si * step.get(i).copied().unwrap_or(0.0) * inv;
                                }
                                if let Some(slot) = cand.get_mut(j) {
                                    *slot += sj * step.get(j).copied().unwrap_or(0.0) * inv;
                                }
                                candidates.push(cand);
                            }
                        }
                    }
                }
            }
            for mut cand in candidates {
                for i in 0..n {
                    let lo = spec.lb.get(i).copied().unwrap_or(f64::NEG_INFINITY);
                    let hi = spec.ub.get(i).copied().unwrap_or(f64::INFINITY);
                    if let Some(slot) = cand.get_mut(i) {
                        *slot = slot.clamp(lo, hi);
                    }
                }
                let f = self.eval_objective(&spec.target, &cand, &mut stats)?;
                let (cc, ce) = self.eval_nonlcon(&spec.nonlcon, &cand, &mut stats)?;
                let v = nlp_violation(&spec, &cand, &cc, &ce);
                let m = f + penalty(f) * v;
                if m < best_merit {
                    best_x = cand;
                    best_f = f;
                    best_c = cc;
                    best_ceq = ce;
                    best_merit = m;
                }
                if stats.exhausted() {
                    break;
                }
            }
            let improved = best_merit < merit;
            if improved {
                x = best_x;
                fx = best_f;
                c = best_c;
                ceq = best_ceq;
                viol = nlp_violation(&spec, &x, &c, &ceq);
                merit = best_merit;
                for s in &mut step {
                    *s = (*s * 2.0).min(1e6);
                }
            } else {
                for s in &mut step {
                    *s *= 0.5;
                }
            }
            let smax = step.iter().copied().fold(0.0f64, f64::max);
            if smax <= spec.opts.tol_x * (1.0 + norm2(&x)) && viol <= spec.opts.tol_constr {
                converged = true;
                message = "Optimization terminated: the poll step and constraint violation are below the tolerances.".to_string();
                break;
            }
        }
        let exitflag = if converged { 1.0 } else { 0.0 };
        Ok(PatternResult { x, fval: fx, exitflag, iters: stats.iters, fevals: stats.fevals, constrviolation: viol, message })
    }

    fn eval_nonlcon(&mut self, target: &Option<Value>, x: &[f64], stats: &mut Stats) -> Result<(Vec<f64>, Vec<f64>), LangError> {
        let Some(t) = target else {
            return Ok((Vec::new(), Vec::new()));
        };
        stats.fevals = stats.fevals.saturating_add(1);
        let xv = Value::Mat(Matrix::col_vector(x.to_vec()));
        let outs = self.call_feval_target(t, vec![xv], 2)?;
        let empty = Value::Mat(Matrix::zeros(0, 0));
        let (cval, ceqval) = match outs.as_slice() {
            [c] => match c {
                Value::Cell(ca) if ca.numel() == 2 => {
                    (ca.items.first().cloned().unwrap_or_else(|| empty.clone()), ca.items.get(1).cloned().unwrap_or_else(|| empty.clone()))
                }
                _ => (c.clone(), empty),
            },
            [c, ceq, ..] => (c.clone(), ceq.clone()),
            [] => (empty.clone(), empty),
        };
        let c = cval.to_matrix()?.data().to_vec();
        let ceq = ceqval.to_matrix()?.data().to_vec();
        if c.iter().chain(ceq.iter()).any(|v| !v.is_finite()) {
            return Err(LangError::new("nonlinear constraint function returned NaN or Inf", 0));
        }
        Ok((c, ceq))
    }

    fn nonlcon_jacobian(
        &mut self,
        target: &Option<Value>,
        x: &[f64],
        c0: &[f64],
        ceq0: &[f64],
        opts: &Opts,
        stats: &mut Stats,
    ) -> Result<(Matrix, Matrix), LangError> {
        let n = x.len();
        let mc = c0.len();
        let pc = ceq0.len();
        let mut jc = Matrix::zeros(mc, n);
        let mut jceq = Matrix::zeros(pc, n);
        if target.is_none() || (mc == 0 && pc == 0) {
            return Ok((jc, jceq));
        }
        for col in 0..n {
            let xj = x.get(col).copied().unwrap_or(0.0);
            let h = fd_step(xj, opts);
            let mut xp = x.to_vec();
            let mut xm = x.to_vec();
            if let Some(slot) = xp.get_mut(col) {
                *slot = xj + h;
            }
            if let Some(slot) = xm.get_mut(col) {
                *slot = xj - h;
            }
            let (cp, ceqp) = self.eval_nonlcon(target, &xp, stats)?;
            let (cm, ceqm) = self.eval_nonlcon(target, &xm, stats)?;
            if cp.len() != mc || cm.len() != mc || ceqp.len() != pc || ceqm.len() != pc {
                return Err(LangError::new("nonlinear constraint function returned a changing number of elements", 0));
            }
            for r in 0..mc {
                let v = (cp.get(r).copied().unwrap_or(0.0) - cm.get(r).copied().unwrap_or(0.0)) / (2.0 * h);
                let _ = jc.set(r, col, v);
            }
            for r in 0..pc {
                let v = (ceqp.get(r).copied().unwrap_or(0.0) - ceqm.get(r).copied().unwrap_or(0.0)) / (2.0 * h);
                let _ = jceq.set(r, col, v);
            }
        }
        Ok((jc, jceq))
    }

    fn solve_nlp(&mut self, spec: NlpSpec) -> Result<NlpResult, LangError> {
        let n = spec.x0.len();
        if n == 0 {
            return Err(LangError::new("fmincon: x0 must not be empty", 0));
        }
        if spec.a.cols() != n || spec.aeq.cols() != n {
            return Err(LangError::new("fmincon: constraint matrices must match x0", 0));
        }
        if spec.b.len() != spec.a.rows() || spec.beq.len() != spec.aeq.rows() {
            return Err(LangError::new("fmincon: constraint right-hand sides have the wrong length", 0));
        }
        for i in 0..n {
            if spec.lb.get(i).copied().unwrap_or(f64::NEG_INFINITY) > spec.ub.get(i).copied().unwrap_or(f64::INFINITY) {
                return Err(LangError::new("fmincon: lower bounds must not exceed upper bounds", 0));
            }
        }
        let m = spec.a.rows();
        let p = spec.aeq.rows();
        let mut stats = Stats::new(spec.opts.max_fevals);
        let mut x = spec.x0.clone();
        for i in 0..n {
            let lo = spec.lb.get(i).copied().unwrap_or(f64::NEG_INFINITY);
            let hi = spec.ub.get(i).copied().unwrap_or(f64::INFINITY);
            if let Some(slot) = x.get_mut(i) {
                if *slot < lo {
                    *slot = lo;
                }
                if *slot > hi {
                    *slot = hi;
                }
            }
        }
        let mut fx = self.eval_objective(&spec.target, &x, &mut stats)?;
        let mut g = self.gradient(&spec.target, &x, &spec.opts, &mut stats)?;
        let (mut c, mut ceq) = self.eval_nonlcon(&spec.nonlcon, &x, &mut stats)?;
        let (mut jc, mut jceq) = self.nonlcon_jacobian(&spec.nonlcon, &x, &c, &ceq, &spec.opts, &mut stats)?;
        let mc = c.len();
        let pc = ceq.len();
        let mut bmat = identity(n);
        let mut lam_ineq = vec![0.0; m.saturating_add(mc)];
        let mut lam_eq = vec![0.0; p.saturating_add(pc)];
        let mut lower = vec![0.0; n];
        let mut upper = vec![0.0; n];
        let mut converged = false;
        let mut message = "Maximum number of iterations or function evaluations exceeded.".to_string();
        let mut firstorderopt = norm_inf(&g);
        for iter in 0..spec.opts.max_iter {
            stats.iters = iter.saturating_add(1);
            if stats.exhausted() {
                break;
            }
            let mut qa = Matrix::zeros(m.saturating_add(mc), n);
            let mut qb = vec![0.0; m.saturating_add(mc)];
            for (r, slot) in qb.iter_mut().enumerate().take(m) {
                for col in 0..n {
                    let _ = qa.set(r, col, spec.a.get(r, col).unwrap_or(0.0));
                }
                *slot = spec.b.get(r).copied().unwrap_or(0.0) - row_dot(&spec.a, r, &x);
            }
            for r in 0..mc {
                for col in 0..n {
                    let _ = qa.set(m.saturating_add(r), col, jc.get(r, col).unwrap_or(0.0));
                }
                qb[m.saturating_add(r)] = -c.get(r).copied().unwrap_or(0.0);
            }
            let mut qaeq = Matrix::zeros(p.saturating_add(pc), n);
            let mut qbeq = vec![0.0; p.saturating_add(pc)];
            for (r, slot) in qbeq.iter_mut().enumerate().take(p) {
                for col in 0..n {
                    let _ = qaeq.set(r, col, spec.aeq.get(r, col).unwrap_or(0.0));
                }
                *slot = spec.beq.get(r).copied().unwrap_or(0.0) - row_dot(&spec.aeq, r, &x);
            }
            for r in 0..pc {
                for col in 0..n {
                    let _ = qaeq.set(p.saturating_add(r), col, jceq.get(r, col).unwrap_or(0.0));
                }
                qbeq[p.saturating_add(r)] = -ceq.get(r).copied().unwrap_or(0.0);
            }
            let qlb: Vec<f64> = (0..n).map(|i| spec.lb.get(i).copied().unwrap_or(f64::NEG_INFINITY) - x.get(i).copied().unwrap_or(0.0)).collect();
            let qub: Vec<f64> = (0..n).map(|i| spec.ub.get(i).copied().unwrap_or(f64::INFINITY) - x.get(i).copied().unwrap_or(0.0)).collect();
            let qopts = Opts { max_iter: 200, max_fevals: 2000, ..spec.opts.clone() };
            let qp_spec = QpSpec {
                h: matrix_from_row_major(&bmat, n),
                f: g.clone(),
                a: qa,
                b: qb,
                aeq: qaeq,
                beq: qbeq,
                lb: qlb,
                ub: qub,
                x0: Some(vec![0.0; n]),
                opts: qopts,
            };
            let qp = solve_qp(qp_spec);
            let (step, lamv) = match qp {
                Ok(q) if q.exitflag == 1.0 => (q.x, Some(q.lambda)),
                _ => {
                    let mut pv: Vec<f64> = g.iter().map(|v| -v).collect();
                    for i in 0..n {
                        let xi = x.get(i).copied().unwrap_or(0.0);
                        let lo = spec.lb.get(i).copied().unwrap_or(f64::NEG_INFINITY);
                        let hi = spec.ub.get(i).copied().unwrap_or(f64::INFINITY);
                        let mut cand = xi + pv.get(i).copied().unwrap_or(0.0);
                        if cand < lo {
                            cand = lo;
                        }
                        if cand > hi {
                            cand = hi;
                        }
                        if let Some(slot) = pv.get_mut(i) {
                            *slot = cand - xi;
                        }
                    }
                    (pv, None)
                }
            };
            if norm_inf(&step) <= spec.opts.tol_x * (1.0 + norm2(&x)) {
                converged = true;
                message = "Optimization terminated: the step is below the step tolerance.".to_string();
            }
            if let Some(Value::Struct(s)) = &lamv {
                let (li, le, lo, up) = lambda_vectors(s, m.saturating_add(mc), p.saturating_add(pc), n);
                lam_ineq = li;
                lam_eq = le;
                lower = lo;
                upper = up;
            }
            let sigma = 1.0 + lam_ineq.iter().chain(lam_eq.iter()).fold(0.0f64, |a, &v| a.max(v.abs()));
            let viol = nlp_violation(&spec, &x, &c, &ceq);
            if spec.opts.display {
                self.print_buf.push_str(&format!(
                    " Iteration {:>3}: f(x) = {:.6e}, constraint violation = {:.3e}, step = {:.3e}, penalty = {:.3e}\n",
                    stats.iters,
                    fx,
                    viol,
                    norm_inf(&step),
                    sigma
                ));
            }
            let merit = fx + sigma * viol;
            let dir = dot(&g, &step) - sigma * viol;
            let mut alpha = 1.0;
            let mut accepted = false;
            let mut best_x = x.clone();
            let mut best_f = fx;
            let mut best_c = c.clone();
            let mut best_ceq = ceq.clone();
            for _ in 0..30 {
                let xnew: Vec<f64> = (0..n).map(|i| x.get(i).copied().unwrap_or(0.0) + alpha * step.get(i).copied().unwrap_or(0.0)).collect();
                let fnew = self.eval_objective(&spec.target, &xnew, &mut stats)?;
                let (cnew, ceqnew) = self.eval_nonlcon(&spec.nonlcon, &xnew, &mut stats)?;
                let vnew = nlp_violation(&spec, &xnew, &cnew, &ceqnew);
                if fnew + sigma * vnew <= merit + 1e-4 * alpha * dir {
                    best_x = xnew;
                    best_f = fnew;
                    best_c = cnew;
                    best_ceq = ceqnew;
                    accepted = true;
                    break;
                }
                alpha *= 0.5;
                if stats.exhausted() {
                    break;
                }
            }
            if !accepted {
                if spec.opts.display {
                    self.print_buf.push_str(&format!(" Line search failed at step length {alpha:.3e}.\n"));
                }
                message = "Optimization stopped: no feasible descent step was found.".to_string();
                break;
            }
            if spec.opts.display {
                self.print_buf.push_str(&format!(" Accepted step length {alpha:.3e}, f(x) = {best_f:.6e}.\n"));
            }
            let gnew = self.gradient(&spec.target, &best_x, &spec.opts, &mut stats)?;
            let (jcnew, jceqnew) = self.nonlcon_jacobian(&spec.nonlcon, &best_x, &best_c, &best_ceq, &spec.opts, &mut stats)?;
            let lg_old = lagrangian_grad(&g, &spec.a, &spec.aeq, &jc, &jceq, &lam_ineq, &lam_eq, m, p);
            let lg_new = lagrangian_grad(&gnew, &spec.a, &spec.aeq, &jcnew, &jceqnew, &lam_ineq, &lam_eq, m, p);
            let s: Vec<f64> = (0..n).map(|i| best_x.get(i).copied().unwrap_or(0.0) - x.get(i).copied().unwrap_or(0.0)).collect();
            let y: Vec<f64> = (0..n).map(|i| lg_new.get(i).copied().unwrap_or(0.0) - lg_old.get(i).copied().unwrap_or(0.0)).collect();
            bfgs_hessian_update(&mut bmat, &s, &y, n);
            let old_f = fx;
            x = best_x;
            fx = best_f;
            g = gnew;
            c = best_c;
            ceq = best_ceq;
            jc = jcnew;
            jceq = jceqnew;
            firstorderopt = norm_inf(&lg_new);
            let viol_new = nlp_violation(&spec, &x, &c, &ceq);
            if viol_new <= spec.opts.tol_constr
                && (norm2(&s) <= spec.opts.tol_x * (1.0 + norm2(&x))
                    || (fx - old_f).abs() <= spec.opts.tol_fun
                    || firstorderopt <= spec.opts.tol_grad)
            {
                converged = true;
                message = "Optimization terminated: constraints satisfied and the step/gradient tolerances are met.".to_string();
                break;
            }
        }
        let cv = nlp_violation(&spec, &x, &c, &ceq);
        let exitflag = if converged {
            1.0
        } else if cv > spec.opts.tol_constr * 10.0 {
            -2.0
        } else {
            0.0
        };
        let lambda = nlp_lambda(m, mc, p, pc, n, &lam_ineq, &lam_eq, &lower, &upper)?;
        let fval = fx;
        let hessian = matrix_from_row_major(&bmat, n);
        Ok(NlpResult {
            x,
            fval,
            exitflag,
            lambda,
            grad: g,
            hessian,
            iters: stats.iters,
            fevals: stats.fevals,
            constrviolation: cv,
            firstorderopt,
            message,
        })
    }

    fn qp_outputs(&self, res: QpResult, nout: usize) -> Result<Vec<Value>, LangError> {
        let out = vec![
            mat_or_num(Matrix::col_vector(res.x)),
            Value::Num(res.fval),
            Value::Num(res.exitflag),
            qp_output(res.iters, &res.message, res.constrviolation)?,
            res.lambda,
        ];
        Ok(pad_outputs(out, nout))
    }
}

struct QpSpec {
    h: Matrix,
    f: Vec<f64>,
    a: Matrix,
    b: Vec<f64>,
    aeq: Matrix,
    beq: Vec<f64>,
    lb: Vec<f64>,
    ub: Vec<f64>,
    x0: Option<Vec<f64>>,
    opts: Opts,
}

struct QpResult {
    x: Vec<f64>,
    fval: f64,
    lambda: Value,
    iters: usize,
    exitflag: f64,
    message: String,
    constrviolation: f64,
}

struct NlpSpec {
    target: Value,
    nonlcon: Option<Value>,
    x0: Vec<f64>,
    a: Matrix,
    b: Vec<f64>,
    aeq: Matrix,
    beq: Vec<f64>,
    lb: Vec<f64>,
    ub: Vec<f64>,
    opts: Opts,
}

struct NlpResult {
    x: Vec<f64>,
    fval: f64,
    exitflag: f64,
    lambda: Value,
    grad: Vec<f64>,
    hessian: Matrix,
    iters: usize,
    fevals: usize,
    constrviolation: f64,
    firstorderopt: f64,
    message: String,
}

struct PatternResult {
    x: Vec<f64>,
    fval: f64,
    exitflag: f64,
    iters: usize,
    fevals: usize,
    constrviolation: f64,
    message: String,
}

fn matrix_arg(v: Option<&Value>, n: usize, what: &str) -> Result<Matrix, LangError> {
    let Some(v) = v else {
        return Ok(Matrix::zeros(0, n));
    };
    let m = v.to_matrix()?;
    if m.numel() == 0 {
        return Ok(Matrix::zeros(0, n));
    }
    if m.cols() != n {
        return Err(LangError::new(format!("{what} must have {n} columns"), 0));
    }
    Ok(m)
}

fn square_matrix(v: Option<&Value>, what: &str) -> Result<Matrix, LangError> {
    let v = v.ok_or_else(|| LangError::new(format!("{what} is required"), 0))?;
    let m = v.to_matrix()?;
    if m.rows() != m.cols() || m.numel() == 0 {
        return Err(LangError::new(format!("{what} must be a nonempty square matrix"), 0));
    }
    Ok(m)
}

fn vec_or_empty(v: Option<&Value>) -> Result<Vec<f64>, LangError> {
    match v {
        None => Ok(Vec::new()),
        Some(x) => Ok(x.to_matrix()?.data().to_vec()),
    }
}

fn check_len(v: &[f64], rows: usize, what: &str) -> Result<(), LangError> {
    if v.len() != rows {
        return Err(LangError::new(format!("{what} must have {rows} elements"), 0));
    }
    Ok(())
}

fn x0_vec(v: Option<&Value>, n: usize) -> Result<Option<Vec<f64>>, LangError> {
    let Some(v) = v else {
        return Ok(None);
    };
    if matches!(v, Value::Struct(_)) {
        return Ok(None);
    }
    let m = v.to_matrix()?;
    if m.numel() == 0 {
        return Ok(None);
    }
    if m.numel() != n {
        return Err(LangError::new("x0 must have the same length as f", 0));
    }
    Ok(Some(m.data().to_vec()))
}

/// `(x0, options)` from a positional call. MATLAB accepts options in the
/// `x0` slot when no start point is supplied, so a struct there wins.
fn parse_x0_options(argv: &[Value], x0_idx: usize, options_idx: usize) -> (Option<&Value>, Option<&Value>) {
    let opt = if matches!(argv.get(options_idx), Some(Value::Struct(_))) {
        argv.get(options_idx)
    } else if matches!(argv.get(x0_idx), Some(Value::Struct(_))) {
        argv.get(x0_idx)
    } else {
        None
    };
    let x0 = if matches!(argv.get(x0_idx), Some(Value::Struct(_))) { None } else { argv.get(x0_idx) };
    (x0, opt)
}

fn linprog_problem(p: &StructArray) -> Result<QpSpec, LangError> {
    let f = vector_arg(field_matches(p, &["f"]).as_ref(), "f")?;
    let n = f.len();
    let a_val = field_matches(p, &["aineq", "a"]);
    let a = matrix_arg(a_val.as_ref(), n, "A")?;
    let b = vec_or_empty(field_matches(p, &["bineq", "b"]).as_ref())?;
    check_len(&b, a.rows(), "b")?;
    let aeq_val = field_matches(p, &["aeq"]);
    let aeq = matrix_arg(aeq_val.as_ref(), n, "Aeq")?;
    let beq = vec_or_empty(field_matches(p, &["beq"]).as_ref())?;
    check_len(&beq, aeq.rows(), "beq")?;
    let lb = bound_vec(field_matches(p, &["lb"]).as_ref(), n, f64::NEG_INFINITY)?;
    let ub = bound_vec(field_matches(p, &["ub"]).as_ref(), n, f64::INFINITY)?;
    let x0 = x0_vec(field_matches(p, &["x0"]).as_ref(), n)?;
    let opts = Opts::from_value(field_matches(p, &["options", "optimoptions"]).as_ref(), n)?;
    Ok(QpSpec { h: Matrix::zeros(n, n), f, a, b, aeq, beq, lb, ub, x0, opts })
}

fn quadprog_problem(p: &StructArray) -> Result<QpSpec, LangError> {
    let h = square_matrix(field_matches(p, &["h"]).as_ref(), "H")?;
    let n = h.rows();
    let f = vector_arg(field_matches(p, &["f"]).as_ref(), "f")?;
    check_len(&f, n, "f")?;
    let a_val = field_matches(p, &["aineq", "a"]);
    let a = matrix_arg(a_val.as_ref(), n, "A")?;
    let b = vec_or_empty(field_matches(p, &["bineq", "b"]).as_ref())?;
    check_len(&b, a.rows(), "b")?;
    let aeq_val = field_matches(p, &["aeq"]);
    let aeq = matrix_arg(aeq_val.as_ref(), n, "Aeq")?;
    let beq = vec_or_empty(field_matches(p, &["beq"]).as_ref())?;
    check_len(&beq, aeq.rows(), "beq")?;
    let lb = bound_vec(field_matches(p, &["lb"]).as_ref(), n, f64::NEG_INFINITY)?;
    let ub = bound_vec(field_matches(p, &["ub"]).as_ref(), n, f64::INFINITY)?;
    let x0 = x0_vec(field_matches(p, &["x0"]).as_ref(), n)?;
    let opts = Opts::from_value(field_matches(p, &["options", "optimoptions"]).as_ref(), n)?;
    Ok(QpSpec { h, f, a, b, aeq, beq, lb, ub, x0, opts })
}

fn optional_target(v: Option<&Value>) -> Option<Value> {
    match v {
        Some(t @ (Value::Func(_) | Value::Str(_))) => Some(t.clone()),
        _ => None,
    }
}

fn nlp_spec_from_args(argv: &[Value], name: &str) -> Result<NlpSpec, LangError> {
    if let Some(Value::Struct(p)) = argv.first() {
        return nlp_problem(p, name);
    }
    let target = target_arg(argv.first(), name)?;
    let x0 = vector_arg(argv.get(1), "x0")?;
    let n = x0.len();
    let a = matrix_arg(argv.get(2), n, "A")?;
    let b = vec_or_empty(argv.get(3))?;
    check_len(&b, a.rows(), "b")?;
    let aeq = matrix_arg(argv.get(4), n, "Aeq")?;
    let beq = vec_or_empty(argv.get(5))?;
    check_len(&beq, aeq.rows(), "beq")?;
    let lb = bound_vec(argv.get(6), n, f64::NEG_INFINITY)?;
    let ub = bound_vec(argv.get(7), n, f64::INFINITY)?;
    let (nonlcon, opt_val) =
        if matches!(argv.get(8), Some(Value::Struct(_))) { (None, argv.get(8)) } else { (optional_target(argv.get(8)), argv.get(9)) };
    Ok(NlpSpec { target, nonlcon, x0, a, b, aeq, beq, lb, ub, opts: Opts::from_value(opt_val, n)? })
}

fn nlp_problem(p: &StructArray, name: &str) -> Result<NlpSpec, LangError> {
    let target = target_arg(field_matches(p, &["objective", "fun"]).as_ref(), name)?;
    let x0 = vector_arg(field_matches(p, &["x0"]).as_ref(), "x0")?;
    let n = x0.len();
    let a_val = field_matches(p, &["aineq", "a"]);
    let a = matrix_arg(a_val.as_ref(), n, "A")?;
    let b = vec_or_empty(field_matches(p, &["bineq", "b"]).as_ref())?;
    check_len(&b, a.rows(), "b")?;
    let aeq_val = field_matches(p, &["aeq"]);
    let aeq = matrix_arg(aeq_val.as_ref(), n, "Aeq")?;
    let beq = vec_or_empty(field_matches(p, &["beq"]).as_ref())?;
    check_len(&beq, aeq.rows(), "beq")?;
    let lb = bound_vec(field_matches(p, &["lb"]).as_ref(), n, f64::NEG_INFINITY)?;
    let ub = bound_vec(field_matches(p, &["ub"]).as_ref(), n, f64::INFINITY)?;
    let nonlcon = optional_target(field_matches(p, &["nonlcon"]).as_ref());
    let opts = Opts::from_value(field_matches(p, &["options", "optimoptions"]).as_ref(), n)?;
    Ok(NlpSpec { target, nonlcon, x0, a, b, aeq, beq, lb, ub, opts })
}

fn nlp_violation(spec: &NlpSpec, x: &[f64], c: &[f64], ceq: &[f64]) -> f64 {
    let mut v = 0.0f64;
    for (r, br) in spec.b.iter().enumerate() {
        v = v.max(row_dot(&spec.a, r, x) - *br);
    }
    for (r, br) in spec.beq.iter().enumerate() {
        v = v.max((row_dot(&spec.aeq, r, x) - *br).abs());
    }
    for ci in c {
        v = v.max(*ci);
    }
    for ci in ceq {
        v = v.max(ci.abs());
    }
    for (i, xi) in x.iter().enumerate() {
        let lo = spec.lb.get(i).copied().unwrap_or(f64::NEG_INFINITY);
        let hi = spec.ub.get(i).copied().unwrap_or(f64::INFINITY);
        if *xi < lo {
            v = v.max(lo - *xi);
        }
        if *xi > hi {
            v = v.max(*xi - hi);
        }
    }
    v.max(0.0)
}

#[allow(clippy::too_many_arguments)]
fn lagrangian_grad(g: &[f64], a: &Matrix, aeq: &Matrix, jc: &Matrix, jceq: &Matrix, li: &[f64], le: &[f64], m: usize, p: usize) -> Vec<f64> {
    let n = g.len();
    let mut out = g.to_vec();
    for (r, lam) in li.iter().enumerate().take(a.rows()) {
        for (c, slot) in out.iter_mut().enumerate().take(n) {
            *slot += a.get(r, c).unwrap_or(0.0) * *lam;
        }
    }
    for (r, lam) in li.iter().skip(m).enumerate().take(jc.rows()) {
        for (c, slot) in out.iter_mut().enumerate().take(n) {
            *slot += jc.get(r, c).unwrap_or(0.0) * *lam;
        }
    }
    for (r, lam) in le.iter().enumerate().take(aeq.rows()) {
        for (c, slot) in out.iter_mut().enumerate().take(n) {
            *slot += aeq.get(r, c).unwrap_or(0.0) * *lam;
        }
    }
    for (r, lam) in le.iter().skip(p).enumerate().take(jceq.rows()) {
        for (c, slot) in out.iter_mut().enumerate().take(n) {
            *slot += jceq.get(r, c).unwrap_or(0.0) * *lam;
        }
    }
    out
}

/// Direct BFGS Hessian update (the SQP subproblem needs B, not B^-1).
fn bfgs_hessian_update(b: &mut [f64], s: &[f64], y: &[f64], n: usize) {
    let sy = dot(s, y);
    if sy.is_nan() || sy <= 1e-12 {
        return;
    }
    let bs = mat_vec(b, s, n);
    let sbs = dot(s, &bs);
    if sbs.is_nan() || sbs <= 1e-12 {
        return;
    }
    for i in 0..n {
        let bsi = bs.get(i).copied().unwrap_or(0.0);
        let yi = y.get(i).copied().unwrap_or(0.0);
        for j in 0..n {
            let idx = i.saturating_mul(n).saturating_add(j);
            if let Some(slot) = b.get_mut(idx) {
                *slot += -(bsi * bs.get(j).copied().unwrap_or(0.0)) / sbs + (yi * y.get(j).copied().unwrap_or(0.0)) / sy;
            }
        }
    }
}

fn matrix_from_row_major(v: &[f64], n: usize) -> Matrix {
    let mut m = Matrix::zeros(n, n);
    for (i, row) in v.chunks(n).enumerate().take(n) {
        for (j, val) in row.iter().enumerate().take(n) {
            let _ = m.set(i, j, *val);
        }
    }
    m
}

fn lambda_vectors(s: &StructArray, mi: usize, me: usize, n: usize) -> (Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>) {
    fn get(s: &StructArray, name: &str, len: usize) -> Vec<f64> {
        let mut out = vec![0.0; len];
        if let Some(v) = s.field_at(0, name) {
            if let Ok(m) = v.to_matrix() {
                for (slot, val) in out.iter_mut().zip(m.data().iter()) {
                    *slot = *val;
                }
            }
        }
        out
    }
    (get(s, "ineqlin", mi), get(s, "eqlin", me), get(s, "lower", n), get(s, "upper", n))
}

#[allow(clippy::too_many_arguments)]
fn nlp_lambda(m: usize, mc: usize, p: usize, pc: usize, n: usize, li: &[f64], le: &[f64], lower: &[f64], upper: &[f64]) -> Result<Value, LangError> {
    let ineqlin = Matrix::col_vector(li.iter().take(m).copied().collect());
    let ineqnonlin = Matrix::col_vector(li.iter().skip(m).take(mc).copied().collect());
    let eqlin = Matrix::col_vector(le.iter().take(p).copied().collect());
    let eqnonlin = Matrix::col_vector(le.iter().skip(p).take(pc).copied().collect());
    let mut lo = vec![0.0; n];
    let mut up = vec![0.0; n];
    for (slot, val) in lo.iter_mut().zip(lower.iter()) {
        *slot = *val;
    }
    for (slot, val) in up.iter_mut().zip(upper.iter()) {
        *slot = *val;
    }
    let mut s = StructArray::new();
    s.set_field(0, "ineqlin", Value::Mat(ineqlin))?;
    s.set_field(0, "eqlin", Value::Mat(eqlin))?;
    s.set_field(0, "lower", Value::Mat(Matrix::col_vector(lo)))?;
    s.set_field(0, "upper", Value::Mat(Matrix::col_vector(up)))?;
    s.set_field(0, "ineqnonlin", Value::Mat(ineqnonlin))?;
    s.set_field(0, "eqnonlin", Value::Mat(eqnonlin))?;
    Ok(Value::Struct(s))
}

fn nlp_output(iters: usize, fevals: usize, message: &str, constrviolation: f64, firstorderopt: f64) -> Result<Value, LangError> {
    let mut s = StructArray::new();
    s.set_field(0, "iterations", Value::Num(iters as f64))?;
    s.set_field(0, "funcCount", Value::Num(fevals as f64))?;
    s.set_field(0, "constrviolation", Value::Num(constrviolation))?;
    s.set_field(0, "algorithm", Value::Str("sqp".to_string()))?;
    s.set_field(0, "message", Value::Str(message.to_string()))?;
    s.set_field(0, "firstorderopt", Value::Num(firstorderopt))?;
    Ok(Value::Struct(s))
}

fn row_dot(m: &Matrix, row: usize, x: &[f64]) -> f64 {
    (0..x.len()).map(|c| m.get(row, c).unwrap_or(0.0) * x.get(c).copied().unwrap_or(0.0)).sum()
}

fn quad_form(h: &Matrix, x: &[f64]) -> f64 {
    let n = x.len();
    let mut s = 0.0;
    for i in 0..n {
        for j in 0..n {
            s += x.get(i).copied().unwrap_or(0.0) * h.get(i, j).unwrap_or(0.0) * x.get(j).copied().unwrap_or(0.0);
        }
    }
    s
}

fn dense_solve(a: &Matrix, b: &[f64]) -> Result<Vec<f64>, LangError> {
    if a.rows() != b.len() {
        return Err(LangError::new("linear system dimension mismatch", 0));
    }
    let rhs = Matrix::col_vector(b.to_vec());
    match a.solve(&rhs) {
        Ok(x) => Ok(x.data().to_vec()),
        Err(_) => {
            let pinv = matcore::pinv(a).map_err(lang)?;
            let x = pinv.matmul(&rhs).map_err(lang)?;
            Ok(x.data().to_vec())
        }
    }
}

fn qp_step(s: &[f64], lam: &[f64], ds: &[f64], dlam: &[f64]) -> f64 {
    let mut alpha: f64 = 1.0;
    for i in 0..s.len() {
        if let Some(&d) = ds.get(i) {
            if d < 0.0 {
                alpha = alpha.min(-s.get(i).copied().unwrap_or(0.0) / d);
            }
        }
        if let Some(&d) = dlam.get(i) {
            if d < 0.0 {
                alpha = alpha.min(-lam.get(i).copied().unwrap_or(0.0) / d);
            }
        }
    }
    (0.995 * alpha).clamp(0.0, 1.0)
}

fn qp_kkt_matrix(h: &Matrix, g: &Matrix, s: &[f64], lam: &[f64], n: usize, rows: usize) -> Matrix {
    let mut k = h.clone();
    for r in 0..rows {
        let di = lam.get(r).copied().unwrap_or(0.0) / s.get(r).copied().unwrap_or(1.0).max(1e-12);
        for i in 0..n {
            let gi = g.get(r, i).unwrap_or(0.0);
            for j in 0..n {
                let gj = g.get(r, j).unwrap_or(0.0);
                let cur = k.get(i, j).unwrap_or(0.0);
                let _ = k.set(i, j, cur + di * gi * gj);
            }
        }
    }
    for i in 0..n {
        let _ = k.set(i, i, k.get(i, i).unwrap_or(0.0) + 1e-10);
    }
    k
}

#[allow(clippy::too_many_arguments)]
fn qp_direction(
    k: &Matrix,
    g: &Matrix,
    aeq: &Matrix,
    s: &[f64],
    lam: &[f64],
    rd: &[f64],
    rp: &[f64],
    req: &[f64],
    rcent: &[f64],
) -> Result<QpDirection, LangError> {
    let n = rd.len();
    let rows = rp.len();
    let p = req.len();
    let mut rhs = vec![0.0; n];
    for (i, slot) in rhs.iter_mut().enumerate().take(n) {
        *slot = -rd.get(i).copied().unwrap_or(0.0);
    }
    for (r, rp_r) in rp.iter().enumerate().take(rows) {
        let coeff =
            (-rcent.get(r).copied().unwrap_or(0.0) + lam.get(r).copied().unwrap_or(0.0) * *rp_r) / s.get(r).copied().unwrap_or(1.0).max(1e-12);
        for (i, slot) in rhs.iter_mut().enumerate().take(n) {
            *slot -= g.get(r, i).unwrap_or(0.0) * coeff;
        }
    }
    let dx0 = dense_solve(k, &rhs)?;
    let mut dx1: Vec<Vec<f64>> = Vec::with_capacity(p);
    for col in 0..p {
        let mut colrhs = vec![0.0; n];
        for (i, slot) in colrhs.iter_mut().enumerate().take(n) {
            *slot = aeq.get(col, i).unwrap_or(0.0);
        }
        dx1.push(dense_solve(k, &colrhs)?);
    }
    let (dx, dy) = if p == 0 {
        (dx0, Vec::new())
    } else {
        let mut schur = Matrix::zeros(p, p);
        for i in 0..p {
            for j in 0..p {
                let mut v = 0.0;
                for c in 0..n {
                    v += aeq.get(i, c).unwrap_or(0.0) * dx1.get(j).and_then(|d| d.get(c)).copied().unwrap_or(0.0);
                }
                let _ = schur.set(i, j, v);
            }
        }
        let mut rhs_dy = vec![0.0; p];
        for (i, slot) in rhs_dy.iter_mut().enumerate().take(p) {
            let mut v = req.get(i).copied().unwrap_or(0.0);
            for c in 0..n {
                v += aeq.get(i, c).unwrap_or(0.0) * dx0.get(c).copied().unwrap_or(0.0);
            }
            *slot = v;
        }
        let dy = dense_solve(&schur, &rhs_dy)?;
        let mut dx = dx0;
        for (c, slot) in dx.iter_mut().enumerate().take(n) {
            for j in 0..p {
                *slot -= dx1.get(j).and_then(|d| d.get(c)).copied().unwrap_or(0.0) * dy.get(j).copied().unwrap_or(0.0);
            }
        }
        (dx, dy)
    };
    let mut ds = vec![0.0; rows];
    for (r, slot) in ds.iter_mut().enumerate().take(rows) {
        let mut v = 0.0;
        for i in 0..n {
            v += g.get(r, i).unwrap_or(0.0) * dx.get(i).copied().unwrap_or(0.0);
        }
        *slot = -rp.get(r).copied().unwrap_or(0.0) - v;
    }
    let mut dlam = vec![0.0; rows];
    for (r, slot) in dlam.iter_mut().enumerate().take(rows) {
        *slot = (-rcent.get(r).copied().unwrap_or(0.0) - lam.get(r).copied().unwrap_or(0.0) * ds.get(r).copied().unwrap_or(0.0))
            / s.get(r).copied().unwrap_or(1.0).max(1e-12);
    }
    Ok(QpDirection { dx, ds, dlam, dy })
}

fn qp_lambda(
    m: usize,
    n: usize,
    p: usize,
    lam: &[f64],
    y: &[f64],
    upper_row: &[Option<usize>],
    lower_row: &[Option<usize>],
) -> Result<Value, LangError> {
    let mut ineq = Matrix::zeros(m, 1);
    for r in 0..m {
        let _ = ineq.set(r, 0, lam.get(r).copied().unwrap_or(0.0));
    }
    let mut lower = Matrix::zeros(n, 1);
    let mut upper = Matrix::zeros(n, 1);
    for i in 0..n {
        if let Some(r) = lower_row.get(i).copied().flatten() {
            let _ = lower.set(i, 0, lam.get(r).copied().unwrap_or(0.0));
        }
        if let Some(r) = upper_row.get(i).copied().flatten() {
            // MATLAB reports upper-bound multipliers with the opposite sign
            // from the x <= ub inequality convention.
            let _ = upper.set(i, 0, -lam.get(r).copied().unwrap_or(0.0));
        }
    }
    let mut eq = Matrix::zeros(p, 1);
    for i in 0..p {
        let _ = eq.set(i, 0, y.get(i).copied().unwrap_or(0.0));
    }
    let mut s = StructArray::new();
    s.set_field(0, "ineqlin", Value::Mat(ineq))?;
    s.set_field(0, "eqlin", Value::Mat(eq))?;
    s.set_field(0, "lower", Value::Mat(lower))?;
    s.set_field(0, "upper", Value::Mat(upper))?;
    Ok(Value::Struct(s))
}

fn qp_output(iters: usize, message: &str, constrviolation: f64) -> Result<Value, LangError> {
    let mut s = StructArray::new();
    s.set_field(0, "iterations", Value::Num(iters as f64))?;
    s.set_field(0, "constrviolation", Value::Num(constrviolation))?;
    s.set_field(0, "algorithm", Value::Str("interior-point".to_string()))?;
    s.set_field(0, "message", Value::Str(message.to_string()))?;
    Ok(Value::Struct(s))
}

fn qp_constrviolation(spec: &QpSpec, x: &[f64]) -> f64 {
    let mut v = 0.0f64;
    for r in 0..spec.a.rows() {
        v = v.max(row_dot(&spec.a, r, x) - spec.b.get(r).copied().unwrap_or(0.0));
    }
    for r in 0..spec.aeq.rows() {
        v = v.max((row_dot(&spec.aeq, r, x) - spec.beq.get(r).copied().unwrap_or(0.0)).abs());
    }
    for i in 0..x.len() {
        let xi = x.get(i).copied().unwrap_or(0.0);
        let lo = spec.lb.get(i).copied().unwrap_or(f64::NEG_INFINITY);
        let hi = spec.ub.get(i).copied().unwrap_or(f64::INFINITY);
        if xi < lo {
            v = v.max(lo - xi);
        }
        if xi > hi {
            v = v.max(xi - hi);
        }
    }
    v.max(0.0)
}

/// Active-set polish after the interior-point solve. Near-active rows are
/// treated as equalities and the resulting KKT system is solved directly,
/// which removes the barrier method's sqrt(mu) error on degenerate bounds
/// (the common "unconstrained optimum lies on a bound" case). The candidate
/// is accepted only when it stays feasible, keeps nonnegative multipliers
/// for the `<=` rows, and does not increase the objective.
fn polish_qp(spec: &QpSpec, x: &[f64], g: &Matrix, gb: &[f64], tol: f64) -> Option<Vec<f64>> {
    let n = x.len();
    let p = spec.aeq.rows();
    let mut active: Vec<usize> = Vec::new();
    for r in 0..g.rows() {
        let slack = gb.get(r).copied().unwrap_or(0.0) - row_dot(g, r, x);
        if slack.abs() <= 1e-4 {
            active.push(r);
        }
    }
    if active.is_empty() {
        return None;
    }
    let q = p.saturating_add(active.len());
    let size = n.saturating_add(q);
    let mut kkt = Matrix::zeros(size, size);
    for i in 0..n {
        for j in 0..n {
            let _ = kkt.set(i, j, spec.h.get(i, j).unwrap_or(0.0));
        }
    }
    let mut rhs = vec![0.0; size];
    for (i, slot) in rhs.iter_mut().enumerate().take(n) {
        *slot = -spec.f.get(i).copied().unwrap_or(0.0);
    }
    for i in 0..p {
        for j in 0..n {
            let v = spec.aeq.get(i, j).unwrap_or(0.0);
            let _ = kkt.set(n.saturating_add(i), j, v);
            let _ = kkt.set(j, n.saturating_add(i), v);
        }
        rhs[n.saturating_add(i)] = spec.beq.get(i).copied().unwrap_or(0.0);
    }
    for (k, &r) in active.iter().enumerate() {
        let idx = n.saturating_add(p).saturating_add(k);
        for j in 0..n {
            let v = g.get(r, j).unwrap_or(0.0);
            let _ = kkt.set(idx, j, v);
            let _ = kkt.set(j, idx, v);
        }
        rhs[idx] = gb.get(r).copied().unwrap_or(0.0);
    }
    let sol = dense_solve(&kkt, &rhs).ok()?;
    let xp: Vec<f64> = sol.iter().take(n).copied().collect();
    if qp_constrviolation(spec, &xp) > 1e-7 {
        return None;
    }
    for k in 0..active.len() {
        let mu = sol.get(n.saturating_add(p).saturating_add(k)).copied().unwrap_or(0.0);
        if mu < -1e-7 {
            return None;
        }
    }
    let f_old = 0.5 * quad_form(&spec.h, x) + dot(&spec.f, x);
    let f_new = 0.5 * quad_form(&spec.h, &xp) + dot(&spec.f, &xp);
    if f_new > f_old + tol.max(1e-8) {
        return None;
    }
    Some(xp)
}

fn solve_qp(spec: QpSpec) -> Result<QpResult, LangError> {
    let n = spec.f.len();
    if spec.h.rows() != n || spec.h.cols() != n {
        return Err(LangError::new("H must be square with the same length as f", 0));
    }
    if spec.a.cols() != n || spec.aeq.cols() != n {
        return Err(LangError::new("constraint matrices must have the same number of columns as f", 0));
    }
    if spec.b.len() != spec.a.rows() || spec.beq.len() != spec.aeq.rows() {
        return Err(LangError::new("constraint right-hand sides have the wrong length", 0));
    }
    for i in 0..n {
        let lo = spec.lb.get(i).copied().unwrap_or(f64::NEG_INFINITY);
        let hi = spec.ub.get(i).copied().unwrap_or(f64::INFINITY);
        if lo > hi {
            return Err(LangError::new("lower bounds must not exceed upper bounds", 0));
        }
    }
    let tol = spec.opts.tol_fun.max(1e-9);
    let m = spec.a.rows();
    let p = spec.aeq.rows();

    let mut upper_row = vec![None; n];
    let mut lower_row = vec![None; n];
    let mut rows = m;
    for i in 0..n {
        if spec.ub.get(i).copied().unwrap_or(f64::INFINITY).is_finite() {
            rows += 1;
        }
        if spec.lb.get(i).copied().unwrap_or(f64::NEG_INFINITY).is_finite() {
            rows += 1;
        }
    }

    // No inequalities at all: solve the equality-constrained (or plain)
    // quadratic program directly.
    if rows == 0 {
        let (x, y) = if p == 0 {
            let f_norm = norm_inf(&spec.f);
            if spec.h.data().iter().all(|v| *v == 0.0) {
                if f_norm <= tol {
                    (vec![0.0; n], Vec::new())
                } else {
                    let fval = dot(&spec.f, &vec![0.0; n]);
                    return Ok(QpResult {
                        x: vec![0.0; n],
                        fval,
                        lambda: qp_lambda(0, n, 0, &[], &[], &upper_row, &lower_row)?,
                        iters: 0,
                        exitflag: -3.0,
                        message: "The problem is unbounded.".to_string(),
                        constrviolation: 0.0,
                    });
                }
            } else {
                let rhs: Vec<f64> = spec.f.iter().map(|v| -v).collect();
                (dense_solve(&spec.h, &rhs)?, Vec::new())
            }
        } else {
            let size = n.saturating_add(p);
            let mut kkt = Matrix::zeros(size, size);
            for i in 0..n {
                for j in 0..n {
                    let _ = kkt.set(i, j, spec.h.get(i, j).unwrap_or(0.0));
                }
            }
            for i in 0..p {
                for j in 0..n {
                    let v = spec.aeq.get(i, j).unwrap_or(0.0);
                    let _ = kkt.set(n.saturating_add(i), j, v);
                    let _ = kkt.set(j, n.saturating_add(i), v);
                }
            }
            let mut rhs = vec![0.0; size];
            for (i, slot) in rhs.iter_mut().enumerate().take(n) {
                *slot = -spec.f.get(i).copied().unwrap_or(0.0);
            }
            for i in 0..p {
                rhs[n.saturating_add(i)] = spec.beq.get(i).copied().unwrap_or(0.0);
            }
            let sol = dense_solve(&kkt, &rhs)?;
            let x: Vec<f64> = sol.iter().take(n).copied().collect();
            let y: Vec<f64> = sol.iter().skip(n).take(p).copied().collect();
            (x, y)
        };
        let fval = 0.5 * quad_form(&spec.h, &x) + dot(&spec.f, &x);
        let cv = qp_constrviolation(&spec, &x);
        let exitflag = if cv <= tol * 10.0 { 1.0 } else { -2.0 };
        let message = if exitflag > 0.0 { "Optimal solution found." } else { "No feasible point was found." };
        return Ok(QpResult {
            lambda: qp_lambda(0, n, p, &[], &y, &upper_row, &lower_row)?,
            x,
            fval,
            iters: 0,
            exitflag,
            message: message.to_string(),
            constrviolation: cv,
        });
    }

    let mut g = Matrix::zeros(rows, n);
    let mut gb = vec![0.0; rows];
    for (r, slot) in gb.iter_mut().enumerate().take(m) {
        for c in 0..n {
            let _ = g.set(r, c, spec.a.get(r, c).unwrap_or(0.0));
        }
        *slot = spec.b.get(r).copied().unwrap_or(0.0);
    }
    let mut row = m;
    for i in 0..n {
        let hi = spec.ub.get(i).copied().unwrap_or(f64::INFINITY);
        if hi.is_finite() {
            let _ = g.set(row, i, 1.0);
            gb[row] = hi;
            upper_row[i] = Some(row);
            row += 1;
        }
        let lo = spec.lb.get(i).copied().unwrap_or(f64::NEG_INFINITY);
        if lo.is_finite() {
            let _ = g.set(row, i, -1.0);
            gb[row] = -lo;
            lower_row[i] = Some(row);
            row += 1;
        }
    }

    let mut x = spec.x0.clone().unwrap_or_else(|| vec![0.0; n]);
    for i in 0..n {
        let lo = spec.lb.get(i).copied().unwrap_or(f64::NEG_INFINITY);
        let hi = spec.ub.get(i).copied().unwrap_or(f64::INFINITY);
        if let Some(slot) = x.get_mut(i) {
            if *slot < lo {
                *slot = lo;
            }
            if *slot > hi {
                *slot = hi;
            }
        }
    }
    let mut s = vec![0.0; rows];
    for (r, slot) in s.iter_mut().enumerate().take(rows) {
        let v = gb.get(r).copied().unwrap_or(0.0) - row_dot(&g, r, &x);
        *slot = if v.is_finite() && v > 0.0 { v } else { 1.0 };
    }
    let mut lam = vec![1.0; rows];
    let mut y = vec![0.0; p];
    let mut iters = 0;
    let mut converged = false;
    let mut message = "Maximum number of iterations exceeded.".to_string();
    for iter in 0..spec.opts.max_iter {
        iters = iter.saturating_add(1);
        let rd = qp_dual_residual(&spec.h, &spec.f, &g, &lam, &spec.aeq, &y, &x);
        let rp: Vec<f64> = (0..rows).map(|r| row_dot(&g, r, &x) + s.get(r).copied().unwrap_or(0.0) - gb.get(r).copied().unwrap_or(0.0)).collect();
        let req: Vec<f64> = (0..p).map(|r| row_dot(&spec.aeq, r, &x) - spec.beq.get(r).copied().unwrap_or(0.0)).collect();
        let mu = dot(&s, &lam) / rows as f64;
        if norm_inf(&rd) <= tol && norm_inf(&rp) <= tol && norm_inf(&req) <= tol && mu <= tol {
            converged = true;
            message = "Optimal solution found.".to_string();
            break;
        }
        if !mu.is_finite() || mu > 1e16 {
            break;
        }
        let k = qp_kkt_matrix(&spec.h, &g, &s, &lam, n, rows);
        let rcent_aff: Vec<f64> = s.iter().zip(lam.iter()).map(|(a, b)| a * b).collect();
        let QpDirection { ds, dlam, .. } = qp_direction(&k, &g, &spec.aeq, &s, &lam, &rd, &rp, &req, &rcent_aff)?;
        let alpha_aff = qp_step(&s, &lam, &ds, &dlam);
        let s_aff: Vec<f64> = s.iter().zip(ds.iter()).map(|(a, b)| a + alpha_aff * b).collect();
        let l_aff: Vec<f64> = lam.iter().zip(dlam.iter()).map(|(a, b)| a + alpha_aff * b).collect();
        let mu_aff = dot(&s_aff, &l_aff) / rows as f64;
        let sigma = if mu > 0.0 { (mu_aff / mu).powi(3).clamp(0.0, 1.0) } else { 0.0 };
        let rcent: Vec<f64> = (0..rows)
            .map(|i| {
                s.get(i).copied().unwrap_or(0.0) * lam.get(i).copied().unwrap_or(0.0) - sigma * mu
                    + ds.get(i).copied().unwrap_or(0.0) * dlam.get(i).copied().unwrap_or(0.0)
            })
            .collect();
        let QpDirection { dx, ds, dlam, dy } = qp_direction(&k, &g, &spec.aeq, &s, &lam, &rd, &rp, &req, &rcent)?;
        let alpha = qp_step(&s, &lam, &ds, &dlam);
        if alpha <= 0.0 {
            break;
        }
        for (i, slot) in x.iter_mut().enumerate().take(n) {
            *slot += alpha * dx.get(i).copied().unwrap_or(0.0);
        }
        for (r, slot) in s.iter_mut().enumerate().take(rows) {
            *slot = (*slot + alpha * ds.get(r).copied().unwrap_or(0.0)).max(1e-12);
            lam[r] = (lam.get(r).copied().unwrap_or(0.0) + alpha * dlam.get(r).copied().unwrap_or(0.0)).max(1e-12);
        }
        for (i, slot) in y.iter_mut().enumerate().take(p) {
            *slot += alpha * dy.get(i).copied().unwrap_or(0.0);
        }
    }
    if let Some(xp) = polish_qp(&spec, &x, &g, &gb, tol) {
        x = xp;
    }
    // Clean tiny numerical dust at zero.
    for v in &mut x {
        if v.abs() < 1e-10 {
            *v = 0.0;
        }
    }
    let rd = qp_dual_residual(&spec.h, &spec.f, &g, &lam, &spec.aeq, &y, &x);
    let cv = qp_constrviolation(&spec, &x);
    let mu = dot(&s, &lam) / rows as f64;
    if !converged && mu <= tol * 10.0 && cv <= tol * 10.0 && norm_inf(&rd) <= tol * 10.0 {
        converged = true;
        message = "Optimal solution found.".to_string();
    }
    let exitflag = if converged {
        1.0
    } else if cv > 1e-4 {
        message = "The problem appears to be infeasible.".to_string();
        -2.0
    } else {
        0.0
    };
    let fval = 0.5 * quad_form(&spec.h, &x) + dot(&spec.f, &x);
    Ok(QpResult { lambda: qp_lambda(m, n, p, &lam, &y, &upper_row, &lower_row)?, x, fval, iters, exitflag, message, constrviolation: cv })
}

fn qp_dual_residual(h: &Matrix, f: &[f64], g: &Matrix, lam: &[f64], aeq: &Matrix, y: &[f64], x: &[f64]) -> Vec<f64> {
    let n = f.len();
    let mut rd = vec![0.0; n];
    for (i, slot) in rd.iter_mut().enumerate().take(n) {
        let mut v = f.get(i).copied().unwrap_or(0.0);
        for j in 0..n {
            v += h.get(i, j).unwrap_or(0.0) * x.get(j).copied().unwrap_or(0.0);
        }
        for r in 0..lam.len() {
            v += g.get(r, i).unwrap_or(0.0) * lam.get(r).copied().unwrap_or(0.0);
        }
        for r in 0..y.len() {
            v += aeq.get(r, i).unwrap_or(0.0) * y.get(r).copied().unwrap_or(0.0);
        }
        *slot = v;
    }
    rd
}

fn pad_outputs(mut out: Vec<Value>, nout: usize) -> Vec<Value> {
    out.truncate(nout.max(1));
    while out.len() < nout {
        out.push(Value::Mat(Matrix::zeros(0, 0)));
    }
    out
}

fn sort_simplex(simplex: &mut [Vec<f64>], fvals: &mut [f64]) {
    let n = fvals.len();
    for i in 0..n {
        for j in 0..n.saturating_sub(1).saturating_sub(i) {
            let a = fvals.get(j).copied().unwrap_or(0.0);
            let b = fvals.get(j.saturating_add(1)).copied().unwrap_or(0.0);
            if a.total_cmp(&b) == std::cmp::Ordering::Greater {
                fvals.swap(j, j.saturating_add(1));
                simplex.swap(j, j.saturating_add(1));
            }
        }
    }
}

fn replace_vertex(simplex: &mut [Vec<f64>], fvals: &mut [f64], n: usize, x: Vec<f64>, f: f64) {
    if let Some(slot) = simplex.get_mut(n) {
        *slot = x;
    }
    if let Some(slot) = fvals.get_mut(n) {
        *slot = f;
    }
}

fn jtj_matrix(j: &Matrix) -> Matrix {
    let n = j.cols();
    let m = j.rows();
    let mut out = Matrix::zeros(n, n);
    for i in 0..n {
        for k in 0..n {
            let mut s = 0.0;
            for row in 0..m {
                s += j.get(row, i).unwrap_or(0.0) * j.get(row, k).unwrap_or(0.0);
            }
            let _ = out.set(i, k, s);
        }
    }
    out
}

fn jtj_grad(j: &Matrix, r: &[f64]) -> Vec<f64> {
    let n = j.cols();
    let m = j.rows();
    (0..n).map(|i| (0..m).map(|row| j.get(row, i).unwrap_or(0.0) * r.get(row).copied().unwrap_or(0.0)).sum()).collect()
}
