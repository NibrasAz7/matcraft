//! Control System Toolbox builtins.
//!
//! LTI systems are `Value::Object`s with class tags `tf`, `ss` or `zpk`:
//!
//! * `tf`  — `Num`, `Den`, `Variable`, `Ts` (SISO polynomial ratio)
//! * `ss`  — `A`, `B`, `C`, `D`, `Ts`
//! * `zpk` — `Z`, `P`, `K`, `Variable`, `Ts` (real zeros/poles)
//!
//! Continuous-time responses use zero-order-hold state-space simulation
//! built on `matcore::expm`; frequency responses evaluate the transfer
//! function at `jω`. All algorithms are independent implementations from
//! the standard state-space/transfer-function definitions.

use super::*;
use matcore::CNum;

struct ResponseSystem {
    a: Matrix,
    b: Matrix,
    c: Matrix,
    d: Matrix,
    t: Vec<f64>,
    ts: f64,
}

fn is_class(v: &Value, class: &str) -> bool {
    matches!(v, Value::Object(o) if o.class_name == class)
}

fn lti_class(v: &Value) -> Option<String> {
    match v {
        Value::Object(o) if matches!(o.class_name.as_str(), "tf" | "ss" | "zpk") => Some(o.class_name.clone()),
        _ => None,
    }
}

fn make_object(class: &str, fields: Vec<(&str, Value)>) -> Result<Value, LangError> {
    let mut s = StructArray::new();
    for (name, value) in fields {
        s.set_field(0, name, value)?;
    }
    Ok(Value::Object(MObject::owned(class.to_string(), s)))
}

fn make_tf(num: Vec<f64>, den: Vec<f64>, variable: &str, ts: f64) -> Result<Value, LangError> {
    if den.is_empty() || den.first().copied().unwrap_or(0.0) == 0.0 {
        return Err(LangError::new("tf: denominator must have a nonzero leading coefficient", 0));
    }
    let d0 = den.first().copied().unwrap_or(1.0);
    let num = trim_zeros(num.into_iter().map(|v| v / d0).collect());
    let den = trim_zeros(den.into_iter().map(|v| v / d0).collect());
    make_object(
        "tf",
        vec![
            ("Num", Value::Mat(Matrix::row_vector(num))),
            ("Den", Value::Mat(Matrix::row_vector(den))),
            ("Variable", Value::Str(variable.to_string())),
            ("Ts", Value::Num(ts)),
        ],
    )
}

fn make_ss(a: Matrix, b: Matrix, c: Matrix, d: Matrix, ts: f64) -> Result<Value, LangError> {
    if a.rows() != a.cols() {
        return Err(LangError::new("ss: A must be square", 0));
    }
    let n = a.rows();
    if b.rows() != n || c.cols() != n || d.rows() != c.rows() || d.cols() != b.cols() {
        return Err(LangError::new("ss: A/B/C/D dimensions are inconsistent", 0));
    }
    make_object("ss", vec![("A", Value::Mat(a)), ("B", Value::Mat(b)), ("C", Value::Mat(c)), ("D", Value::Mat(d)), ("Ts", Value::Num(ts))])
}

fn wrap_degrees(mut x: f64) -> f64 {
    while x > 180.0 {
        x -= 360.0;
    }
    while x < -180.0 {
        x += 360.0;
    }
    x
}

/// Frequency response of `Kp + Ki/s + Kd*s/(Tf*s + 1)` at `s = j*wc`.
fn pid_response(wc: f64, kp: f64, ki: f64, kd: f64, tf: f64) -> CNum {
    let s = CNum::new(0.0, wc);
    let one = CNum::real(1.0);
    let mut out = CNum::real(kp) + CNum::real(ki).div(s);
    if kd != 0.0 {
        out = out + CNum::real(kd) * s.div(one + CNum::real(tf) * s);
    }
    out
}

/// Pick the derivative time that gives the requested controller phase with a
/// first-order derivative filter. Bisection is monotone here: more derivative
/// action adds phase until the filter roll-off dominates.
fn solve_filtered_derivative(wc: f64, ki0: f64, tf: f64, target: f64) -> f64 {
    let phase = |td: f64| {
        let c = pid_response(wc, 1.0, ki0, td, tf);
        c.im.atan2(c.re)
    };
    if phase(0.0) >= target {
        return 0.0;
    }
    let mut hi = 1.0 / wc;
    for _ in 0..60 {
        if phase(hi) >= target {
            break;
        }
        hi *= 2.0;
        if hi > 1.0e6 / wc {
            break;
        }
    }
    let mut lo = 0.0;
    for _ in 0..100 {
        let mid = 0.5 * (lo + hi);
        if phase(mid) < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    hi
}

fn roots_value(r: &[CNum]) -> Result<Value, LangError> {
    if r.iter().all(|z| z.im.abs() <= 1e-12) {
        Ok(mat_or_num(Matrix::col_vector(r.iter().map(|z| z.re).collect())))
    } else {
        Ok(cmat_or_value(CMatrix::from_col_major(r.len(), 1, r.to_vec()).map_err(lang)?))
    }
}

fn complex_rows(v: &Value, what: &str) -> Result<Vec<CNum>, LangError> {
    let m = v.to_cmatrix().map_err(|_| LangError::new(format!("{what} must be numeric"), 0))?;
    if m.rows() > 1 && m.cols() > 1 {
        return Err(LangError::new(format!("{what} must be a vector"), 0));
    }
    Ok(m.data().to_vec())
}

fn make_zpk(z: Vec<CNum>, p: Vec<CNum>, k: f64, variable: &str, ts: f64) -> Result<Value, LangError> {
    make_object(
        "zpk",
        vec![
            ("Z", roots_value(&z)?),
            ("P", roots_value(&p)?),
            ("K", Value::Num(k)),
            ("Variable", Value::Str(variable.to_string())),
            ("Ts", Value::Num(ts)),
        ],
    )
}

fn trim_zeros(v: Vec<f64>) -> Vec<f64> {
    let mut v = v;
    while v.len() > 1 && v.first().copied().unwrap_or(0.0) == 0.0 {
        v.remove(0);
    }
    if v.is_empty() {
        v.push(0.0);
    }
    v
}

fn row(v: &[f64]) -> Matrix {
    Matrix::row_vector(v.to_vec())
}

fn value_rows(v: &Value) -> Result<Vec<f64>, LangError> {
    Ok(v.to_numeric()?.data().to_vec())
}

fn tf_parts(v: &Value) -> Result<(Vec<f64>, Vec<f64>), LangError> {
    match v {
        Value::Object(o) if o.class_name == "tf" => {
            let num = o.prop("Num").ok_or_else(|| LangError::new("tf object has no Num", 0))?.to_matrix()?.data().to_vec();
            let den = o.prop("Den").ok_or_else(|| LangError::new("tf object has no Den", 0))?.to_matrix()?.data().to_vec();
            Ok((num, den))
        }
        Value::Object(o) if o.class_name == "ss" => {
            let (a, b, c, d) = ss_data(o)?;
            ss_to_tf_parts(&a, &b, &c, &d)
        }
        Value::Object(o) if o.class_name == "zpk" => {
            let (z, p, k) = zpk_data(o)?;
            let num = real_poly("zero", &z)?.into_iter().map(|v| v * k).collect();
            let den = real_poly("pole", &p)?;
            Ok((num, den))
        }
        Value::Num(x) => Ok((vec![*x], vec![1.0])),
        Value::Mat(m) if m.numel() == 1 => Ok((vec![m.data().first().copied().unwrap_or(0.0)], vec![1.0])),
        _ => Err(LangError::new("expected an LTI system (tf, ss or zpk) or a scalar", 0)),
    }
}

fn ss_data(o: &MObject) -> Result<(Matrix, Matrix, Matrix, Matrix), LangError> {
    let a = o.prop("A").ok_or_else(|| LangError::new("ss object has no A", 0))?.to_matrix()?;
    let b = o.prop("B").ok_or_else(|| LangError::new("ss object has no B", 0))?.to_matrix()?;
    let c = o.prop("C").ok_or_else(|| LangError::new("ss object has no C", 0))?.to_matrix()?;
    let d = o.prop("D").ok_or_else(|| LangError::new("ss object has no D", 0))?.to_matrix()?;
    Ok((a, b, c, d))
}

fn zpk_data(o: &MObject) -> Result<(Vec<CNum>, Vec<CNum>, f64), LangError> {
    let z = o.prop("Z").ok_or_else(|| LangError::new("zpk object has no Z", 0))?.to_cmatrix()?.data().to_vec();
    let p = o.prop("P").ok_or_else(|| LangError::new("zpk object has no P", 0))?.to_cmatrix()?.data().to_vec();
    let k = o.prop("K").ok_or_else(|| LangError::new("zpk object has no K", 0))?.scalar()?;
    Ok((z, p, k))
}

fn ss_parts(v: &Value) -> Result<(Matrix, Matrix, Matrix, Matrix), LangError> {
    match v {
        Value::Object(o) if o.class_name == "ss" => ss_data(o),
        _ => {
            let (num, den) = tf_parts(v)?;
            tf_to_ss_parts(&num, &den)
        }
    }
}

fn poly_from_roots(roots: &[CNum]) -> Vec<CNum> {
    let mut c = vec![CNum::real(1.0)];
    for &root in roots {
        let mut next = vec![CNum::real(0.0); c.len().saturating_add(1)];
        for (i, &a) in c.iter().enumerate() {
            if let Some(slot) = next.get_mut(i) {
                *slot = *slot + a;
            }
            if let Some(slot) = next.get_mut(i.saturating_add(1)) {
                *slot = *slot - a * root;
            }
        }
        c = next;
    }
    c
}

fn real_poly(what: &str, roots: &[CNum]) -> Result<Vec<f64>, LangError> {
    let c = poly_from_roots(roots);
    if c.iter().any(|z| z.im.abs() > 1e-9) {
        return Err(LangError::new(format!("{what}s must occur in complex-conjugate pairs to produce a real transfer function"), 0));
    }
    Ok(c.iter().map(|z| z.re).collect())
}

fn conv(a: &[f64], b: &[f64]) -> Vec<f64> {
    if a.is_empty() || b.is_empty() {
        return vec![0.0];
    }
    let mut out = vec![0.0; a.len().saturating_add(b.len()).saturating_sub(1)];
    for (i, &x) in a.iter().enumerate() {
        for (j, &y) in b.iter().enumerate() {
            if let Some(slot) = out.get_mut(i.saturating_add(j)) {
                *slot += x * y;
            }
        }
    }
    out
}

fn poly_add(a: &[f64], b: &[f64]) -> Vec<f64> {
    let n = a.len().max(b.len());
    let mut out = vec![0.0; n];
    let ai = n.saturating_sub(a.len());
    let bi = n.saturating_sub(b.len());
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = a.get(i.wrapping_sub(ai)).copied().unwrap_or(0.0) + b.get(i.wrapping_sub(bi)).copied().unwrap_or(0.0);
    }
    out
}

