//! The command registry. Each command: stable id, label, params doc, and a
//! `run` over the session. Programmatic calls never block or prompt.

use std::fmt;

use serde_json::{json, Value as Json};

use matcore::Matrix;
use matlang::Value;

use crate::{EngineError, Session};

/// A registered command.
pub struct CommandSpec {
    /// Stable id, `area.verb` style: `eval.line`, `workspace.clear`.
    pub id: &'static str,
    /// One-line description.
    pub label: &'static str,
    /// JSON params documentation (for `engine.list` and MCP tool schemas).
    pub params: &'static str,
    pub run: fn(&mut Session, &Json) -> Result<Json, EngineError>,
}

/// Every command, in stable order.
pub fn command_specs() -> &'static [CommandSpec] {
    COMMANDS
}

pub fn find_command(id: &str) -> Option<&'static CommandSpec> {
    COMMANDS.iter().find(|c| c.id == id)
}

/// Builtin-library groups — shared by `functions.list` and `matcraft-cli
/// functions` so both print the same catalogue. A test in this module keeps
/// it in lock-step with `matlang::BUILTIN_NAMES`.
pub const FUNCTION_GROUPS: &[(&str, &str)] = &[
    ("construction", "zeros ones eye magic rand randn randi randperm linspace logspace inf Inf nan NaN eps"),
    ("size/shape", "size numel length ndims reshape squeeze transpose ctranspose repmat"),
    ("matrix ops", "diag triu tril flipud fliplr kron cat horzcat vertcat circshift rot90 meshgrid"),
    ("cells", "cell iscell iscellstr cell2mat num2cell celldisp"),
    ("structs", "struct isstruct fieldnames isfield rmfield orderfields getfield setfield deal"),
    ("sparse", "sparse full issparse nnz nzmax nonzeros speye spalloc spones"),
    ("linear algebra", "det inv norm dot cross eig svd qr lu chol rref pinv rank cond trace null orth"),
    ("reductions", "sum mean prod min max std var median any all"),
    ("running/sorting", "cumsum cumprod diff sort unique find"),
    ("elementwise", "abs sqrt exp log log10 log2 expm1 log1p sin cos tan asin acos atan atan2 sinh cosh tanh sech csch coth floor ceil round fix sign mod rem hypot nthroot nextpow2 pow2 deg2rad rad2deg db2pow pow2db uint8"),
    ("types/predicates", "isnan isinf isfinite isempty isscalar isvector isrow iscolumn ismatrix isnumeric ischar islogical isreal isa isequal isequaln"),
    ("complex", "complex real imag conj angle"),
    ("poly/signal", "polyval polyfit roots poly conv deconv interp1 trapz histcounts"),
    ("dsp", "fft ifft fft2 ifft2 fftshift ifftshift dftmtx filter xcorr freqz unwrap hamming hann blackman bartlett rectwin triang kaiser gausswin sinc rectpuls tripuls"),
    ("solvers", "fzero fminsearch integral quad"),
    ("function handles", "feval func2str str2func functions arrayfun cellfun"),
    ("debugging", "dbstop dbclear dbcont dbstatus dbstep dbquit run"),
    ("strings", "num2str str2double str2num strcmp strcmpi strcat upper lower strtrim strrep strfind double logical char"),
    ("workspace/env", "eval error warning lasterror exist which clc home tic toc pause nargin nargout narginchk nargoutchk disp display format fprintf sprintf who whos clear assert rng normrnd"),
    ("files/path", "save load type addpath rmpath path pwd cd dir ls genpath fullfile csvread mkdir"),
    ("plotting", "figure gcf clf close subplot plot scatter stem bar histogram area stairs errorbar semilogx semilogy loglog hold grid title xlabel ylabel zlabel legend xlim ylim zlim xscale yscale view colormap colorbar caxis clim axis box imagesc image pie contour plot3 surf mesh saveas exportgraphics print"),
];

/// Every builtin name listed in [`FUNCTION_GROUPS`] — used by tests.
#[cfg(test)]
fn grouped_names() -> Vec<&'static str> {
    FUNCTION_GROUPS.iter().flat_map(|(_, names)| names.split(' ')).collect()
}

