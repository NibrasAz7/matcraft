//! The toolstrip — MATLAB-style tabbed ribbon above the panes.
//!
//! Tabs: HOME, PLOTS, EDITOR, VIEW. The EDITOR tab raises automatically when
//! an editor document is selected. Sections are separated by a thin divider
//! with a caption underneath, like the desktop's FILE / VARIABLE / PLOT
//! groups.

/// Which toolstrip tab is open.
#[derive(Clone, Copy, PartialEq)]
pub enum Tab {
    Home,
    Plots,
    Editor,
    View,
}

/// What a toolstrip control asks the app to do.
pub enum RibbonCmd {
    NewScript,
    Open,
    Save,
    /// Evaluate the whole current editor buffer (F5).
    Run,
    /// Evaluate the editor selection (F9).
    RunSelection,
    /// `clear` — empty the workspace.
    ClearWorkspace,
    /// `save` — write the workspace to a `.mat`.
    SaveWorkspace,
    /// Open the selected variable in the Variable Editor.
    OpenVar,
    /// Run an M-code line (plot gallery with a selected variable).
    Eval(String),
    /// Load a command stub into the Command Window input (plot gallery
    /// without a selection — the user supplies the variable).
    Insert(String),
    /// Open the Preferences window.
    Preferences,
    /// Open the editor's find/replace bar (Ctrl+F).
    Find,
    /// Open the Find in Files window (Ctrl+Shift+F).
    FindFiles,
}

/// View-panel toggles the VIEW tab edits in place.
pub struct ViewFlags<'a> {
    pub left: &'a mut bool,
    pub right: &'a mut bool,
    pub cmdwin: &'a mut bool,
    pub editor: &'a mut bool,
}

/// MATLAB-ish palette: pale chrome, white content, MatCraft accent blue.
pub fn studio_visuals() -> egui::Visuals {
    let mut v = egui::Visuals::light();
    let chrome = egui::Color32::from_rgb(0xF2, 0xF4, 0xF7);
    let white = egui::Color32::WHITE;
    v.panel_fill = chrome;
    v.window_fill = white;
    v.extreme_bg_color = white;
    v.faint_bg_color = egui::Color32::from_rgb(0xE9, 0xED, 0xF1);
    v.selection.bg_fill = egui::Color32::from_rgb(0xCD, 0xE3, 0xF3);
    v.selection.stroke.color = egui::Color32::from_rgb(0x20, 0x20, 0x20);
    v.hyperlink_color = ACCENT;
    v.widgets.active.weak_bg_fill = egui::Color32::from_rgb(0xD8, 0xE4, 0xEE);
    v
}

/// The accent blue used for active tabs and links (MatCraft teal-blue).
pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x00, 0x72, 0xBD);

/// The tab strip — a row of tab captions plus the open tab's ribbon.
/// `sel_var` is the Workspace selection; `has_editor` whether the raised
/// document is an editor buffer.
pub fn ui(ui: &mut egui::Ui, tab: &mut Tab, sel_var: Option<&str>, has_editor: bool, view: &mut ViewFlags<'_>) -> Option<RibbonCmd> {
    let mut cmd = None;
    // Tab captions — active tab gets the accent underline, MATLAB-style.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        ui.add_space(6.0);
        for (t, label) in [(Tab::Home, "HOME"), (Tab::Plots, "PLOTS"), (Tab::Editor, "EDITOR"), (Tab::View, "VIEW")] {
            let active = *tab == t;
            let text = if active { egui::RichText::new(label).strong().color(ACCENT) } else { egui::RichText::new(label) };
            let resp = ui.add(egui::Label::new(text).sense(egui::Sense::click()));
            if active {
                let r = resp.rect;
                ui.painter().hline(r.left()..=r.right(), r.bottom() + 1.0, egui::Stroke::new(2.0, ACCENT));
            }
            if resp.clicked() {
                *tab = t;
            }
        }
    });
    ui.separator();

    ui.horizontal(|ui| {
        ui.add_space(4.0);
        match *tab {
            Tab::Home => cmd = home_tab(ui, sel_var),
            Tab::Plots => cmd = plots_tab(ui, sel_var),
            Tab::Editor => cmd = editor_tab(ui, has_editor),
            Tab::View => view_tab(ui, view),
        }
    });
    ui.separator();
    cmd
}

