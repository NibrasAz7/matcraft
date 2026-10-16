//! The .m editor pane: path row, file buttons, find/replace bar, a gutter
//! with line numbers and breakpoint marks, and the multiline buffer.

use std::collections::BTreeSet;

/// What the editor pane asks the app to do.
pub enum EditorCmd {
    /// Evaluate the buffer in the session; `dir` joins the function search
    /// path first when the buffer came from a file. `path` is echoed like
    /// `run 'path'` so error links in the Command Window resolve to it.
    Run { source: String, dir: Option<std::path::PathBuf>, path: Option<String> },
    /// Internal marker: the Run Selection button was clicked (resolved to a
    /// `Run` with the selected text once the editor's cursor range is known).
    RunSelection,
}

/// The Ctrl+F find/replace bar over the buffer.
#[derive(Default)]
pub struct FindBar {
    pub open: bool,
    pub query: String,
    /// Case-sensitive matching.
    pub case: bool,
    /// Index into the current matches of the highlighted ("current") one.
    pub cur: usize,
    /// The replace row is expanded.
    pub show_replace: bool,
    pub replace: String,
    /// Focus the query field next frame (set by Ctrl+F).
    pub want_focus: bool,
}

pub struct Editor {
    /// File backing the buffer (empty = unsaved scratch).
    pub path: String,
    pub text: String,
    /// One-line status under the buttons (saved/error notes).
    pub status: String,
    /// Unsaved changes — the document tab shows `*` like MATLAB.
    pub dirty: bool,
    /// Set by the toolstrip's Run Selection; consumed on the next `ui` pass
    /// (the selection range only exists while the TextEdit is drawn).
    pub want_run_selection: bool,
    /// The find/replace bar (Ctrl+F).
    pub find: FindBar,
    /// 1-based lines with a breakpoint mark in the gutter. UI-only markers —
    /// `dbstop`/`dbclear` need interpreter support that doesn't exist yet.
    pub breakpoints: BTreeSet<usize>,
    /// Scroll to this 1-based line next `ui` pass (error links, find hits).
    pub want_line: Option<usize>,
    /// Scroll to this char index next `ui` pass (find next/prev, replace).
    want_char: Option<usize>,
    /// Focus the buffer next frame (set when the find bar closes via Esc).
    want_text_focus: bool,
}

/// Byte ranges `[start, end)` of every `query` in `text`. Case-insensitive
/// mode compares ASCII letters case-blindly. Capped at 10 000 matches.
pub fn find_matches(text: &str, query: &str, case: bool) -> Vec<(usize, usize)> {
    const MAX_MATCHES: usize = 10_000;
    let mut out = Vec::new();
    if query.is_empty() {
        return out;
    }
    if case {
        let mut i = 0;
        while out.len() < MAX_MATCHES {
            match text.get(i..).and_then(|s| s.find(query)) {
                Some(m) => {
                    out.push((i + m, i + m + query.len()));
                    i += m + query.len().max(1);
                }
                None => break,
            }
        }
    } else {
        let q: Vec<char> = query.chars().collect();
        'outer: for (i, _) in text.char_indices() {
            let mut it = text[i..].chars();
            if q.iter().all(|&b| it.next().is_some_and(|a| a.eq_ignore_ascii_case(&b))) {
                let end = text[i..].char_indices().nth(q.len()).map(|(b, _)| i + b).unwrap_or(text.len());
                out.push((i, end));
                if out.len() >= MAX_MATCHES {
                    break 'outer;
                }
            }
        }
    }
    out
}

/// Char index of each line's first character (0-based lines).
fn line_start_offsets(text: &str) -> Vec<usize> {
    let mut v = vec![0];
    for (i, c) in text.chars().enumerate() {
        if c == '\n' {
            v.push(i + 1);
        }
    }
    v
}

impl Editor {
    pub fn new() -> Self {
        Editor {
            path: String::new(),
            text: "% MatCraft script\n".to_string(),
            status: String::new(),
            dirty: false,
            want_run_selection: false,
            find: FindBar::default(),
            breakpoints: BTreeSet::new(),
            want_line: None,
            want_char: None,
            want_text_focus: false,
        }
    }

    /// Load a file into the buffer (also used by `apps/matcraft file.m`).
    pub fn open_path(&mut self, path: &std::path::Path) {
        match std::fs::read_to_string(path) {
            Ok(t) => {
                self.text = t;
                self.path = path.to_string_lossy().to_string();
                self.status.clear();
                self.dirty = false;
            }
            Err(e) => self.status = format!("cannot open {}: {e}", path.display()),
        }
    }