/// Faddeev–LeVerrier characteristic polynomial, highest power first.
fn charpoly(a: &Matrix) -> Result<Vec<f64>, LangError> {
    let n = a.rows();
    if n != a.cols() {
        return Err(LangError::new("charpoly needs a square matrix", 0));
    }
    let mut c = vec![0.0; n.saturating_add(1)];
    if let Some(slot) = c.first_mut() {
        *slot = 1.0;
    }
    let mut m = Matrix::eye(n, n);
    for k in 1..=n {
        let am = a.matmul(&m).map_err(lang)?;
        let ck = -matcore::trace(&am) / k as f64;
        if let Some(slot) = c.get_mut(k) {
            *slot = ck;
        }
        m = am.elemwise(&Matrix::eye(n, n), |x, eye| x + ck * eye).map_err(lang)?;
    }
    Ok(c)
}

fn tf_to_ss_parts(num: &[f64], den: &[f64]) -> Result<(Matrix, Matrix, Matrix, Matrix), LangError> {
    if den.is_empty() || den.first().copied().unwrap_or(0.0) == 0.0 {
        return Err(LangError::new("tf2ss: denominator must have a nonzero leading coefficient", 0));
    }
    let n = den.len().saturating_sub(1);
    if num.len() > n.saturating_add(1) {
        return Err(LangError::new("tf2ss: improper transfer function (numerator degree exceeds denominator)", 0));
    }
    let d0 = den.first().copied().unwrap_or(1.0);
    let den: Vec<f64> = den.iter().map(|v| v / d0).collect();
    let mut np = vec![0.0; n.saturating_add(1)];
    let offset = n.saturating_add(1).saturating_sub(num.len());
    for (i, v) in num.iter().enumerate() {
        if let Some(slot) = np.get_mut(offset.saturating_add(i)) {
            *slot = *v / d0;
        }
    }
    if n == 0 {
        return Ok((Matrix::zeros(0, 0), Matrix::zeros(0, 1), Matrix::zeros(1, 0), Matrix::scalar(np.first().copied().unwrap_or(0.0))));
    }
    let mut a = Matrix::zeros(n, n);
    for i in 0..n.saturating_sub(1) {
        let _ = a.set(i, i.saturating_add(1), 1.0);
    }
    for j in 0..n {
        let _ = a.set(n.saturating_sub(1), j, -den.get(n.saturating_sub(j)).copied().unwrap_or(0.0));
    }
    let mut b = Matrix::zeros(n, 1);
    let _ = b.set(n.saturating_sub(1), 0, 1.0);
    let mut c = Matrix::zeros(1, n);
    for j in 0..n {
        let idx = n.saturating_sub(j);
        let v = np.get(idx).copied().unwrap_or(0.0) - np.first().copied().unwrap_or(0.0) * den.get(idx).copied().unwrap_or(0.0);
        let _ = c.set(0, j, v);
    }
    Ok((a, b, c, Matrix::scalar(np.first().copied().unwrap_or(0.0))))
}

fn ss_to_tf_parts(a: &Matrix, b: &Matrix, c: &Matrix, d: &Matrix) -> Result<(Vec<f64>, Vec<f64>), LangError> {
    if b.cols() != 1 || c.rows() != 1 || d.numel() != 1 {
        return Err(LangError::new("ss2tf currently supports SISO systems only", 0));
    }
    let n = a.rows();
    if n == 0 {
        return Ok((vec![d.data().first().copied().unwrap_or(0.0)], vec![1.0]));
    }
    let den = charpoly(a)?;
    let bc = b.matmul(c).map_err(lang)?;
    let a_bc = a.elemwise(&bc, |x, y| x - y).map_err(lang)?;
    let p1 = charpoly(&a_bc)?;
    let dv = d.data().first().copied().unwrap_or(0.0);
    let num: Vec<f64> = (0..den.len()).map(|i| p1.get(i).copied().unwrap_or(0.0) + (dv - 1.0) * den.get(i).copied().unwrap_or(0.0)).collect();
    Ok((trim_zeros(num), den))
}

fn tf_to_zpk_parts(num: &[f64], den: &[f64]) -> Result<(Vec<CNum>, Vec<CNum>, f64), LangError> {
    let z = roots(&row(num))?;
    let p = roots(&row(den))?;
    let k = num.first().copied().unwrap_or(0.0) / den.first().copied().unwrap_or(1.0);
    Ok((z, p, k))
}

fn maybe_ss(v: Value, want_ss: bool) -> Result<Value, LangError> {
    if !want_ss {
        return Ok(v);
    }
    let (num, den) = tf_parts(&v)?;
    let (a, b, c, d) = tf_to_ss_parts(&num, &den)?;
    make_ss(a, b, c, d, 0.0)
}

fn is_ss(v: &Value) -> bool {
    is_class(v, "ss")
}

fn scalar_arg(v: &Value, what: &str) -> Result<f64, LangError> {
    v.scalar().map_err(|_| LangError::new(format!("{what} must be a real scalar"), 0))
}

fn time_vec(v: &Value, what: &str) -> Result<Vec<f64>, LangError> {
    let m = v.to_numeric()?;
    if m.numel() < 2 {
        return Err(LangError::new(format!("{what} must have at least two samples"), 0));
    }
    let t = m.data().to_vec();
    if t.windows(2).any(|w| w.get(1).copied().unwrap_or(0.0) <= w.first().copied().unwrap_or(0.0)) {
        return Err(LangError::new(format!("{what} must be strictly increasing"), 0));
    }
    Ok(t)
}

