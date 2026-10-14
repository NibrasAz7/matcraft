//! Current Folder browser and the Details pane under it.

use std::path::{Path, PathBuf};

/// What the file browser asks the app to do.
pub enum FileCmd {
    /// `cd` into a directory (double-clicked folder).
    Cd(PathBuf),
    /// Open a file in the editor (double-clicked `.m`, New Script).
    Open(PathBuf),
    /// Run a `.m` file in the session (context menu).
    Run(PathBuf),
}

/// One row in the Current Folder listing.
#[derive(Clone)]
struct Entry {
    path: PathBuf,
    name: String,
    dir: bool,
    size: u64,
    /// Seconds since the unix epoch (0 when unknown).
    modified: u64,
}

/// Sort key for the listing — directories always group first.
#[derive(Default, Clone, Copy, PartialEq)]
enum Sort {
    #[default]
    Name,
    Size,
    Modified,
    Type,
}

impl Sort {
    fn label(self) -> &'static str {
        match self {
            Sort::Name => "name",
            Sort::Size => "size",
            Sort::Modified => "date",
            Sort::Type => "type",
        }
    }

    fn cycle(self) -> Self {
        match self {
            Sort::Name => Sort::Size,
            Sort::Size => Sort::Modified,
            Sort::Modified => Sort::Type,
            Sort::Type => Sort::Name,
        }
    }
}

/// A pending file-operation dialog.
enum PendingOp {
    /// New script file in the cwd (name without extension gets `.m`).
    NewScript,
    /// New folder in the cwd.
    NewFolder,
    /// Rename this path to `op_text`.
    Rename(PathBuf),
}

/// List `dir` — directories first; the rest sorted by `sort`.
fn list_dir(dir: &Path, sort: Sort) -> Vec<Entry> {
    let mut out: Vec<Entry> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| {
                    let meta = e.metadata().ok();
                    Entry {
                        path: e.path(),
                        name: e.file_name().to_string_lossy().to_string(),
                        dir: meta.as_ref().map(|m| m.is_dir()).unwrap_or(false),
                        size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
                        modified: meta
                            .and_then(|m| m.modified().ok())
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_secs())
                            .unwrap_or(0),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| {
        b.dir.cmp(&a.dir).then_with(|| match sort {
            Sort::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            Sort::Size => a.size.cmp(&b.size).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
            Sort::Modified => b.modified.cmp(&a.modified),
            Sort::Type => {
                let ext = |e: &Entry| e.path.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
                ext(a).cmp(&ext(b)).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            }
        })
    });
    out
}

/// Cached filesystem reads for the file panels. `read_dir`, `metadata` and
/// `read_to_string` every frame were the main cost while dragging panel
/// borders — refresh only when the cwd changes, when a command may have
/// written files (`invalidate`), or on ⟳.
#[derive(Default)]
pub struct FileCache {
    cwd: PathBuf,
    entries: Vec<Entry>,
    detail_key: Option<PathBuf>,
    detail_meta: String,
    detail_preview: Option<String>,
    dir_dirty: bool,
    detail_dirty: bool,
    /// Substring filter for the listing (the search box).
    filter: String,
    sort: Sort,
    /// An open New/Rename dialog and its text field.
    pending: Option<PendingOp>,
    op_text: String,
    /// Path awaiting delete confirmation.
    confirming_delete: Option<PathBuf>,
}

impl FileCache {
    /// Mark both caches stale (a command may have created/deleted files).
    pub fn invalidate(&mut self) {
        self.dir_dirty = true;
        self.detail_dirty = true;
    }

    fn entries(&mut self, cwd: &Path) -> &[Entry] {
        if self.dir_dirty || self.cwd != cwd {
            self.entries = list_dir(cwd, self.sort);
            self.cwd = cwd.to_path_buf();
            self.dir_dirty = false;
        }
        &self.entries
    }

    fn resort(&mut self, sort: Sort) {
        if self.sort != sort {
            self.sort = sort;
            self.entries = list_dir(&self.cwd, sort);
        }
    }

