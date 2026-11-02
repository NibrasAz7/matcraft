//! Genetic algorithm builtin (`ga`).
//!
//! Real-valued, seeded GA with tournament selection, blend crossover,
//! Gaussian mutation, elitism, box bounds, linear equality/inequality
//! constraints and an optional `nonlcon` returning `[c, ceq]`. Constraint
//! handling is penalty-based; among feasible points the objective decides.

use super::*;

#[derive(Clone)]
struct GaOptions {
    pop: usize,
    max_gen: usize,
    stall: usize,
    crossover: f64,
    elite: usize,
    initial: Option<Matrix>,
    display: bool,
    tol: f64,
}

impl GaOptions {
    fn defaults(nvars: usize) -> Self {
        GaOptions {
            pop: 50,
            max_gen: 200usize.saturating_mul(nvars.max(1)).min(2000),
            stall: 50,
            crossover: 0.8,
            elite: 2,
            initial: None,
            display: false,
            tol: 1e-6,
        }
    }
}

struct GaSpec {
    target: Value,
    nvars: usize,
    a: Matrix,
    b: Vec<f64>,
    aeq: Matrix,
    beq: Vec<f64>,
    lb: Vec<f64>,
    ub: Vec<f64>,
    nonlcon: Option<Value>,
    opts: GaOptions,
}

#[derive(Clone)]
struct Individual {
    x: Vec<f64>,
    f: f64,
    viol: f64,
    merit: f64,
}

fn opt_field(s: &StructArray, names: &[&str]) -> Option<Value> {
    for name in s.field_names() {
        if names.contains(&name.to_ascii_lowercase().as_str()) {
            return s.field_at(0, &name).cloned();
        }
    }
    None
}

fn opt_number(s: &StructArray, names: &[&str]) -> Option<f64> {
    opt_field(s, names).and_then(|v| v.to_matrix().ok()).and_then(|m| m.as_scalar())
}

fn opts_from_value(v: Option<&Value>, nvars: usize) -> GaOptions {
    let mut o = GaOptions::defaults(nvars);
    let Some(Value::Struct(s)) = v else {
        return o;
    };
    if let Some(x) = opt_number(s, &["populationsize", "popsize"]) {
        if x >= 2.0 && x.is_finite() {
            o.pop = x as usize;
        }
    }
    if let Some(x) = opt_number(s, &["maxgenerations", "generations"]) {
        if x >= 1.0 && x.is_finite() {
            o.max_gen = x as usize;
        }
    }
    if let Some(x) = opt_number(s, &["maxstallgenerations", "stallgenerations"]) {
        if x >= 1.0 && x.is_finite() {
            o.stall = x as usize;
        }
    }
    if let Some(x) = opt_number(s, &["crossoverfraction"]) {
        if (0.0..=1.0).contains(&x) {
            o.crossover = x;
        }
    }
    if let Some(x) = opt_number(s, &["elitecount"]) {
        if x >= 0.0 && x.is_finite() {
            o.elite = x as usize;
        }
    }
    if let Some(x) = opt_number(s, &["functiontolerance", "tolfun"]) {
        if x > 0.0 && x.is_finite() {
            o.tol = x;
        }
    }
    if let Some(Value::Mat(m)) = opt_field(s, &["initialpopulation", "initialpopulationmatrix"]) {
        o.initial = Some(m);
    }
    if let Some(Value::Str(d)) = opt_field(s, &["display"]) {
        o.display = !matches!(d.to_ascii_lowercase().as_str(), "off" | "none");
    }
    o
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

fn vec_or_empty(v: Option<&Value>) -> Result<Vec<f64>, LangError> {
    match v {
        None => Ok(Vec::new()),
        Some(x) => Ok(x.to_matrix()?.data().to_vec()),
    }
}

fn check_len(v: &[f64], n: usize, what: &str) -> Result<(), LangError> {
    if v.len() != n {
        return Err(LangError::new(format!("{what} must have {n} elements"), 0));
    }
    Ok(())
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
        return Ok(vec![m.data().first().copied().unwrap_or(default); n]);
    }
    if m.numel() == n {
        return Ok(m.data().to_vec());
    }
    Err(LangError::new("ga: bounds must be scalar or nvars long", 0))
}

fn check_bounds(lb: &[f64], ub: &[f64]) -> Result<(), LangError> {
    if lb.iter().zip(ub).any(|(lo, hi)| lo > hi) {
        return Err(LangError::new("ga: lower bounds must not exceed upper bounds", 0));
    }
    Ok(())
}

