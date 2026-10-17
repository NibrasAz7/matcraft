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
mod findfiles;
mod history;
mod repl;
mod ribbon;
mod workspace;

use std::sync::mpsc::Receiver;
use std::sync::Arc;

use serde_json::{json, Value as Json};

use control::ControlRequest;
use editor::{Editor, EditorCmd};
use figure::Rasterizer;
use files::FileCmd;
use findfiles::FindFiles;
use history::HistCmd;
use repl::Repl;
use ribbon::{RibbonCmd, Tab as RibbonTab};
use workspace::WsCmd;

/// Persisted desktop state — panel toggles, zoom, working directory — held
/// in `config.toml` via `craft-settings` (atomic writes, a backup of the
/// previous file, damaged-file recovery and portable mode). The field names
/// match the legacy `layout.json` keys and a [`craft_settings::LegacyFile`]
/// imports that JSON file on first run, so existing installs keep their
/// layout.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct MatLayout {
    pub show_left: bool,
    pub show_right: bool,
    pub show_cmdwin: bool,
    pub show_editor: bool,
    pub zoom: f32,
    pub cwd: String,
}

impl Default for MatLayout {
    fn default() -> Self {
        MatLayout { show_left: true, show_right: true, show_cmdwin: true, show_editor: true, zoom: 1.0, cwd: String::new() }
    }
}

impl craft_settings::Settings for MatLayout {
    const APP_ID: &'static str = "matcraft";
    const SCHEMA_VERSION: u32 = 1;
}

/// Crash-safe write: atomic replace via `craft-atomic` on native targets.
/// On wasm32 there is no filesystem — `std::fs::write` errors cleanly,
/// while `craft-atomic`'s temp-name (`std::process::id`) would panic.
pub(crate) fn write_file(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        craft_atomic::atomic_write(path, bytes)
    }
    #[cfg(target_arch = "wasm32")]
    {
        std::fs::write(path, bytes)
    }
}

/// One open document tab in the centre area.
#[derive(Clone, Copy, PartialEq)]
enum Doc {
    /// Index into `MatApp::editors`.
    Edit(usize),
    /// Figure number minus one.
    Fig(usize),
}

/// What the interactive figure view needs from a figure: the axis flags
/// and the resolved limits (axis space — log axes are log10-transformed).
struct XyView {
    xlog: bool,
    ylog: bool,
    yrev: bool,
    lim: (f64, f64, f64, f64),
}

/// Where the interpreter is suspended (mirrored from
/// `matlang::Session::pause_info` by [`MatApp::sync_debug`]). `file` is ""
/// for interactive input; `err` is the caught error on a `dbstop if error`
/// stop.
struct DebugPause {
    file: String,
    line: usize,
    err: Option<String>,
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
    /// Rubber-band anchor while box-zooming a figure axes.
    fig_box_from: Option<egui::Pos2>,
    /// View-menu toggles for the docked panels.
    show_left: bool,
    show_right: bool,
    show_cmdwin: bool,
    /// Command History multi-selection (entry indices).
    hist_sel: std::collections::BTreeSet<usize>,
    /// Workspace rename dialog: the variable being renamed + name field.
    ws_rename: Option<String>,
    ws_rename_buf: String,
    /// Open toolstrip tab (HOME/PLOTS/EDITOR/VIEW).
    ribbon_tab: RibbonTab,
    /// Path-bar navigation: dirs visited back/forward plus recents.
    cd_back: Vec<std::path::PathBuf>,
    cd_fwd: Vec<std::path::PathBuf>,
    cd_recent: std::collections::VecDeque<std::path::PathBuf>,
    /// The Preferences window is open.
    prefs_open: bool,
    /// Find in Files window (Ctrl+Shift+F).
    find_files: FindFiles,
    /// Figures shown as floating windows instead of document tabs.
    fig_popped: std::collections::BTreeSet<usize>,
    /// The debugger's current stop — drives the `K>>` prompt, the debug
    /// toolbar, the status line and the editor's paused-line highlight.
    dbg: Option<DebugPause>,
    /// The unsaved-changes prompt is up (a close request with dirty editors).
    close_prompt: bool,
    /// The user confirmed discarding unsaved buffers on close.
    discard_confirmed: bool,
    /// `config.toml` store for the persisted layout (`MatApp::new` only).
    #[cfg(not(target_arch = "wasm32"))]
    settings_store: Option<craft_settings::SettingsStore<MatLayout>>,
    /// Zoom factor restored from the layout, applied on the first frame.
    want_zoom: Option<f32>,
    /// Last seen interface zoom — saved with the layout on exit.
    last_zoom: f32,
}