fn default_time(a: &Matrix, ts: f64) -> Result<Vec<f64>, LangError> {
    if ts > 0.0 {
        return Ok((0..100).map(|i| i as f64 * ts).collect());
    }
    let mut tfinal = 10.0f64;
    if a.rows() > 0 {
        let eigs = matcore::eigvals_c(a).map_err(lang)?;
        let decay = eigs.iter().map(|z| z.re).fold(f64::NEG_INFINITY, f64::max);
        if decay < -1e-9 {
            tfinal = (10.0 / -decay).clamp(1.0, 1000.0);
        }
    }
    let n = 200usize;
    let mut t = Vec::with_capacity(n);
    for i in 0..n {
        t.push(tfinal * i as f64 / (n.saturating_sub(1)) as f64);
    }
    Ok(t)
}

/// State-space simulation. Continuous systems use a zero-order-hold
/// matrix exponential; discrete systems (`Ts > 0`) use `x[k+1] = A x[k] +
/// B u[k]` directly. MIMO: `u` is N×m, output is N×p.
#[allow(clippy::too_many_arguments)]
fn simulate(a: &Matrix, b: &Matrix, c: &Matrix, d: &Matrix, u: &Matrix, t: &[f64], x0: &[f64], ts: f64) -> Result<Matrix, LangError> {
    let n = a.rows();
    let m = b.cols();
    let p = c.rows();
    if a.rows() != a.cols() || b.rows() != n || c.cols() != n || d.rows() != p || d.cols() != m {
        return Err(LangError::new("state-space matrices have inconsistent dimensions", 0));
    }
    if u.rows() != t.len() || u.cols() != m {
        return Err(LangError::new(format!("u must be {}x{} for this system", t.len(), m), 0));
    }
    if x0.len() != n {
        return Err(LangError::new("initial state has the wrong length", 0));
    }
    check_elems(t.len().saturating_mul(p))?;
    let mut x = Matrix::col_vector(x0.to_vec());
    let mut y = Matrix::zeros(t.len(), p);
    let discrete = ts > 0.0;
    let mut cached: Option<(f64, Matrix, Matrix)> = None;
    for k in 0..t.len() {
        let cx = c.matmul(&x).map_err(lang)?;
        for i in 0..p {
            let mut val = cx.get(i, 0).unwrap_or(0.0);
            for j in 0..m {
                val += d.get(i, j).unwrap_or(0.0) * u.get(k, j).unwrap_or(0.0);
            }
            let _ = y.set(k, i, val);
        }
        if k.saturating_add(1) >= t.len() {
            break;
        }
        let (ad, bd) = if discrete {
            (a.clone(), b.clone())
        } else {
            let dt = t.get(k.saturating_add(1)).copied().unwrap_or(0.0) - t.get(k).copied().unwrap_or(0.0);
            if dt.is_nan() || dt <= 0.0 {
                return Err(LangError::new("t must be strictly increasing", 0));
            }
            match &cached {
                Some((cached_dt, ad, bd)) if (*cached_dt - dt).abs() <= 1e-12 * dt.abs().max(1.0) => (ad.clone(), bd.clone()),
                _ => {
                    let size = n.saturating_add(m);
                    let mut aug = Matrix::zeros(size, size);
                    for i in 0..n {
                        for j in 0..n {
                            let _ = aug.set(i, j, a.get(i, j).unwrap_or(0.0) * dt);
                        }
                        for j in 0..m {
                            let _ = aug.set(i, n.saturating_add(j), b.get(i, j).unwrap_or(0.0) * dt);
                        }
                    }
                    let e = matcore::expm(&aug).map_err(lang)?;
                    let mut ad = Matrix::zeros(n, n);
                    let mut bd = Matrix::zeros(n, m);
                    for i in 0..n {
                        for j in 0..n {
                            let _ = ad.set(i, j, e.get(i, j).unwrap_or(0.0));
                        }
                        for j in 0..m {
                            let _ = bd.set(i, j, e.get(i, n.saturating_add(j)).unwrap_or(0.0));
                        }
                    }
                    cached = Some((dt, ad.clone(), bd.clone()));
                    (ad, bd)
                }
            }
        };
        let ax = ad.matmul(&x).map_err(lang)?;
        let mut xn = Matrix::zeros(n, 1);
        for i in 0..n {
            let mut val = ax.get(i, 0).unwrap_or(0.0);
            for j in 0..m {
                val += bd.get(i, j).unwrap_or(0.0) * u.get(k, j).unwrap_or(0.0);
            }
            let _ = xn.set(i, 0, val);
        }
        x = xn;
    }
    Ok(y)
}

fn polyval_c(coefs: &[f64], x: CNum) -> CNum {
    let mut acc = CNum::real(0.0);
    for &c in coefs {
        acc = acc.mul(x).add(CNum::real(c));
    }
    acc
}

fn logspace_vec(lo_exp: f64, hi_exp: f64, n: usize) -> Vec<f64> {
    let n = n.max(2);
    (0..n).map(|i| 10f64.powf(lo_exp + (hi_exp - lo_exp) * i as f64 / (n.saturating_sub(1)) as f64)).collect()
}

impl Session {
    pub(super) fn is_lti(&self, v: &Value) -> bool {
        lti_class(v).is_some()
    }

    /// Native operator dispatch for LTI objects (`s = tf('s'); 1/(s+1)`).
    pub(super) fn lti_binary(&mut self, op: BinOp, a: Value, b: Value) -> Result<Value, LangError> {
        let (n1, d1) = tf_parts(&a)?;
        let (n2, d2) = tf_parts(&b)?;
        let want_ss = is_ss(&a) || is_ss(&b);
        match op {
            BinOp::Add => maybe_ss(make_tf(poly_add(&conv(&n1, &d2), &conv(&n2, &d1)), conv(&d1, &d2), "s", 0.0)?, want_ss),
            BinOp::Sub => {
                let n2: Vec<f64> = n2.iter().map(|v| -v).collect();
                maybe_ss(make_tf(poly_add(&conv(&n1, &d2), &conv(&n2, &d1)), conv(&d1, &d2), "s", 0.0)?, want_ss)
            }
            BinOp::Mul | BinOp::ElemMul => maybe_ss(make_tf(conv(&n1, &n2), conv(&d1, &d2), "s", 0.0)?, want_ss),
            BinOp::Div | BinOp::ElemDiv => maybe_ss(make_tf(conv(&n1, &d2), conv(&d1, &n2), "s", 0.0)?, want_ss),
            BinOp::Pow | BinOp::ElemPow => {
                let e = scalar_arg(&b, "LTI exponent")?;
                if e.fract() != 0.0 || e.abs() > 64.0 {
                    return Err(LangError::new("LTI power needs an integer exponent in [-64, 64]", 0));
                }
                let power = e.abs() as usize;
                let (mut num, mut den) = (vec![1.0], vec![1.0]);
                for _ in 0..power {
                    num = conv(&num, &n1);
                    den = conv(&den, &d1);
                }
                if e < 0.0 {
                    std::mem::swap(&mut num, &mut den);
                }
                maybe_ss(make_tf(num, den, "s", 0.0)?, want_ss)
            }
            _ => Err(LangError::new("that operator is not defined for LTI systems", 0)),
        }
    }

    pub(super) fn control_builtin(&mut self, name: &str, argv: Vec<Value>, nout: usize) -> Result<Vec<Value>, LangError> {
        match name {
            "tf" => self.tf_call(&argv),
            "ss" => self.ss_call(&argv),
            "zpk" => self.zpk_call(&argv),
            "tfdata" => self.tfdata_call(&argv, nout),
            "ssdata" => self.ssdata_call(&argv, nout),
            "zpkdata" => self.zpkdata_call(&argv, nout),
            "pole" => Ok(vec![pole_value(&argv)?]),
            "zero" => Ok(vec![zero_value(&argv)?]),
            "dcgain" => Ok(vec![dcgain_value(&argv)?]),
            "series" => self.series_call(&argv),
            "parallel" => self.parallel_call(&argv),
            "feedback" => self.feedback_call(&argv),
            "tf2ss" => self.tf2ss_call(&argv),
            "ss2tf" => self.ss2tf_call(&argv),
            "step" => self.step_call(&argv, nout),
            "impulse" => self.impulse_call(&argv, nout),
            "initial" => self.initial_call(&argv, nout),
            "lsim" => self.lsim_call(&argv, nout),
            "bode" => self.bode_call(&argv),
            "freqresp" => self.freqresp_call(&argv),
            "pid" => self.pid_call(&argv),
            "pidtune" => self.pidtune_call(&argv, nout),
            "margin" => self.margin_call(&argv),
            "nyquist" => self.nyquist_call(&argv),
            "isstable" => Ok(vec![Value::Num(is_stable(argv.first())? as u8 as f64)]),
            "damp" => self.damp_call(&argv),
            "bandwidth" => Ok(vec![Value::Num(bandwidth_value(&argv)?)]),
            _ => Err(LangError::new(format!("{name}: not a control builtin"), 0)),
        }
    }