/// One ribbon section: a row of buttons over a greyed caption, separated
/// from the next section by a divider — the desktop's group look.
fn section(ui: &mut egui::Ui, caption: &str, add: impl FnOnce(&mut egui::Ui) -> Option<RibbonCmd>) -> Option<RibbonCmd> {
    let out = ui
        .vertical(|ui| {
            let r = ui.horizontal(|ui| add(ui)).inner;
            ui.label(egui::RichText::new(caption).small().weak());
            r
        })
        .inner;
    ui.separator();
    out
}

fn home_tab(ui: &mut egui::Ui, sel_var: Option<&str>) -> Option<RibbonCmd> {
    let mut cmd = None;
    if let Some(c) = section(ui, "FILE", |ui| {
        let mut c = None;
        if ui.button("New Script").on_hover_text("new .m script — Ctrl+N").clicked() {
            c = Some(RibbonCmd::NewScript);
        }
        if ui.button("Open…").on_hover_text("open an .m file — Ctrl+O").clicked() {
            c = Some(RibbonCmd::Open);
        }
        if ui.button("Save").on_hover_text("save the editor buffer — Ctrl+S").clicked() {
            c = Some(RibbonCmd::Save);
        }
        c
    }) {
        cmd = Some(c);
    }
    if let Some(c) = section(ui, "VARIABLE", |ui| {
        let mut c = None;
        if ui
            .add_enabled(sel_var.is_some(), egui::Button::new("Open Variable"))
            .on_hover_text("edit in the Variable Editor (Workspace selection)")
            .clicked()
        {
            c = Some(RibbonCmd::OpenVar);
        }
        if ui.button("Save Workspace").on_hover_text("save all variables to a .mat").clicked() {
            c = Some(RibbonCmd::SaveWorkspace);
        }
        if ui.button("Clear Workspace").on_hover_text("clear all variables").clicked() {
            c = Some(RibbonCmd::ClearWorkspace);
        }
        c
    }) {
        cmd = Some(c);
    }
    if let Some(c) = section(ui, "CODE", |ui| {
        let mut c = None;
        if ui.button("▶ Run").on_hover_text("run the editor buffer — F5").clicked() {
            c = Some(RibbonCmd::Run);
        }
        c
    }) {
        cmd = Some(c);
    }
    if let Some(c) = section(ui, "ENVIRONMENT", |ui| {
        let mut c = None;
        if ui.button("Preferences").on_hover_text("desktop settings").clicked() {
            c = Some(RibbonCmd::Preferences);
        }
        c
    }) {
        cmd = Some(c);
    }
    if let Some(c) = section(ui, "RESOURCES", |ui| {
        let mut c = None;
        if ui.button("Help").on_hover_text("`help` in the Command Window").clicked() {
            c = Some(RibbonCmd::Eval("help".to_string()));
        }
        c
    }) {
        cmd = Some(c);
    }
    cmd
}

