//! MatCraft Studio — the egui front end.
//!
//! Panels: workspace inspector (left), editor (right, toggleable), command
//! window (bottom), figure dock (centre). Everything runs through the
//! engine's command registry — the UI is thin — and the control channel
//! (`matcraft --control PORT`) reaches the same commands plus `ui.*` views.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod control;
mod editor;
pub mod figure;
mod files;
mod history;
mod repl;
mod workspace;

use std::sync::mpsc::Receiver;
use std::sync::Arc;

use serde_json::{json, Value as Json};

use control::ControlRequest;
use editor::{Editor, EditorCmd};
use figure::Rasterizer;
use files::FileCmd;
use history::HistCmd;
use repl::Repl;
use workspace::WsCmd;

/// One open document tab in the centre area.
#[derive(Clone, Copy, PartialEq)]
enum Doc {
    /// Index into `MatApp::editors`.
    Edit(usize),
    /// Figure number minus one.
    Fig(usize),
}

/// Construction options for [`MatApp`].
#[derive(Default)]
pub struct MatAppOptions {
    /// Receives control-channel requests when `matcraft --control PORT` is
    /// active (`None` on wasm or when the flag wasn't passed).
    pub control_rx: Option<Receiver<ControlRequest>>,
    /// A `.m` file to open in the editor at startup.
    pub startup_script: Option<std::path::PathBuf>,
}

/// The Studio application state.
pub struct MatApp {
    /// The engine session — workspace, figures, command registry.
    pub session: matcraft_engine::Session,
    /// SVG rasterizer shared by the figure dock and `ui.render`.
    pub raster: Rasterizer,
    /// `ui.screenshot` requests waiting for a presented frame.
    pub pending_shots: Vec<(ControlRequest, Option<String>)>,
    control_rx: Option<Receiver<ControlRequest>>,
    repl: Repl,
    /// Open editor buffers — each owns one document tab.
    editors: Vec<Editor>,
    /// Open document tabs in order (editors + figures).
    docs: Vec<Doc>,
    /// Index into `docs` of the raised tab.
    doc_sel: usize,
    /// Figure tabs the user closed (reopened automatically only for a fresh figure).
    fig_docs_closed: std::collections::BTreeSet<usize>,
    show_editor: bool,
    ws_sel: Option<String>,
    /// File previewed in the Details pane (Current Folder selection).
    file_sel: Option<std::path::PathBuf>,
    fig_tex: Vec<Option<egui::TextureHandle>>,
    figs_dirty: bool,
    /// Editable current-folder text in the path bar.
    cwd_text: String,
    /// Cached dir listing + details (the fs doesn't get polled every frame).
    fcache: files::FileCache,
    /// Variable Editor window: the open variable's name.
    var_editor: Option<String>,
    /// Cell edit buffers for the Variable Editor, keyed (row, col) 1-based.
    var_edits: std::collections::HashMap<(usize, usize), String>,
    /// Pan/zoom bounds of the figure viewer, per figure index.
    fig_scene: Option<(usize, egui::Rect)>,
    /// View-menu toggles for the docked panels.
    show_left: bool,
    show_right: bool,
    show_cmdwin: bool,
    /// Command History multi-selection (entry indices).
    hist_sel: std::collections::BTreeSet<usize>,
    /// Workspace rename dialog: the variable being renamed + name field.
    ws_rename: Option<String>,
    ws_rename_buf: String,
}

impl MatApp {
    /// `eframe::run_native` entry point.
    pub fn new(_cc: &eframe::CreationContext<'_>, opts: MatAppOptions) -> Self {
        Self::with_options(opts)
    }

