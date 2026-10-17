//! Control-channel dispatch — the sibling apps' protocol, MatCraft flavour.
//!
//! The TCP server lives in `apps/matcraft` (`control_server`); it turns
//! JSON-lines requests into [`ControlRequest`]s handed to the app, which
//! calls [`handle`] from `update()`. Methods:
//!
//! - `engine.execute` `{command, params}` — run any engine command.
//! - `engine.list` / `commands` — the command catalogue.
//! - `ui.state` — app/workspace/figure summary.
//! - `ui.render` `{figure?, path?, scale?}` — PNG of a figure:
//!   `{ok, bytes, path?, pngBase64}`.
//! - `ui.screenshot` `{path?}` — PNG of the window once the next frame is
//!   presented; the request is deferred until the `Screenshot` event lands.
//! - `ping` — `{ok, pong}`.
//!
//! Replies are JSON objects carrying `ok` (+ payload) or `error`; the
//! connection thread adds the request `id` before writing back.

use std::sync::{Arc, Condvar, Mutex};

use serde_json::{json, Value as Json};

use crate::MatApp;

/// Shared slot between a [`ControlRequest`] (app side) and its
/// [`PendingReply`] (connection-thread side).
type Slot = Arc<(Mutex<Option<Json>>, Condvar)>;

/// One request handed to the app over the control channel.
pub struct ControlRequest {
    pub method: String,
    pub params: Json,
    slot: Slot,
}

/// The connection thread's handle for the reply.
pub struct PendingReply {
    slot: Slot,
}

impl ControlRequest {
    /// `new` splits a request into the app side and the connection side.
    pub fn new(method: String, params: Json) -> (Self, PendingReply) {
        let slot: Slot = Arc::new((Mutex::new(None), Condvar::new()));
        (ControlRequest { method, params, slot: slot.clone() }, PendingReply { slot })
    }

    /// Deliver the reply to the waiting connection thread.
    pub fn reply(self, v: Json) {
        let (m, cv) = &*self.slot;
        if let Ok(mut g) = m.lock() {
            *g = Some(v);
        }
        cv.notify_one();
    }
}

impl PendingReply {
    /// Block until the app replies or `dur` elapses.
    pub fn wait(&self, dur: std::time::Duration) -> Json {
        let (m, cv) = &*self.slot;
        let Ok(mut g) = m.lock() else {
            return json!({"ok": false, "error": "reply lock poisoned"});
        };
        loop {
            if let Some(v) = g.take() {
                return v;
            }
            match cv.wait_timeout(g, dur) {
                Ok((g2, timed_out)) => {
                    g = g2;
                    if timed_out.timed_out() {
                        return json!({"ok": false, "error": "control request timed out"});
                    }
                }
                Err(_) => return json!({"ok": false, "error": "reply lock poisoned"}),
            }
        }
    }
}

