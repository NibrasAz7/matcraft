//! External LP/QP/MILP solver bridges.
//!
//! Models are written in the CPLEX LP format (linear terms, quadratic
//! objective brackets, bounds, `Generals`/`Binaries`) and solved by a
//! command-line executable. The Rust core only spawns the process and
//! parses its solution file, so no solver SDK or `unsafe` code is needed.
//!
//! Discovery order for `Solver = 'auto'`: HiGHS, GLPK, Gurobi, CPLEX.
//! `MATCRAFT_HIGHS`, `MATCRAFT_GLPK`, `MATCRAFT_GUROBI` and
//! `MATCRAFT_CPLEX` override the executable path; otherwise the binary is
//! looked up on `PATH`.
//!
//! Solution readers understand GLPK `-o` reports, HiGHS `--solution_file`
//! reports, Gurobi `ResultFile` solution files and CPLEX XML `.sol` files.

use super::*;
#[cfg(not(target_arch = "wasm32"))]
use std::collections::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::process::{Command, Stdio};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(not(target_arch = "wasm32"))]
use std::time::{Duration, Instant};

/// Cap on captured stdout/stderr from a solver process.
#[cfg(not(target_arch = "wasm32"))]
const MAX_OUTPUT: usize = 4 << 20;
/// Default wall-clock limit when the caller gives none.
const DEFAULT_TIME_LIMIT: f64 = 300.0;
/// Safety cap for the built-in branch-and-bound node count.
const MAX_BB_NODES: usize = 200_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProblemKind {
    Linear,
    Quadratic,
    Milp,
}

#[cfg(not(target_arch = "wasm32"))]
static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

#[cfg(not(target_arch = "wasm32"))]
struct TempDir(PathBuf);

#[cfg(not(target_arch = "wasm32"))]
impl TempDir {
    fn new() -> Result<Self, LangError> {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("matcraft_solver_{}_{}", std::process::id(), seq));
        std::fs::create_dir_all(&dir).map_err(|e| LangError::new(format!("solver: cannot create temporary directory: {e}"), 0))?;
        Ok(TempDir(dir))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct RunOutput {
    status: Option<i32>,
    stdout: String,
    stderr: String,
    timed_out: bool,
}

/// Run a program with a wall-clock limit, capturing bounded stdout/stderr.
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::disallowed_methods)]
fn run_bounded(exe: &Path, args: &[String], timeout: Duration, work: &Path) -> Result<RunOutput, LangError> {
    let out_path = work.join("stdout.txt");
    let err_path = work.join("stderr.txt");
    let out_file = std::fs::File::create(&out_path).map_err(|e| LangError::new(format!("solver: cannot create output capture: {e}"), 0))?;
    let err_file = std::fs::File::create(&err_path).map_err(|e| LangError::new(format!("solver: cannot create error capture: {e}"), 0))?;
    let mut cmd = Command::new(exe);
    cmd.current_dir(work).args(args).stdin(Stdio::null()).stdout(Stdio::from(out_file)).stderr(Stdio::from(err_file));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().map_err(|e| LangError::new(format!("solver: cannot start '{}': {e}", exe.display()), 0))?;
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    let stdout = read_capped(&out_path);
                    let stderr = read_capped(&err_path);
                    return Ok(RunOutput { status: None, stdout, stderr, timed_out: true });
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(LangError::new(format!("solver: cannot wait for '{}': {e}", exe.display()), 0)),
        }
    };
    Ok(RunOutput { status: status.code(), stdout: read_capped(&out_path), stderr: read_capped(&err_path), timed_out: false })
}