    /// Save the buffer to `self.path` (asks for a path when the buffer is new).
    pub fn save(&mut self) {
        if self.path.is_empty() {
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(p) = rfd::FileDialog::new().add_filter("M-code", &["m"]).save_file() {
                self.path = p.to_string_lossy().to_string();
            }
            if self.path.is_empty() {
                return;
            }
        }
        match std::fs::write(&self.path, &self.text) {
            Ok(()) => {
                self.status = format!("saved {}", self.path);
                self.dirty = false;
            }
            Err(e) => self.status = format!("save failed: {e}"),
        }
    }

    /// Reset to a fresh scratch buffer (toolbar "New Script" shares this).
    pub fn new_file(&mut self) {
        self.path.clear();
        self.text = "% MatCraft script\n".to_string();
        self.status.clear();
        self.dirty = false;
    }

    /// The Ctrl+F bar: query field, `n/m` count, prev/next (Enter /
    /// Shift+Enter), `Aa` case toggle, an expandable replace row, `x` and
    /// Esc to close.
    fn find_bar(&mut self, ui: &mut egui::Ui, salt: usize, matches: &[(usize, usize)]) {
        let mut nav: Option<bool> = None; // true = next, false = prev
        let mut do_replace = false;
        let mut do_replace_all = false;
        let mut close = false;
        ui.horizontal(|ui| {
            ui.label("Find:");
            let r = ui.add(egui::TextEdit::singleline(&mut self.find.query).desired_width(160.0).id(egui::Id::new(("find-query", salt))));
            if std::mem::take(&mut self.find.want_focus) {
                r.request_focus();
            }
            ui.weak(if matches.is_empty() { "0/0".to_string() } else { format!("{}/{}", self.find.cur + 1, matches.len()) });
            if ui.small_button("<").on_hover_text("previous match — Shift+Enter").clicked() {
                nav = Some(false);
            }
            if ui.small_button(">").on_hover_text("next match — Enter").clicked() {
                nav = Some(true);
            }
            if ui.selectable_label(self.find.case, "Aa").on_hover_text("match case").clicked() {
                self.find.case = !self.find.case;
                self.find.cur = 0;
            }
            if ui.selectable_label(self.find.show_replace, "±").on_hover_text("show/hide replace").clicked() {
                self.find.show_replace = !self.find.show_replace;
            }
            if ui.small_button("x").on_hover_text("close — Esc").clicked() {
                close = true;
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                nav = Some(!ui.input(|i| i.modifiers.shift));
                r.request_focus();
            }
        });
        if self.find.show_replace {
            ui.horizontal(|ui| {
                ui.label("Replace:");
                let r = ui.add(egui::TextEdit::singleline(&mut self.find.replace).desired_width(160.0));
                if ui.small_button("Replace").on_hover_text("replace the current match").clicked() {
                    do_replace = true;
                }
                if ui.small_button("All").on_hover_text("replace all matches").clicked() {
                    do_replace_all = true;
                }
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    do_replace = true;
                }
            });
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }
        if let Some(next) = nav {
            if !matches.is_empty() {
                self.find.cur = if next { (self.find.cur + 1) % matches.len() } else { (self.find.cur + matches.len() - 1) % matches.len() };
                self.want_char = matches.get(self.find.cur).map(|m| self.text[..m.0].chars().count());
            }
        }
        if (do_replace || do_replace_all) && !matches.is_empty() {
            if do_replace_all {
                for m in matches.iter().rev() {
                    self.text.replace_range(m.0..m.1, &self.find.replace);
                }
            } else if let Some(m) = matches.get(self.find.cur.min(matches.len() - 1)).copied() {
                self.text.replace_range(m.0..m.1, &self.find.replace);
                self.want_char = Some(self.text[..m.0].chars().count());
            }
            self.dirty = true;
        }
        if close {
            self.find.open = false;
            self.want_text_focus = true;
        }
    }

    /// Draw the pane. `salt` keeps widget ids unique across editor tabs.
    /// Returns the command for the app to run.
    pub fn ui(&mut self, ui: &mut egui::Ui, salt: usize) -> Option<EditorCmd> {
        let mut cmd = None;
        ui.heading("Editor");
        ui.horizontal(|ui| {
            ui.label("file:");
            ui.add(egui::TextEdit::singleline(&mut self.path).desired_width(f32::INFINITY).hint_text("(scratch buffer)"));
        });
        ui.horizontal(|ui| {
            #[cfg(not(target_arch = "wasm32"))]
            {
                if ui.button("Open…").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("M-code", &["m"]).pick_file() {
                        self.open_path(&p);
                    }
                }
                if ui.button("Save").clicked() {
                    self.save();
                }
            }
            if ui.button("New").clicked() {
                self.new_file();
            }
            if ui.button("Run").clicked() {
                let dir = std::path::Path::new(&self.path).parent().filter(|p| !p.as_os_str().is_empty()).map(|p| p.to_path_buf());
                cmd = Some(EditorCmd::Run { source: self.text.clone(), dir, path: (!self.path.is_empty()).then(|| self.path.clone()) });
            }
            if ui.button("Run Selection").on_hover_text("run the highlighted code — F9").clicked() {
                cmd = Some(EditorCmd::RunSelection);
            }
        });
        if !self.status.is_empty() {
            ui.weak(&self.status);
        }

        // ---- find/replace bar -------------------------------------------
        let matches: Vec<(usize, usize)> = if self.find.open { find_matches(&self.text, &self.find.query, self.find.case) } else { Vec::new() };
        if self.find.open {
            self.find.cur = self.find.cur.min(matches.len().saturating_sub(1));
            self.find_bar(ui, salt, &matches);
        }
        let cur_match = matches.get(self.find.cur).copied();

        // ---- buffer with gutter ------------------------------------------
        let edit_id = egui::Id::new(("editor-text", salt));
        let avail_h = ui.available_height().max(40.0);
        let mut scroll_id = egui::Id::new(0u8);
        let (grect, gresp, output) = ui
            .horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let (grect, gresp) = ui.allocate_exact_size(egui::vec2(30.0, avail_h), egui::Sense::click());
                scroll_id = ui.make_persistent_id("editor-scroll");
                let output = egui::ScrollArea::vertical()
                    .id_salt("editor-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::TextEdit::multiline(&mut self.text)
                            .id(edit_id)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY)
                            .min_size(egui::vec2(ui.available_width(), avail_h))
                            .lock_focus(true)
                            .layouter(&mut |ui, s, wrap| {
                                let mut job = highlight_matches(s.as_str(), ui.visuals().text_color(), &matches, cur_match);
                                job.wrap.max_width = wrap;
                                ui.fonts_mut(|f| f.layout_job(job))
                            })
                            .show(ui)
                    })
                    .inner;
                (grect, gresp, output)
            })
            .inner;
        if output.response.changed() {
            self.dirty = true;
        }

        // Gutter: line numbers on each row that begins a line, breakpoint
        // dots on marked lines. Rows that start a line always begin right
        // after a `\n` (or at char 0), so their start offset equals a line
        // start.
        let line_starts = line_start_offsets(&self.text);
        {
            let painter = ui.painter().with_clip_rect(grect);
            let num_font = egui::FontId::monospace(11.0);
            let num_col = ui.visuals().weak_text_color();
            let bp_col = egui::Color32::from_rgb(0xB0, 0x00, 0x00);
            let mut cum = 0usize;
            let mut li = 0usize;
            for row in &output.galley.rows {
                let y = output.galley_pos.y + row.pos.y;
                if y > grect.bottom() {
                    break;
                }
                if line_starts.get(li) == Some(&cum) {
                    painter.text(egui::pos2(grect.right() - 4.0, y), egui::Align2::RIGHT_TOP, (li + 1).to_string(), num_font.clone(), num_col);
                    if self.breakpoints.contains(&(li + 1)) {
                        painter.circle_filled(egui::pos2(grect.left() + 9.0, y + row.size.y / 2.0), 4.0, bp_col);
                    }
                    li += 1;
                }
                cum += row.char_count_including_newline().0;
            }
        }
        let gresp = gresp.on_hover_text("click to set/clear a breakpoint\n(markers only — interpreter support pending)");
        if gresp.clicked() {
            if let Some(p) = gresp.interact_pointer_pos() {
                let ci = output.galley.cursor_from_pos(p - output.galley_pos).index.0;
                let line = line_starts.partition_point(|s| *s <= ci).max(1);
                if !self.breakpoints.remove(&line) {
                    self.breakpoints.insert(line);
                }
                self.status = format!("breakpoint at line {line} — markers only (interpreter support pending)");
            }
        }

        // Scroll/cursor requests: `want_line` (error links) places the
        // cursor at the line start; both it and `want_char` (find/replace)
        // steer the scroll offset through the persisted ScrollArea state.
        let mut want_char = self.want_char.take();
        if let Some(line) = self.want_line.take() {
            let li = line.saturating_sub(1).min(line_starts.len().saturating_sub(1));
            let ci = line_starts.get(li).copied().unwrap_or(0);
            want_char = Some(ci);
            if let Some(mut st) = egui::text_edit::TextEditState::load(ui.ctx(), edit_id) {
                st.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(ci))));
                st.store(ui.ctx(), edit_id);
            }
            ui.ctx().memory_mut(|m| m.request_focus(edit_id));
        }
        if let Some(c) = want_char {
            let lc = output.galley.layout_from_cursor(egui::text::CCursor::new(c));
            if let Some(row) = output.galley.rows.get(lc.row) {
                let mut st = egui::containers::scroll_area::State::load(ui.ctx(), scroll_id).unwrap_or_default();
                // `galley_pos` already carries the current scroll offset —
                // fold it back to content coordinates, minus a small margin.
                st.offset.y = (st.offset.y + output.galley_pos.y + row.pos.y - output.text_clip_rect.top() - 4.0).max(0.0);
                st.store(ui.ctx(), scroll_id);
            }
        }
        if std::mem::take(&mut self.want_text_focus) {
            output.response.request_focus();
        }

        // F9 (or the button above) evaluates the highlighted selection.
        let want_sel = std::mem::take(&mut self.want_run_selection);
        if (ui.input(|i| i.key_pressed(egui::Key::F9)) && output.response.has_focus()) || matches!(cmd, Some(EditorCmd::RunSelection)) || want_sel {
            if let Some(sel) = output.cursor_range.and_then(|cr| {
                let n = self.text.chars().count();
                let (a, b) = if cr.primary.index <= cr.secondary.index {
                    (cr.primary.index.0, cr.secondary.index.0)
                } else {
                    (cr.secondary.index.0, cr.primary.index.0)
                };
                let (a, b) = (a.min(n), b.min(n));
                (b > a).then(|| self.text.chars().skip(a).take(b - a).collect::<String>())
            }) {
                let dir = std::path::Path::new(&self.path).parent().filter(|p| !p.as_os_str().is_empty()).map(|p| p.to_path_buf());
                cmd = Some(EditorCmd::Run { source: sel, dir, path: (!self.path.is_empty()).then(|| self.path.clone()) });
            }
        }
        cmd
    }
}