#[cfg(test)]
mod group_tests {
    use super::grouped_names;

    /// The catalogue must cover `matlang::BUILTIN_NAMES` exactly — no missing
    /// or phantom entries, no duplicates.
    #[test]
    fn function_groups_match_builtins() {
        let mut grouped = grouped_names();
        grouped.sort_unstable();
        let mut builtin = matlang::BUILTIN_NAMES.to_vec();
        builtin.sort_unstable();
        assert_eq!(grouped, builtin);
    }
}

fn need_str<'a>(p: &'a Json, key: &str, cmd: &str) -> Result<&'a str, EngineError> {
    p.get(key).and_then(Json::as_str).ok_or_else(|| EngineError::bad_params(cmd, format!("missing string `{key}`")))
}

fn opt_usize(p: &Json, key: &str) -> Option<usize> {
    p.get(key).and_then(Json::as_u64).map(|n| n as usize)
}

/// JSON → `Value` for `workspace.set`: number, string, `[1 2 3]` (row
/// vector), `[[1 2];[3 4]]` (matrix rows) or `{rows,cols,data}`.
fn json_to_value(v: &Json, cmd: &str) -> Result<Value, EngineError> {
    match v {
        Json::Number(n) => Ok(Value::Num(n.as_f64().ok_or_else(|| EngineError::bad_params(cmd, "number out of range"))?)),
        Json::String(s) => Ok(Value::Str(s.clone())),
        Json::Array(rows) => {
            if rows.is_empty() {
                return Ok(Value::Mat(Matrix::zeros(0, 0)));
            }
            if rows.iter().all(|r| r.is_number()) {
                let data: Vec<f64> =
                    rows.iter().map(|r| r.as_f64().ok_or_else(|| EngineError::bad_params(cmd, "bad number in array"))).collect::<Result<_, _>>()?;
                return Ok(Value::Mat(Matrix::row_vector(data)));
            }
            let ncols = match rows.first() {
                Some(Json::Array(r)) => r.len(),
                _ => return Err(EngineError::bad_params(cmd, "matrix rows must be arrays")),
            };
            let mut data = Vec::with_capacity(rows.len().saturating_mul(ncols));
            for r in rows {
                let Json::Array(cols) = r else {
                    return Err(EngineError::bad_params(cmd, "matrix rows must be arrays"));
                };
                if cols.len() != ncols {
                    return Err(EngineError::bad_params(cmd, "matrix rows have different lengths"));
                }
                for c in cols {
                    data.push(c.as_f64().ok_or_else(|| EngineError::bad_params(cmd, "matrix entries must be numbers"))?);
                }
            }
            let nrows = rows.len();
            // JSON rows arrive row-major; the matrix is column-major.
            let mut col_major = Vec::with_capacity(data.len());
            for c in 0..ncols {
                for r in 0..nrows {
                    col_major.push(data.get(r * ncols + c).copied().unwrap_or(0.0));
                }
            }
            Matrix::from_col_major(nrows, ncols, col_major).map(Value::Mat).map_err(|e| EngineError::bad_params(cmd, e.to_string()))
        }
        Json::Object(_) => {
            let r = opt_usize(v, "rows").ok_or_else(|| EngineError::bad_params(cmd, "object value needs `rows`"))?;
            let c = opt_usize(v, "cols").ok_or_else(|| EngineError::bad_params(cmd, "object value needs `cols`"))?;
            let data = v
                .get("data")
                .and_then(Json::as_array)
                .ok_or_else(|| EngineError::bad_params(cmd, "object value needs `data` array"))?
                .iter()
                .map(|x| x.as_f64().ok_or_else(|| EngineError::bad_params(cmd, "`data` must be numbers")))
                .collect::<Result<Vec<f64>, _>>()?;
            // `data` is column-major, matching `Matrix::from_col_major`.
            Matrix::from_col_major(r, c, data).map(Value::Mat).map_err(|e| EngineError::bad_params(cmd, e.to_string()))
        }
        _ => Err(EngineError::bad_params(cmd, "value must be a number, string, array or {rows,cols,data}")),
    }
}