/// The PLOTS gallery — each button acts on the Workspace selection (like the
/// desktop's plots tab), or drops a `f(` stub on the command line when
/// nothing is selected.
fn plots_tab(ui: &mut egui::Ui, sel_var: Option<&str>) -> Option<RibbonCmd> {
    let mut cmd = None;
    let gallery: &[(&str, &str)] = &[
        ("plot", "line plot"),
        ("scatter", "scatter"),
        ("stem", "stem"),
        ("bar", "bar"),
        ("histogram", "histogram"),
        ("area", "area"),
        ("stairs", "stairs"),
        ("errorbar", "error bars"),
        ("pie", "pie"),
        ("imagesc", "scaled image"),
        ("contour", "contour"),
        ("surf", "surface"),
        ("mesh", "wireframe"),
    ];
    if let Some(c) = section(ui, "PLOT", |ui| {
        let mut c = None;
        ui.horizontal_wrapped(|ui| {
            for (f, hint) in gallery {
                let resp = ui.button(*f).on_hover_text(match sel_var {
                    Some(v) => format!("{hint}: {f}({v})"),
                    None => format!("{hint}: {f}(…) — select a workspace variable, or type one"),
                });
                if resp.clicked() {
                    c = Some(match sel_var {
                        Some(v) => RibbonCmd::Eval(format!("{f}({v})")),
                        None => RibbonCmd::Insert(format!("{f}(")),
                    });
                }
            }
        });
        c
    }) {
        cmd = Some(c);
    }
    if let Some(c) = section(ui, "AXES", |ui| {
        let mut c = None;
        for (f, args) in [("grid", "on"), ("hold", "on"), ("box", "on")] {
            if ui.button(format!("{f} {args}")).clicked() {
                c = Some(RibbonCmd::Eval(format!("{f}('{args}')")));
            }
        }
        for f in ["xlabel", "ylabel", "title", "legend"] {
            if ui.button(f).clicked() {
                c = Some(RibbonCmd::Insert(format!("{f}('")));
            }
        }
        c
    }) {
        cmd = Some(c);
    }
    cmd
}

fn editor_tab(ui: &mut egui::Ui, has_editor: bool) -> Option<RibbonCmd> {
    let mut cmd = None;
    if let Some(c) = section(ui, "FILE", |ui| {
        let mut c = None;
        if ui.button("New").on_hover_text("new script — Ctrl+N").clicked() {
            c = Some(RibbonCmd::NewScript);
        }
        if ui.button("Open…").clicked() {
            c = Some(RibbonCmd::Open);
        }
        if ui.add_enabled(has_editor, egui::Button::new("Save")).on_hover_text("Ctrl+S").clicked() {
            c = Some(RibbonCmd::Save);
        }
        c
    }) {
        cmd = Some(c);
    }
    if let Some(c) = section(ui, "RUN", |ui| {
        let mut c = None;
        if ui.add_enabled(has_editor, egui::Button::new("▶ Run")).on_hover_text("run the buffer — F5").clicked() {
            c = Some(RibbonCmd::Run);
        }
        if ui.add_enabled(has_editor, egui::Button::new("Run Selection")).on_hover_text("evaluate the highlighted code — F9").clicked() {
            c = Some(RibbonCmd::RunSelection);
        }
        c
    }) {
        cmd = Some(c);
    }
    if let Some(c) = section(ui, "FIND", |ui| {
        let mut c = None;
        if ui.add_enabled(has_editor, egui::Button::new("Find")).on_hover_text("find in the editor buffer — Ctrl+F").clicked() {
            c = Some(RibbonCmd::Find);
        }
        if ui.button("Find Files").on_hover_text("search *.m files under the current folder — Ctrl+Shift+F").clicked() {
            c = Some(RibbonCmd::FindFiles);
        }
        c
    }) {
        cmd = Some(c);
    }
    cmd
}

fn view_tab(ui: &mut egui::Ui, view: &mut ViewFlags<'_>) {
    let _ = section(ui, "LAYOUT", |ui| {
        ui.checkbox(view.left, "Current Folder + Details");
        ui.checkbox(view.right, "Workspace + History");
        ui.checkbox(view.cmdwin, "Command Window");
        ui.checkbox(view.editor, "Editor");
        None
    });
    let _ = section(ui, "ZOOM", |ui| {
        if ui.small_button("−").on_hover_text("zoom out — Ctrl+-").clicked() {
            ui.ctx().set_zoom_factor((ui.ctx().zoom_factor() / 1.1).max(0.5));
        }
        ui.weak(format!("{:.0}%", ui.ctx().zoom_factor() * 100.0));
        if ui.small_button("+").on_hover_text("zoom in — Ctrl+=").clicked() {
            ui.ctx().set_zoom_factor((ui.ctx().zoom_factor() * 1.1).min(3.0));
        }
        if ui.button("Reset").clicked() {
            ui.ctx().set_zoom_factor(1.0);
        }
        None
    });
}