/// M-code keywords coloured as MATLAB does (keywords blue).
const KEYWORDS: &[&str] = &[
    "break",
    "case",
    "catch",
    "classdef",
    "continue",
    "else",
    "elseif",
    "end",
    "enumeration",
    "events",
    "for",
    "function",
    "global",
    "if",
    "methods",
    "otherwise",
    "parfor",
    "persistent",
    "properties",
    "return",
    "spmd",
    "switch",
    "try",
    "while",
];

/// `highlight` plus find-bar marks: every match gets a light-yellow
/// background, the current one a stronger amber — like the Editor's find
/// highlighting.
fn highlight_matches(text: &str, default_color: egui::Color32, matches: &[(usize, usize)], cur: Option<(usize, usize)>) -> egui::text::LayoutJob {
    let mut job = highlight(text, default_color);
    if matches.is_empty() {
        return job;
    }
    // Split sections at every match boundary, keeping each piece's format.
    let mut cuts: Vec<usize> = matches.iter().flat_map(|m| [m.0, m.1]).collect();
    cuts.sort_unstable();
    cuts.dedup();
    let mut secs: Vec<egui::text::LayoutSection> = Vec::with_capacity(job.sections.len() + cuts.len());
    for sec in job.sections.drain(..) {
        let (s, e) = (sec.byte_range.start.0, sec.byte_range.end.0);
        let mut pts: Vec<usize> = Vec::with_capacity(4);
        pts.push(s);
        pts.extend(cuts.iter().copied().filter(|c| *c > s && *c < e));
        pts.push(e);
        let mut first = true;
        for w in pts.windows(2) {
            if w[0] == w[1] {
                continue;
            }
            secs.push(egui::text::LayoutSection {
                leading_space: if first { sec.leading_space } else { 0.0 },
                byte_range: w[0].into()..w[1].into(),
                format: sec.format.clone(),
            });
            first = false;
        }
    }
    let mark = egui::Color32::from_rgb(0xFF, 0xEF, 0x9C);
    let mark_cur = egui::Color32::from_rgb(0xF6, 0xC5, 0x5C);
    for &(a, b) in matches {
        let bg = if cur == Some((a, b)) { mark_cur } else { mark };
        for sec in secs.iter_mut().filter(|s| s.byte_range.start.0 >= a && s.byte_range.end.0 <= b) {
            sec.format.background = bg;
        }
    }
    job.sections = secs;
    job
}

