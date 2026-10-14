//! Workspace inspector: a table of variables plus a value preview.

use matlang::{fmt_value, Session, Value};

/// What the workspace panel asks the app to do.
pub enum WsCmd {
    /// Open the Variable Editor on this variable (double-click or menu).
    OpenVar(String),
    /// `clear name` — remove it from the workspace.
    Clear(String),
    /// Prompt for a new name, then `new = old; clear old`.
    Rename(String),
    /// `save '<file>' <name>` after a save-file dialog.
    SaveAs(String),
}

/// Draw the workspace panel. `sel` is the variable whose full value is
/// shown under the table; returns the requested action, if any.
pub fn ui(ui: &mut egui::Ui, session: &Session, sel: &mut Option<String>) -> Option<WsCmd> {
    let mut cmd = None;
    ui.label(egui::RichText::new("Workspace").strong());
    ui.separator();
    let names = session.var_names();
    if names.is_empty() {
        ui.weak("empty — assign something, e.g. `A = [1 2; 3 4]`");
    } else {
        egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("workspace-grid").num_columns(2).striped(true).show(ui, |ui| {
                ui.strong("Name");
                ui.strong("Value");
                ui.end_row();
                for name in &names {
                    let Some(v) = session.get(name) else { continue };
                    let selected = sel.as_deref() == Some(name.as_str());
                    let resp = ui.selectable_label(selected, name);
                    if resp.clicked() {
                        *sel = if selected { None } else { Some(name.clone()) };
                    }
                    if resp.double_clicked() {
                        cmd = Some(WsCmd::OpenVar(name.clone()));
                    }
                    resp.context_menu(|ui| {
                        if ui.button("Open in Variable Editor").clicked() {
                            cmd = Some(WsCmd::OpenVar(name.clone()));
                            ui.close();
                        }
                        if ui.button("Rename…").clicked() {
                            cmd = Some(WsCmd::Rename(name.clone()));
                            ui.close();
                        }
                        if ui.button("Save As .mat…").clicked() {
                            cmd = Some(WsCmd::SaveAs(name.clone()));
                            ui.close();
                        }
                        if ui.button("Copy name").clicked() {
                            ui.ctx().copy_text(name.clone());
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Clear").clicked() {
                            cmd = Some(WsCmd::Clear(name.clone()));
                            ui.close();
                        }
                    });
                    // MATLAB's Value column: matrices describe shape+class,
                    // scalars and strings show content.
                    let mut val = match v {
                        Value::Mat(m) => format!("{}x{} double", m.rows(), m.cols()),
                        Value::Str(s) => format!("'{s}'"),
                        Value::Cell(c) => format!("{}x{} cell", c.shape().0, c.shape().1),
                        Value::Struct(_) => "1x1 struct".to_string(),
                        other => fmt_value(other, false).to_string().trim().to_string(),
                    };
                    if val.len() > 40 {
                        val.truncate(37);
                        val.push('…');
                    }
                    ui.monospace(val);
                    ui.end_row();
                }
            });
        });
    }
    if let Some(name) = sel.clone() {
        if let Some(v) = session.get(&name) {
            ui.separator();
            ui.strong(format!("{name} ="));
            egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                let text = fmt_value(v, true).to_string();
                ui.label(egui::RichText::new(text).monospace());
            });
        }
    }
    cmd
}