    fn detail(&mut self, path: Option<&Path>) -> (&str, Option<&str>) {
        if self.detail_dirty || self.detail_key.as_deref() != path {
            self.detail_key = path.map(Path::to_path_buf);
            self.detail_meta = String::new();
            self.detail_preview = None;
            if let Some(p) = path {
                match std::fs::metadata(p) {
                    Ok(m) => {
                        self.detail_meta = if m.is_file() { format!("{} bytes", m.len()) } else { String::new() };
                    }
                    Err(_) => {
                        self.detail_meta = "(file no longer exists)".to_string();
                    }
                }
                if p.extension().is_some_and(|x| x == "m" || x == "txt" || x == "md") {
                    if let Ok(text) = std::fs::read_to_string(p) {
                        self.detail_preview = Some(text.lines().take(60).collect::<Vec<_>>().join("\n"));
                    }
                }
            }
            self.detail_dirty = false;
        }
        (&self.detail_meta, self.detail_preview.as_deref())
    }
}

/// Current Folder panel: directory listing of the session's working directory.
/// `sel` is the file previewed in the Details pane.
pub fn folder_ui(ui: &mut egui::Ui, cache: &mut FileCache, cwd: &Path, sel: &mut Option<PathBuf>) -> Option<FileCmd> {
    let mut cmd = None;
    // Title on its own row — the toolbar row below would otherwise squeeze
    // "Current Folder" into "Current Fold" at the default panel width.
    ui.label(egui::RichText::new("Current Folder").strong());
    ui.horizontal(|ui| {
        ui.menu_button("New", |ui| {
            if ui.button("New Script").clicked() {
                cache.pending = Some(PendingOp::NewScript);
                cache.op_text.clear();
                ui.close();
            }
            if ui.button("New Folder").clicked() {
                cache.pending = Some(PendingOp::NewFolder);
                cache.op_text.clear();
                ui.close();
            }
        });
        let sort = cache.sort;
        if ui.small_button(format!("sort: {}", sort.label())).on_hover_text("sort — click to cycle name/size/date/type").clicked() {
            cache.resort(sort.cycle());
        }
        if ui.small_button("⟳").on_hover_text("refresh").clicked() {
            cache.invalidate();
        }
    });
    ui.add(egui::TextEdit::singleline(&mut cache.filter).desired_width(f32::INFINITY).hint_text("search files"));
    ui.separator();
    let filter = cache.filter.to_lowercase();
    // Snapshot the listing — context menus mutate the cache (dialogs).
    let entries: Vec<Entry> = cache.entries(cwd).to_vec();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        // Parent entry.
        if filter.is_empty() {
            if let Some(parent) = cwd.parent() {
                let resp = ui.selectable_label(false, egui::RichText::new("[..]").monospace());
                if resp.double_clicked() {
                    cmd = Some(FileCmd::Cd(parent.to_path_buf()));
                }
            }
        }
        for e in &entries {
            if !filter.is_empty() && !e.name.to_lowercase().contains(&filter) {
                continue;
            }
            let selected = sel.as_ref() == Some(&e.path);
            let text = if e.dir { egui::RichText::new(format!("{}/", e.name)).strong() } else { egui::RichText::new(&e.name) };
            let resp = ui.selectable_label(selected, text);
            if resp.clicked() {
                *sel = Some(e.path.clone());
            }
            if resp.double_clicked() {
                if e.dir {
                    cmd = Some(FileCmd::Cd(e.path.clone()));
                } else if e.path.extension().is_some_and(|x| x == "m") {
                    cmd = Some(FileCmd::Open(e.path.clone()));
                }
            }
            let resp = resp.on_hover_text(if e.dir { e.path.display().to_string() } else { format!("{} — {} bytes", e.path.display(), e.size) });
            let is_m = e.path.extension().is_some_and(|x| x == "m");
            resp.context_menu(|ui| {
                if e.dir {
                    if ui.button("Open folder").clicked() {
                        cmd = Some(FileCmd::Cd(e.path.clone()));
                        ui.close();
                    }
                } else {
                    if ui.button("Open").clicked() {
                        cmd = Some(FileCmd::Open(e.path.clone()));
                        ui.close();
                    }
                    if is_m && ui.button("Run").clicked() {
                        cmd = Some(FileCmd::Run(e.path.clone()));
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button("Rename…").clicked() {
                    cache.pending = Some(PendingOp::Rename(e.path.clone()));
                    cache.op_text = e.name.clone();
                    ui.close();
                }
                if ui.button("Delete…").clicked() {
                    cache.confirming_delete = Some(e.path.clone());
                    ui.close();
                }
                ui.separator();
                if ui.button("Copy path").clicked() {
                    ui.ctx().copy_text(e.path.display().to_string());
                    ui.close();
                }
            });
        }
        if entries.is_empty() || (!filter.is_empty() && !entries.iter().any(|e| e.name.to_lowercase().contains(&filter))) {
            ui.weak("(no matching files)");
        }
    });

    // ---- file-op dialogs --------------------------------------------------
    if let Some(target) = cache.confirming_delete.clone() {
        let mut open = true;
        let mut do_it = false;
        let mut cancel = false;
        egui::Window::new("Confirm Delete").collapsible(false).resizable(false).open(&mut open).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(
            ui.ctx(),
            |ui| {
                ui.label(format!("Delete {} ?", target.display()));
                ui.horizontal(|ui| {
                    if ui.button("Delete").clicked() {
                        do_it = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            },
        );
        if do_it {
            let res = if target.is_dir() { std::fs::remove_dir_all(&target) } else { std::fs::remove_file(&target) };
            if res.is_err() {
                cache.detail_meta = format!("delete failed: {}", res.err().map(|e| e.to_string()).unwrap_or_default());
            }
            cache.invalidate();
            cache.confirming_delete = None;
        } else if !open || cancel {
            cache.confirming_delete = None;
        }
    }

    if let Some(op) = cache.pending.as_ref().map(|p| (p, ())) {
        let title = match op.0 {
            PendingOp::NewScript => "New Script",
            PendingOp::NewFolder => "New Folder",
            PendingOp::Rename(_) => "Rename",
        };
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new(title).collapsible(false).resizable(false).open(&mut open).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(
            ui.ctx(),
            |ui| {
                ui.horizontal(|ui| {
                    ui.label("name:");
                    let r = ui.add(egui::TextEdit::singleline(&mut cache.op_text).desired_width(240.0));
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        apply = true;
                    }
                    r.request_focus();
                });
                ui.horizontal(|ui| {
                    if ui.button("OK").clicked() {
                        apply = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            },
        );
        if apply {
            let name = cache.op_text.trim().to_string();
            if !name.is_empty() {
                match cache.pending.take() {
                    Some(PendingOp::NewScript) => {
                        let file = if name.ends_with(".m") { name.clone() } else { format!("{name}.m") };
                        let p = cwd.join(&file);
                        if std::fs::write(&p, "% MatCraft script\n").is_ok() {
                            cmd = Some(FileCmd::Open(p));
                        }
                        cache.invalidate();
                    }
                    Some(PendingOp::NewFolder) => {
                        let _ = std::fs::create_dir_all(cwd.join(&name));
                        cache.invalidate();
                    }
                    Some(PendingOp::Rename(from)) => {
                        let to = from.parent().map(|d| d.join(&name)).unwrap_or_else(|| PathBuf::from(&name));
                        let _ = std::fs::rename(&from, &to);
                        cache.invalidate();
                    }
                    None => {}
                }
            }
            cancel = true;
        }
        if !open || cancel {
            cache.pending = None;
            cache.op_text.clear();
        }
    }
    cmd
}

/// Details pane: file metadata and a preview of the first lines.
pub fn details_ui(ui: &mut egui::Ui, cache: &mut FileCache, sel: Option<&Path>) {
    ui.label(egui::RichText::new("Details").strong());
    ui.separator();
    let Some(path) = sel else {
        let _ = cache.detail(None);
        ui.weak("select a file to view details");
        return;
    };
    ui.label(path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
    let (meta, preview) = cache.detail(Some(path));
    if !meta.is_empty() {
        ui.weak(meta);
    }
    if let Some(preview) = preview {
        ui.add_space(4.0);
        let preview = preview.to_string();
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add(egui::Label::new(egui::RichText::new(preview).monospace().size(11.0)).wrap());
        });
    }
}