    fn tf_call(&mut self, argv: &[Value]) -> Result<Vec<Value>, LangError> {
        if argv.len() == 1 {
            if let Value::Str(var) = &argv[0] {
                if matches!(var.as_str(), "s" | "p" | "z") {
                    return Ok(vec![make_tf(vec![1.0, 0.0], vec![1.0], var, 0.0)?]);
                }
            }
            let (num, den) = tf_parts(&argv[0])?;
            return Ok(vec![make_tf(num, den, "s", 0.0)?]);
        }
        let num = value_rows(argv.first().ok_or_else(|| LangError::new("tf needs a numerator", 0))?)?;
        let den = value_rows(argv.get(1).ok_or_else(|| LangError::new("tf needs a denominator", 0))?)?;
        let ts = argv.get(2).map(|v| scalar_arg(v, "Ts")).transpose()?.unwrap_or(0.0);
        Ok(vec![make_tf(num, den, "s", ts)?])
    }

    fn ss_call(&mut self, argv: &[Value]) -> Result<Vec<Value>, LangError> {
        if argv.len() == 1 {
            let (a, b, c, d) = ss_parts(&argv[0])?;
            return Ok(vec![make_ss(a, b, c, d, 0.0)?]);
        }
        if argv.len() < 4 {
            return Err(LangError::new("ss needs A, B, C and D", 0));
        }
        let a = argv[0].to_matrix()?;
        let b = argv[1].to_matrix()?;
        let c = argv[2].to_matrix()?;
        let d = argv[3].to_matrix()?;
        let ts = argv.get(4).map(|v| scalar_arg(v, "Ts")).transpose()?.unwrap_or(0.0);
        Ok(vec![make_ss(a, b, c, d, ts)?])
    }

    fn zpk_call(&mut self, argv: &[Value]) -> Result<Vec<Value>, LangError> {
        if argv.len() == 1 {
            let (num, den) = tf_parts(&argv[0])?;
            let (z, p, k) = tf_to_zpk_parts(&num, &den)?;
            return Ok(vec![make_zpk(z, p, k, "s", 0.0)?]);
        }
        if argv.len() < 3 {
            return Err(LangError::new("zpk needs zeros, poles and gain", 0));
        }
        let z = complex_rows(&argv[0], "zeros")?;
        let p = complex_rows(&argv[1], "poles")?;
        let k = scalar_arg(&argv[2], "gain")?;
        Ok(vec![make_zpk(z, p, k, "s", 0.0)?])
    }

    fn tfdata_call(&mut self, argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
        let (num, den) = tf_parts(argv.first().ok_or_else(|| LangError::new("tfdata needs a system", 0))?)?;
        if nout >= 2 {
            Ok(vec![Value::Mat(row(&num)), Value::Mat(row(&den))])
        } else {
            Ok(vec![Value::Cell(CellArray::row(vec![Value::Mat(row(&num)), Value::Mat(row(&den))]))])
        }
    }

    fn ssdata_call(&mut self, argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
        let (a, b, c, d) = ss_parts(argv.first().ok_or_else(|| LangError::new("ssdata needs a system", 0))?)?;
        if nout >= 4 {
            Ok(vec![Value::Mat(a), Value::Mat(b), Value::Mat(c), Value::Mat(d)])
        } else {
            Ok(vec![Value::Cell(CellArray::row(vec![Value::Mat(a), Value::Mat(b), Value::Mat(c), Value::Mat(d)]))])
        }
    }

    fn zpkdata_call(&mut self, argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
        let v = argv.first().ok_or_else(|| LangError::new("zpkdata needs a system", 0))?;
        let (z, p, k) = if let Value::Object(o) = v {
            if o.class_name == "zpk" {
                zpk_data(o)?
            } else {
                let (num, den) = tf_parts(v)?;
                tf_to_zpk_parts(&num, &den)?
            }
        } else {
            let (num, den) = tf_parts(v)?;
            tf_to_zpk_parts(&num, &den)?
        };
        if nout >= 3 {
            Ok(vec![roots_value(&z)?, roots_value(&p)?, Value::Num(k)])
        } else {
            Ok(vec![Value::Cell(CellArray::row(vec![roots_value(&z)?, roots_value(&p)?, Value::Num(k)]))])
        }
    }

    fn series_call(&mut self, argv: &[Value]) -> Result<Vec<Value>, LangError> {
        if argv.len() < 2 {
            return Err(LangError::new("series needs two systems", 0));
        }
        let (n1, d1) = tf_parts(&argv[0])?;
        let (n2, d2) = tf_parts(&argv[1])?;
        let out = make_tf(conv(&n1, &n2), conv(&d1, &d2), "s", 0.0)?;
        Ok(vec![maybe_ss(out, is_ss(&argv[0]) || is_ss(&argv[1]))?])
    }

    fn parallel_call(&mut self, argv: &[Value]) -> Result<Vec<Value>, LangError> {
        if argv.len() < 2 {
            return Err(LangError::new("parallel needs two systems", 0));
        }
        let (n1, d1) = tf_parts(&argv[0])?;
        let (n2, d2) = tf_parts(&argv[1])?;
        let num = poly_add(&conv(&n1, &d2), &conv(&n2, &d1));
        let out = make_tf(num, conv(&d1, &d2), "s", 0.0)?;
        Ok(vec![maybe_ss(out, is_ss(&argv[0]) || is_ss(&argv[1]))?])
    }

    fn feedback_call(&mut self, argv: &[Value]) -> Result<Vec<Value>, LangError> {
        if argv.is_empty() {
            return Err(LangError::new("feedback needs a system", 0));
        }
        let (n1, d1) = tf_parts(&argv[0])?;
        let (n2, d2) = if let Some(second) = argv.get(1) { tf_parts(second)? } else { (vec![1.0], vec![1.0]) };
        let sign = argv.get(2).map(|v| scalar_arg(v, "feedback sign")).transpose()?.unwrap_or(-1.0);
        let num = conv(&n1, &d2);
        let cross = conv(&n1, &n2);
        let cross: Vec<f64> = cross.iter().map(|v| -sign * v).collect();
        let den = poly_add(&conv(&d1, &d2), &cross);
        let out = make_tf(num, den, "s", 0.0)?;
        Ok(vec![maybe_ss(out, is_ss(&argv[0]))?])
    }

    fn tf2ss_call(&mut self, argv: &[Value]) -> Result<Vec<Value>, LangError> {
        let (num, den) = if argv.len() == 1 {
            tf_parts(&argv[0])?
        } else {
            (
                value_rows(argv.first().ok_or_else(|| LangError::new("tf2ss needs a numerator", 0))?)?,
                value_rows(argv.get(1).ok_or_else(|| LangError::new("tf2ss needs a denominator", 0))?)?,
            )
        };
        let (a, b, c, d) = tf_to_ss_parts(&num, &den)?;
        Ok(vec![Value::Mat(a), Value::Mat(b), Value::Mat(c), Value::Mat(d)])
    }

    fn ss2tf_call(&mut self, argv: &[Value]) -> Result<Vec<Value>, LangError> {
        let (a, b, c, d) = if argv.len() == 1 {
            ss_parts(&argv[0])?
        } else {
            if argv.len() < 4 {
                return Err(LangError::new("ss2tf needs A, B, C and D", 0));
            }
            (argv[0].to_matrix()?, argv[1].to_matrix()?, argv[2].to_matrix()?, argv[3].to_matrix()?)
        };
        let (num, den) = ss_to_tf_parts(&a, &b, &c, &d)?;
        Ok(vec![Value::Mat(row(&num)), Value::Mat(row(&den))])
    }