fn optional_target(v: Option<&Value>) -> Option<Value> {
    match v {
        Some(t @ (Value::Func(_) | Value::Str(_))) => Some(t.clone()),
        _ => None,
    }
}

fn row_dot(m: &Matrix, row: usize, x: &[f64]) -> f64 {
    (0..x.len()).map(|c| m.get(row, c).unwrap_or(0.0) * x.get(c).copied().unwrap_or(0.0)).sum()
}

impl Session {
    pub(super) fn ga_builtin(&mut self, _name: &str, argv: Vec<Value>, nout: usize) -> Result<Vec<Value>, LangError> {
        let spec = ga_spec(&argv)?;
        self.ga_run(spec, nout)
    }

    fn ga_run(&mut self, spec: GaSpec, nout: usize) -> Result<Vec<Value>, LangError> {
        let n = spec.nvars;
        let pop_size = spec.opts.pop.max(2);
        let elite = spec.opts.elite.min(pop_size.saturating_sub(1));
        let mut population: Vec<Vec<f64>> = Vec::with_capacity(pop_size);
        if let Some(init) = &spec.opts.initial {
            if init.cols() != n {
                return Err(LangError::new("ga: InitialPopulation must have nvars columns", 0));
            }
            for r in 0..init.rows().min(pop_size) {
                let mut x = Vec::with_capacity(n);
                for c in 0..n {
                    x.push(init.get(r, c).unwrap_or(0.0));
                }
                let clamped = clamp_x(&x, &spec.lb, &spec.ub);
                population.push(clamped);
            }
        }
        while population.len() < pop_size {
            let x = self.random_x(&spec.lb, &spec.ub, n);
            population.push(x);
        }
        let mut individuals: Vec<Individual> = Vec::with_capacity(pop_size);
        for x in &population {
            individuals.push(self.eval_ga(&spec, x)?);
        }
        sort_individuals(&mut individuals);
        let mut best = individuals.first().cloned().ok_or_else(|| LangError::new("ga: empty population", 0))?;
        let mut exitflag = 0.0;
        let mut generations = 0usize;
        let mut stall = 0usize;
        let mut message = "Maximum number of generations reached.".to_string();
        for gen in 0..spec.opts.max_gen {
            generations = gen.saturating_add(1);
            let mut next: Vec<Individual> = individuals.iter().take(elite).cloned().collect();
            while next.len() < pop_size {
                let p1 = tournament(&mut self.rng, &individuals);
                let p2 = tournament(&mut self.rng, &individuals);
                let mut child = crossover(&mut self.rng, p1, p2, spec.opts.crossover, &spec.lb, &spec.ub);
                mutate(&mut self.rng, &mut child, &spec.lb, &spec.ub, n);
                let child = clamp_x(&child, &spec.lb, &spec.ub);
                next.push(self.eval_ga(&spec, &child)?);
            }
            sort_individuals(&mut next);
            let gen_best = next.first().cloned().ok_or_else(|| LangError::new("ga: empty generation", 0))?;
            if gen_best.merit < best.merit - spec.opts.tol {
                best = gen_best;
                stall = 0;
            } else {
                stall = stall.saturating_add(1);
            }
            individuals = next;
            if spec.opts.display {
                self.print_buf
                    .push_str(&format!(" Generation {:>4}: best f = {:.6e}, constraint violation = {:.3e}\n", generations, best.f, best.viol));
            }
            if stall >= spec.opts.stall {
                exitflag = 1.0;
                message = "Optimization terminated: the best objective stalled.".to_string();
                break;
            }
        }
        sort_individuals(&mut individuals);
        let best = individuals.first().cloned().unwrap_or(best);
        let funccount = individuals.len().saturating_mul(generations.max(1));
        let mut out = vec![
            mat_or_num(Matrix::col_vector(best.x.clone())),
            Value::Num(best.f),
            Value::Num(exitflag),
            ga_output(generations, funccount, &message, best.viol)?,
        ];
        if nout >= 5 {
            let mut pop = Matrix::zeros(individuals.len(), n);
            for (r, ind) in individuals.iter().enumerate() {
                for (c, v) in ind.x.iter().enumerate() {
                    let _ = pop.set(r, c, *v);
                }
            }
            out.push(Value::Mat(pop));
        }
        if nout >= 6 {
            out.push(Value::Mat(Matrix::col_vector(individuals.iter().map(|i| i.f).collect())));
        }
        Ok(out)
    }