/// `(rows, cols)` of a value — shared by `workspace.list` and
/// `workspace.get` so the two can never disagree.
fn value_size(v: &Value) -> (usize, usize) {
    match v {
        Value::Num(_) => (1, 1),
        Value::Mat(m) => (m.rows(), m.cols()),
        Value::Cplx(m) => (m.rows(), m.cols()),
        Value::Str(s) => (1, s.chars().count()),
        Value::Cell(c) => c.shape(),
        Value::Struct(s) => (s.rows(), s.cols()),
        Value::Sparse(s) => (s.rows(), s.cols()),
        Value::Func(_) => (1, 1),
    }
}

/// `Value` → JSON: `{class, size, display, data?}` for `workspace.get`.
fn value_to_json(v: &Value, long: bool) -> Json {
    let display = matlang::fmt_value(v, long).to_string();
    let class = matlang::describe(v);
    let (r, c) = value_size(v);
    match v {
        Value::Num(n) => json!({"class": class, "size": [r, c], "display": display, "data": n}),
        Value::Str(s) => json!({"class": class, "size": [r, c], "display": display, "data": s}),
        Value::Mat(m) => {
            // Row-major nested arrays, matching `workspace.set` input.
            let rows: Vec<Json> =
                (0..m.rows()).map(|r| Json::Array((0..m.cols()).map(|c| json!(m.get(r, c).unwrap_or(f64::NAN))).collect())).collect();
            json!({"class": class, "size": [r, c], "display": display, "data": rows})
        }
        Value::Cplx(m) => {
            let rows: Vec<Json> = (0..m.rows())
                .map(|r| {
                    Json::Array(
                        (0..m.cols())
                            .map(|c| {
                                let z = m.get(r, c).unwrap_or(matcore::CNum::real(0.0));
                                json!({"re": z.re, "im": z.im})
                            })
                            .collect(),
                    )
                })
                .collect();
            json!({"class": class, "size": [r, c], "display": display, "data": rows})
        }
        Value::Cell(_) => json!({"class": class, "size": [r, c], "display": display}),
        Value::Struct(m) => json!({"class": class, "size": [r, c], "display": display, "fields": m.field_names()}),
        // Sparse ships dense data + a sparse flag — the grid display is
        // identical, and `nnz` is in the display string.
        Value::Sparse(s) => {
            let m = s.full();
            let rows: Vec<Json> =
                (0..m.rows()).map(|r| Json::Array((0..m.cols()).map(|c| json!(m.get(r, c).unwrap_or(f64::NAN))).collect())).collect();
            json!({"class": class, "size": [r, c], "display": display, "data": rows, "sparse": true, "nnz": s.nnz()})
        }
        Value::Func(_) => json!({"class": class, "size": [r, c], "display": display}),
    }
}

fn eval_line(s: &mut Session, p: &Json) -> Result<Json, EngineError> {
    let code = need_str(p, "code", "eval.line")?;
    let output = s.lang_mut().run(code)?;
    Ok(json!({"ok": true, "output": output}))
}

fn script_run(s: &mut Session, p: &Json) -> Result<Json, EngineError> {
    let path = need_str(p, "path", "script.run")?;
    let src = std::fs::read_to_string(path).map_err(|e| EngineError::Other(format!("cannot read {path}: {e}")))?;
    // The script's directory joins the search path so it can call sibling
    // `<name>.m` function files.
    if let Some(dir) = std::path::Path::new(path).parent() {
        s.lang_mut().add_path(dir.to_path_buf());
    }
    let output = s.lang_mut().run(&src)?;
    Ok(json!({"ok": true, "output": output}))
}

fn workspace_list(s: &mut Session, _p: &Json) -> Result<Json, EngineError> {
    let vars: Vec<Json> = s
        .lang()
        .var_names()
        .iter()
        .filter_map(|n| s.lang().get(n).map(|v| (n, v)))
        .map(|(n, v)| {
            let (r, c) = value_size(v);
            json!({"name": n, "class": matlang::describe(v), "size": [r, c]})
        })
        .collect();
    Ok(json!({"vars": vars}))
}