    fn response_inputs(&mut self, argv: &[Value], time_idx: usize) -> Result<ResponseSystem, LangError> {
        let sys = argv.first().ok_or_else(|| LangError::new("a system is required", 0))?;
        let (a, b, c, d) = ss_parts(sys)?;
        let ts = system_ts(sys);
        let t = match argv.get(time_idx) {
            Some(v) if v.to_matrix().map(|m| m.numel()).unwrap_or(0) > 1 => time_vec(v, "t")?,
            _ => default_time(&a, ts)?,
        };
        Ok(ResponseSystem { a, b, c, d, t, ts })
    }

    fn response_outputs(&self, y: Matrix, t: Vec<f64>, nout: usize) -> Result<Vec<Value>, LangError> {
        let mut out = vec![mat_or_num(y), Value::Mat(Matrix::col_vector(t))];
        out.truncate(nout.max(1));
        Ok(out)
    }

    fn step_call(&mut self, argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
        let ResponseSystem { a, b, c, d, t, ts } = self.response_inputs(argv, 1)?;
        let (n, m, p) = (a.rows(), b.cols(), c.rows());
        if m == 1 {
            let mut u = Matrix::zeros(t.len(), 1);
            for k in 0..t.len() {
                let _ = u.set(k, 0, 1.0);
            }
            let y = simulate(&a, &b, &c, &d, &u, &t, &vec![0.0; n], ts)?;
            return self.response_outputs(y, t, nout);
        }
        let mut data = vec![0.0; t.len().saturating_mul(p).saturating_mul(m)];
        for j in 0..m {
            let mut u = Matrix::zeros(t.len(), m);
            for k in 0..t.len() {
                let _ = u.set(k, j, 1.0);
            }
            let y = simulate(&a, &b, &c, &d, &u, &t, &vec![0.0; n], ts)?;
            for k in 0..t.len() {
                for i in 0..p {
                    if let Some(slot) =
                        data.get_mut(k.saturating_add(i.saturating_mul(t.len())).saturating_add(j.saturating_mul(t.len()).saturating_mul(p)))
                    {
                        *slot = y.get(k, i).unwrap_or(0.0);
                    }
                }
            }
        }
        let y = Matrix::from_col_major_nd(&[t.len(), p, m], data).map_err(lang)?;
        self.response_outputs(y, t, nout)
    }

    fn impulse_call(&mut self, argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
        let ResponseSystem { a, b, c, d, t, ts } = self.response_inputs(argv, 1)?;
        let (n, m, p) = (a.rows(), b.cols(), c.rows());
        if m == 1 {
            let x0: Vec<f64> = (0..n).map(|i| b.get(i, 0).unwrap_or(0.0)).collect();
            let u = Matrix::zeros(t.len(), 1);
            let y = simulate(&a, &b, &c, &d, &u, &t, &x0, ts)?;
            return self.response_outputs(y, t, nout);
        }
        let mut data = vec![0.0; t.len().saturating_mul(p).saturating_mul(m)];
        for j in 0..m {
            let x0: Vec<f64> = (0..n).map(|i| b.get(i, j).unwrap_or(0.0)).collect();
            let u = Matrix::zeros(t.len(), m);
            let y = simulate(&a, &b, &c, &d, &u, &t, &x0, ts)?;
            for k in 0..t.len() {
                for i in 0..p {
                    if let Some(slot) =
                        data.get_mut(k.saturating_add(i.saturating_mul(t.len())).saturating_add(j.saturating_mul(t.len()).saturating_mul(p)))
                    {
                        *slot = y.get(k, i).unwrap_or(0.0);
                    }
                }
            }
        }
        let y = Matrix::from_col_major_nd(&[t.len(), p, m], data).map_err(lang)?;
        self.response_outputs(y, t, nout)
    }

    fn initial_call(&mut self, argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
        let sys = argv.first().ok_or_else(|| LangError::new("initial needs a system", 0))?;
        let (a, b, c, d) = ss_parts(sys)?;
        let ts = system_ts(sys);
        let x0 = value_rows(argv.get(1).ok_or_else(|| LangError::new("initial needs x0", 0))?)?;
        let t = match argv.get(2) {
            Some(v) => time_vec(v, "t")?,
            None => default_time(&a, ts)?,
        };
        let u = Matrix::zeros(t.len(), b.cols());
        let y = simulate(&a, &b, &c, &d, &u, &t, &x0, ts)?;
        self.response_outputs(y, t, nout)
    }

    fn lsim_call(&mut self, argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
        let sys = argv.first().ok_or_else(|| LangError::new("lsim needs a system", 0))?;
        let (a, b, c, d) = ss_parts(sys)?;
        let ts = system_ts(sys);
        let t = time_vec(argv.get(2).ok_or_else(|| LangError::new("lsim needs t", 0))?, "t")?;
        let um = argv.get(1).ok_or_else(|| LangError::new("lsim needs u", 0))?.to_numeric()?;
        let m = b.cols();
        let u = if um.rows() == t.len() && um.cols() == m {
            um
        } else if m == 1 && um.numel() == t.len() {
            Matrix::col_vector(um.data().to_vec())
        } else {
            return Err(LangError::new(format!("lsim: u must be a {}x{} matrix", t.len(), m), 0));
        };
        let x0 = match argv.get(3) {
            Some(v) => value_rows(v)?,
            None => vec![0.0; a.rows()],
        };
        let y = simulate(&a, &b, &c, &d, &u, &t, &x0, ts)?;
        self.response_outputs(y, t, nout)
    }

    fn bode_call(&mut self, argv: &[Value]) -> Result<Vec<Value>, LangError> {
        let sys = argv.first().ok_or_else(|| LangError::new("bode needs a system", 0))?;
        let (w, column) = match argv.get(1) {
            Some(v) => {
                let m = v.to_numeric()?;
                (m.data().to_vec(), m.rows() > m.cols())
            }
            None => (logspace_vec(-2.0, 2.0, 200), false),
        };
        if let Some((a, b, c, d)) = mimo_ss_parts(sys)? {
            let resp = ss_freq_response(&a, &b, &c, &d, &w)?;
            let (p, m) = (c.rows(), b.cols());
            let nw = w.len();
            check_elems(p.saturating_mul(m).saturating_mul(nw))?;
            let mut mag = vec![0.0f64; p.saturating_mul(m).saturating_mul(nw)];
            let mut phase = vec![0.0f64; p.saturating_mul(m).saturating_mul(nw)];
            for (k, h) in resp.iter().enumerate() {
                for out in 0..p {
                    for inp in 0..m {
                        let z = h.get(out.saturating_mul(m).saturating_add(inp)).copied().unwrap_or(CNum::real(0.0));
                        let idx = k.saturating_mul(p).saturating_mul(m).saturating_add(inp.saturating_mul(p)).saturating_add(out);
                        if let Some(slot) = mag.get_mut(idx) {
                            *slot = (z.re * z.re + z.im * z.im).sqrt();
                        }
                        if let Some(slot) = phase.get_mut(idx) {
                            *slot = z.im.atan2(z.re).to_degrees();
                        }
                    }
                }
            }
            let magm = Matrix::from_col_major_nd(&[p, m, nw], mag).map_err(lang)?;
            let pham = Matrix::from_col_major_nd(&[p, m, nw], phase).map_err(lang)?;
            let wrap = |v: Vec<f64>| if column { Value::Mat(Matrix::col_vector(v)) } else { Value::Mat(Matrix::row_vector(v)) };
            return Ok(vec![Value::Mat(magm), Value::Mat(pham), wrap(w)]);
        }
        let (num, den) = tf_parts(sys)?;
        let mut mag = Vec::with_capacity(w.len());
        let mut phase = Vec::with_capacity(w.len());
        for &wi in &w {
            let jw = CNum::new(0.0, wi);
            let h = polyval_c(&num, jw).div(polyval_c(&den, jw));
            mag.push((h.re * h.re + h.im * h.im).sqrt());
            phase.push(h.im.atan2(h.re).to_degrees());
        }
        let wrap = |v: Vec<f64>| if column { Value::Mat(Matrix::col_vector(v)) } else { Value::Mat(Matrix::row_vector(v)) };
        Ok(vec![wrap(mag), wrap(phase), wrap(w)])
    }