    /// Build the app without an eframe context — for tests and tools.
    pub fn with_options(opts: MatAppOptions) -> Self {
        let mut app = MatApp {
            session: matcraft_engine::Session::new(),
            raster: Rasterizer::new(),
            pending_shots: Vec::new(),
            control_rx: opts.control_rx,
            repl: Repl::new(),
            editors: vec![Editor::new()],
            docs: vec![Doc::Edit(0)],
            doc_sel: 0,
            fig_docs_closed: std::collections::BTreeSet::new(),
            show_editor: true,
            ws_sel: None,
            file_sel: None,
            fig_tex: Vec::new(),
            figs_dirty: false,
            cwd_text: std::env::current_dir().map(|d| d.display().to_string()).unwrap_or_default(),
            fcache: files::FileCache::default(),
            var_editor: None,
            var_edits: std::collections::HashMap::new(),
            fig_scene: None,
            show_left: true,
            show_right: true,
            show_cmdwin: true,
            hist_sel: std::collections::BTreeSet::new(),
            ws_rename: None,
            ws_rename_buf: String::new(),
        };
        let version = match app.session.execute("engine.version", json!({})) {
            Ok(v) => v.get("version").and_then(Json::as_str).unwrap_or("?").to_string(),
            Err(_) => "?".to_string(),
        };
        app.repl.push(&format!("MatCraft {version} — type M-code below; `who` lists the workspace.\n"));
        if let Some(p) = &opts.startup_script {
            app.open_editor(p);
        }
        app
    }

    /// The raised editor buffer, if the current tab is an editor.
    fn cur_editor(&self) -> Option<&Editor> {
        match self.docs.get(self.doc_sel) {
            Some(Doc::Edit(i)) => self.editors.get(*i),
            _ => self.editors.first(),
        }
    }

    fn cur_editor_mut(&mut self) -> Option<&mut Editor> {
        match self.docs.get(self.doc_sel) {
            Some(Doc::Edit(i)) => self.editors.get_mut(*i),
            _ => self.editors.first_mut(),
        }
    }

    /// Raise `doc`'s tab, opening it if it isn't open yet.
    fn raise_doc(&mut self, doc: Doc) {
        if let Some(i) = self.docs.iter().position(|d| *d == doc) {
            self.doc_sel = i;
        } else {
            self.docs.push(doc);
            self.doc_sel = self.docs.len() - 1;
        }
        self.show_editor = true;
    }

    /// Open `path` in the editor — focuses an existing tab for the same file.
    fn open_editor(&mut self, path: &std::path::Path) {
        let canon = path.to_string_lossy().to_string();
        if let Some(i) = self.editors.iter().position(|e| e.path == canon && !canon.is_empty()) {
            self.raise_doc(Doc::Edit(i));
            return;
        }
        let mut e = Editor::new();
        e.open_path(path);
        self.editors.push(e);
        self.raise_doc(Doc::Edit(self.editors.len() - 1));
    }

    /// A fresh scratch-editor tab ("New Script" / "Create Script").
    fn new_editor_tab(&mut self, text: Option<String>) {
        let mut e = Editor::new();
        if let Some(t) = text {
            e.text = t;
        }
        self.editors.push(e);
        self.raise_doc(Doc::Edit(self.editors.len() - 1));
    }

    /// A tab's display name: file name, `untitled n`, or `Figure n`.
    fn doc_title(&self, doc: Doc) -> String {
        match doc {
            Doc::Edit(i) => match self.editors.get(i) {
                Some(e) if !e.path.is_empty() => {
                    std::path::Path::new(&e.path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| e.path.clone())
                }
                _ => {
                    let n = self.editors.iter().take(i + 1).filter(|e| e.path.is_empty()).count();
                    if n <= 1 {
                        "untitled".to_string()
                    } else {
                        format!("untitled{n}")
                    }
                }
            },
            Doc::Fig(i) => format!("Figure {}", i + 1),
        }
    }

    /// Run an engine command, returning `{ok:true, …}` or `{ok:false,error}`.
    /// Marks figures dirty since commands may draw.
    pub fn exec_command(&mut self, id: &str, params: Json) -> Json {
        let out = match self.session.execute(id, params) {
            Ok(mut v) => {
                if let Some(o) = v.as_object_mut() {
                    o.insert("ok".into(), json!(true));
                } else {
                    v = json!({"value": v});
                    v["ok"] = json!(true);
                }
                v
            }
            Err(e) => json!({"ok": false, "error": e.to_string()}),
        };
        self.figs_dirty = true;
        out
    }

