//! The .m editor pane: path row, file buttons, multiline buffer, Run.

/// What the editor pane asks the app to do.
pub enum EditorCmd {
    /// Evaluate the buffer in the session; `dir` joins the function search
    /// path first when the buffer came from a file.
    Run { source: String, dir: Option<std::path::PathBuf> },
    /// Internal marker: the Run Selection button was clicked (resolved to a
    /// `Run` with the selected text once the editor's cursor range is known).
    RunSelection,
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
}

impl Editor {
    pub fn new() -> Self {
        Editor { path: String::new(), text: "% MatCraft script\n".to_string(), status: String::new(), dirty: false, want_run_selection: false }
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

    /// Draw the pane. Returns the command for the app to run.
    pub fn ui(&mut self, ui: &mut egui::Ui) -> Option<EditorCmd> {
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
                cmd = Some(EditorCmd::Run { source: self.text.clone(), dir });
            }
            if ui.button("Run Selection").on_hover_text("run the highlighted code — F9").clicked() {
                cmd = Some(EditorCmd::RunSelection);
            }
        });
        if !self.status.is_empty() {
            ui.weak(&self.status);
        }
        let output = egui::TextEdit::multiline(&mut self.text)
            .font(egui::TextStyle::Monospace)
            .desired_width(f32::INFINITY)
            .min_size(ui.available_size())
            .lock_focus(true)
            .layouter(&mut |ui, s, wrap| {
                let mut job = highlight(s.as_str(), ui.visuals().text_color());
                job.wrap.max_width = wrap;
                ui.fonts_mut(|f| f.layout_job(job))
            })
            .show(ui);
        if output.response.changed() {
            self.dirty = true;
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
                cmd = Some(EditorCmd::Run { source: sel, dir });
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