    fn freqresp_call(&mut self, argv: &[Value]) -> Result<Vec<Value>, LangError> {
        let sys = argv.first().ok_or_else(|| LangError::new("freqresp needs a system", 0))?;
        let wm = argv.get(1).ok_or_else(|| LangError::new("freqresp needs a frequency vector", 0))?.to_numeric()?;
        let w = wm.data();
        if let Some((a, b, c, d)) = mimo_ss_parts(sys)? {
            let resp = ss_freq_response(&a, &b, &c, &d, w)?;
            let channels = c.rows().saturating_mul(b.cols());
            let nw = w.len();
            check_elems(channels.saturating_mul(nw))?;
            // MATLAB MIMO freqresp is p x m x N; our complex matrices are
            // 2-D, so return the same data as (p*m) x N. Row `out*m + inp`
            // is the response from input `inp` to output `out`.
            let mut data = Vec::with_capacity(channels.saturating_mul(nw));
            for h in &resp {
                data.extend(h.iter().copied());
            }
            let cm = CMatrix::from_col_major(channels, nw, data).map_err(lang)?;
            return Ok(vec![cmat_or_value(cm)]);
        }
        let (num, den) = tf_parts(sys)?;
        let column = wm.rows() > wm.cols();
        let hs = freq_response(&num, &den, w);
        let cm =
            if column { CMatrix::from_col_major(hs.len(), 1, hs).map_err(lang)? } else { CMatrix::from_col_major(1, hs.len(), hs).map_err(lang)? };
        Ok(vec![cmat_or_value(cm)])
    }

    fn default_crossover(den: &[f64]) -> f64 {
        let mut best = f64::INFINITY;
        if let Ok(roots) = roots(&row(den)) {
            for z in roots {
                let m = (z.re * z.re + z.im * z.im).sqrt();
                if m > 1e-9 && m < best {
                    best = m;
                }
            }
        }
        if best.is_finite() {
            best.clamp(1e-3, 1e4)
        } else {
            1.0
        }
    }

    fn pid_call(&mut self, argv: &[Value]) -> Result<Vec<Value>, LangError> {
        let kp = argv.first().map(|v| scalar_arg(v, "Kp")).transpose()?.unwrap_or(0.0);
        let ki = argv.get(1).map(|v| scalar_arg(v, "Ki")).transpose()?.unwrap_or(0.0);
        let kd = argv.get(2).map(|v| scalar_arg(v, "Kd")).transpose()?.unwrap_or(0.0);
        let tf = argv.get(3).map(|v| scalar_arg(v, "Tf")).transpose()?.unwrap_or(0.0).max(0.0);
        let (num, den) = if tf > 0.0 { (vec![kp * tf + kd, kp + ki * tf, ki], vec![tf, 1.0, 0.0]) } else { (vec![kd, kp, ki], vec![1.0, 0.0]) };
        Ok(vec![make_tf(num, den, "s", 0.0)?])
    }

    /// `pidtune(sys, type, wc)` — shape a PI/PID/PD controller so the open
    /// loop crosses 0 dB at `wc` with a 60 degree phase margin. The plant is
    /// evaluated at `j*wc`; integral/derivative breakpoints are chosen from
    /// the required controller phase, then `Kp` normalises the magnitude.
    fn pidtune_call(&mut self, argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
        let sys = argv.first().ok_or_else(|| LangError::new("pidtune needs a plant model", 0))?;
        let (num, den) = tf_parts(sys)?;
        if num.is_empty() || den.is_empty() {
            return Err(LangError::new("pidtune: plant transfer function is empty", 0));
        }
        let mut kind = "pid".to_string();
        let mut requested_wc: Option<f64> = None;
        for arg in argv.iter().skip(1) {
            match arg {
                Value::Str(s) => kind = s.to_ascii_lowercase(),
                other => requested_wc = Some(scalar_arg(other, "wc")?),
            }
        }
        if !matches!(kind.as_str(), "p" | "pi" | "pd" | "pid" | "pidf" | "pid2") {
            return Err(LangError::new(format!("pidtune: unknown controller type '{kind}'"), 0));
        }
        let wc = requested_wc.unwrap_or_else(|| Self::default_crossover(&den));
        if !wc.is_finite() || wc <= 0.0 {
            return Err(LangError::new("pidtune: wc must be positive and finite", 0));
        }
        let g = freq_response(&num, &den, &[wc]).into_iter().next().ok_or_else(|| LangError::new("pidtune: cannot evaluate the plant", 0))?;
        let mg = (g.re * g.re + g.im * g.im).sqrt();
        if !mg.is_finite() || mg <= 0.0 {
            return Err(LangError::new("pidtune: plant response is zero or infinite at wc", 0));
        }
        let pg = g.im.atan2(g.re);
        // Controller phase needed to leave the open loop at the 60 degree target.
        let target = -std::f64::consts::PI + 60.0f64.to_radians() - pg;
        let tf = if kind == "pidf" { 1.0 / (10.0 * wc) } else { 0.0 };
        let (ti0, td0) = match kind.as_str() {
            "p" => (0.0, 0.0),
            "pi" => {
                if target < 0.0 && target.tan() < 0.0 {
                    (-1.0 / (wc * target.tan()), 0.0)
                } else {
                    (10.0 / wc, 0.0)
                }
            }
            "pd" => (0.0, (target.tan() / wc).max(0.0)),
            "pid" | "pid2" => {
                let ti = 10.0 / wc;
                if tf > 0.0 {
                    let td = solve_filtered_derivative(wc, 1.0 / ti, tf, target);
                    (ti, td)
                } else {
                    let td = (target.tan() + 1.0 / (ti * wc)) / wc;
                    if td > 0.0 {
                        (ti, td)
                    } else if target < 0.0 && target.tan() < 0.0 {
                        (-1.0 / (wc * target.tan()), 0.0)
                    } else {
                        (ti, 0.0)
                    }
                }
            }
            _ => {
                let ti = 10.0 / wc;
                let td = solve_filtered_derivative(wc, 1.0 / ti, tf, target);
                if td > 0.0 {
                    (ti, td)
                } else if target < 0.0 && target.tan() < 0.0 {
                    (-1.0 / (wc * target.tan()), 0.0)
                } else {
                    (ti, 0.0)
                }
            }
        };
        let ki0 = if kind == "p" || kind == "pd" || ti0 <= 0.0 { 0.0 } else { 1.0 / ti0 };
        let kd0 = if kind == "p" || kind == "pi" { 0.0 } else { td0 };
        let c0 = pid_response(wc, 1.0, ki0, kd0, tf);
        let mc = (c0.re * c0.re + c0.im * c0.im).sqrt();
        let pc = c0.im.atan2(c0.re);
        if !mc.is_finite() || mc <= 0.0 {
            return Err(LangError::new("pidtune: controller response is degenerate at wc", 0));
        }
        let kp = 1.0 / (mg * mc);
        let ki = kp * ki0;
        let kd = kp * kd0;
        let (cnum, cden) = if tf > 0.0 { (vec![kp * tf + kd, kp + ki * tf, ki], vec![tf, 1.0, 0.0]) } else { (vec![kd, kp, ki], vec![1.0, 0.0]) };
        let c = make_tf(cnum, cden, "s", 0.0)?;
        let pm = wrap_degrees(180.0 + (pg + pc).to_degrees());
        let mut info = StructArray::new();
        info.set_field(0, "C", c.clone())?;
        info.set_field(0, "Kp", Value::Num(kp))?;
        info.set_field(0, "Ki", Value::Num(ki))?;
        info.set_field(0, "Kd", Value::Num(kd))?;
        info.set_field(0, "Tf", Value::Num(tf))?;
        info.set_field(0, "wc", Value::Num(wc))?;
        info.set_field(0, "PhaseMargin", Value::Num(pm))?;
        info.set_field(0, "GainMargin", Value::Num(f64::INFINITY))?;
        info.set_field(0, "Stable", Value::Num((pm > 0.0) as u8 as f64))?;
        info.set_field(0, "Type", Value::Str(kind.to_ascii_uppercase()))?;
        let out = vec![c, Value::Struct(info)];
        Ok(out.into_iter().take(nout.max(1)).collect())
    }