/// Handle one control request. Most reply immediately; `ui.screenshot`
/// is queued in `app.pending_shots` until the frame is captured.
pub fn handle(app: &mut MatApp, ctx: &egui::Context, req: ControlRequest) {
    let params = req.params.clone();
    let method = req.method.clone();
    match method.as_str() {
        "engine.execute" | "command" => {
            let cmd = params.get("command").and_then(Json::as_str).unwrap_or("").to_string();
            let p = params.get("params").cloned().unwrap_or(json!({}));
            // `eval.line`/`script.run` go through the echo path so control
            // evals appear in the Command Window and Command History exactly
            // like typed input; every other command stays headless.
            let reply = match cmd.as_str() {
                "eval.line" => match p.get("code").and_then(Json::as_str) {
                    Some(code) => app.run_line(code),
                    None => app.exec_command(&cmd, p),
                },
                "script.run" => match p.get("path").and_then(Json::as_str) {
                    Some(path) => app.run_script(path),
                    None => app.exec_command(&cmd, p),
                },
                _ => app.exec_command(&cmd, p),
            };
            req.reply(reply);
        }
        "engine.list" | "commands" => {
            let cmds: Vec<Json> = app.session.commands().iter().map(|c| json!({"id": c.id, "label": c.label, "params": c.params})).collect();
            req.reply(json!({"ok": true, "commands": cmds}));
        }
        "ping" => req.reply(json!({"ok": true, "pong": true})),
        "ui.state" => {
            let lang = app.session.lang();
            let vars: Vec<Json> =
                lang.var_names().iter().filter_map(|n| lang.get(n).map(|v| json!({"name": n, "class": matlang::describe(v)}))).collect();
            req.reply(json!({
                "ok": true,
                "app": "matcraft",
                "figures": lang.figures().len(),
                "currentFigure": lang.current_figure(),
                "vars": vars,
            }));
        }
        "ui.render" => {
            let n = params.get("figure").and_then(Json::as_u64).map(|v| v as usize).unwrap_or_else(|| app.session.lang().current_figure());
            let scale = params.get("scale").and_then(Json::as_f64).map(|v| v as f32).unwrap_or(2.0);
            let path = params.get("path").and_then(Json::as_str).map(str::to_string);
            match app.session.lang().figure_svg(n) {
                Some(svg) => match app.raster.svg_to_png(&svg, scale) {
                    Ok(png) => {
                        use base64::Engine;
                        let mut out = json!({
                            "ok": true,
                            "figure": n,
                            "bytes": png.len(),
                            "pngBase64": base64::engine::general_purpose::STANDARD.encode(&png),
                        });
                        if let Some(p) = &path {
                            match crate::write_file(std::path::Path::new(p), &png) {
                                Ok(()) => out["path"] = json!(p),
                                Err(e) => out = json!({"ok": false, "error": format!("cannot write {p}: {e}")}),
                            }
                        }
                        req.reply(out);
                    }
                    Err(e) => req.reply(json!({"ok": false, "error": e})),
                },
                None => req.reply(json!({"ok": false, "error": format!("no figure {n}")})),
            }
        }
        "ui.screenshot" => {
            let path = params.get("path").and_then(Json::as_str).map(str::to_string);
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            app.pending_shots.push((req, path));
            ctx.request_repaint();
        }
        other => req.reply(json!({"ok": false, "error": format!("unknown method `{other}`")})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MatApp, MatAppOptions};

    fn app() -> MatApp {
        MatApp::with_options(MatAppOptions::default())
    }

    /// Dispatch `method` and return the reply (screenshot requests are queued
    /// instead — those stay in `app.pending_shots`).
    fn dispatch(app: &mut MatApp, method: &str, params: Json) -> Json {
        let (req, pending) = ControlRequest::new(method.to_string(), params);
        handle(app, &egui::Context::default(), req);
        pending.wait(std::time::Duration::from_secs(10))
    }

    #[test]
    fn ping_and_unknown_method() {
        let mut a = app();
        assert_eq!(dispatch(&mut a, "ping", json!({}))["pong"], json!(true));
        let bad = dispatch(&mut a, "ui.frobnicate", json!({}));
        assert_eq!(bad["ok"], json!(false));
        assert!(bad["error"].as_str().unwrap_or_default().contains("unknown method"));
    }

    #[test]
    fn engine_execute_runs_mcode() {
        let mut a = app();
        let r = dispatch(&mut a, "engine.execute", json!({"command": "eval.line", "params": {"code": "d = det([1 2; 3 4])"}}));
        assert_eq!(r["ok"], json!(true));
        assert!(r["output"].as_str().unwrap_or_default().contains("-2"));
        assert!(a.figs_dirty);
    }

    #[test]
    fn eval_line_via_control_echoes_and_records() {
        let mut a = app();
        let r = dispatch(&mut a, "engine.execute", json!({"command": "eval.line", "params": {"code": "x = 5"}}));
        assert_eq!(r["ok"], json!(true));
        // Same echo + Command History entry as a typed line.
        assert!(a.repl.scrollback.contains(">> x = 5\n"));
        assert!(a.repl.scrollback.contains("x ="));
        assert!(a.repl.entries().iter().any(|e| e.cmd == "x = 5"));
    }

    #[test]
    fn script_run_via_control_echoes_and_records() {
        let mut a = app();
        let tmp = std::env::temp_dir().join("matcraft_ctrl_script_run_test.m");
        std::fs::write(&tmp, "y = 7").unwrap();
        let r = dispatch(&mut a, "engine.execute", json!({"command": "script.run", "params": {"path": tmp.to_string_lossy()}}));
        assert_eq!(r["ok"], json!(true));
        assert!(a.repl.scrollback.contains(">> run '"));
        assert!(a.repl.entries().iter().any(|e| e.cmd.starts_with("run '")));
        assert!(a.session.lang().get("y").is_some());
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn engine_execute_reports_errors() {
        let mut a = app();
        let r = dispatch(&mut a, "engine.execute", json!({"command": "eval.line", "params": {"code": "no_such_fn(1)"}}));
        assert_eq!(r["ok"], json!(false));
    }

    #[test]
    fn engine_list_includes_eval_line() {
        let mut a = app();
        let r = dispatch(&mut a, "engine.list", json!({}));
        let ids: Vec<&str> = r["commands"].as_array().map(|v| v.iter().filter_map(|c| c["id"].as_str()).collect()).unwrap_or_default();
        assert!(ids.contains(&"eval.line"));
    }

    #[test]
    fn ui_state_lists_vars_and_figures() {
        let mut a = app();
        let _ = dispatch(&mut a, "engine.execute", json!({"command": "eval.line", "params": {"code": "A = [1 2; 3 4]; plot([1 2 3], [1 4 9])"}}));
        let r = dispatch(&mut a, "ui.state", json!({}));
        assert_eq!(r["ok"], json!(true));
        assert_eq!(r["figures"], json!(1));
        let names: Vec<&str> = r["vars"].as_array().map(|v| v.iter().filter_map(|v| v["name"].as_str()).collect()).unwrap_or_default();
        assert!(names.contains(&"A"));
    }

    #[test]
    fn ui_render_returns_png_bytes() {
        let mut a = app();
        let _ = dispatch(&mut a, "engine.execute", json!({"command": "eval.line", "params": {"code": "plot([1 2 3], [3 2 1])"}}));
        let r = dispatch(&mut a, "ui.render", json!({}));
        assert_eq!(r["ok"], json!(true));
        assert_eq!(r["figure"], json!(1));
        let bytes = r["bytes"].as_u64().unwrap_or(0);
        assert!(bytes > 500);
        use base64::Engine;
        let png = base64::engine::general_purpose::STANDARD.decode(r["pngBase64"].as_str().unwrap_or_default()).unwrap_or_default();
        assert_eq!(png.len() as u64, bytes);
        assert_eq!(&png[..4], &[0x89, 0x50, 0x4e, 0x47]);
    }

    #[test]
    fn ui_render_errors_without_figure() {
        let mut a = app();
        let r = dispatch(&mut a, "ui.render", json!({}));
        assert_eq!(r["ok"], json!(false));
    }

    #[test]
    fn ui_screenshot_queues_until_frame_presented() {
        let mut a = app();
        assert!(a.pending_shots.is_empty());
        let (req, _pending) = ControlRequest::new("ui.screenshot".to_string(), json!({}));
        handle(&mut a, &egui::Context::default(), req);
        assert_eq!(a.pending_shots.len(), 1);
    }

    #[test]
    fn pending_reply_times_out() {
        let (_req, pending) = ControlRequest::new("ui.screenshot".to_string(), json!({}));
        let r = pending.wait(std::time::Duration::from_millis(5));
        assert_eq!(r["ok"], json!(false));
        assert!(r["error"].as_str().unwrap_or_default().contains("timed out"));
    }
}
