//! Command History panel: timestamped log of submitted lines, MATLAB-style
//! `%-- m/d/Y h:mm AM/PM --%` day separators. Entries can be multi-selected
//! (checkbox / Ctrl+click) for Copy and Create Script.

use std::collections::{BTreeSet, VecDeque};

use crate::repl::HistEntry;

/// What the history panel asks the app to do with picked entries.
pub enum HistCmd {
    /// Load the command into the input line (single click).
    Insert(String),
    /// Execute it immediately (double click / Evaluate Selection).
    Run(String),
    /// Open the selected commands as a new editor script.
    CreateScript(String),
}

/// Civil date from days since epoch (Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `epoch` seconds → `(year, month, day, hour, minute)` UTC.
fn break_down(epoch: u64) -> (i64, u32, u32, u32, u32) {
    let days = (epoch / 86_400) as i64;
    let secs = (epoch % 86_400) as u32;
    let (y, m, d) = civil_from_days(days);
    (y, m, d, secs / 3600, (secs % 3600) / 60)
}

/// Draw the Command History list. `sel` is the multi-selection (entry
/// indices); Ctrl+click or the row checkbox toggles membership.
pub fn ui(ui: &mut egui::Ui, entries: &VecDeque<HistEntry>, sel: &mut BTreeSet<usize>) -> Option<HistCmd> {
    let mut cmd = None;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Command History").strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if !sel.is_empty() && ui.small_button("x").on_hover_text("clear selection").clicked() {
                sel.clear();
            }
        });
    });
    ui.separator();
    egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
        let mut last_day = None;
        for (idx, e) in entries.iter().enumerate() {
            let (y, mo, d, h, mi) = break_down(e.epoch);
            let day = (y, mo, d);
            if last_day != Some(day) {
                last_day = Some(day);
                let ampm = if h < 12 { "AM" } else { "PM" };
                let h12 = if h % 12 == 0 { 12 } else { h % 12 };
                ui.weak(format!("%-- {mo}/{d}/{y} {h12}:{mi:02} {ampm} --%"));
            }
            let marked = sel.contains(&idx);
            let resp = ui
                .horizontal(|ui| {
                    if !sel.is_empty() {
                        let mut m = marked;
                        if ui.checkbox(&mut m, "").changed() {
                            if m {
                                sel.insert(idx);
                            } else {
                                sel.remove(&idx);
                            }
                        }
                    }
                    ui.selectable_label(marked, egui::RichText::new(&e.cmd).monospace().size(11.5))
                })
                .inner;
            if resp.double_clicked() {
                cmd = Some(HistCmd::Run(e.cmd.clone()));
            } else if resp.clicked() {
                if ui.input(|i| i.modifiers.ctrl) {
                    if marked {
                        sel.remove(&idx);
                    } else {
                        sel.insert(idx);
                    }
                } else if !sel.is_empty() {
                    // Plain click while a selection is active adds the row.
                    sel.insert(idx);
                } else {
                    cmd = Some(HistCmd::Insert(e.cmd.clone()));
                }
            }
            resp.context_menu(|ui| {
                if ui.button("Evaluate Selection").clicked() {
                    cmd = Some(HistCmd::Run(e.cmd.clone()));
                    ui.close();
                }
                if !sel.is_empty() && ui.button("Create Script from selection").clicked() {
                    let body = sel.iter().filter_map(|&i| entries.get(i)).map(|x| x.cmd.as_str()).collect::<Vec<_>>().join("\n");
                    cmd = Some(HistCmd::CreateScript(format!("{body}\n")));
                    sel.clear();
                    ui.close();
                }
                if ui.button(if sel.is_empty() { "Copy" } else { "Copy selection" }).clicked() {
                    if sel.is_empty() {
                        ui.ctx().copy_text(e.cmd.clone());
                    } else {
                        let body = sel.iter().filter_map(|&i| entries.get(i)).map(|x| x.cmd.as_str()).collect::<Vec<_>>().join("\n");
                        ui.ctx().copy_text(body);
                    }
                    ui.close();
                }
            });
        }
        if entries.is_empty() {
            ui.weak("(empty)");
        }
    });
    cmd
}

#[cfg(test)]
mod tests {
    use super::break_down;

    #[test]
    fn epoch_breakdown() {
        // 2025-10-14 00:00:00 UTC = 1760400000.
        assert_eq!(break_down(1_760_400_000), (2025, 10, 14, 0, 0));
        // 1970-01-01.
        assert_eq!(break_down(0), (1970, 1, 1, 0, 0));
    }
}