    fn margin_call(&mut self, argv: &[Value]) -> Result<Vec<Value>, LangError> {
        let sys = argv.first().ok_or_else(|| LangError::new("margin needs a system", 0))?;
        let (gm, pm, wcg, wcp) = margin_values(sys)?;
        Ok(vec![Value::Num(gm), Value::Num(pm), Value::Num(wcg), Value::Num(wcp)])
    }

    fn nyquist_call(&mut self, argv: &[Value]) -> Result<Vec<Value>, LangError> {
        let sys = argv.first().ok_or_else(|| LangError::new("nyquist needs a system", 0))?;
        let (w, column) = match argv.get(1) {
            Some(v) => {
                let m = v.to_numeric()?;
                (m.data().to_vec(), m.rows() > m.cols())
            }
            None => (logspace_vec(-2.0, 2.0, 500), false),
        };
        if let Some((a, b, c, d)) = mimo_ss_parts(sys)? {
            let resp = ss_freq_response(&a, &b, &c, &d, &w)?;
            let (p, m) = (c.rows(), b.cols());
            let nw = w.len();
            check_elems(p.saturating_mul(m).saturating_mul(nw))?;
            let mut re = vec![0.0f64; p.saturating_mul(m).saturating_mul(nw)];
            let mut im = vec![0.0f64; p.saturating_mul(m).saturating_mul(nw)];
            for (k, h) in resp.iter().enumerate() {
                for out in 0..p {
                    for inp in 0..m {
                        let z = h.get(out.saturating_mul(m).saturating_add(inp)).copied().unwrap_or(CNum::real(0.0));
                        let idx = k.saturating_mul(p).saturating_mul(m).saturating_add(inp.saturating_mul(p)).saturating_add(out);
                        if let Some(slot) = re.get_mut(idx) {
                            *slot = z.re;
                        }
                        if let Some(slot) = im.get_mut(idx) {
                            *slot = z.im;
                        }
                    }
                }
            }
            let rem = Matrix::from_col_major_nd(&[p, m, nw], re).map_err(lang)?;
            let imm = Matrix::from_col_major_nd(&[p, m, nw], im).map_err(lang)?;
            let wrap = |v: Vec<f64>| if column { Value::Mat(Matrix::col_vector(v)) } else { Value::Mat(Matrix::row_vector(v)) };
            return Ok(vec![Value::Mat(rem), Value::Mat(imm), wrap(w)]);
        }
        let (num, den) = tf_parts(sys)?;
        let h = freq_response(&num, &den, &w);
        let re: Vec<f64> = h.iter().map(|z| z.re).collect();
        let im: Vec<f64> = h.iter().map(|z| z.im).collect();
        let wrap = |v: Vec<f64>| if column { Value::Mat(Matrix::col_vector(v)) } else { Value::Mat(Matrix::row_vector(v)) };
        Ok(vec![wrap(re), wrap(im), wrap(w)])
    }

    fn damp_call(&mut self, argv: &[Value]) -> Result<Vec<Value>, LangError> {
        let sys = argv.first().ok_or_else(|| LangError::new("damp needs a system", 0))?;
        let poles = system_poles(sys)?;
        let ts = system_ts(sys);
        let mut wn = Vec::with_capacity(poles.len());
        let mut zeta = Vec::with_capacity(poles.len());
        for z in &poles {
            let s = if ts > 0.0 { z.ln().div(CNum::real(ts)) } else { *z };
            let w = (s.re * s.re + s.im * s.im).sqrt();
            wn.push(w);
            zeta.push(if w > 0.0 { -s.re / w } else { 0.0 });
        }
        Ok(vec![
            Value::Mat(Matrix::col_vector(wn)),
            Value::Mat(Matrix::col_vector(zeta)),
            cmat_or_value(CMatrix::from_col_major(poles.len(), 1, poles).map_err(lang)?),
        ])
    }
}

fn mimo_ss_parts(v: &Value) -> Result<Option<(Matrix, Matrix, Matrix, Matrix)>, LangError> {
    match v {
        Value::Object(o) if o.class_name == "ss" => {
            let (a, b, c, d) = ss_data(o)?;
            if b.cols() > 1 || c.rows() > 1 {
                Ok(Some((a, b, c, d)))
            } else {
                Ok(None)
            }
        }
        _ => Ok(None),
    }
}

/// `H(jw) = C (jwI - A)^-1 B + D` for every frequency. Each returned vector
/// is row-major `p*m`: row index `out*m + inp`.
fn ss_freq_response(a: &Matrix, b: &Matrix, c: &Matrix, d: &Matrix, w: &[f64]) -> Result<Vec<Vec<CNum>>, LangError> {
    let n = a.rows();
    let p = c.rows();
    let m = b.cols();
    if a.rows() != a.cols() || b.rows() != n || c.cols() != n || d.rows() != p || d.cols() != m {
        return Err(LangError::new("ss: A/B/C/D dimensions are inconsistent", 0));
    }
    let cc = CMatrix::from_real(c);
    let bc = CMatrix::from_real(b);
    let mut out = Vec::with_capacity(w.len());
    for &wi in w {
        let mut s = CMatrix::from_real(&a.map(|v| -v));
        for i in 0..n {
            let z = s.get(i, i).unwrap_or(CNum::real(0.0));
            s.set(i, i, CNum::new(z.re, wi)).map_err(lang)?;
        }
        let x = s.solve(&bc).map_err(|_| LangError::new(format!("bode/freqresp: singular system at frequency {wi}"), 0))?;
        let mut h = cc.matmul(&x).map_err(lang)?;
        for r in 0..p {
            for col in 0..m {
                let z = h.get(r, col).unwrap_or(CNum::real(0.0)) + CNum::real(d.get(r, col).unwrap_or(0.0));
                h.set(r, col, z).map_err(lang)?;
            }
        }
        let mut flat = Vec::with_capacity(p.saturating_mul(m));
        for r in 0..p {
            for col in 0..m {
                flat.push(h.get(r, col).unwrap_or(CNum::real(0.0)));
            }
        }
        out.push(flat);
    }
    Ok(out)
}

fn freq_response(num: &[f64], den: &[f64], w: &[f64]) -> Vec<CNum> {
    w.iter()
        .map(|&wi| {
            let jw = CNum::new(0.0, wi);
            polyval_c(num, jw).div(polyval_c(den, jw))
        })
        .collect()
}

fn system_poles(v: &Value) -> Result<Vec<CNum>, LangError> {
    match v {
        Value::Object(o) if o.class_name == "zpk" => {
            let (_, p, _) = zpk_data(o)?;
            Ok(p)
        }
        Value::Object(o) if o.class_name == "ss" => {
            let (a, _, _, _) = ss_data(o)?;
            matcore::eigvals_c(&a).map_err(lang)
        }
        _ => {
            let (_, den) = tf_parts(v)?;
            roots(&row(&den))
        }
    }
}

fn system_ts(v: &Value) -> f64 {
    match v {
        Value::Object(o) => o.prop("Ts").and_then(|x| x.scalar().ok()).unwrap_or(0.0),
        _ => 0.0,
    }
}

fn is_stable(v: Option<&Value>) -> Result<bool, LangError> {
    let v = v.ok_or_else(|| LangError::new("isstable needs a system", 0))?;
    let poles = system_poles(v)?;
    let ts = system_ts(v);
    Ok(poles.iter().all(|z| if ts > 0.0 { (z.re * z.re + z.im * z.im).sqrt() < 1.0 } else { z.re < 0.0 }))
}