fn workspace_get(s: &mut Session, p: &Json) -> Result<Json, EngineError> {
    let name = need_str(p, "name", "workspace.get")?;
    match s.lang().get(name) {
        Some(v) => {
            let mut out = value_to_json(v, true);
            if let Json::Object(ref mut o) = out {
                o.insert("name".into(), json!(name));
            }
            Ok(out)
        }
        None => Err(EngineError::Other(format!("no variable `{name}`"))),
    }
}

fn workspace_set(s: &mut Session, p: &Json) -> Result<Json, EngineError> {
    let name = need_str(p, "name", "workspace.set")?;
    let v = p.get("value").ok_or_else(|| EngineError::bad_params("workspace.set", "missing `value`"))?;
    let value = json_to_value(v, "workspace.set")?;
    s.lang_mut().set(name, value);
    Ok(json!({"ok": true}))
}

fn workspace_clear(s: &mut Session, p: &Json) -> Result<Json, EngineError> {
    let names: Vec<String> =
        p.get("names").and_then(Json::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
    s.lang_mut().clear_vars(&names);
    Ok(json!({"ok": true}))
}

fn session_reset(s: &mut Session, _p: &Json) -> Result<Json, EngineError> {
    let paths = s.lang().paths().to_vec();
    let mut fresh = matlang::Session::new();
    for d in paths {
        fresh.add_path(d);
    }
    *s.lang_mut() = fresh;
    Ok(json!({"ok": true}))
}

fn path_add(s: &mut Session, p: &Json) -> Result<Json, EngineError> {
    let dir = need_str(p, "dir", "path.add")?;
    s.lang_mut().add_path(dir);
    Ok(json!({"ok": true}))
}

fn path_list(s: &mut Session, _p: &Json) -> Result<Json, EngineError> {
    let paths: Vec<Json> = s.lang().paths().iter().map(|d| json!(d.to_string_lossy())).collect();
    Ok(json!({"paths": paths}))
}

fn figure_list(s: &mut Session, _p: &Json) -> Result<Json, EngineError> {
    Ok(json!({
        "count": s.lang().figures().len(),
        "current": s.lang().current_figure(),
    }))
}

fn figure_export(s: &mut Session, p: &Json) -> Result<Json, EngineError> {
    let path = need_str(p, "path", "figure.export")?;
    let n = opt_usize(p, "figure").unwrap_or_else(|| s.lang().current_figure());
    let svg = s.lang().figure_svg(n).ok_or_else(|| EngineError::Other(format!("no figure {n}")))?;
    let bytes = svg.len();
    std::fs::write(path, &svg).map_err(|e| EngineError::Other(format!("cannot write {path}: {e}")))?;
    Ok(json!({"ok": true, "path": path, "bytes": bytes}))
}

// ---- SimCraft interconnection ----------------------------------------------
// The SimCraft session lives in `Session::sim` (lazy). `sim.run` loads a
// `.simcraft` model, runs it and publishes every sink trace into this
// session's workspace via `simcraft_engine::publish` — `out.t`, `out.y`,
// `out.<name>`, plus a `{t, y}` struct per `to_workspace` var.

/// A sim-side error (model, solver or inner command) → user-facing message.
fn sim_e(e: impl fmt::Display) -> EngineError {
    EngineError::Other(e.to_string())
}

/// `sim.run`'s `{solver?, dt?, tstop?}` overrides → `RunOpts`.
fn run_opts(p: &Json, cmd: &str) -> Result<simcraft_engine::RunOpts, EngineError> {
    let solver = match p.get("solver") {
        None | Some(Json::Null) => None,
        Some(v) => {
            let name = v.as_str().ok_or_else(|| EngineError::bad_params(cmd, "`solver` must be a string"))?;
            Some(
                simcraft_engine::Solver::parse(name)
                    .ok_or_else(|| EngineError::bad_params(cmd, format!("unknown solver `{name}` (have: euler, rk4)")))?,
            )
        }
    };
    let num = |key: &str| -> Result<Option<f64>, EngineError> {
        match p.get(key) {
            None | Some(Json::Null) => Ok(None),
            Some(v) => v.as_f64().map(Some).ok_or_else(|| EngineError::bad_params(cmd, format!("`{key}` must be a number"))),
        }
    };
    Ok(simcraft_engine::RunOpts { solver, dt: num("dt")?, tstop: num("tstop")? })
}

fn sim_run(s: &mut Session, p: &Json) -> Result<Json, EngineError> {
    let ov = run_opts(p, "sim.run")?;
    // `path` may be omitted once a model is loaded (e.g. after
    // `model.set_param` — reloading would discard the new params).
    if let Some(path) = p.get("path").and_then(Json::as_str) {
        s.sim().load(path).map_err(sim_e)?;
    }
    let run = s.sim().run(&ov).map_err(sim_e)?;
    simcraft_engine::publish(s.lang_mut(), &run);
    Ok(simcraft_engine::run_json(&run))
}

fn sim_check(s: &mut Session, p: &Json) -> Result<Json, EngineError> {
    let path = need_str(p, "path", "sim.check")?;
    s.sim().load(path).map_err(sim_e)?;
    let info = s.sim().check().map_err(sim_e)?;
    Ok(simcraft_engine::check_json(&info))
}

/// Forward a pure-sim command into the linked SimCraft session's registry.
fn fwd(s: &mut Session, id: &str, params: Json) -> Result<Json, EngineError> {
    s.sim().execute(id, params).map_err(sim_e)
}

fn sim_blocks(s: &mut Session, _p: &Json) -> Result<Json, EngineError> {
    fwd(s, "block.list", json!({}))
}

fn model_list(s: &mut Session, _p: &Json) -> Result<Json, EngineError> {
    fwd(s, "model.list", json!({}))
}

fn model_get_param(s: &mut Session, p: &Json) -> Result<Json, EngineError> {
    let block = need_str(p, "block", "model.get_param")?;
    fwd(s, "block.get", json!({"path": block}))
}

fn model_set_param(s: &mut Session, p: &Json) -> Result<Json, EngineError> {
    let block = need_str(p, "block", "model.set_param")?;
    let param = need_str(p, "param", "model.set_param")?;
    let value = p.get("value").ok_or_else(|| EngineError::bad_params("model.set_param", "missing `value`"))?;
    fwd(s, "block.set", json!({"path": block, "params": {param: value.clone()}}))
}

fn functions_list(_s: &mut Session, _p: &Json) -> Result<Json, EngineError> {
    let groups: Vec<Json> = FUNCTION_GROUPS.iter().map(|(g, names)| json!({"group": g, "names": names})).collect();
    Ok(json!({"groups": groups}))
}

fn engine_version(_s: &mut Session, _p: &Json) -> Result<Json, EngineError> {
    Ok(json!({"app": "matcraft", "version": env!("CARGO_PKG_VERSION")}))
}

fn engine_list(_s: &mut Session, _p: &Json) -> Result<Json, EngineError> {
    let cmds: Vec<Json> = COMMANDS.iter().map(|c| json!({"id": c.id, "label": c.label, "params": c.params})).collect();
    Ok(json!({"commands": cmds}))
}

static COMMANDS: &[CommandSpec] = &[
    CommandSpec { id: "engine.version", label: "App name and version", params: "{}", run: engine_version },
    CommandSpec { id: "engine.list", label: "List every command", params: "{}", run: engine_list },
    CommandSpec { id: "eval.line", label: "Evaluate M-code", params: "{code: string} → {output}", run: eval_line },
    CommandSpec { id: "script.run", label: "Run a .m script file", params: "{path: string} → {output}", run: script_run },
    CommandSpec { id: "workspace.list", label: "List workspace variables", params: "{} → {vars}", run: workspace_list },
    CommandSpec { id: "workspace.get", label: "Read one variable", params: "{name} → {class,size,display,data}", run: workspace_get },
    CommandSpec { id: "workspace.set", label: "Set a variable", params: "{name, value}", run: workspace_set },
    CommandSpec { id: "workspace.clear", label: "Clear variables", params: "{names?: [string]}", run: workspace_clear },
    CommandSpec { id: "session.reset", label: "Reset the session", params: "{}", run: session_reset },
    CommandSpec { id: "path.add", label: "Add a function search dir", params: "{dir}", run: path_add },
    CommandSpec { id: "path.list", label: "List the search path", params: "{}", run: path_list },
    CommandSpec { id: "figure.list", label: "List open figures", params: "{} → {count,current}", run: figure_list },
    CommandSpec { id: "figure.export", label: "Export a figure to SVG", params: "{path, figure?}", run: figure_export },
    CommandSpec { id: "functions.list", label: "List builtin functions", params: "{}", run: functions_list },
    CommandSpec {
        id: "sim.run",
        label: "Run a .simcraft model; traces land in the workspace",
        params: "{path?, solver?, dt?, tstop?} → {t, out:{name:{t,y}}}",
        run: sim_run,
    },
    CommandSpec {
        id: "sim.check",
        label: "Load and compile-check a .simcraft model",
        params: "{path} → {blocks,wires,states,steps}",
        run: sim_check,
    },
    CommandSpec { id: "sim.blocks", label: "List the SimCraft block catalog", params: "{} → {blocks}", run: sim_blocks },
    CommandSpec { id: "model.list", label: "List the loaded SimCraft model's blocks and wires", params: "{} → {blocks,wires}", run: model_list },
    CommandSpec { id: "model.get_param", label: "Read a SimCraft block's params", params: "{block} → {path,type,params}", run: model_get_param },
    CommandSpec { id: "model.set_param", label: "Set a SimCraft block param (loaded model)", params: "{block, param, value}", run: model_set_param },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_and_workspace() {
        let mut s = Session::new();
        let r = s.execute("eval.line", json!({"code": "A = [2 1; 1 3]; det(A)"})).unwrap();
        assert!(r["output"].as_str().unwrap().contains("ans ="));
        let vars = s.execute("workspace.list", json!({})).unwrap();
        assert!(vars["vars"].as_array().unwrap().iter().any(|v| v["name"] == "A"));
        let a = s.execute("workspace.get", json!({"name": "A"})).unwrap();
        assert_eq!(a["size"], json!([2, 2]));
        assert_eq!(a["data"][0][0], json!(2.0));
    }

    #[test]
    fn workspace_list_sizes_match_get() {
        let mut s = Session::new();
        s.execute("eval.line", json!({"code": "c = {1 2; 'x' 'y'}; txt = 'hello'; st.a = 1; st.b = 2"})).unwrap();
        let vars = s.execute("workspace.list", json!({})).unwrap();
        for (name, size) in [("c", json!([2, 2])), ("txt", json!([1, 5])), ("st", json!([1, 1]))] {
            let listed = vars["vars"].as_array().unwrap().iter().find(|v| v["name"] == name).unwrap();
            assert_eq!(listed["size"], size, "workspace.list size for {name}");
            let got = s.execute("workspace.get", json!({"name": name})).unwrap();
            assert_eq!(got["size"], listed["size"], "list/get disagree for {name}");
        }
    }

    #[test]
    fn workspace_set_roundtrip() {
        let mut s = Session::new();
        s.execute("workspace.set", json!({"name": "M", "value": [[1.0, 2.0], [3.0, 4.0]]})).unwrap();
        let r = s.execute("eval.line", json!({"code": "trace = M(1,1) + M(2,2)"})).unwrap();
        assert!(r["output"].as_str().unwrap().contains("5"));
        let m = s.execute("workspace.get", json!({"name": "M"})).unwrap();
        assert_eq!(m["data"][1][1], json!(4.0));
    }

    #[test]
    fn unknown_command_and_bad_params() {
        let mut s = Session::new();
        assert!(matches!(s.execute("nope.cmd", json!({})), Err(EngineError::UnknownCommand(_))));
        assert!(matches!(s.execute("eval.line", json!({})), Err(EngineError::BadParams { .. })));
    }

    #[test]
    fn lang_error_propagates() {
        let mut s = Session::new();
        assert!(matches!(s.execute("eval.line", json!({"code": "1 +"} )), Err(EngineError::Lang(_))));
    }

    #[test]
    fn figure_export_writes_svg() {
        let mut s = Session::new();
        s.execute("eval.line", json!({"code": "plot(1:5, [1 4 9 16 25])"})).unwrap();
        let tmp = std::env::temp_dir().join("matcraft_engine_test_fig.svg");
        let path = tmp.to_string_lossy().to_string();
        let r = s.execute("figure.export", json!({"path": path})).unwrap();
        assert!(r["bytes"].as_u64().unwrap() > 100);
        let svg = std::fs::read_to_string(&tmp).unwrap();
        assert!(svg.contains("<svg"));
        let _ = std::fs::remove_file(&tmp);
    }

    // The sibling repo's example model — `<engine>/../../../simcraft`.
    fn pendulum() -> String {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../simcraft/examples/pendulum.simcraft").to_string_lossy().to_string()
    }

    #[test]
    fn sim_run_pendulum_publishes_workspace() {
        let mut s = Session::new();
        let r = s.execute("sim.run", json!({"path": pendulum()})).unwrap();
        assert_eq!(r["ok"], json!(true));
        assert_eq!(r["solver"], json!("rk4"));
        assert_eq!(r["t"].as_array().unwrap().len(), 10001);
        assert_eq!(r["out"]["theta"]["y"][0][0], json!(0.2));
        assert_eq!(r["out"]["angle_scope"]["y"][0][0], json!(0.2));
        // `theta` = {t, y} struct (to_workspace); `out.theta` indexes y.
        let r = s.execute("eval.line", json!({"code": "theta.y(1)\nout.theta(1)\nnumel(out.t)"})).unwrap();
        let out = r["output"].as_str().unwrap();
        assert!(out.contains("0.2"), "{out}");
        assert!(out.contains("10001"), "{out}");
        let r = s.execute("eval.line", json!({"code": "out.theta(1:10)"})).unwrap();
        assert!(r["output"].as_str().unwrap().contains("ans ="), "{}", r["output"]);
    }

    #[test]
    fn sim_check_and_blocks() {
        let mut s = Session::new();
        let r = s.execute("sim.check", json!({"path": pendulum()})).unwrap();
        assert_eq!(r["blocks"], json!(5));
        assert_eq!(r["states"], json!(2));
        let r = s.execute("sim.blocks", json!({})).unwrap();
        assert!(r["blocks"].as_array().unwrap().iter().any(|b| b["type"] == "integrator"));
        let r = s.execute("model.get_param", json!({"block": "gl"})).unwrap();
        assert_eq!(r["params"]["gain"], json!(-9.81));
    }

    #[test]
    fn model_set_param_then_run_loaded() {
        let mut s = Session::new();
        s.execute("sim.check", json!({"path": pendulum()})).unwrap();
        s.execute("model.set_param", json!({"block": "gl", "param": "gain", "value": -1.0})).unwrap();
        let r = s.execute("model.get_param", json!({"block": "gl"})).unwrap();
        assert_eq!(r["params"]["gain"], json!(-1.0));
        // `sim.run` without `path` runs the loaded model, keeping the param.
        let r = s.execute("sim.run", json!({"tstop": 0.5, "solver": "euler"})).unwrap();
        assert_eq!(r["solver"], json!("euler"));
        assert_eq!(r["t"].as_array().unwrap().len(), 501);
        assert_eq!(r["out"]["theta"]["y"][0][0], json!(0.2));
    }

    #[test]
    fn sim_errors_not_panics() {
        let mut s = Session::new();
        assert!(s.execute("sim.run", json!({})).is_err()); // no path, no model
        assert!(s.execute("sim.run", json!({"path": "nope.simcraft"})).is_err());
        assert!(s.execute("model.set_param", json!({"block": "x", "param": "p", "value": 1})).is_err());
    }

    #[test]
    fn reset_keeps_paths() {
        let mut s = Session::new();
        s.execute("path.add", json!({"dir": "/tmp/mymat"})).unwrap();
        s.execute("eval.line", json!({"code": "x = 42"})).unwrap();
        s.execute("session.reset", json!({})).unwrap();
        let vars = s.execute("workspace.list", json!({})).unwrap();
        assert_eq!(vars["vars"].as_array().unwrap().len(), 0);
        let paths = s.execute("path.list", json!({})).unwrap();
        assert!(paths["paths"].as_array().unwrap().iter().any(|p| p == "/tmp/mymat"));
    }
}