impl MatApp {
    /// `eframe::run_native` entry point.
    pub fn new(cc: &eframe::CreationContext<'_>, opts: MatAppOptions) -> Self {
        cc.egui_ctx.set_visuals(ribbon::studio_visuals());
        #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
        let mut app = Self::with_options(opts);
        #[cfg(not(target_arch = "wasm32"))]
        app.restore_layout();
        app
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
            fig_box_from: None,
            show_left: true,
            show_right: true,
            show_cmdwin: true,
            hist_sel: std::collections::BTreeSet::new(),
            ws_rename: None,
            ws_rename_buf: String::new(),
            ribbon_tab: RibbonTab::Home,
            cd_back: Vec::new(),
            cd_fwd: Vec::new(),
            cd_recent: std::collections::VecDeque::new(),
            prefs_open: false,
            find_files: FindFiles::default(),
            fig_popped: std::collections::BTreeSet::new(),
            dbg: None,
            close_prompt: false,
            discard_confirmed: false,
            #[cfg(not(target_arch = "wasm32"))]
            settings_store: None,
            want_zoom: None,
            last_zoom: 1.0,
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
        // MATLAB surfaces the EDITOR toolstrip tab when an editor is raised.
        if matches!(doc, Doc::Edit(_)) {
            self.ribbon_tab = RibbonTab::Editor;
        }
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

    /// Open `path` and jump to a 1-based line (error links, Find in Files).
    fn open_editor_at(&mut self, path: &std::path::Path, line: usize) {
        self.open_editor(path);
        if line > 0 {
            if let Some(e) = self.cur_editor_mut() {
                e.want_line = Some(line);
            }
        }
    }

    /// A scrollback file link was clicked: resolve `file` (absolute, cwd or
    /// the function search path) and open it at `line` (0 = no line).
    fn open_link(&mut self, file: &str, line: usize) {
        let p = std::path::Path::new(file);
        let resolved = if p.is_absolute() || p.exists() {
            p.to_path_buf()
        } else {
            std::env::current_dir()
                .ok()
                .map(|d| d.join(p))
                .filter(|d| d.exists())
                .or_else(|| self.session.lang().paths().iter().map(|d| d.join(p)).find(|d| d.exists()))
                .unwrap_or_else(|| p.to_path_buf())
        };
        self.open_editor_at(&resolved, line);
    }

    /// A tab's display name: file name, `untitled n`, or `Figure n`.
    fn doc_title(&self, doc: Doc) -> String {
        match doc {
            Doc::Edit(i) => match self.editors.get(i) {
                Some(e) => {
                    let mut t = if !e.path.is_empty() {
                        std::path::Path::new(&e.path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| e.path.clone())
                    } else {
                        let n = self.editors.iter().take(i + 1).filter(|e| e.path.is_empty()).count();
                        if n <= 1 {
                            "untitled".to_string()
                        } else {
                            format!("untitled{n}")
                        }
                    };
                    if e.dirty || e.path.is_empty() {
                        t.push('*');
                    }
                    t
                }
                _ => "?".to_string(),
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
            Err(e) => match e {
                // A debug pause unwinds as an error carrying DEBUG_PAUSE_ID —
                // flag it so the UI shows the stop instead of an error line.
                matcraft_engine::EngineError::Lang(le) if le.id == matlang::DEBUG_PAUSE_ID => {
                    json!({"ok": false, "paused": true, "error": le.message})
                }
                _ => json!({"ok": false, "error": e.to_string()}),
            },
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
        if out.get("paused").and_then(Json::as_bool) == Some(true) {
            // A debug stop, not a failure — the message names where we stopped.
            if let Some(m) = out.get("error").and_then(Json::as_str) {
                self.repl.push(m);
            }
        } else if let Some(o) = out.get("output").and_then(Json::as_str).filter(|s| !s.is_empty()) {
            self.repl.push(o);
        } else if let Some(e) = out.get("error").and_then(Json::as_str) {
            self.repl.push(&format!("Error: {e}"));
        }
        self.fcache.invalidate();
    }

    /// Mirror the interpreter's debug state into the UI — runs every frame
    /// so pauses entered any way (gutter breakpoints, `dbstop`, `dbstep`,
    /// control channel) surface identically: `self.dbg` drives the `K>>`
    /// prompt, toolbar and status line, the stopped file is raised and its
    /// line highlighted, and everything clears when the run resumes/quits.
    fn sync_debug(&mut self) {
        match self.session.lang().pause_info() {
            Some(info) => {
                let changed = self.dbg.as_ref().map(|d| (d.file.as_str(), d.line)) != Some((info.file.as_str(), info.line));
                if changed && !info.file.is_empty() {
                    self.open_editor_at(std::path::Path::new(&info.file), info.line);
                }
                self.dbg = Some(DebugPause { file: info.file.clone(), line: info.line, err: info.error.as_ref().map(|e| e.message.clone()) });
                for e in &mut self.editors {
                    e.pause_line = if !info.file.is_empty() && same_path(&e.path, &info.file) { Some(info.line) } else { None };
                }
            }
            None => {
                if self.dbg.is_some() {
                    self.dbg = None;
                    for e in &mut self.editors {
                        e.pause_line = None;
                    }
                }
            }
        }
    }

    /// A gutter toggle: mirror the mark into the interpreter with
    /// `dbstop`/`dbclear` so the next run honours it. Call syntax (not
    /// `dbstop in f at n`) keeps `C:\…` paths and spaces inside one string
    /// argument.
    fn toggle_breakpoint(&mut self, file: &str, line: usize, set: bool) {
        if file.is_empty() || line == 0 {
            return;
        }
        let esc = file.replace('\'', "''");
        let code = if set { format!("dbstop('in','{esc}','at',{line})") } else { format!("dbclear('in','{esc}','at',{line})") };
        let _ = self.exec_command("eval.line", json!({"code": code}));
    }

    /// `dbcont`/`dbstep` from the debug toolbar: echo the action, drive the
    /// interpreter, surface its output. A pause-id error means still paused
    /// — `sync_debug` picks up the new stop line.
    fn debug_resume(&mut self, step: bool) {
        self.repl.push(if step { ">> dbstep\n" } else { ">> dbcont\n" });
        match if step { self.session.lang_mut().step() } else { self.session.lang_mut().resume() } {
            Ok(out) => {
                if !out.is_empty() {
                    self.repl.push(&out);
                }
            }
            Err(e) if e.id == matlang::DEBUG_PAUSE_ID => self.repl.push(&e.message),
            Err(e) => self.repl.push(&format!("Error: {e}")),
        }
        self.figs_dirty = true;
        self.fcache.invalidate();
        self.sync_debug();
    }

    /// `dbquit` from the debug toolbar — abandon the suspended run.
    fn debug_quit(&mut self) {
        self.repl.push(">> dbquit\n");
        self.session.lang_mut().quit_debug();
        self.sync_debug();
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
                        Some(p) => match write_file(std::path::Path::new(&p), &png) {
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
        self.fig_popped.retain(|i| *i < n);
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
    /// `record` pushes the outgoing dir onto the back stack — false for
    /// back/forward navigation itself.
    fn cd_to(&mut self, dir: &std::path::Path, record: bool) {
        let escaped = dir.display().to_string().replace('\'', "''");
        let out = self.exec_command("eval.line", json!({"code": format!("cd '{escaped}'")}));
        if out.get("ok").and_then(Json::as_bool) == Some(true) {
            if record {
                if let Ok(cur) = std::env::current_dir() {
                    self.cd_back.push(cur);
                }
                self.cd_fwd.clear();
            }
            let d = dir.to_path_buf();
            self.cd_recent.retain(|p| *p != d);
            self.cd_recent.push_front(d);
            self.cd_recent.truncate(12);
            self.file_sel = None;
            self.fcache.invalidate();
        } else if let Some(e) = out.get("error").and_then(Json::as_str) {
            self.repl.push(&format!("Error: {e}"));
        }
    }

    /// The pre-`config.toml` layout file — `%APPDATA%\matcraft\layout.json`
    /// on Windows, `$HOME/.matcraft/layout.json` elsewhere. Imported once by
    /// the settings store's legacy hook; newer sessions never touch it.
    #[cfg(not(target_arch = "wasm32"))]
    fn layout_path() -> Option<std::path::PathBuf> {
        if let Some(d) = std::env::var_os("APPDATA") {
            return Some(std::path::PathBuf::from(d).join("matcraft").join("layout.json"));
        }
        std::env::var_os("HOME").map(|d| std::path::PathBuf::from(d).join(".matcraft").join("layout.json"))
    }

    /// Load the persisted layout through `craft-settings` (`config.toml` in
    /// the platform config dir, or `config/` beside a portable exe) and keep
    /// the store for the save on exit (called from `MatApp::new`).
    #[cfg(not(target_arch = "wasm32"))]
    fn restore_layout(&mut self) {
        let legacy = Self::layout_path()
            .map(|path| craft_settings::LegacyFile::<MatLayout> {
                path,
                // The field names match `layout.json`'s keys, so the old
                // file parses as a `MatLayout` directly.
                parse: |bytes| serde_json::from_slice::<MatLayout>(bytes).map_err(|e| e.to_string()),
            })
            .into_iter()
            .collect();
        let (store, _report) =
            craft_settings::SettingsStore::<MatLayout>::open_with(craft_settings::OpenOptions::new(craft_settings::Location::Auto).legacy(legacy));
        if let Ok(v) = serde_json::to_value(store.get()) {
            self.apply_layout(&v);
        }
        self.settings_store = Some(store);
    }

    /// Apply a parsed layout object (split out so tests need no filesystem).
    /// On wasm there is no persisted layout; only tests call this.
    #[cfg(any(not(target_arch = "wasm32"), test))]
    fn apply_layout(&mut self, v: &Json) {
        for (key, field) in [
            ("show_left", &mut self.show_left),
            ("show_right", &mut self.show_right),
            ("show_cmdwin", &mut self.show_cmdwin),
            ("show_editor", &mut self.show_editor),
        ] {
            if let Some(b) = v.get(key).and_then(Json::as_bool) {
                *field = b;
            }
        }
        self.want_zoom = v.get("zoom").and_then(Json::as_f64).map(|z| z as f32);
        if let Some(d) = v.get("cwd").and_then(Json::as_str) {
            let p = std::path::PathBuf::from(d);
            if p.is_dir() {
                self.cd_to(&p, false);
            }
        }
    }

    /// Evaluate the editor buffer in the session (toolbar ▶ Run and the
    /// editor pane's own Run share this). `path` is echoed like `run 'path'`
    /// so error links in the Command Window point back at the file.
    /// `as_file` runs a clean file-backed buffer through `run 'path'` — the
    /// interpreter then knows which file is executing and `dbstop`
    /// breakpoints can hit (dirty and scratch buffers evaluate the text
    /// directly; there is no file label to stop in).
    fn run_editor_buffer(&mut self, source: String, dir: Option<std::path::PathBuf>, path: Option<String>, as_file: bool) {
        if let Some(d) = dir {
            let _ = self.exec_command("path.add", json!({"dir": d.to_string_lossy()}));
        }
        if as_file {
            if let Some(p) = path.as_deref().filter(|p| std::path::Path::new(p).is_file()) {
                self.run_line(&format!("run '{}'", p.replace('\'', "''")));
                return;
            }
        }
        match &path {
            Some(p) => self.repl.push(&format!(">> run '{}'\n", p.replace('\'', "''"))),
            None => self.repl.push(">> run editor buffer\n"),
        }
        let out = self.exec_command("eval.line", json!({"code": source}));
        self.print_cmd_out(&out);
    }

    /// The currently displayed figure. A plain 2-D axes gets MATLAB-style
    /// data-space interaction — drag pans the limits, scroll zooms at the
    /// cursor, right/Shift-drag box-zooms and double-click restores `axis
    /// auto`, each re-rendering with fresh ticks. Figures that aren't a
    /// single x/y axes (3-D, subplots, pie-only) keep the pixel pan/zoom
    /// `Scene`.
    fn figure_view(&mut self, ui: &mut egui::Ui, idx: usize, popped: bool) {
        let Some(tex) = self.fig_tex.get(idx).and_then(|t| t.as_ref()) else { return };
        let (id, size) = (tex.id(), tex.size_vec2());
        let view = self.session.lang().figures().get(idx).and_then(|f| {
            if matplot::is_xy_view(f) {
                Some(XyView { xlog: f.xlog, ylog: f.ylog, yrev: f.yrev, lim: matplot::view_limits(f) })
            } else {
                None
            }
        });
        if view.is_none() && !matches!(&self.fig_scene, Some((i, _)) if *i == idx) {
            self.fig_scene = Some((idx, egui::Rect::from_min_size(egui::Pos2::ZERO, size)));
        }
        let mut reset = false;
        let mut save_as = false;
        let mut zoom_k = 0.0;
        let mut pop = false;
        let mut dock = false;
        ui.horizontal(|ui| {
            ui.weak(format!("Figure {}", idx + 1));
            ui.separator();
            if ui.small_button("+").on_hover_text("zoom in").clicked() {
                zoom_k = 0.8;
            }
            if ui.small_button("-").on_hover_text("zoom out").clicked() {
                zoom_k = 1.25;
            }
            if ui.small_button("reset").on_hover_text(if view.is_some() { "axis auto — restore auto limits" } else { "reset view" }).clicked() {
                reset = true;
            }
            ui.separator();
            if ui.small_button("Save As…").on_hover_text("saveas — SVG/PNG via extension").clicked() {
                save_as = true;
            }
            if popped {
                if ui.small_button("Dock").on_hover_text("dock back into the document tabs").clicked() {
                    dock = true;
                }
            } else if ui.small_button("Pop Out").on_hover_text("open in a separate window").clicked() {
                pop = true;
            }
            if view.is_some() {
                ui.weak("drag to pan · scroll to zoom · right-drag box zoom · double-click reset");
            } else {
                ui.weak("drag to pan · scroll to zoom");
            }
        });
        ui.separator();
        if pop {
            self.fig_popped.insert(idx);
            return;
        }
        if dock {
            self.fig_popped.remove(&idx);
            return;
        }
        if save_as {
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(p) = rfd::FileDialog::new().add_filter("SVG", &["svg"]).add_filter("PNG", &["png"]).save_file() {
                self.run_line(&format!("saveas('{}')", p.display().to_string().replace('\'', "''")));
            }
        }
        if let Some(v) = view {
            self.figure_xy(ui, idx, id, v, zoom_k, reset);
            return;
        }
        if reset {
            self.fig_scene = Some((idx, egui::Rect::from_min_size(egui::Pos2::ZERO, size)));
        }
        if zoom_k != 0.0 {
            if let Some((_, b)) = &mut self.fig_scene {
                *b = b.scale_from_center(zoom_k as f32);
            }
        }
        let Some((_, bounds)) = &mut self.fig_scene else { return };
        egui::Scene::new().zoom_range(0.05..=20.0).show(ui, bounds, |ui| {
            ui.image(egui::load::SizedTexture::new(id, size));
        });
    }

    /// Interactive single 2-D axes: the texture is fit to the dock and the
    /// axes rectangle is sensed for pan / zoom / box-zoom / hover. New
    /// limits go back through `xlim`/`ylim` so the change is real session
    /// state — `gcf`, `saveas`, scripts and the control channel all see it.
    fn figure_xy(&mut self, ui: &mut egui::Ui, idx: usize, id: egui::TextureId, view: XyView, zoom_k: f64, reset: bool) {
        let XyView { xlog, ylog, yrev, lim } = view;
        let (sw, sh) = matplot::svg_size();
        let (sw, sh) = (sw as f32, sh.max(1.0) as f32);
        let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
        let w = rect.width().min(rect.height() * sw / sh).max(1.0);
        let h = w * sh / sw;
        let img = egui::Rect::from_min_size(rect.center() - egui::vec2(w / 2.0, h / 2.0), egui::vec2(w, h));
        ui.painter().image(id, img, egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1.0, 1.0)), egui::Color32::WHITE);

        // The axes rectangle in display space: SVG-space rect scaled by the
        // fit factor.
        let (ax, ay, aw, ah) = matplot::axes_rect();
        let s = img.width() / sw;
        let axr = egui::Rect::from_min_size(img.min + egui::vec2(ax as f32 * s, ay as f32 * s), egui::vec2(aw as f32 * s, ah as f32 * s));
        if axr.width() < 4.0 || axr.height() < 4.0 {
            return;
        }
        let (x0, x1, y0, y1) = lim;
        let (span_x, span_y) = (x1 - x0, y1 - y0);
        // Display-space position of a drawn (axis-space) point.
        let to_px = |xv: f64, yv: f64| {
            let fx = ((xv - x0) / span_x) as f32;
            let fy = ((yv - y0) / span_y) as f32;
            egui::pos2(axr.left() + fx * axr.width(), if yrev { axr.top() + fy * axr.height() } else { axr.bottom() - fy * axr.height() })
        };
        let to_data = |t: f64, log: bool| if log { 10f64.powf(t) } else { t };

        let resp = ui.interact(axr, ui.id().with(("fig-ax", idx)), egui::Sense::click_and_drag());
        let mut new_lim: Option<(f64, f64, f64, f64)> = None;
        let mut auto = reset;

        if resp.double_clicked() {
            auto = true;
        }
        if zoom_k != 0.0 {
            let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
            new_lim = Some((cx - (cx - x0) * zoom_k, cx + (x1 - cx) * zoom_k, cy - (cy - y0) * zoom_k, cy + (y1 - cy) * zoom_k));
        }
        // Scroll zooms both axes about the cursor — the MATLAB zoom tool /
        // Plotly wheel behaviour.
        if resp.hovered() {
            let d = ui.input(|i| i.smooth_scroll_delta.y);
            if d != 0.0 {
                let k = (f64::from(-d) * 0.002).exp().clamp(0.02, 50.0);
                if let Some(hp) = resp.hover_pos() {
                    let fx = f64::from(hp.x - axr.left()) / f64::from(axr.width());
                    let fy = f64::from(hp.y - axr.top()) / f64::from(axr.height());
                    let xv = x0 + fx * span_x;
                    let yv = if yrev { y0 + fy * span_y } else { y1 - fy * span_y };
                    new_lim = Some((xv - (xv - x0) * k, xv + (x1 - xv) * k, yv - (yv - y0) * k, yv + (y1 - yv) * k));
                }
            }
        }
        // Primary drag pans; Shift+primary or secondary drag box-zooms.
        let shift = ui.input(|i| i.modifiers.shift);
        let box_drag = resp.dragged_by(egui::PointerButton::Secondary) || (resp.dragged_by(egui::PointerButton::Primary) && shift);
        if resp.dragged_by(egui::PointerButton::Primary) && !shift {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            let d = ui.input(|i| i.pointer.delta());
            if d.x != 0.0 || d.y != 0.0 {
                let dx = f64::from(d.x) / f64::from(axr.width()) * span_x;
                let dy = f64::from(d.y) / f64::from(axr.height()) * span_y;
                let ny = if yrev { -dy } else { dy };
                new_lim = Some((x0 - dx, x1 - dx, y0 + ny, y1 + ny));
            }
        } else if box_drag {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ZoomIn);
            if let Some(hp) = resp.interact_pointer_pos() {
                let from = self.fig_box_from.get_or_insert(hp);
                let r = egui::Rect::from_two_pos(*from, hp).intersect(axr);
                ui.painter().rect_stroke(r, 0.0, egui::Stroke::new(1.0, egui::Color32::from_rgb(0x00, 0x72, 0xBD)), egui::StrokeKind::Outside);
            }
        } else if let Some(from) = self.fig_box_from.take() {
            // Released — map the rubber band back to axis limits.
            if let Some(hp) = ui.input(|i| i.pointer.interact_pos().or_else(|| i.pointer.latest_pos())) {
                let r = egui::Rect::from_two_pos(from, hp).intersect(axr);
                if r.width() > 6.0 && r.height() > 6.0 {
                    let fx = |p: f32| f64::from(p - axr.left()) / f64::from(axr.width());
                    let fy = |p: f32| f64::from(p - axr.top()) / f64::from(axr.height());
                    let (xa, xb) = (x0 + fx(r.left()) * span_x, x0 + fx(r.right()) * span_x);
                    let (ya, yb) = if yrev {
                        (y0 + fy(r.top()) * span_y, y0 + fy(r.bottom()) * span_y)
                    } else {
                        (y1 - fy(r.bottom()) * span_y, y1 - fy(r.top()) * span_y)
                    };
                    new_lim = Some((xa.min(xb), xa.max(xb), ya.min(yb), ya.max(yb)));
                }
            }
        }

        // Hover: axis coordinates plus a plotly-style data tip on the
        // nearest vertex within snapping distance.
        if let Some(hp) = resp.hover_pos() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            // Spikelines through the cursor, clipped to the axes.
            let clip = ui.painter().with_clip_rect(axr);
            let spike = egui::Stroke::new(0.75, egui::Color32::from_gray(140));
            clip.hline(axr.x_range(), hp.y, spike);
            clip.vline(hp.x, axr.y_range(), spike);
            let fx = f64::from(hp.x - axr.left()) / f64::from(axr.width());
            let fy = f64::from(hp.y - axr.top()) / f64::from(axr.height());
            let xd = to_data(x0 + fx * span_x, xlog);
            let yd = to_data(if yrev { y0 + fy * span_y } else { y1 - fy * span_y }, ylog);
            let mut text = format!("x: {}\ny: {}", matlang::fmt_num(xd, false), matlang::fmt_num(yd, false));
            let mut marker: Option<egui::Pos2> = None;
            if let Some(fig) = self.session.lang().figures().get(idx) {
                let mut best = f32::INFINITY;
                for s in &fig.series {
                    if !matches!(
                        s.kind,
                        matplot::SeriesKind::Line
                            | matplot::SeriesKind::Scatter
                            | matplot::SeriesKind::Stairs
                            | matplot::SeriesKind::Stem
                            | matplot::SeriesKind::ErrorBar
                    ) {
                        continue;
                    }
                    // Snapping stays interactive: skip enormous series.
                    if s.x.len() > 100_000 {
                        continue;
                    }
                    for (px, py) in s.x.iter().zip(s.y.iter()) {
                        let (txv, tyv) = (if xlog { px.log10() } else { *px }, if ylog { py.log10() } else { *py });
                        if !txv.is_finite() || !tyv.is_finite() {
                            continue;
                        }
                        let pos = to_px(txv, tyv);
                        let d = hp.distance(pos);
                        if d < best {
                            best = d;
                            marker = Some(pos);
                            text = format!("x: {}\ny: {}", matlang::fmt_num(to_data(txv, xlog), false), matlang::fmt_num(to_data(tyv, ylog), false));
                        }
                    }
                }
                if best > 14.0 {
                    marker = None;
                    text = format!("x: {}\ny: {}", matlang::fmt_num(xd, false), matlang::fmt_num(yd, false));
                }
            }
            if let Some(p) = marker {
                ui.painter().circle_stroke(p, 5.0, egui::Stroke::new(1.5, egui::Color32::WHITE));
                ui.painter().circle_stroke(p, 5.0, egui::Stroke::new(1.0, egui::Color32::from_rgb(0x00, 0x72, 0xBD)));
            }
            let galley = ui.painter().layout_no_wrap(text.clone(), egui::FontId::monospace(11.0), egui::Color32::WHITE);
            let mut tp = hp + egui::vec2(14.0, -galley.size().y - 10.0);
            tp.x = tp.x.min(axr.right() - galley.size().x - 6.0);
            tp.y = tp.y.max(axr.top() + 4.0);
            let tr = egui::Rect::from_min_size(tp - egui::vec2(4.0, 4.0), galley.size() + egui::vec2(8.0, 8.0));
            ui.painter().rect_filled(tr, 3.0, egui::Color32::from_black_alpha(200));
            ui.painter().rect_stroke(tr, 3.0, egui::Stroke::new(1.0, egui::Color32::from_gray(120)), egui::StrokeKind::Outside);
            ui.painter().galley(tp, galley, egui::Color32::WHITE);
        }

        if auto {
            let _ = self.exec_command("eval.line", json!({"code": "axis auto"}));
        } else if let Some((a, b, c, d)) = new_lim {
            if [a, b, c, d].iter().all(|v| v.is_finite()) {
                let code =
                    format!("xlim([{:.6e} {:.6e}]); ylim([{:.6e} {:.6e}])", to_data(a, xlog), to_data(b, xlog), to_data(c, ylog), to_data(d, ylog));
                let _ = self.exec_command("eval.line", json!({"code": code}));
            }
        }
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
                matlang::Value::Func(_) => {
                    ui.weak("function handle — read-only (edit by reassigning the variable)");
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

/// Path comparison tolerant of `\` vs `/` — the debugger labels runs with
/// `path.display()`, editors store `to_string_lossy`.
fn same_path(a: &str, b: &str) -> bool {
    a == b || a.replace('\\', "/") == b.replace('\\', "/")
}

impl eframe::App for MatApp {
    /// Non-painting work: control requests, screenshot replies, figure
    /// texture refresh. Runs before `ui`, also while the window is hidden.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_control(ctx);
        self.sync_debug();
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
        self.last_zoom = zoom;
        // Desktop shortcuts: Ctrl+N/O/S/W/F, Ctrl+Shift+F, F5 run — gathered
        // first so the actions can take `self` after the input borrow ends.
        let (new_doc, _open_doc, save_doc, run_doc, close_doc, find, find_files) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::CTRL, egui::Key::N),
                i.consume_key(egui::Modifiers::CTRL, egui::Key::O),
                i.consume_key(egui::Modifiers::CTRL, egui::Key::S),
                i.consume_key(egui::Modifiers::NONE, egui::Key::F5),
                i.consume_key(egui::Modifiers::CTRL, egui::Key::W),
                i.consume_key(egui::Modifiers::CTRL, egui::Key::F),
                i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::F),
            )
        });
        if new_doc {
            self.new_editor_tab(None);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if _open_doc {
            if let Some(p) = rfd::FileDialog::new().add_filter("M-code", &["m"]).pick_file() {
                self.open_editor(&p);
            }
        }
        if save_doc {
            if let Some(e) = self.cur_editor_mut() {
                e.save();
            }
        }
        if run_doc {
            if let Some(e) = self.cur_editor() {
                let dir = std::path::Path::new(&e.path).parent().filter(|p| !p.as_os_str().is_empty()).map(|p| p.to_path_buf());
                let path = (!e.path.is_empty()).then(|| e.path.clone());
                self.run_editor_buffer(e.text.clone(), dir, path, !e.dirty);
            }
        }
        if find && matches!(self.docs.get(self.doc_sel), Some(Doc::Edit(_))) {
            if let Some(e) = self.cur_editor_mut() {
                e.find.open = true;
                e.find.want_focus = true;
            }
        }
        if find_files {
            self.find_files.open = true;
            self.find_files.want_focus = true;
        }
        if close_doc {
            if let Some(Doc::Fig(f)) = self.docs.get(self.doc_sel).copied() {
                self.fig_docs_closed.insert(f);
            }
            if !self.docs.is_empty() {
                self.docs.remove(self.doc_sel);
                self.doc_sel = self.doc_sel.min(self.docs.len().saturating_sub(1));
            }
        }
        // A close request with unsaved editor buffers asks first — the
        // window stays open and the prompt renders in `ui`.
        if ctx.input(|i| i.viewport().close_requested()) && self.editors.iter().any(|e| e.dirty) && !self.discard_confirmed {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_prompt = true;
        }
        // Last, so it sees our `CancelClose`: where the platform has its own
        // quit (macOS) the window close is routed through it.
        craft_quit::on_close_request(ctx);
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
        if let Some(z) = self.want_zoom.take() {
            ui.ctx().set_zoom_factor(z);
        }
        let mut run_lines: Vec<String> = Vec::new();
        // (dir, record-in-back-history) — back/forward navigation passes false.
        let mut cd_dirs: Vec<(std::path::PathBuf, bool)> = Vec::new();
        // Scrollback file links clicked this frame.
        let mut link_open: Option<(String, usize)> = None;
        // Figure tab asked to pop out into a floating window.
        let mut pop_fig: Option<usize> = None;
        // Debug toolbar clicks — Continue/Step/Quit while paused.
        let (mut dbg_cont, mut dbg_step, mut dbg_quit) = (false, false, false);

        // ---- toolstrip: tabbed ribbon (HOME/PLOTS/EDITOR/VIEW) -----------
        egui::Panel::top("toolstrip").show(ui, |ui| {
            let has_editor = matches!(self.docs.get(self.doc_sel), Some(Doc::Edit(_))) && self.show_editor;
            let rc = ribbon::ui(
                ui,
                &mut self.ribbon_tab,
                self.ws_sel.as_deref(),
                has_editor,
                &mut ribbon::ViewFlags {
                    left: &mut self.show_left,
                    right: &mut self.show_right,
                    cmdwin: &mut self.show_cmdwin,
                    editor: &mut self.show_editor,
                },
            );
            match rc {
                Some(RibbonCmd::NewScript) => self.new_editor_tab(None),
                Some(RibbonCmd::Open) =>
                {
                    #[cfg(not(target_arch = "wasm32"))]
                    if let Some(p) = rfd::FileDialog::new().add_filter("M-code", &["m"]).pick_file() {
                        self.open_editor(&p);
                    }
                }
                Some(RibbonCmd::Save) => {
                    if let Some(e) = self.cur_editor_mut() {
                        e.save();
                    }
                }
                Some(RibbonCmd::Run) => {
                    if let Some(e) = self.cur_editor() {
                        let dir = std::path::Path::new(&e.path).parent().filter(|p| !p.as_os_str().is_empty()).map(|p| p.to_path_buf());
                        let src = e.text.clone();
                        let path = (!e.path.is_empty()).then(|| e.path.clone());
                        self.run_editor_buffer(src, dir, path, !e.dirty);
                    }
                }
                Some(RibbonCmd::RunSelection) => {
                    if let Some(e) = self.cur_editor_mut() {
                        e.want_run_selection = true;
                    }
                }
                Some(RibbonCmd::ClearWorkspace) => run_lines.push("clear".to_string()),
                Some(RibbonCmd::SaveWorkspace) =>
                {
                    #[cfg(not(target_arch = "wasm32"))]
                    if let Some(p) = rfd::FileDialog::new().add_filter("MAT v5", &["mat"]).save_file() {
                        let esc = p.display().to_string().replace('\'', "''");
                        run_lines.push(format!("save '{esc}'"));
                    }
                }
                Some(RibbonCmd::OpenVar) => {
                    if let Some(n) = self.ws_sel.clone() {
                        self.var_editor = Some(n);
                        self.var_edits.clear();
                    }
                }
                Some(RibbonCmd::Eval(line)) => run_lines.push(line),
                Some(RibbonCmd::Insert(s)) => self.repl.load_input(&s),
                Some(RibbonCmd::Preferences) => self.prefs_open = true,
                Some(RibbonCmd::Find) => {
                    if let Some(e) = self.cur_editor_mut() {
                        e.find.open = true;
                        e.find.want_focus = true;
                    }
                }
                Some(RibbonCmd::FindFiles) => {
                    self.find_files.open = true;
                    self.find_files.want_focus = true;
                }
                None => {}
            }
            // Path bar — back/forward, up, recent dirs, editable breadcrumb.
            ui.horizontal(|ui| {
                if ui.add_enabled(!self.cd_back.is_empty(), egui::Button::new("◀").small()).on_hover_text("previous folder").clicked() {
                    if let Some(d) = self.cd_back.pop() {
                        self.cd_fwd.push(cwd.clone());
                        cd_dirs.push((d, false));
                    }
                }
                if ui.add_enabled(!self.cd_fwd.is_empty(), egui::Button::new("▶").small()).on_hover_text("next folder").clicked() {
                    if let Some(d) = self.cd_fwd.pop() {
                        self.cd_back.push(cwd.clone());
                        cd_dirs.push((d, false));
                    }
                }
                if ui.small_button("↑").on_hover_text("up one level").clicked() {
                    if let Some(p) = cwd.parent() {
                        cd_dirs.push((p.to_path_buf(), true));
                    }
                }
                ui.menu_button("⌄", |ui| {
                    ui.set_min_width(280.0);
                    if self.cd_recent.is_empty() {
                        ui.weak("(no recent folders)");
                    }
                    let mut pick = None;
                    for d in &self.cd_recent {
                        if ui.button(d.display().to_string()).clicked() {
                            pick = Some(d.clone());
                            ui.close();
                        }
                    }
                    if let Some(d) = pick {
                        cd_dirs.push((d, true));
                    }
                });
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
                        cd_dirs.push((std::path::PathBuf::from(t), true));
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
                    Some(FileCmd::Cd(d)) => cd_dirs.push((d, true)),
                    Some(FileCmd::Open(p)) => self.open_editor(&p),
                    Some(FileCmd::Run(p)) => {
                        if let Ok(src) = std::fs::read_to_string(&p) {
                            let path = p.to_string_lossy().to_string();
                            self.run_editor_buffer(src, p.parent().map(|d| d.to_path_buf()), Some(path), true);
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
                    Some(WsCmd::SaveAs(_n)) =>
                    {
                        #[cfg(not(target_arch = "wasm32"))]
                        if let Some(p) = rfd::FileDialog::new().add_filter("MAT v5", &["mat"]).save_file() {
                            let esc = p.display().to_string().replace('\'', "''");
                            run_lines.push(format!("save '{esc}' {_n}"));
                        }
                    }
                    None => {}
                }
            });
        }

        // ---- status bar ---------------------------------------------------
        egui::Panel::bottom("status").exact_size(22.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                match &self.dbg {
                    Some(d) => ui.weak(match &d.err {
                        Some(m) => format!("Stopped on error: {m}"),
                        None if d.file.is_empty() => format!("Paused at line {}", d.line),
                        None => format!("Paused at {}:{}", d.file, d.line),
                    }),
                    None => ui.weak("Ready"),
                };
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
                        if self.dbg.is_some() {
                            ui.separator();
                            if ui.small_button("Continue").on_hover_text("dbcont — resume until the next breakpoint").clicked() {
                                dbg_cont = true;
                            }
                            if ui.small_button("Step").on_hover_text("dbstep — run the next statement, then pause").clicked() {
                                dbg_step = true;
                            }
                            if ui.small_button("Quit").on_hover_text("dbquit — abandon the paused run").clicked() {
                                dbg_quit = true;
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(egui::RichText::new("fx").monospace().weak());
                        });
                    });
                    ui.separator();
                    let rout = self.repl.ui(ui, &words, self.dbg.is_some());
                    if let Some(line) = rout.line {
                        run_lines.push(line);
                    }
                    if let Some((file, line)) = rout.open {
                        link_open = Some((file, line));
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
                    if self.doc_sel == i {
                        // Accent underline under the raised tab, like the
                        // desktop's document bar.
                        let r = resp.rect;
                        ui.painter().hline(r.left() + 2.0..=r.right() - 2.0, r.bottom(), egui::Stroke::new(2.0, ribbon::ACCENT));
                    }
                    if resp.clicked() {
                        self.doc_sel = i;
                        if matches!(d, Doc::Fig(_)) {
                            self.fig_scene = None;
                        }
                    }
                    if matches!(d, Doc::Fig(_)) && ui.small_button("↗").on_hover_text("pop out to a separate window").clicked() {
                        pop_fig = Some(i);
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
            if let Some(i) = pop_fig {
                if let Some(Doc::Fig(f)) = docs.get(i) {
                    self.fig_popped.insert(*f);
                }
            }
            match docs.get(self.doc_sel) {
                Some(Doc::Edit(ei)) if self.show_editor => {
                    let ei = *ei;
                    if let Some(e) = self.editors.get_mut(ei) {
                        let cmds = e.ui(ui, ei);
                        let epath = e.path.clone();
                        for c in cmds {
                            match c {
                                EditorCmd::Run { source, dir, path, as_file } => self.run_editor_buffer(source, dir, path, as_file),
                                EditorCmd::BpToggle { line, set } => self.toggle_breakpoint(&epath, line, set),
                                EditorCmd::RunSelection => {}
                            }
                        }
                    }
                }
                Some(Doc::Fig(fi)) if self.fig_popped.contains(fi) => {
                    ui.centered_and_justified(|ui| {
                        ui.weak(format!("Figure {} is undocked — close or Dock its window to return it here", fi + 1));
                    });
                }
                Some(Doc::Fig(fi)) => self.figure_view(ui, *fi, false),
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

        // Preferences window (HOME ▸ ENVIRONMENT).
        if self.prefs_open {
            let mut open = true;
            egui::Window::new("Preferences")
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ui.ctx(), |ui| {
                    ui.strong("Desktop");
                    ui.separator();
                    let mut zoom = ui.ctx().zoom_factor();
                    ui.horizontal(|ui| {
                        ui.label("Interface zoom");
                        if ui.add(egui::Slider::new(&mut zoom, 0.5..=3.0).suffix("x")).changed() {
                            ui.ctx().set_zoom_factor(zoom);
                        }
                    });
                    ui.separator();
                    if ui.button("Restore default layout").clicked() {
                        self.show_left = true;
                        self.show_right = true;
                        self.show_cmdwin = true;
                        self.show_editor = true;
                        ui.ctx().set_zoom_factor(1.0);
                    }
                    ui.weak("Pane visibility lives on the VIEW tab; per-session for now.");
                });
            if !open {
                self.prefs_open = false;
            }
        }

        // Variable Editor window (Workspace double-click / context menu).
        self.variable_editor(ui.ctx());

        // Undocked figure windows (Pop Out / "↗" on the doc tab). Closing
        // the window docks the figure back into its tab.
        let popped: Vec<usize> = self.fig_popped.iter().copied().collect();
        for i in popped {
            if i >= self.fig_tex.len() {
                self.fig_popped.remove(&i);
                continue;
            }
            let mut open = true;
            egui::Window::new(format!("Figure {}", i + 1)).open(&mut open).default_size([620.0, 460.0]).resizable(true).show(ui.ctx(), |ui| {
                self.figure_view(ui, i, true);
            });
            if !open {
                self.fig_popped.remove(&i);
            }
        }

        // Find in Files window (Ctrl+Shift+F, EDITOR ▸ FIND).
        if self.find_files.open {
            let mut open = true;
            let mut hit = None;
            egui::Window::new("Find in Files").open(&mut open).default_size([440.0, 300.0]).show(ui.ctx(), |ui| {
                hit = findfiles::ui(ui, &mut self.find_files, &cwd);
            });
            if !open {
                self.find_files.open = false;
            }
            if let Some((p, line)) = hit {
                self.open_editor_at(&p, line);
            }
        }

        // Unsaved-changes prompt — raised from `logic` on a close request.
        if self.close_prompt {
            let mut open = true;
            let mut save = false;
            let mut discard = false;
            let mut cancel = false;
            egui::Window::new("Unsaved changes")
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ui.ctx(), |ui| {
                    let n = self.editors.iter().filter(|e| e.dirty).count();
                    ui.label(format!("{n} editor buffer(s) have unsaved changes. Save before closing?"));
                    ui.horizontal(|ui| {
                        if ui.button("Save All").clicked() {
                            save = true;
                        }
                        if ui.button("Don't Save").clicked() {
                            discard = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
            if save {
                for e in &mut self.editors {
                    if e.dirty {
                        e.save();
                    }
                }
                // Close only once every buffer saved — a failed or cancelled
                // save keeps the app open with the prompt still up.
                if self.editors.iter().all(|e| !e.dirty) {
                    self.close_prompt = false;
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            if discard {
                self.discard_confirmed = true;
                self.close_prompt = false;
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
            if !open || cancel {
                self.close_prompt = false;
            }
        }

        // Deferred runs (single borrow of self per line).
        if let Some((file, line)) = link_open {
            self.open_link(&file, line);
        }
        for (d, record) in cd_dirs {
            self.cd_to(&d, record);
        }
        for line in run_lines {
            self.run_line(&line);
        }
        if dbg_cont {
            self.debug_resume(false);
        }
        if dbg_step {
            self.debug_resume(true);
        }
        if dbg_quit {
            self.debug_quit();
        }
    }
}

/// Persist the desktop layout — panel toggles, zoom, working directory —
/// through the settings store (atomic write with a backup of the previous
/// file) so the next session restores it. Only the real eframe app opens a
/// store (`MatApp::new`), so headless tests never write.
#[cfg(not(target_arch = "wasm32"))]
impl Drop for MatApp {
    fn drop(&mut self) {
        let Some(store) = &mut self.settings_store else { return };
        let cwd = std::env::current_dir().unwrap_or_default();
        store.set(MatLayout {
            show_left: self.show_left,
            show_right: self.show_right,
            show_cmdwin: self.show_cmdwin,
            show_editor: self.show_editor,
            zoom: self.last_zoom,
            cwd: cwd.display().to_string(),
        });
        let _ = store.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_layout_restores_toggles_and_zoom() {
        let mut app = MatApp::with_options(MatAppOptions::default());
        app.apply_layout(&json!({"show_left": false, "show_cmdwin": false, "zoom": 1.5}));
        assert!(!app.show_left);
        assert!(!app.show_cmdwin);
        assert!(app.show_right);
        assert_eq!(app.want_zoom, Some(1.5));
    }

    #[test]
    fn open_editor_at_raises_tab_and_marks_line() {
        let mut app = MatApp::with_options(MatAppOptions::default());
        let tmp = std::env::temp_dir().join("matcraft_open_at_test.m");
        std::fs::write(&tmp, "x = 1;\ny = 2;\n").unwrap();
        app.open_editor_at(&tmp, 2);
        let doc = app.docs.get(app.doc_sel).copied();
        let ei = match doc {
            Some(Doc::Edit(i)) => i,
            _ => panic!("expected an editor doc"),
        };
        assert_eq!(app.editors.get(ei).map(|e| e.want_line), Some(Some(2)));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn open_link_resolves_against_search_path() {
        let mut app = MatApp::with_options(MatAppOptions::default());
        let dir = std::env::temp_dir().join("matcraft_linkpath_test");
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("myfun.m");
        std::fs::write(&f, "function y = myfun(x)\ny = x;\n").unwrap();
        app.session.lang_mut().add_path(dir.clone());
        app.open_link("myfun.m", 2);
        let doc = app.docs.get(app.doc_sel).copied();
        let ei = match doc {
            Some(Doc::Edit(i)) => i,
            _ => panic!("expected an editor doc"),
        };
        let e = app.editors.get(ei);
        assert_eq!(e.map(|e| e.path.as_str()), Some(f.to_string_lossy().as_ref()));
        assert_eq!(e.map(|e| e.want_line), Some(Some(2)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn figure_popout_syncs_with_tab() {
        let mut app = MatApp::with_options(MatAppOptions::default());
        let _ = app.exec_command("eval.line", json!({"code": "plot(1:5)"}));
        app.rebuild_figures(&egui::Context::default());
        assert!(app.docs.iter().any(|d| matches!(d, Doc::Fig(0))));
        app.fig_popped.insert(0);
        // "Closing" the figure removes the tab AND the popout together.
        let _ = app.exec_command("eval.line", json!({"code": "close all"}));
        app.rebuild_figures(&egui::Context::default());
        assert!(app.fig_popped.is_empty());
    }

    /// A scratch `.m` dir unique to this test (parallel tests share the pid).
    fn dbg_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("matcraft_uidbg_{tag}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn breakpoint_toggle_registers_dbstop_and_file_run_pauses() {
        let mut app = MatApp::with_options(MatAppOptions::default());
        let dir = dbg_dir("bp");
        let f = dir.join("brk.m");
        std::fs::write(&f, "a = 1;\nb = 2;\nc = 3;\n").unwrap();
        let fp = f.to_string_lossy().to_string();
        // A gutter toggle registers `dbstop` on the session.
        app.toggle_breakpoint(&fp, 2, true);
        assert!(app.session.lang().breakpoints().iter().any(|b| b.line == Some(2)));
        // Running the file pauses — the reply is flagged, not an error.
        let out = app.exec_command("eval.line", json!({"code": format!("run '{fp}'")}));
        assert_eq!(out["ok"], json!(false));
        assert_eq!(out["paused"], json!(true));
        assert!(app.session.lang().is_paused());
        app.sync_debug();
        assert_eq!(app.dbg.as_ref().map(|d| d.line), Some(2));
        // The paused file opens with its line highlighted.
        assert!(app.editors.iter().any(|e| e.path == fp && e.pause_line == Some(2)));
        // dbstep advances to line 3; dbquit ends the session.
        app.debug_resume(true);
        assert_eq!(app.dbg.as_ref().map(|d| d.line), Some(3));
        app.debug_quit();
        assert!(app.dbg.is_none());
        assert!(!app.session.lang().is_paused());
        assert!(app.editors.iter().all(|e| e.pause_line.is_none()));
        // dbclear removes the registration.
        app.toggle_breakpoint(&fp, 2, false);
        assert!(app.session.lang().breakpoints().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pause_via_run_line_shows_message_not_error() {
        let mut app = MatApp::with_options(MatAppOptions::default());
        let dir = dbg_dir("stoperr");
        let f = dir.join("bad.m");
        std::fs::write(&f, "a = 1;\nerror('boom');\nb = 2;\n").unwrap();
        let fp = f.to_string_lossy().to_string();
        app.run_line("dbstop if error");
        app.run_line(&format!("run '{fp}'"));
        assert!(app.repl.scrollback.contains("Paused on error at"), "{}", app.repl.scrollback);
        assert!(!app.repl.scrollback.contains("Error: Paused"));
        app.sync_debug();
        assert_eq!(app.dbg.as_ref().and_then(|d| d.err.as_deref()), Some("boom"));
        // dbcont re-raises the caught error and ends the pause.
        app.debug_resume(false);
        assert!(app.dbg.is_none());
        assert!(app.repl.scrollback.contains("Error:"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn editor_file_run_enters_pause_mode() {
        let mut app = MatApp::with_options(MatAppOptions::default());
        let dir = dbg_dir("filerun");
        let f = dir.join("step.m");
        std::fs::write(&f, "a = 1;\nb = 2;\n").unwrap();
        let fp = f.to_string_lossy().to_string();
        app.toggle_breakpoint(&fp, 1, true);
        app.run_editor_buffer(String::new(), None, Some(fp), true);
        // Echoed like `run 'path'` and paused at line 1.
        assert!(app.repl.scrollback.contains(">> run '"));
        assert!(app.session.lang().is_paused());
        app.sync_debug();
        assert_eq!(app.dbg.as_ref().map(|d| d.line), Some(1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn editor_run_echo_leaves_a_file_link() {
        let mut app = MatApp::with_options(MatAppOptions::default());
        let tmp = std::env::temp_dir().join("matcraft_echo_link_test.m");
        std::fs::write(&tmp, "x = 1;\n").unwrap();
        app.run_editor_buffer("no_such_fn(1);".to_string(), None, Some(tmp.to_string_lossy().to_string()), false);
        assert!(app.repl.scrollback.contains(">> run '"));
        // The error's bare `line 1` links back to the echoed file.
        let links = repl::find_links(&app.repl.scrollback);
        assert!(links.iter().any(|l| l.file.ends_with("matcraft_echo_link_test.m") && l.line == 1));
        let _ = std::fs::remove_file(&tmp);
    }
}