fn margins_from_response(w: &[f64], h: &[CNum]) -> (f64, f64, f64, f64) {
    let mag: Vec<f64> = h.iter().map(|z| (z.re * z.re + z.im * z.im).sqrt()).collect();
    let mut phase: Vec<f64> = h.iter().map(|z| z.im.atan2(z.re).to_degrees()).collect();
    for i in 1..phase.len() {
        let prev = phase.get(i.saturating_sub(1)).copied().unwrap_or(0.0);
        let mut p = phase.get(i).copied().unwrap_or(0.0);
        while p - prev > 180.0 {
            p -= 360.0;
        }
        while prev - p > 180.0 {
            p += 360.0;
        }
        phase[i] = p;
    }
    let interp = |a: f64, b: f64, va: f64, vb: f64, target: f64| -> f64 {
        if (vb - va).abs() < 1e-15 {
            a
        } else {
            a + (b - a) * (target - va) / (vb - va)
        }
    };
    let mut gm = f64::INFINITY;
    let mut pm = f64::NAN;
    let mut wcg = f64::NAN;
    let mut wcp = f64::NAN;
    for i in 1..w.len() {
        let m0 = mag.get(i.saturating_sub(1)).copied().unwrap_or(1.0);
        let m1 = mag.get(i).copied().unwrap_or(1.0);
        if (m0 - 1.0) * (m1 - 1.0) <= 0.0 && wcp.is_nan() {
            let ww = interp(w[i - 1], w[i], m0, m1, 1.0);
            let t = if (m1 - m0).abs() < 1e-15 { 0.0 } else { (1.0 - m0) / (m1 - m0) };
            let ph = phase[i - 1] + t * (phase[i] - phase[i - 1]);
            wcp = ww;
            pm = 180.0 + ph;
        }
        let p0 = phase.get(i.saturating_sub(1)).copied().unwrap_or(0.0);
        let p1 = phase.get(i).copied().unwrap_or(0.0);
        if (p0 + 180.0) * (p1 + 180.0) <= 0.0 && wcg.is_nan() {
            let ww = interp(w[i - 1], w[i], p0, p1, -180.0);
            let t = if (p1 - p0).abs() < 1e-15 { 0.0 } else { (-180.0 - p0) / (p1 - p0) };
            let mm = m0 + t * (m1 - m0);
            wcg = ww;
            gm = if mm > 0.0 { 1.0 / mm } else { f64::INFINITY };
        }
    }
    (gm, pm, wcg, wcp)
}

fn margin_values(v: &Value) -> Result<(f64, f64, f64, f64), LangError> {
    if let Some((a, b, c, d)) = mimo_ss_parts(v)? {
        let (p, m) = (c.rows(), b.cols());
        if p != m {
            return Err(LangError::new("margin: MIMO systems must have as many outputs as inputs", 0));
        }
        let w = logspace_vec(-3.0, 3.0, 4000);
        let resp = ss_freq_response(&a, &b, &c, &d, &w)?;
        let mut worst_gm = f64::INFINITY;
        let mut worst_pm = f64::NAN;
        let mut worst_wcg = f64::NAN;
        let mut worst_wcp = f64::NAN;
        let mut gm_dist = f64::INFINITY;
        for out in 0..p {
            for inp in 0..m {
                let channel: Vec<CNum> =
                    resp.iter().map(|h| h.get(out.saturating_mul(m).saturating_add(inp)).copied().unwrap_or(CNum::real(0.0))).collect();
                let (gm, pm, wcg, wcp) = margins_from_response(&w, &channel);
                if pm.is_finite() && (worst_pm.is_nan() || pm < worst_pm) {
                    worst_pm = pm;
                    worst_wcp = wcp;
                }
                if gm.is_finite() && gm > 0.0 {
                    let dist = gm.ln().abs();
                    if dist < gm_dist {
                        gm_dist = dist;
                        worst_gm = gm;
                        worst_wcg = wcg;
                    }
                }
            }
        }
        return Ok((worst_gm, worst_pm, worst_wcg, worst_wcp));
    }
    let (num, den) = tf_parts(v)?;
    let w = logspace_vec(-3.0, 3.0, 4000);
    let h = freq_response(&num, &den, &w);
    Ok(margins_from_response(&w, &h))
}

fn bandwidth_value(argv: &[Value]) -> Result<f64, LangError> {
    let (num, den) = tf_parts(argv.first().ok_or_else(|| LangError::new("bandwidth needs a system", 0))?)?;
    let dc = freq_response(&num, &den, &[0.0]).first().map(|z| (z.re * z.re + z.im * z.im).sqrt()).unwrap_or(0.0);
    if dc.is_nan() || dc <= 0.0 {
        return Ok(f64::NAN);
    }
    let target = dc / 2f64.sqrt();
    let w = logspace_vec(-4.0, 4.0, 4000);
    let h = freq_response(&num, &den, &w);
    for i in 1..w.len() {
        let m0 = h.get(i.saturating_sub(1)).map(|z| (z.re * z.re + z.im * z.im).sqrt()).unwrap_or(0.0);
        let m1 = h.get(i).map(|z| (z.re * z.re + z.im * z.im).sqrt()).unwrap_or(0.0);
        if m0 >= target && m1 <= target {
            let t = (m1 - m0).abs();
            return Ok(if t < 1e-15 { w[i] } else { w[i - 1] + (w[i] - w[i - 1]) * (target - m0) / (m1 - m0) });
        }
    }
    Ok(f64::NAN)
}

fn pole_value(argv: &[Value]) -> Result<Value, LangError> {
    let v = argv.first().ok_or_else(|| LangError::new("pole needs a system", 0))?;
    match v {
        Value::Object(o) if o.class_name == "zpk" => {
            let (_, p, _) = zpk_data(o)?;
            roots_value(&p)
        }
        Value::Object(o) if o.class_name == "ss" => {
            let (a, _, _, _) = ss_data(o)?;
            let w = matcore::eigvals_c(&a).map_err(lang)?;
            Ok(cmat_or_value(CMatrix::from_col_major(w.len(), 1, w).map_err(lang)?))
        }
        _ => {
            let (_, den) = tf_parts(v)?;
            let r = roots(&row(&den))?;
            Ok(cmat_or_value(CMatrix::from_col_major(r.len(), 1, r).map_err(lang)?))
        }
    }
}

fn zero_value(argv: &[Value]) -> Result<Value, LangError> {
    let v = argv.first().ok_or_else(|| LangError::new("zero needs a system", 0))?;
    match v {
        Value::Object(o) if o.class_name == "zpk" => {
            let (z, _, _) = zpk_data(o)?;
            roots_value(&z)
        }
        _ => {
            let (num, _) = tf_parts(v)?;
            let r = roots(&row(&num))?;
            Ok(cmat_or_value(CMatrix::from_col_major(r.len(), 1, r).map_err(lang)?))
        }
    }
}

fn dcgain_value(argv: &[Value]) -> Result<Value, LangError> {
    let v = argv.first().ok_or_else(|| LangError::new("dcgain needs a system", 0))?;
    if let Value::Object(o) = v {
        if o.class_name == "zpk" {
            let (z, p, k) = zpk_data(o)?;
            if p.iter().any(|x| x.re.abs() < 1e-12 && x.im.abs() < 1e-12) {
                return Ok(Value::Num(f64::INFINITY));
            }
            let mut nz = CNum::real(1.0);
            for &x in &z {
                nz = nz * (-x);
            }
            let mut np = CNum::real(1.0);
            for &x in &p {
                np = np * (-x);
            }
            let g = nz.div(np) * CNum::real(k);
            return Ok(cmat_or_value(CMatrix::from_col_major(1, 1, vec![g]).map_err(lang)?));
        }
    }
    let (num, den) = tf_parts(v)?;
    let n = num.last().copied().unwrap_or(0.0);
    let d = den.last().copied().unwrap_or(0.0);
    if d == 0.0 {
        Ok(Value::Num(if n == 0.0 { f64::NAN } else { f64::INFINITY }))
    } else {
        Ok(Value::Num(n / d))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tf_to_ss_roundtrip_canonical() {
        let (num, den) = (vec![1.0, 2.0], vec![1.0, 3.0, 2.0]);
        let (a, b, c, d) = tf_to_ss_parts(&num, &den).unwrap();
        assert_eq!(a.shape(), (2, 2));
        assert_eq!(b.shape(), (2, 1));
        assert_eq!(c.shape(), (1, 2));
        assert_eq!(d.as_scalar(), Some(0.0));
        let (n2, d2) = ss_to_tf_parts(&a, &b, &c, &d).unwrap();
        assert_eq!(n2, num);
        assert_eq!(d2, den);
    }

    #[test]
    fn charpoly_companion() {
        let (a, _, _, _) = tf_to_ss_parts(&[1.0], &[1.0, 3.0, 2.0]).unwrap();
        assert_eq!(charpoly(&a).unwrap(), vec![1.0, 3.0, 2.0]);
    }
}