#[cfg(not(target_arch = "wasm32"))]
fn read_capped(path: &Path) -> String {
    let bytes = std::fs::read(path).unwrap_or_default();
    let end = bytes.len().min(MAX_OUTPUT);
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

#[cfg(not(target_arch = "wasm32"))]
fn solver_binary(name: &str) -> &'static str {
    match name {
        "glpk" => "glpsol",
        "highs" => "highs",
        "gurobi" => "gurobi_cl",
        "cplex" => "cplex",
        _ => "",
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn find_on_path(binary: &str) -> Option<PathBuf> {
    let direct = PathBuf::from(binary);
    if direct.components().count() > 1 && direct.is_file() {
        return Some(direct);
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for cand in [binary.to_string(), format!("{binary}.exe")] {
            let p = dir.join(cand);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Resolve a solver executable from the environment override or `PATH`.
#[cfg(not(target_arch = "wasm32"))]
fn solver_exe(name: &str) -> Option<PathBuf> {
    let key = format!("MATCRAFT_{}", name.to_ascii_uppercase());
    if let Ok(v) = std::env::var(&key) {
        let v = v.trim();
        if !v.is_empty() {
            let p = PathBuf::from(v);
            if p.components().count() > 1 && !p.is_file() {
                return None;
            }
            return Some(p);
        }
    }
    find_on_path(solver_binary(name))
}

#[cfg(not(target_arch = "wasm32"))]
fn auto_candidates(kind: ProblemKind) -> &'static [&'static str] {
    match kind {
        ProblemKind::Quadratic => &["highs", "gurobi", "cplex"],
        _ => &["highs", "glpk", "gurobi", "cplex"],
    }
}

/// Solve a model with an external solver (or the internal solver when
/// `Solver = 'auto'` finds nothing installed).
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn solve(spec: &QpSpec, ints: &[usize], kind: ProblemKind, caller: &str) -> Result<QpResult, LangError> {
    let chosen = match &spec.opts.solver {
        SolverChoice::Named(n) => n.clone(),
        SolverChoice::Auto => {
            let mut found = None;
            for cand in auto_candidates(kind) {
                if solver_exe(cand).is_some() {
                    found = Some((*cand).to_string());
                    break;
                }
            }
            match found {
                Some(n) => n,
                None => return super::solve_qp(spec.clone()),
            }
        }
        SolverChoice::Internal => return super::solve_qp(spec.clone()),
    };
    if kind == ProblemKind::Quadratic && chosen == "glpk" {
        return Err(LangError::new(format!("{caller}: GLPK solves linear and integer programs, not quadratic objectives"), 0));
    }
    let exe = solver_exe(&chosen).ok_or_else(|| {
        LangError::new(
            format!(
                "{caller}: external solver '{chosen}' was not found. Install {} or set MATCRAFT_{} to its executable.",
                solver_binary(&chosen),
                chosen.to_ascii_uppercase()
            ),
            0,
        )
    })?;
    let work = TempDir::new()?;
    let model_path = work.0.join("model.lp");
    let sol_path = work.0.join("solution.sol");
    std::fs::write(&model_path, lp_text(spec, kind, ints)).map_err(|e| LangError::new(format!("{caller}: cannot write the LP model: {e}"), 0))?;
    let secs = spec.opts.time_limit.unwrap_or(DEFAULT_TIME_LIMIT).max(0.1);
    let gap = spec.opts.mip_gap;
    let (args, kind_of_solution) = match chosen.as_str() {
        "glpk" => {
            let mut a = vec!["--lp".into(), model_path.display().to_string(), "-o".into(), sol_path.display().to_string()];
            if spec.opts.time_limit.is_some() {
                a.push("--tmlim".into());
                a.push(format!("{}", secs.ceil() as u64));
            }
            (a, "glpk")
        }
        "highs" => {
            let a = vec![
                model_path.display().to_string(),
                "--solution_file".into(),
                sol_path.display().to_string(),
                "--time_limit".into(),
                format!("{secs}"),
            ];
            (a, "highs")
        }
        "gurobi" => {
            let mut a = vec![format!("ResultFile={}", sol_path.display()), format!("TimeLimit={secs}")];
            if kind == ProblemKind::Milp {
                a.push(format!("MIPGap={gap}"));
            }
            a.push(model_path.display().to_string());
            (a, "gurobi")
        }
        "cplex" => {
            let mut a = vec!["-c".into(), format!("read \"{}\"", model_path.display())];
            if spec.opts.time_limit.is_some() {
                a.push(format!("set timelimit {secs}"));
            }
            if kind == ProblemKind::Milp {
                a.push(format!("set mip tolerances mipgap {gap}"));
            }
            a.push("optimize".into());
            a.push(format!("write \"{}\"", sol_path.display()));
            a.push("quit".into());
            (a, "cplex")
        }
        other => return Err(LangError::new(format!("{caller}: unsupported external solver '{other}'"), 0)),
    };
    let run = run_bounded(&exe, &args, Duration::from_secs_f64(secs + 10.0), &work.0)?;
    if run.timed_out {
        return Err(LangError::new(format!("{caller}: '{chosen}' exceeded the {} second time limit", secs + 10.0), 0));
    }
    let solution_text = std::fs::read_to_string(&sol_path).unwrap_or_default();
    let parsed = parse_solution(kind_of_solution, &solution_text, &run.stdout);
    let n = spec.f.len();
    let (mut x, status) = match parsed {
        Ok(v) => v,
        Err(parse_err) => {
            let detail = if run.stderr.trim().is_empty() { run.stdout.trim() } else { run.stderr.trim() };
            let detail = detail.chars().take(600).collect::<String>();
            return Err(LangError::new(
                format!("{caller}: '{chosen}' did not produce a usable solution ({}; exit {:?}). {detail}", parse_err, run.status),
                0,
            ));
        }
    };
    if x.len() < n {
        x.resize(n, 0.0);
    }
    x.truncate(n);
    for &i in ints {
        if let Some(slot) = x.get_mut(i) {
            *slot = slot.round();
            let lo = spec.lb.get(i).copied().unwrap_or(f64::NEG_INFINITY);
            let hi = spec.ub.get(i).copied().unwrap_or(f64::INFINITY);
            *slot = slot.clamp(lo, hi);
        }
    }
    let fval = objective(spec, &x);
    let cv = constraint_violation(spec, &x);
    let lower = status.to_ascii_lowercase();
    let exitflag = if lower.contains("optimal") {
        1.0
    } else if lower.contains("infeasible") {
        -2.0
    } else if lower.contains("unbounded") {
        -3.0
    } else {
        0.0
    };
    let message = format!("Solved with {chosen} (status: {status}).");
    Ok(QpResult { x, fval, lambda: super::qp_lambda(0, n, 0, &[], &[], &[], &[])?, iters: 0, exitflag, message, constrviolation: cv })
}

#[cfg(not(target_arch = "wasm32"))]
fn objective(spec: &QpSpec, x: &[f64]) -> f64 {
    let n = spec.f.len();
    let mut f = 0.0;
    for i in 0..n {
        f += spec.f.get(i).copied().unwrap_or(0.0) * x.get(i).copied().unwrap_or(0.0);
    }
    let mut q = 0.0;
    for i in 0..n {
        for j in 0..n {
            q += spec.h.get(i, j).unwrap_or(0.0) * x.get(i).copied().unwrap_or(0.0) * x.get(j).copied().unwrap_or(0.0);
        }
    }
    f + 0.5 * q
}

fn constraint_violation(spec: &QpSpec, x: &[f64]) -> f64 {
    let n = spec.f.len();
    let mut cv = 0.0f64;
    for r in 0..spec.a.rows() {
        let mut s = 0.0;
        for c in 0..n {
            s += spec.a.get(r, c).unwrap_or(0.0) * x.get(c).copied().unwrap_or(0.0);
        }
        cv = cv.max(s - spec.b.get(r).copied().unwrap_or(0.0));
    }
    for r in 0..spec.aeq.rows() {
        let mut s = 0.0;
        for c in 0..n {
            s += spec.aeq.get(r, c).unwrap_or(0.0) * x.get(c).copied().unwrap_or(0.0);
        }
        cv = cv.max((s - spec.beq.get(r).copied().unwrap_or(0.0)).abs());
    }
    for i in 0..n {
        let xi = x.get(i).copied().unwrap_or(0.0);
        cv = cv.max(spec.lb.get(i).copied().unwrap_or(f64::NEG_INFINITY) - xi);
        cv = cv.max(xi - spec.ub.get(i).copied().unwrap_or(f64::INFINITY));
    }
    cv.max(0.0)
}

#[cfg(not(target_arch = "wasm32"))]
fn fmt_num(v: f64) -> String {
    if v == 0.0 {
        "0".into()
    } else if v.is_finite() {
        format!("{v:.15e}")
    } else {
        "0".into()
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn var(i: usize) -> String {
    format!("x{}", i + 1)
}

#[cfg(not(target_arch = "wasm32"))]
fn expression(terms: Vec<(f64, String)>) -> String {
    let mut out = String::new();
    for (c, t) in terms {
        if c == 0.0 || !c.is_finite() {
            continue;
        }
        if out.is_empty() {
            if c < 0.0 {
                out.push('-');
            }
            out.push_str(&fmt_num(c.abs()));
            out.push(' ');
            out.push_str(&t);
        } else {
            out.push_str(if c < 0.0 { " - " } else { " + " });
            out.push_str(&fmt_num(c.abs()));
            out.push(' ');
            out.push_str(&t);
        }
    }
    if out.is_empty() {
        "0".into()
    } else {
        out
    }
}

/// Render the model in CPLEX LP format, the common input understood by
/// GLPK, HiGHS, Gurobi and CPLEX.
#[cfg(not(target_arch = "wasm32"))]
fn lp_text(spec: &QpSpec, kind: ProblemKind, ints: &[usize]) -> String {
    let n = spec.f.len();
    let mut s = String::new();
    s.push_str("Minimize\n obj: ");
    let linear: Vec<(f64, String)> = (0..n).map(|i| (spec.f.get(i).copied().unwrap_or(0.0), var(i))).collect();
    let linear_text = expression(linear);
    let mut quadratic_text = String::new();
    if kind == ProblemKind::Quadratic {
        let mut q: Vec<(f64, String)> = Vec::new();
        for i in 0..n {
            let hii = spec.h.get(i, i).unwrap_or(0.0);
            if hii != 0.0 {
                q.push((hii, format!("{} ^ 2", var(i))));
            }
            for j in (i + 1)..n {
                let hij = spec.h.get(i, j).unwrap_or(0.0);
                if hij != 0.0 {
                    // LP quadratic expressions are halved as written, and a
                    // single product term represents both symmetric entries,
                    // so the bracket carries 2*H(i,j).
                    q.push((2.0 * hij, format!("{} * {}", var(i), var(j))));
                }
            }
        }
        if !q.is_empty() {
            quadratic_text = format!("[ {} ] / 2", expression(q));
        }
    }
    if quadratic_text.is_empty() {
        s.push_str(&linear_text);
    } else if linear_text == "0" {
        s.push_str(&quadratic_text);
    } else {
        s.push_str(&format!("{linear_text} + {quadratic_text}"));
    }
    s.push_str("\nSubject To\n");
    for r in 0..spec.a.rows() {
        let terms: Vec<(f64, String)> = (0..n).map(|c| (spec.a.get(r, c).unwrap_or(0.0), var(c))).collect();
        s.push_str(&format!(" c{}: {} <= {}\n", r + 1, expression(terms), fmt_num(spec.b.get(r).copied().unwrap_or(0.0))));
    }
    for r in 0..spec.aeq.rows() {
        let terms: Vec<(f64, String)> = (0..n).map(|c| (spec.aeq.get(r, c).unwrap_or(0.0), var(c))).collect();
        s.push_str(&format!(" e{}: {} = {}\n", r + 1, expression(terms), fmt_num(spec.beq.get(r).copied().unwrap_or(0.0))));
    }
    s.push_str("Bounds\n");
    for i in 0..n {
        let lb = spec.lb.get(i).copied().unwrap_or(f64::NEG_INFINITY);
        let ub = spec.ub.get(i).copied().unwrap_or(f64::INFINITY);
        let lb = if lb.is_finite() { lb } else { f64::NEG_INFINITY };
        let ub = if ub.is_finite() { ub } else { f64::INFINITY };
        match (lb.is_finite(), ub.is_finite()) {
            (true, true) => {
                if lb == ub {
                    s.push_str(&format!(" {} = {}\n", var(i), fmt_num(lb)));
                } else {
                    s.push_str(&format!(" {} <= {} <= {}\n", fmt_num(lb), var(i), fmt_num(ub)));
                }
            }
            (true, false) => s.push_str(&format!(" {} >= {}\n", var(i), fmt_num(lb))),
            (false, true) => s.push_str(&format!(" -1.000000000000000e30 <= {} <= {}\n", var(i), fmt_num(ub))),
            (false, false) => s.push_str(&format!(" {} free\n", var(i))),
        }
    }
    if kind == ProblemKind::Milp && !ints.is_empty() {
        let mut generals = Vec::new();
        let mut binaries = Vec::new();
        for &i in ints {
            let lb = spec.lb.get(i).copied().unwrap_or(0.0);
            let ub = spec.ub.get(i).copied().unwrap_or(f64::INFINITY);
            if lb >= -1e-9 && ub <= 1.0 + 1e-9 {
                binaries.push(var(i));
            } else {
                generals.push(var(i));
            }
        }
        if !binaries.is_empty() {
            s.push_str("Binaries\n");
            for v in binaries {
                s.push_str(&format!(" {v}\n"));
            }
        }
        if !generals.is_empty() {
            s.push_str("Generals\n");
            for v in generals {
                s.push_str(&format!(" {v}\n"));
            }
        }
    }
    s.push_str("End\n");
    s
}

#[cfg(target_arch = "wasm32")]
pub(super) fn solve(spec: &QpSpec, _ints: &[usize], _kind: ProblemKind, caller: &str) -> Result<QpResult, LangError> {
    if spec.opts.solver == SolverChoice::Auto {
        return super::solve_qp(spec.clone());
    }
    Err(LangError::new(format!("{caller}: external solvers are not available in the web build"), 0))
}

/// Parse a solver's solution report. Returns variable values in `x1..xn`
/// order only when the names are contiguous; otherwise a name map is
/// resolved by the caller. All supported formats use `x1`-style names.
#[cfg(not(target_arch = "wasm32"))]
fn parse_solution(format: &str, solution_text: &str, stdout: &str) -> Result<(Vec<f64>, String), String> {
    match format {
        "glpk" => parse_glpk(solution_text),
        "highs" => parse_highs(solution_text),
        "gurobi" => parse_gurobi(solution_text),
        "cplex" => parse_cplex(solution_text),
        _ => {
            let _ = stdout;
            Err("unknown solution format".into())
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn values_to_vec(map: &HashMap<String, f64>, status: &str) -> Result<(Vec<f64>, String), String> {
    if map.is_empty() {
        return Err(format!("no variable values (status: {status})"));
    }
    let mut n = 0usize;
    for name in map.keys() {
        if let Some(rest) = name.strip_prefix('x') {
            if let Ok(i) = rest.parse::<usize>() {
                if i > n {
                    n = i;
                }
            }
        }
    }
    if n == 0 {
        return Err("solution has no x1..xn variables".into());
    }
    let mut x = vec![0.0; n];
    for (name, value) in map {
        if let Some(rest) = name.strip_prefix('x') {
            if let Ok(i) = rest.parse::<usize>() {
                if i >= 1 {
                    if let Some(slot) = x.get_mut(i - 1) {
                        *slot = *value;
                    }
                }
            }
        }
    }
    Ok((x, status.to_string()))
}

#[cfg(not(target_arch = "wasm32"))]
fn parse_glpk(text: &str) -> Result<(Vec<f64>, String), String> {
    let mut status = "unknown".to_string();
    for line in text.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("Status:") {
            status = rest.trim().to_string();
            break;
        }
    }
    let mut map = HashMap::new();
    let mut in_cols = false;
    for line in text.lines() {
        if line.contains("Column name") {
            in_cols = true;
            continue;
        }
        if !in_cols {
            continue;
        }
        if line.trim().is_empty() {
            if !map.is_empty() {
                break;
            }
            continue;
        }
        if line.starts_with("---") || line.contains("Karush") {
            continue;
        }
        let toks: Vec<&str> = line.split_whitespace().collect();
        if toks.len() < 4 {
            continue;
        }
        // No., name, status, activity, [lower], [upper], [marginal]
        if let (Ok(_idx), Some(name), Some(activity)) = (toks[0].parse::<usize>(), toks.get(1), toks.get(3)) {
            if let Ok(v) = activity.parse::<f64>() {
                map.insert((*name).to_string(), v);
            }
        }
    }
    if status.eq_ignore_ascii_case("undefined") {
        return Err(format!("status {status}"));
    }
    values_to_vec(&map, &status)
}

#[cfg(not(target_arch = "wasm32"))]
fn parse_highs(text: &str) -> Result<(Vec<f64>, String), String> {
    let mut status = "unknown".to_string();
    let lines: Vec<&str> = text.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        if !t.to_ascii_lowercase().starts_with("model status") {
            continue;
        }
        if let Some((_, rhs)) = t.split_once(':') {
            if !rhs.trim().is_empty() {
                status = rhs.trim().to_string();
            } else if let Some(next) = lines.get(i + 1) {
                status = next.trim().to_string();
            }
        } else if let Some(next) = lines.get(i + 1) {
            status = next.trim().to_string();
        }
    }
    let mut map = HashMap::new();
    let mut in_cols = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('#') {
            if t.to_ascii_lowercase().starts_with("# columns") {
                in_cols = true;
            } else if in_cols {
                break;
            }
            continue;
        }
        if !in_cols || t.is_empty() {
            continue;
        }
        let toks: Vec<&str> = t.split_whitespace().collect();
        if toks.len() >= 2 {
            if let Ok(v) = toks[1].parse::<f64>() {
                map.insert(toks[0].to_string(), v);
            }
        }
    }
    values_to_vec(&map, &status)
}

#[cfg(not(target_arch = "wasm32"))]
fn parse_gurobi(text: &str) -> Result<(Vec<f64>, String), String> {
    let mut status = "optimal".to_string();
    let mut map = HashMap::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if t.starts_with('#') {
            let lower = t.to_ascii_lowercase();
            if lower.contains("status") {
                if let Some((_, rhs)) = t.split_once(':') {
                    status = rhs.trim().to_string();
                }
            }
            continue;
        }
        let toks: Vec<&str> = t.split_whitespace().collect();
        if toks.len() >= 2 {
            if let Ok(v) = toks[1].parse::<f64>() {
                map.insert(toks[0].to_string(), v);
            }
        }
    }
    values_to_vec(&map, &status)
}

#[cfg(not(target_arch = "wasm32"))]
fn xml_attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let pat = format!("{name}=\"");
    let start = tag.find(&pat)? + pat.len();
    let rest = tag.get(start..)?;
    let end = rest.find('"')?;
    rest.get(..end)
}

#[cfg(not(target_arch = "wasm32"))]
fn parse_cplex(text: &str) -> Result<(Vec<f64>, String), String> {
    let mut status = "optimal".to_string();
    let mut map = HashMap::new();
    let mut rest = text;
    while let Some(pos) = rest.find("<variable ") {
        let tail = &rest[pos..];
        let end = tail.find('>').map(|e| e + 1).unwrap_or(tail.len());
        let tag = &tail[..end];
        if let (Some(name), Some(value)) = (xml_attr(tag, "name"), xml_attr(tag, "value")) {
            if let Ok(v) = value.parse::<f64>() {
                map.insert(name.to_string(), v);
            }
        }
        rest = &tail[end..];
    }
    if on_off(text, "infeasible") {
        status = "infeasible".into();
    } else if on_off(text, "unbounded") {
        status = "unbounded".into();
    }
    values_to_vec(&map, &status)
}

#[cfg(not(target_arch = "wasm32"))]
fn on_off(text: &str, needle: &str) -> bool {
    text.to_ascii_lowercase().contains(needle)
}

/// Built-in branch-and-bound for `intlinprog`: depth-first branching on
/// the LP relaxation. Bounded by the options' iteration cap, a node cap
/// and an optional wall-clock limit.
#[allow(clippy::disallowed_methods)] // native-only clock use is gated below
pub(super) fn branch_and_bound(spec: &QpSpec, ints: &[usize]) -> Result<QpResult, LangError> {
    let n = spec.f.len();
    #[cfg(not(target_arch = "wasm32"))]
    let started = Instant::now();
    let limit = spec.opts.time_limit.unwrap_or(DEFAULT_TIME_LIMIT);
    #[cfg(target_arch = "wasm32")]
    let _ = limit;
    let max_nodes = spec.opts.max_iter.clamp(16, MAX_BB_NODES);
    let mut stack: Vec<(Vec<f64>, Vec<f64>)> = vec![(spec.lb.clone(), spec.ub.clone())];
    let mut best: Option<(Vec<f64>, f64)> = None;
    let mut nodes = 0usize;
    let mut complete = true;
    while let Some((lb, ub)) = stack.pop() {
        #[cfg(not(target_arch = "wasm32"))]
        let expired = started.elapsed().as_secs_f64() > limit;
        #[cfg(target_arch = "wasm32")]
        let expired = false;
        if nodes >= max_nodes || expired {
            complete = false;
            break;
        }
        let mut node = spec.clone();
        node.lb = lb.clone();
        node.ub = ub.clone();
        let relaxation = match super::solve_qp(node) {
            Ok(r) => r,
            Err(_) => continue,
        };
        if relaxation.exitflag != 1.0 {
            continue;
        }
        nodes = nodes.saturating_add(1);
        let branch = ints.iter().copied().max_by(|&a, &b| {
            let fa = (relaxation.x.get(a).copied().unwrap_or(0.0) - relaxation.x.get(a).copied().unwrap_or(0.0).round()).abs();
            let fb = (relaxation.x.get(b).copied().unwrap_or(0.0) - relaxation.x.get(b).copied().unwrap_or(0.0).round()).abs();
            fa.total_cmp(&fb)
        });
        let fractional = branch.map(|i| {
            let xi = relaxation.x.get(i).copied().unwrap_or(0.0);
            (xi - xi.round()).abs()
        });
        if fractional.is_some_and(|f| f <= 1e-6) {
            if best.as_ref().is_none_or(|(_, f)| relaxation.fval < *f) {
                best = Some((relaxation.x, relaxation.fval));
            }
            continue;
        }
        if let Some((_, bf)) = &best {
            if relaxation.fval >= *bf - spec.opts.mip_gap * (1.0 + bf.abs()) {
                continue;
            }
        }
        let Some(i) = branch else { continue };
        let xi = relaxation.x.get(i).copied().unwrap_or(0.0);
        let mut ub_down = ub.clone();
        if let Some(slot) = ub_down.get_mut(i) {
            *slot = slot.min(xi.floor());
        }
        let mut lb_up = lb.clone();
        if let Some(slot) = lb_up.get_mut(i) {
            *slot = slot.max(xi.ceil());
        }
        stack.push((lb, ub_down));
        stack.push((lb_up, ub));
    }
    let message = if best.is_some() {
        if complete {
            format!("intlinprog: branch-and-bound proved optimality after {nodes} nodes.")
        } else {
            format!("intlinprog: stopped after {nodes} nodes with the best incumbent.")
        }
    } else {
        format!("intlinprog: no integer-feasible point found after {nodes} nodes.")
    };
    match best {
        Some((x, fval)) => Ok(QpResult {
            constrviolation: constraint_violation(spec, &x),
            x,
            fval,
            lambda: super::qp_lambda(0, n, 0, &[], &[], &[], &[])?,
            iters: nodes,
            exitflag: if complete { 1.0 } else { 0.0 },
            message,
        }),
        None => Ok(QpResult {
            x: Vec::new(),
            fval: f64::INFINITY,
            lambda: super::qp_lambda(0, n, 0, &[], &[], &[], &[])?,
            iters: nodes,
            exitflag: -2.0,
            message,
            constrviolation: 0.0,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lp_spec() -> QpSpec {
        let mut a = Matrix::zeros(1, 2);
        let _ = a.set(0, 0, 1.0);
        let _ = a.set(0, 1, 1.0);
        QpSpec {
            h: Matrix::zeros(2, 2),
            f: vec![1.0, 2.0],
            a,
            b: vec![4.0],
            aeq: Matrix::zeros(0, 2),
            beq: Vec::new(),
            lb: vec![0.0, f64::NEG_INFINITY],
            ub: vec![f64::INFINITY, 5.0],
            x0: None,
            opts: Opts::for_problem(2),
        }
    }

    #[test]
    fn lp_writer_has_sections_and_bounds() {
        let text = lp_text(&lp_spec(), ProblemKind::Linear, &[]);
        assert!(text.starts_with("Minimize\n"));
        assert!(text.contains("obj: 1.000000000000000e0 x1 + 2.000000000000000e0 x2"));
        assert!(text.contains("c1: 1.000000000000000e0 x1 + 1.000000000000000e0 x2 <= 4"));
        assert!(text.contains(" x1 >= 0"));
        assert!(text.contains("-1.000000000000000e30 <= x2 <= 5.000000000000000e0"));
        assert!(text.trim_end().ends_with("End"));
    }

    #[test]
    fn quadratic_writer_uses_bracket_form() {
        let mut spec = lp_spec();
        let mut h = Matrix::zeros(2, 2);
        let _ = h.set(0, 0, 2.0);
        let _ = h.set(0, 1, 3.0);
        let _ = h.set(1, 1, 4.0);
        spec.h = h;
        let text = lp_text(&spec, ProblemKind::Quadratic, &[]);
        assert!(text.contains("[ 2.000000000000000e0 x1 ^ 2 + 6.000000000000000e0 x1 * x2 + 4.000000000000000e0 x2 ^ 2 ] / 2"));
    }

    #[test]
    fn milp_writer_marks_binary_and_general_variables() {
        let mut spec = lp_spec();
        spec.lb = vec![0.0, 0.0];
        spec.ub = vec![1.0, f64::INFINITY];
        let text = lp_text(&spec, ProblemKind::Milp, &[0, 1]);
        assert!(text.contains("Binaries\n x1\n"));
        assert!(text.contains("Generals\n x2\n"));
    }

    #[test]
    fn glpk_solution_parser() {
        let text = "\
Problem:    m
Rows:       1
Columns:    2
Non-zeros:  2
Status:     OPTIMAL
Objective:  obj = 3 (MINimum)

   No.   Row name   St   Activity     Lower bound   Upper bound    Marginal
------ ------------ -- ------------- ------------- ------------- -------------
     1 c1           NL             4             4

   No. Column name  St   Activity     Lower bound   Upper bound    Marginal
------ ------------ -- ------------- ------------- ------------- -------------
     1 x1           B              1             0
     2 x2           B              2             0
";
        let (x, status) = parse_glpk(text).unwrap();
        assert_eq!(x, vec![1.0, 2.0]);
        assert_eq!(status, "OPTIMAL");
    }

    #[test]
    fn highs_solution_parser() {
        let text = "\
Model status
Optimal

# Primal solution values
Feasible
Objective 3
# Columns 2
x1 1
x2 2
# Rows 1
c1 4
";
        let (x, status) = parse_highs(text).unwrap();
        assert_eq!(x, vec![1.0, 2.0]);
        assert!(status.eq_ignore_ascii_case("optimal"));
    }

    #[test]
    fn gurobi_solution_parser() {
        let text = "# Objective value = 3\nx1 1\nx2 2\n";
        let (x, status) = parse_gurobi(text).unwrap();
        assert_eq!(x, vec![1.0, 2.0]);
        assert_eq!(status, "optimal");
    }

    #[test]
    fn cplex_solution_parser() {
        let text = r#"<?xml version = "1.0" encoding="UTF-8" standalone="yes"?>
<CPLEXSolution version="1.2">
 <header problemName="model.lp" objectiveValue="3"/>
 <variables>
  <variable name="x1" value="1"/>
  <variable name="x2" value="2"/>
 </variables>
</CPLEXSolution>"#;
        let (x, status) = parse_cplex(text).unwrap();
        assert_eq!(x, vec![1.0, 2.0]);
        assert_eq!(status, "optimal");
    }

    #[test]
    fn incomplete_solution_is_rejected() {
        assert!(parse_highs("Model status : Infeasible\n").is_err());
    }
}