    /// Evaluate one line of M-code: echo it into the scrollback, record it in
    /// the Command History, run it, and print the result. Returns the command
    /// reply so the control channel can forward it (`eval.line` echoes and
    /// logs exactly like a typed line).
    fn run_line(&mut self, line: &str) -> Json {
        self.repl.record(line);
        if line.trim() == "clc" {
            // `clc` clears the whole window, including its own echo.
            self.repl.clear();
            self.fcache.invalidate();
            return json!({"ok": true, "output": ""});
        }
        self.run_echoed(line, "eval.line", json!({"code": line}))
    }

    /// The control channel's `script.run`: echoed and history-logged as
    /// `run 'path'` so a remote run looks like a typed line.
    fn run_script(&mut self, path: &str) -> Json {
        let line = format!("run '{}'", path.replace('\'', "''"));
        self.repl.record(&line);
        self.run_echoed(&line, "script.run", json!({"path": path}))
    }

    /// Echo `line` into the scrollback, run a command, print its output.
    fn run_echoed(&mut self, line: &str, id: &str, params: Json) -> Json {
        self.repl.push(&format!(">> {line}\n"));
        let out = self.exec_command(id, params);
        self.print_cmd_out(&out);
        out
    }

    /// Print a command reply's output (or error) into the scrollback, then
    /// refresh the file panels — commands may create/delete files.
    fn print_cmd_out(&mut self, out: &Json) {
        if let Some(o) = out.get("output").and_then(Json::as_str).filter(|s| !s.is_empty()) {
            self.repl.push(o);
        } else if let Some(e) = out.get("error").and_then(Json::as_str) {
            self.repl.push(&format!("Error: {e}"));
        }
        self.fcache.invalidate();
    }

    fn drain_control(&mut self, ctx: &egui::Context) {
        let mut reqs = Vec::new();
        if let Some(rx) = &self.control_rx {
            while let Ok(r) = rx.try_recv() {
                reqs.push(r);
            }
        }
        for req in reqs {
            control::handle(self, ctx, req);
        }
    }