    fn random_x(&mut self, lb: &[f64], ub: &[f64], n: usize) -> Vec<f64> {
        (0..n)
            .map(|i| {
                let lo = lb.get(i).copied().unwrap_or(f64::NEG_INFINITY);
                let hi = ub.get(i).copied().unwrap_or(f64::INFINITY);
                let lo = if lo.is_finite() {
                    lo
                } else if hi.is_finite() {
                    hi - 10.0
                } else {
                    -10.0
                };
                let hi = if hi.is_finite() {
                    hi
                } else if lo.is_finite() {
                    lo + 10.0
                } else {
                    10.0
                };
                lo + self.next_f64() * (hi - lo)
            })
            .collect()
    }

    fn eval_ga(&mut self, spec: &GaSpec, x: &[f64]) -> Result<Individual, LangError> {
        let arg = Value::Mat(Matrix::col_vector(x.to_vec()));
        let outs = self.call_feval_target(&spec.target, vec![arg], 1)?;
        let f = outs
            .first()
            .ok_or_else(|| LangError::new("ga: objective returned no value", 0))?
            .to_matrix()?
            .as_scalar()
            .ok_or_else(|| LangError::new("ga: objective must return a scalar", 0))?;
        let viol = self.ga_violation(spec, x)?;
        let penalty = 1e6 * (1.0 + f.abs());
        Ok(Individual { x: x.to_vec(), f, viol, merit: f + penalty * viol })
    }

    fn ga_violation(&mut self, spec: &GaSpec, x: &[f64]) -> Result<f64, LangError> {
        let mut v = 0.0f64;
        for (r, br) in spec.b.iter().enumerate() {
            v = v.max(row_dot(&spec.a, r, x) - *br);
        }
        for (r, br) in spec.beq.iter().enumerate() {
            v = v.max((row_dot(&spec.aeq, r, x) - *br).abs());
        }
        if let Some(target) = &spec.nonlcon {
            let (c, ceq) = self.ga_nonlcon(target, x)?;
            for ci in c {
                v = v.max(ci);
            }
            for ci in ceq {
                v = v.max(ci.abs());
            }
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
        Ok(v.max(0.0))
    }

    fn ga_nonlcon(&mut self, target: &Value, x: &[f64]) -> Result<(Vec<f64>, Vec<f64>), LangError> {
        let xv = Value::Mat(Matrix::col_vector(x.to_vec()));
        let outs = self.call_feval_target(target, vec![xv], 2)?;
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
        Ok((cval.to_matrix()?.data().to_vec(), ceqval.to_matrix()?.data().to_vec()))
    }
}

fn ga_spec(argv: &[Value]) -> Result<GaSpec, LangError> {
    if let Some(Value::Struct(p)) = argv.first() {
        let target = optional_target(opt_field(p, &["fitnessfcn", "fun", "objective"]).as_ref())
            .ok_or_else(|| LangError::new("ga: problem.fitnessfcn is required", 0))?;
        let nvars_f = opt_number(p, &["nvars"]).ok_or_else(|| LangError::new("ga: problem.nvars is required", 0))?;
        if nvars_f < 1.0 || nvars_f.fract() != 0.0 {
            return Err(LangError::new("ga: problem.nvars must be a positive integer", 0));
        }
        let nvars = nvars_f as usize;
        let a_val = opt_field(p, &["aineq", "a"]);
        let a = matrix_arg(a_val.as_ref(), nvars, "A")?;
        let b = vec_or_empty(opt_field(p, &["bineq", "b"]).as_ref())?;
        check_len(&b, a.rows(), "b")?;
        let aeq = matrix_arg(opt_field(p, &["aeq"]).as_ref(), nvars, "Aeq")?;
        let beq = vec_or_empty(opt_field(p, &["beq"]).as_ref())?;
        check_len(&beq, aeq.rows(), "beq")?;
        let lb = bound_vec(opt_field(p, &["lb"]).as_ref(), nvars, f64::NEG_INFINITY)?;
        let ub = bound_vec(opt_field(p, &["ub"]).as_ref(), nvars, f64::INFINITY)?;
        check_bounds(&lb, &ub)?;
        let nonlcon = optional_target(opt_field(p, &["nonlcon"]).as_ref());
        let opts = opts_from_value(opt_field(p, &["options", "optimoptions"]).as_ref(), nvars);
        return Ok(GaSpec { target, nvars, a, b, aeq, beq, lb, ub, nonlcon, opts });
    }
    let target = optional_target(argv.first()).ok_or_else(|| LangError::new("ga: first argument must be a function handle", 0))?;
    let nvars = argv.get(1).ok_or_else(|| LangError::new("ga needs nvars", 0))?.scalar()?;
    if nvars < 1.0 || nvars.fract() != 0.0 {
        return Err(LangError::new("ga: nvars must be a positive integer", 0));
    }
    let nvars = nvars as usize;
    let a = matrix_arg(argv.get(2), nvars, "A")?;
    let b = vec_or_empty(argv.get(3))?;
    check_len(&b, a.rows(), "b")?;
    let aeq = matrix_arg(argv.get(4), nvars, "Aeq")?;
    let beq = vec_or_empty(argv.get(5))?;
    check_len(&beq, aeq.rows(), "beq")?;
    let lb = bound_vec(argv.get(6), nvars, f64::NEG_INFINITY)?;
    let ub = bound_vec(argv.get(7), nvars, f64::INFINITY)?;
    check_bounds(&lb, &ub)?;
    let nonlcon = optional_target(argv.get(8));
    let opts = opts_from_value(argv.get(9), nvars);
    Ok(GaSpec { target, nvars, a, b, aeq, beq, lb, ub, nonlcon, opts })
}

fn sort_individuals(v: &mut [Individual]) {
    v.sort_by(|a, b| a.merit.total_cmp(&b.merit));
}

fn tournament<'a>(rng: &mut u64, pop: &'a [Individual]) -> &'a Individual {
    let n = pop.len().max(1);
    let mut best = 0usize;
    for _ in 0..4 {
        *rng = rng.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        let idx = ((*rng >> 11) as usize) % n;
        if pop.get(idx).map(|i| i.merit).unwrap_or(f64::INFINITY) < pop.get(best).map(|i| i.merit).unwrap_or(f64::INFINITY) {
            best = idx;
        }
    }
    &pop[best]
}