/// Colour M-code into a `LayoutJob`: keywords blue, `%`/`...` comments
/// green, `'…'` strings purple, numbers dark cyan. `'` after an operand
/// char is a transpose, not a string start.
fn highlight(text: &str, default_color: egui::Color32) -> egui::text::LayoutJob {
    let font = egui::FontId::monospace(14.0);
    let def = egui::text::TextFormat { font_id: font.clone(), color: default_color, ..Default::default() };
    let kw = egui::text::TextFormat { font_id: font.clone(), color: egui::Color32::from_rgb(0, 0, 220), ..Default::default() };
    let com = egui::text::TextFormat { font_id: font.clone(), color: egui::Color32::from_rgb(34, 139, 34), ..Default::default() };
    let st = egui::text::TextFormat { font_id: font.clone(), color: egui::Color32::from_rgb(163, 21, 134), ..Default::default() };
    let num = egui::text::TextFormat { font_id: font.clone(), color: egui::Color32::from_rgb(0, 110, 110), ..Default::default() };
    let mut job = egui::text::LayoutJob::default();
    for line in text.split_inclusive('\n') {
        let b = line.as_bytes();
        let mut i = 0;
        let mut prev_operand = false;
        while i < b.len() {
            let (len, fmt) = if line[i..].starts_with("...") || b[i] == b'%' {
                (b.len() - i, &com)
            } else if b[i] == b'\'' && !prev_operand {
                // String: to the next unescaped ' (or end of line).
                let mut end = i + 1;
                while end < b.len() {
                    match line[end..].find('\'') {
                        Some(p) if b.get(end + p + 1) == Some(&b'\'') => end += p + 2,
                        Some(p) => {
                            end += p + 1;
                            break;
                        }
                        None => {
                            end = b.len();
                            break;
                        }
                    }
                }
                (end.min(b.len()) - i, &st)
            } else if b[i].is_ascii_alphabetic() || b[i] == b'_' {
                let end = line[i..].find(|c: char| !(c.is_alphanumeric() || c == '_')).map(|p| i + p).unwrap_or(b.len());
                (end - i, if KEYWORDS.contains(&&line[i..end]) { &kw } else { &def })
            } else if b[i].is_ascii_digit() || (b[i] == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit)) {
                // `+`/`-` continue a number only right after an exponent marker.
                let mut end = i;
                while end < b.len() {
                    let c = b[end] as char;
                    let sign_ok = (c == '+' || c == '-') && matches!(b.get(end - 1), Some(&b'e') | Some(&b'E'));
                    if !(c.is_ascii_alphanumeric() || c == '.' || c == '_' || sign_ok) {
                        break;
                    }
                    end += 1;
                }
                (end - i, &num)
            } else {
                (1, &def)
            };
            let chunk = &line[i..i + len.min(b.len() - i)];
            job.append(chunk, 0.0, fmt.clone());
            prev_operand = matches!(b[i], b')' | b']' | b'}' | b'\'') || chunk.chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_');
            i += len.min(b.len() - i);
        }
    }
    job
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_matches_case() {
        let text = "a = Foo;\nb = foo(1);\n";
        assert_eq!(find_matches(text, "foo", true), vec![(13, 16)]);
        assert_eq!(find_matches(text, "foo", false), vec![(4, 7), (13, 16)]);
        assert!(find_matches(text, "", false).is_empty());
    }

    #[test]
    fn line_starts() {
        assert_eq!(line_start_offsets("a\nbc\n"), vec![0, 2, 5]);
        assert_eq!(line_start_offsets("nolf"), vec![0]);
    }

    #[test]
    fn breakpoint_toggle() {
        let mut e = Editor::new();
        assert!(e.breakpoints.is_empty());
        e.breakpoints.insert(3);
        assert!(e.breakpoints.contains(&3));
        e.breakpoints.remove(&3);
        assert!(e.breakpoints.is_empty());
    }

    #[test]
    fn goto_line_marks_want_line() {
        let mut e = Editor::new();
        e.want_line = Some(7);
        assert_eq!(e.want_line, Some(7));
    }
}