    /// When a `Screenshot` event lands, answer every queued `ui.screenshot`.
    fn resolve_screenshots(&mut self, ctx: &egui::Context) {
        if self.pending_shots.is_empty() {
            return;
        }
        let image: Option<Arc<egui::ColorImage>> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
                .next()
        });
        let Some(image) = image else { return };
        let shots = std::mem::take(&mut self.pending_shots);
        for (req, path) in shots {
            let reply = match figure::encode_png(image.as_ref()) {
                Ok(png) => {
                    let mut out = json!({"ok": true, "bytes": png.len(), "width": image.width(), "height": image.height()});
                    match path {
                        Some(p) => match std::fs::write(&p, &png) {
                            Ok(()) => out["path"] = json!(p),
                            Err(e) => out = json!({"ok": false, "error": format!("cannot write {p}: {e}")}),
                        },
                        None => {
                            use base64::Engine;
                            out["pngBase64"] = json!(base64::engine::general_purpose::STANDARD.encode(&png));
                        }
                    }
                    out
                }
                Err(e) => json!({"ok": false, "error": e}),
            };
            req.reply(reply);
        }
    }

    /// Re-rasterize every open figure's SVG into a texture; a newly created
    /// figure raises its document tab (like a figure window appearing).
    fn rebuild_figures(&mut self, ctx: &egui::Context) {
        self.figs_dirty = false;
        let had = self.fig_tex.len();
        self.fig_tex.clear();
        for (i, fig) in self.session.lang().figures().iter().enumerate() {
            let tex = self
                .raster
                .svg_to_image(&matplot::to_svg(fig), 2.0)
                .map(|img| ctx.load_texture(format!("matcraft-fig-{i}"), img, egui::TextureOptions::default()));
            self.fig_tex.push(tex);
        }
        // Keep figure tabs in sync: drop tabs for closed figures, open tabs
        // for figures the user hasn't closed.
        let n = self.fig_tex.len();
        self.docs.retain(|d| !matches!(d, Doc::Fig(i) if *i >= n));
        self.fig_docs_closed.retain(|i| *i < n);
        for i in 0..n {
            if self.fig_docs_closed.contains(&i) || self.docs.iter().any(|d| matches!(d, Doc::Fig(j) if *j == i)) {
                continue;
            }
            self.docs.push(Doc::Fig(i));
        }
        // A newly created figure raises its tab.
        if n > had {
            if let Some(pos) = self.docs.iter().position(|d| matches!(d, Doc::Fig(i) if *i == n - 1)) {
                self.doc_sel = pos;
            }
        }
        self.doc_sel = self.doc_sel.min(self.docs.len().saturating_sub(1));
    }

    /// `cd` into `dir` through the engine, keeping the path bar in sync.
    fn cd_to(&mut self, dir: &std::path::Path) {
        let escaped = dir.display().to_string().replace('\'', "''");
        let out = self.exec_command("eval.line", json!({"code": format!("cd '{escaped}'")}));
        if out.get("ok").and_then(Json::as_bool) == Some(true) {
            self.file_sel = None;
            self.fcache.invalidate();
        } else if let Some(e) = out.get("error").and_then(Json::as_str) {
            self.repl.push(&format!("Error: {e}"));
        }
    }

    /// Evaluate the editor buffer in the session (toolbar ▶ Run and the
    /// editor pane's own Run share this).
    fn run_editor_buffer(&mut self, source: String, dir: Option<std::path::PathBuf>) {
        if let Some(d) = dir {
            let _ = self.exec_command("path.add", json!({"dir": d.to_string_lossy()}));
        }
        self.repl.push(">> run editor buffer\n");
        let out = self.exec_command("eval.line", json!({"code": source}));
        if let Some(o) = out.get("output").and_then(Json::as_str).filter(|s| !s.is_empty()) {
            self.repl.push(o);
        } else if let Some(e) = out.get("error").and_then(Json::as_str) {
            self.repl.push(&format!("Error: {e}"));
        }
        self.fcache.invalidate();
    }

    /// The currently displayed figure: a `Scene` gives drag-to-pan and
    /// scroll-to-zoom around the rendered texture, with a small toolbar.
    fn figure_view(&mut self, ui: &mut egui::Ui, idx: usize) {
        let Some(tex) = self.fig_tex.get(idx).and_then(|t| t.as_ref()) else { return };
        let (id, size) = (tex.id(), tex.size_vec2());
        if !matches!(&self.fig_scene, Some((i, _)) if *i == idx) {
            self.fig_scene = Some((idx, egui::Rect::from_min_size(egui::Pos2::ZERO, size)));
        }
        let mut reset = false;
        let mut save_as = false;
        ui.horizontal(|ui| {
            ui.weak(format!("Figure {}", idx + 1));
            ui.separator();
            if ui.small_button("+").on_hover_text("zoom in").clicked() {
                if let Some((_, b)) = &mut self.fig_scene {
                    *b = b.scale_from_center(0.8);
                }
            }
            if ui.small_button("-").on_hover_text("zoom out").clicked() {
                if let Some((_, b)) = &mut self.fig_scene {
                    *b = b.scale_from_center(1.25);
                }
            }
            if ui.small_button("reset").on_hover_text("reset view").clicked() {
                reset = true;
            }
            ui.separator();
            if ui.small_button("Save As…").on_hover_text("saveas — SVG/PNG via extension").clicked() {
                save_as = true;
            }
            ui.weak("drag to pan · scroll to zoom");
        });
        ui.separator();
        if reset {
            self.fig_scene = Some((idx, egui::Rect::from_min_size(egui::Pos2::ZERO, size)));
        }
        if save_as {
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(p) = rfd::FileDialog::new().add_filter("SVG", &["svg"]).add_filter("PNG", &["png"]).save_file() {
                self.run_line(&format!("saveas('{}')", p.display().to_string().replace('\'', "''")));
            }
        }
        let Some((_, bounds)) = &mut self.fig_scene else { return };
        egui::Scene::new().zoom_range(0.05..=20.0).show(ui, bounds, |ui| {
            ui.image(egui::load::SizedTexture::new(id, size));
        });
    }

    /// Floating Variable Editor: a MATLAB-like grid of the variable's cells.
    /// Editing a cell and leaving it writes `name(r,c) = value` back.
    fn variable_editor(&mut self, ctx: &egui::Context) {
        let Some(name) = self.var_editor.clone() else { return };
        let mut run: Option<String> = None;
        let mut open = true;
        egui::Window::new(format!("Variables — {name}")).open(&mut open).default_size([360.0, 240.0]).show(ctx, |ui| {
            let Some(v) = self.session.lang().get(&name) else {
                ui.weak("(variable no longer exists)");
                return;
            };
            // Sparse matrices edit through the dense grid (writes land via
            // `name(r,c) = …`, which keeps the sparse class).
            let sparse_full;
            let v = if let matlang::Value::Sparse(s) = v {
                sparse_full = matlang::Value::Mat(s.full());
                &sparse_full
            } else {
                v
            };
            match v {
                matlang::Value::Mat(m) => {
                    let (rows, cols) = (m.rows(), m.cols());
                    if rows > 100 || cols > 100 {
                        ui.weak(format!("{rows}x{cols} — showing first 100x100"));
                    }
                    let (rows, cols) = (rows.min(100), cols.min(100));
                    egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                        egui::Grid::new("var-edit").striped(true).show(ui, |ui| {
                            for r in 1..=rows {
                                for c in 1..=cols {
                                    let cur = m.get(r - 1, c - 1).unwrap_or_default();
                                    let buf = self.var_edits.entry((r, c)).or_insert_with(|| matlang::fmt_num(cur, false));
                                    let resp = ui.add(egui::TextEdit::singleline(buf).desired_width(64.0).font(egui::TextStyle::Monospace));
                                    if resp.lost_focus() && buf.as_str() != matlang::fmt_num(cur, false) {
                                        run = Some(format!("{name}({r},{c}) = {buf}"));
                                    }
                                }
                                ui.end_row();
                            }
                        });
                    });
                }
                matlang::Value::Cplx(m) => {
                    // Same editable grid as doubles, `re + imi` per cell.
                    let (rows, cols) = (m.rows(), m.cols());
                    if rows > 100 || cols > 100 {
                        ui.weak(format!("{rows}x{cols} — showing first 100x100"));
                    }
                    let (rows, cols) = (rows.min(100), cols.min(100));
                    egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                        egui::Grid::new("var-edit").striped(true).show(ui, |ui| {
                            for r in 1..=rows {
                                for c in 1..=cols {
                                    let z = m.get(r - 1, c - 1).unwrap_or(matlang::CNum::real(0.0));
                                    let cur = format!("{} + {}i", matlang::fmt_num(z.re, false), matlang::fmt_num(z.im, false));
                                    let buf = self.var_edits.entry((r, c)).or_insert_with(|| cur.clone());
                                    let resp = ui.add(egui::TextEdit::singleline(buf).desired_width(110.0).font(egui::TextStyle::Monospace));
                                    if resp.lost_focus() && buf.as_str() != cur {
                                        run = Some(format!("{name}({r},{c}) = {buf}"));
                                    }
                                }
                                ui.end_row();
                            }
                        });
                    });
                }
                matlang::Value::Str(_) => {
                    let cur = if let matlang::Value::Str(s) = v { s.clone() } else { String::new() };
                    let buf = self.var_edits.entry((1, 1)).or_insert_with(|| cur.clone());
                    let resp = ui.add(egui::TextEdit::singleline(buf).desired_width(f32::INFINITY).font(egui::TextStyle::Monospace));
                    if resp.lost_focus() && *buf != cur {
                        let esc = buf.replace('\'', "''");
                        run = Some(format!("{name} = '{esc}'"));
                    }
                }
                matlang::Value::Num(n) => {
                    let buf = self.var_edits.entry((1, 1)).or_insert_with(|| matlang::fmt_num(*n, false));
                    let resp = ui.add(egui::TextEdit::singleline(buf).desired_width(120.0).font(egui::TextStyle::Monospace));
                    if resp.lost_focus() && buf.as_str() != matlang::fmt_num(*n, false) {
                        run = Some(format!("{name} = {buf}"));
                    }
                }
                matlang::Value::Cell(c) => {
                    ui.weak(format!("{}x{} cell — cell contents are read-only here (edit via c{{i}} = …)", c.shape().0, c.shape().1));
                }
                // Sparse was materialized to dense above — never reached.
                matlang::Value::Sparse(_) => {}
                matlang::Value::Struct(m) => {
                    if m.is_scalar() {
                        ui.weak("struct — field list (read-only; edit via s.field = …):");
                        for f in m.field_names() {
                            let fv = m.field_at(0, &f).cloned().unwrap_or(matlang::Value::Num(0.0));
                            ui.monospace(format!("  .{f} = {}", matlang::fmt_value(&fv, false)));
                        }
                    } else {
                        ui.weak(format!("{}x{} struct array — edit via s(i).field = …", m.rows(), m.cols()));
                        for f in m.field_names() {
                            ui.monospace(format!("  .{f}"));
                        }
                    }
                }
            }
        });
        if !open {
            self.var_editor = None;
            self.var_edits.clear();
        }
        if let Some(line) = run {
            self.run_line(&line);
        }
    }
}