fn crossover(rng: &mut u64, p1: &Individual, p2: &Individual, frac: f64, lb: &[f64], ub: &[f64]) -> Vec<f64> {
    let mut child = Vec::with_capacity(p1.x.len());
    for i in 0..p1.x.len() {
        *rng = rng.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        let u = ((*rng >> 11) as f64) / ((1u64 << 53) as f64);
        let v = if u < frac {
            *rng = rng.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            let a = ((*rng >> 11) as f64) / ((1u64 << 53) as f64);
            let alpha = -0.5 + 2.0 * a;
            alpha * p1.x.get(i).copied().unwrap_or(0.0) + (1.0 - alpha) * p2.x.get(i).copied().unwrap_or(0.0)
        } else {
            p1.x.get(i).copied().unwrap_or(0.0)
        };
        let lo = lb.get(i).copied().unwrap_or(f64::NEG_INFINITY);
        let hi = ub.get(i).copied().unwrap_or(f64::INFINITY);
        child.push(v.clamp(lo, hi));
    }
    child
}

fn mutate(rng: &mut u64, x: &mut [f64], lb: &[f64], ub: &[f64], n: usize) {
    let rate = 1.0 / n.max(1) as f64;
    for (i, slot) in x.iter_mut().enumerate() {
        *rng = rng.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        let u = ((*rng >> 11) as f64) / ((1u64 << 53) as f64);
        if u < rate {
            let lo = lb.get(i).copied().unwrap_or(f64::NEG_INFINITY);
            let hi = ub.get(i).copied().unwrap_or(f64::INFINITY);
            let sigma = if lo.is_finite() && hi.is_finite() { 0.1 * (hi - lo) } else { 0.1 * (1.0 + slot.abs()) };
            *rng = rng.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            let u1 = (((*rng >> 11) as f64) / ((1u64 << 53) as f64)).max(1e-300);
            *rng = rng.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            let u2 = ((*rng >> 11) as f64) / ((1u64 << 53) as f64);
            let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
            *slot = (*slot + sigma * z).clamp(lo, hi);
        }
    }
}

fn clamp_x(x: &[f64], lb: &[f64], ub: &[f64]) -> Vec<f64> {
    x.iter().enumerate().map(|(i, v)| v.clamp(lb.get(i).copied().unwrap_or(f64::NEG_INFINITY), ub.get(i).copied().unwrap_or(f64::INFINITY))).collect()
}

fn ga_output(generations: usize, funccount: usize, message: &str, maxconstraint: f64) -> Result<Value, LangError> {
    let mut s = StructArray::new();
    s.set_field(0, "generations", Value::Num(generations as f64))?;
    s.set_field(0, "funccount", Value::Num(funccount as f64))?;
    s.set_field(0, "message", Value::Str(message.to_string()))?;
    s.set_field(0, "maxconstraint", Value::Num(maxconstraint))?;
    Ok(Value::Struct(s))
}