impl eframe::App for MatApp {
    /// Non-painting work: control requests, screenshot replies, figure
    /// texture refresh. Runs before `ui`, also while the window is hidden.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_control(ctx);
        self.resolve_screenshots(ctx);
        if self.figs_dirty {
            self.rebuild_figures(ctx);
        }
        // Ctrl+= / Ctrl+- / Ctrl+0 zoom the whole interface.
        let mut zoom = ctx.options(|o| o.zoom_factor);
        ctx.input_mut(|i| {
            if i.consume_key(egui::Modifiers::CTRL, egui::Key::Equals) || i.consume_key(egui::Modifiers::CTRL, egui::Key::Plus) {
                zoom = (zoom * 1.1).min(3.0);
            }
            if i.consume_key(egui::Modifiers::CTRL, egui::Key::Minus) {
                zoom = (zoom / 1.1).max(0.5);
            }
            if i.consume_key(egui::Modifiers::CTRL, egui::Key::Num0) {
                zoom = 1.0;
            }
        });
        ctx.set_zoom_factor(zoom);
        // Keep the app live while control requests may arrive.
        if self.control_rx.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }

    /// MATLAB-style docked desktop: toolbar and path bar on top; Current
    /// Folder + Details on the left; Workspace + Command History on the
    /// right; document tabs (Editor / Figures) and the Command Window in the
    /// centre; status line at the bottom. All column boundaries drag.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // The file panels read the process cwd directly, so `cd` typed in the
        // command window stays in sync; mirror it into the path bar unless
        // the user is editing it right now.
        let cwd = std::env::current_dir().unwrap_or_default();
        let mut run_lines: Vec<String> = Vec::new();
        let mut cd_dirs: Vec<std::path::PathBuf> = Vec::new();

        // ---- toolbar ----------------------------------------------------
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("MatCraft").strong());
                ui.separator();
                #[cfg(not(target_arch = "wasm32"))]
                {
                    if ui.button("New Script").clicked() {
                        self.new_editor_tab(None);
                    }
                    if ui.button("Open…").clicked() {
                        if let Some(p) = rfd::FileDialog::new().add_filter("M-code", &["m"]).pick_file() {
                            self.open_editor(&p);
                        }
                    }
                    if ui.button("Save").clicked() {
                        if let Some(e) = self.cur_editor_mut() {
                            e.save();
                        }
                    }
                }
                if ui.button("▶ Run").clicked() {
                    if let Some(e) = self.cur_editor() {
                        let dir = std::path::Path::new(&e.path).parent().filter(|p| !p.as_os_str().is_empty()).map(|p| p.to_path_buf());
                        self.run_editor_buffer(e.text.clone(), dir);
                    }
                }
                ui.separator();
                if ui.selectable_label(self.show_editor, "Editor").clicked() {
                    self.show_editor = !self.show_editor;
                }
                if ui.button("Reset session").clicked() {
                    let _ = self.exec_command("session.reset", json!({}));
                    self.repl.push("session reset\n");
                }
                ui.menu_button("View", |ui| {
                    ui.checkbox(&mut self.show_left, "Current Folder + Details");
                    ui.checkbox(&mut self.show_right, "Workspace + History");
                    ui.checkbox(&mut self.show_cmdwin, "Command Window");
                    ui.checkbox(&mut self.show_editor, "Editor");
                    ui.separator();
                    if ui.button("Reset zoom").clicked() {
                        ui.ctx().set_zoom_factor(1.0);
                        ui.close();
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("+").on_hover_text("zoom in — Ctrl+=").clicked() {
                        ui.ctx().set_zoom_factor((ui.ctx().zoom_factor() * 1.1).min(3.0));
                    }
                    ui.weak(format!("{:.0}%", ui.ctx().zoom_factor() * 100.0));
                    if ui.small_button("−").on_hover_text("zoom out — Ctrl+-").clicked() {
                        ui.ctx().set_zoom_factor((ui.ctx().zoom_factor() / 1.1).max(0.5));
                    }
                });
            });
            // Path bar — the current folder as an editable breadcrumb.
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("▸").monospace());
                if ui.small_button("↑").on_hover_text("up one level").clicked() {
                    if let Some(p) = cwd.parent() {
                        cd_dirs.push(p.to_path_buf());
                    }
                }
                if !ui.memory(|m| m.has_focus(egui::Id::new("cwd_edit"))) {
                    let shown = cwd.display().to_string();
                    if self.cwd_text != shown {
                        self.cwd_text = shown;
                    }
                }
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.cwd_text)
                        .id(egui::Id::new("cwd_edit"))
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace),
                );
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let t = self.cwd_text.trim().to_string();
                    if !t.is_empty() {
                        cd_dirs.push(std::path::PathBuf::from(t));
                    }
                }
            });
        });

        // ---- left column: Current Folder + Details -----------------------
        if self.show_left {
            egui::Panel::left("left_col").resizable(true).default_size(230.0).show(ui, |ui| {
                egui::Panel::bottom("details").resizable(true).default_size(150.0).show(ui, |ui| {
                    files::details_ui(ui, &mut self.fcache, self.file_sel.as_deref());
                });
                match files::folder_ui(ui, &mut self.fcache, &cwd, &mut self.file_sel) {
                    Some(FileCmd::Cd(d)) => cd_dirs.push(d),
                    Some(FileCmd::Open(p)) => self.open_editor(&p),
                    Some(FileCmd::Run(p)) => {
                        if let Ok(src) = std::fs::read_to_string(&p) {
                            self.run_editor_buffer(src, p.parent().map(|d| d.to_path_buf()));
                        }
                    }
                    None => {}
                }
            });
        }

        // ---- right column: Workspace + Command History -------------------
        if self.show_right {
            egui::Panel::right("right_col").resizable(true).default_size(280.0).show(ui, |ui| {
                egui::Panel::bottom("history").resizable(true).default_size(170.0).show(ui, |ui| {
                    match history::ui(ui, self.repl.entries(), &mut self.hist_sel) {
                        Some(HistCmd::Insert(c)) => self.repl.load_input(&c),
                        Some(HistCmd::Run(c)) => run_lines.push(c),
                        Some(HistCmd::CreateScript(t)) => self.new_editor_tab(Some(t)),
                        None => {}
                    }
                });
                match workspace::ui(ui, self.session.lang(), &mut self.ws_sel) {
                    Some(WsCmd::OpenVar(n)) => {
                        self.var_editor = Some(n);
                        self.var_edits.clear();
                    }
                    Some(WsCmd::Clear(n)) => run_lines.push(format!("clear {n}")),
                    Some(WsCmd::Rename(n)) => {
                        self.ws_rename = Some(n.clone());
                        self.ws_rename_buf = n;
                    }
                    Some(WsCmd::SaveAs(n)) =>
                    {
                        #[cfg(not(target_arch = "wasm32"))]
                        if let Some(p) = rfd::FileDialog::new().add_filter("MAT v5", &["mat"]).save_file() {
                            let esc = p.display().to_string().replace('\'', "''");
                            run_lines.push(format!("save '{esc}' {n}"));
                        }
                    }
                    None => {}
                }
            });
        }

        // ---- status bar ---------------------------------------------------
        egui::Panel::bottom("status").exact_size(22.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.weak("Ready");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.weak(if self.control_rx.is_some() { "control: on" } else { "control: off" });
                });
            });
        });

        // ---- centre: document tabs over the Command Window ---------------
        egui::CentralPanel::default().show(ui, |ui| {
            if self.show_cmdwin {
                // Tab-completion words: builtins + workspace vars + user fns.
                let mut words: Vec<String> = matlang::BUILTIN_NAMES.iter().map(|s| (*s).to_string()).collect();
                words.extend(self.session.lang().var_names());
                words.extend(self.session.lang().func_names());
                words.sort();
                words.dedup();
                egui::Panel::bottom("cmdwin").resizable(true).default_size(230.0).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Command Window").strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(egui::RichText::new("fx").monospace().weak());
                        });
                    });
                    ui.separator();
                    if let Some(line) = self.repl.ui(ui, &words) {
                        run_lines.push(line);
                    }
                });
            }
            // Document tab strip: one tab per open editor and figure, each
            // with a ✕ close button (MATLAB-style document bar).
            let docs = self.docs.clone();
            let mut close_doc: Option<usize> = None;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (i, d) in docs.iter().enumerate() {
                    if matches!(d, Doc::Edit(_)) && !self.show_editor {
                        continue;
                    }
                    let title = self.doc_title(*d);
                    let resp = ui.selectable_label(self.doc_sel == i, title);
                    if resp.clicked() {
                        self.doc_sel = i;
                        if matches!(d, Doc::Fig(_)) {
                            self.fig_scene = None;
                        }
                    }
                    if ui.small_button("x").on_hover_text("close tab").clicked() {
                        close_doc = Some(i);
                    }
                    ui.separator();
                }
            });
            ui.separator();
            if let Some(i) = close_doc {
                if let Some(Doc::Fig(f)) = docs.get(i) {
                    self.fig_docs_closed.insert(*f);
                }
                self.docs.remove(i);
                self.doc_sel = self.doc_sel.saturating_sub(if i <= self.doc_sel { 1 } else { 0 }).min(self.docs.len().saturating_sub(1));
            }
            match docs.get(self.doc_sel) {
                Some(Doc::Edit(ei)) if self.show_editor => {
                    let ei = *ei;
                    if let Some(e) = self.editors.get_mut(ei) {
                        if let Some(EditorCmd::Run { source, dir }) = e.ui(ui) {
                            self.run_editor_buffer(source, dir);
                        }
                    }
                }
                Some(Doc::Fig(fi)) => self.figure_view(ui, *fi),
                _ => {
                    ui.centered_and_justified(|ui| {
                        ui.weak("no open documents — New Script or plot(1:10)");
                    });
                }
            }
        });

        // Workspace "Rename…" dialog.
        if let Some(name) = self.ws_rename.clone() {
            let mut open = true;
            let mut apply = false;
            let mut cancel = false;
            egui::Window::new(format!("Rename {name}"))
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ui.ctx(), |ui| {
                    ui.horizontal(|ui| {
                        ui.label("new name:");
                        let r = ui.add(egui::TextEdit::singleline(&mut self.ws_rename_buf).desired_width(180.0));
                        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            apply = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui.button("OK").clicked() {
                            apply = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
            if apply {
                let new = self.ws_rename_buf.trim().to_string();
                if !new.is_empty() && new != name {
                    run_lines.push(format!("{new} = {name}; clear {name}"));
                }
            }
            if apply || !open || cancel {
                self.ws_rename = None;
            }
        }

        // Variable Editor window (Workspace double-click / context menu).
        self.variable_editor(ui.ctx());

        // Deferred runs (single borrow of self per line).
        for d in cd_dirs {
            self.cd_to(&d);
        }
        for line in run_lines {
            self.run_line(&line);
        }
    }
}
