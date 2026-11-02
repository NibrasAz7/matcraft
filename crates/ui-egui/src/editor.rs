//! The .m editor pane: path row, file buttons, find/replace bar, a gutter
//! with line numbers and breakpoint marks, and the multiline buffer.

use std::collections::BTreeSet;

/// What the editor pane asks the app to do.
pub enum EditorCmd {
    /// Evaluate the buffer in the session; `dir` joins the function search
    /// path first when the buffer came from a file. `path` is echoed like
    /// `run 'path'` so error links in the Command Window resolve to it.
    /// `as_file` runs a clean file-backed buffer through `run 'path'` so
    /// `dbstop` breakpoints can hit.
    Run { source: String, dir: Option<std::path::PathBuf>, path: Option<String>, as_file: bool },
    /// Internal marker: the Run Selection button was clicked (resolved to a
    /// `Run` with the selected text once the editor's cursor range is known).
    RunSelection,
    /// A gutter click toggled the breakpoint on `line`; `set` = added —
    /// the app mirrors it into the session with `dbstop`/`dbclear`.
    BpToggle { line: usize, set: bool },
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
    /// 1-based lines with a breakpoint mark in the gutter; toggles are
    /// mirrored into the session as `dbstop`/`dbclear` ([`EditorCmd::BpToggle`]).
    pub breakpoints: BTreeSet<usize>,
    /// The debugger's paused line — painted amber with a `▶` in the gutter.
    pub pause_line: Option<usize>,
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
            pause_line: None,
            want_line: None,
            want_char: None,
            want_text_focus: false,
        }
    }

    /// Load a file into the buffer (also used by `apps/matcraft file.m`).
    pub fn open_path(&mut self, path: &std::path::Path) {
        match matlang::read_m_source(path) {
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
        match crate::write_file(std::path::Path::new(&self.path), self.text.as_bytes()) {
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
    /// Returns the commands for the app to run (usually empty or one).
    pub fn ui(&mut self, ui: &mut egui::Ui, salt: usize) -> Vec<EditorCmd> {
        let mut cmds = Vec::new();
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
                cmds.push(EditorCmd::Run {
                    source: self.text.clone(),
                    dir,
                    path: (!self.path.is_empty()).then(|| self.path.clone()),
                    as_file: !self.dirty && !self.path.is_empty(),
                });
            }
            if ui.button("Run Selection").on_hover_text("run the highlighted code — F9").clicked() {
                cmds.push(EditorCmd::RunSelection);
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
        let pause_ln = self.pause_line;

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
                                let mut job = highlight_matches(s.as_str(), ui.visuals().text_color(), &matches, cur_match, pause_ln);
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

        // ---- parameter hint ----------------------------------------------
        // While the buffer has focus, the innermost call around the caret
        // gets an Excel-style signature popup above the caret line.
        if output.response.has_focus() {
            let hint = output.cursor_range.and_then(|cr| {
                let ci = cr.primary.index.0;
                let byte = self.text.char_indices().nth(ci).map(|(b, _)| b).unwrap_or(self.text.len());
                call_hint(&self.text, byte).and_then(|(name, arg)| param_hint(&name, arg, &self.text)).map(|h| (h, ci))
            });
            if let Some((h, ci)) = hint {
                let caret = output.galley.pos_from_cursor(egui::text::CCursor::new(ci));
                let x = output.galley_pos.x + caret.left();
                let y = output.galley_pos.y + caret.top();
                let accent = egui::Color32::from_rgb(0x1E, 0x6F, 0xD9);
                egui::Area::new(egui::Id::new(("param-hint", salt)))
                    .order(egui::Order::Foreground)
                    .interactable(false)
                    .pivot(egui::Align2::LEFT_BOTTOM)
                    .fixed_pos(egui::pos2(x, (y - 4.0).max(grect.top() + 64.0)))
                    .show(ui.ctx(), |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(&h.name).monospace().strong().color(accent));
                                ui.label(egui::RichText::new("(").monospace());
                                for (i, p) in h.params.iter().enumerate() {
                                    if i > 0 {
                                        ui.label(egui::RichText::new(", ").monospace());
                                    }
                                    let t = egui::RichText::new(p).monospace();
                                    ui.label(if i + 1 == h.arg { t.strong().underline().color(accent) } else { t });
                                }
                                ui.label(egui::RichText::new(")").monospace());
                            });
                            let total = h.params.len().max(1);
                            let which = h.arg.min(total);
                            ui.label(egui::RichText::new(format!("argument {which} of {total} — {}", h.doc)).small().weak());
                        });
                    });
            }
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
                    if self.pause_line == Some(li + 1) {
                        // The debugger's execution pointer (MATLAB's green arrow).
                        painter.text(
                            egui::pos2(grect.left() + 12.0, y + row.size.y / 2.0),
                            egui::Align2::LEFT_CENTER,
                            "▶",
                            egui::FontId::monospace(10.0),
                            egui::Color32::from_rgb(0xB8, 0x86, 0x0B),
                        );
                    }
                    li += 1;
                }
                cum += row.char_count_including_newline().0;
            }
        }
        let gresp = gresp.on_hover_text("click to set/clear a breakpoint");
        if gresp.clicked() {
            if let Some(p) = gresp.interact_pointer_pos() {
                let ci = output.galley.cursor_from_pos(p - output.galley_pos).index.0;
                let line = line_starts.partition_point(|s| *s <= ci).max(1);
                let set = !self.breakpoints.remove(&line);
                if set {
                    self.breakpoints.insert(line);
                }
                cmds.push(EditorCmd::BpToggle { line, set });
                self.status = if self.path.is_empty() {
                    format!("breakpoint at line {line} — save the file for it to take effect")
                } else {
                    format!("breakpoint {} at line {line}", if set { "set" } else { "cleared" })
                };
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
        if (ui.input(|i| i.key_pressed(egui::Key::F9)) && output.response.has_focus())
            || cmds.iter().any(|c| matches!(c, EditorCmd::RunSelection))
            || want_sel
        {
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
                cmds.push(EditorCmd::Run { source: sel, dir, path: (!self.path.is_empty()).then(|| self.path.clone()), as_file: false });
            }
        }
        cmds
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

/// Builtin call signatures used by the editor's parameter hint. The first
/// field is the display signature, the second a one-line description.
const SIGNATURES: &[(&str, &str, &str)] = &[
    ("plot", "plot(X, Y, LineSpec)", "Plot columns of Y against X; LineSpec selects colour, marker and line style."),
    ("plot3", "plot3(X, Y, Z, LineSpec)", "Plot 3-D lines through the columns of X, Y and Z."),
    ("scatter", "scatter(X, Y, Size, Color, Marker)", "Draw a scatter plot with optional marker size, colour and shape."),
    ("bar", "bar(Y, Style)", "Draw a bar chart; Style selects grouped, stacked or horizontal bars."),
    ("histogram", "histogram(X, BinEdges)", "Plot a histogram using automatic or explicit bin edges."),
    ("xlabel", "xlabel(Text)", "Set the x-axis label."),
    ("ylabel", "ylabel(Text)", "Set the y-axis label."),
    ("zlabel", "zlabel(Text)", "Set the z-axis label."),
    ("title", "title(Text)", "Set the axes title."),
    ("legend", "legend(Labels, Location)", "Add a legend using a cellstr of labels."),
    ("axis", "axis(Limits)", "Set axis limits, equal/aspect modes or visibility."),
    ("xlim", "xlim(Limits)", "Set or query the x-axis limits."),
    ("ylim", "ylim(Limits)", "Set or query the y-axis limits."),
    ("grid", "grid(State)", "Turn the axes grid on or off."),
    ("hold", "hold(State)", "Retain the current plot when adding new graphics."),
    ("figure", "figure(Number, Name, Value, ...)", "Create or select a figure window."),
    ("subplot", "subplot(M, N, P)", "Select one cell of an M-by-N axes grid."),
    ("zeros", "zeros(Size, ClassName)", "Create an array of zeros."),
    ("ones", "ones(Size, ClassName)", "Create an array of ones."),
    ("eye", "eye(Size, ClassName)", "Create an identity matrix."),
    ("rand", "rand(Size)", "Uniformly distributed random numbers in (0,1)."),
    ("randn", "randn(Size)", "Normally distributed random numbers."),
    ("randi", "randi(Imax, Size)", "Uniformly distributed random integers."),
    ("linspace", "linspace(A, B, N)", "N linearly spaced points from A to B."),
    ("logspace", "logspace(A, B, N)", "N logarithmically spaced points from 10^A to 10^B."),
    ("size", "size(A, Dim)", "Array dimensions, or the length of one dimension."),
    ("numel", "numel(A)", "Number of elements in an array."),
    ("reshape", "reshape(A, Size)", "Reshape an array without changing its elements."),
    ("repmat", "repmat(A, Reps)", "Repeat copies of an array."),
    ("sum", "sum(A, Dim, NanFlag)", "Sum of elements along a dimension."),
    ("mean", "mean(A, Dim, NanFlag)", "Arithmetic mean along a dimension."),
    ("median", "median(A, Dim, NanFlag)", "Median along a dimension."),
    ("std", "std(A, Weight, Dim)", "Standard deviation."),
    ("var", "var(A, Weight, Dim)", "Variance."),
    ("min", "min(A, B, Dim, NanFlag)", "Smallest elements, or element-wise minimum."),
    ("max", "max(A, B, Dim, NanFlag)", "Largest elements, or element-wise maximum."),
    ("find", "find(A, K, Direction)", "Indices of nonzero elements."),
    ("sort", "sort(A, Dim, Direction)", "Sort elements along a dimension."),
    ("unique", "unique(A, Occurrence)", "Unique values, optionally including stable order."),
    ("ismember", "ismember(A, B)", "True where elements of A occur in B."),
    ("interp1", "interp1(X, V, Xq, Method)", "One-dimensional interpolation."),
    ("polyfit", "polyfit(X, Y, Degree)", "Least-squares polynomial coefficients."),
    ("polyval", "polyval(P, X)", "Evaluate a polynomial at X."),
    ("fft", "fft(X, N, Dim)", "Discrete Fourier transform."),
    ("ifft", "ifft(X, N, Dim)", "Inverse discrete Fourier transform."),
    ("conv", "conv(A, B, Shape)", "Convolution of two vectors."),
    ("filter", "filter(B, A, X)", "One-dimensional digital filter."),
    ("ode45", "ode45(OdeFcn, TSpan, Y0, Options)", "Solve a non-stiff ordinary differential equation."),
    ("fsolve", "fsolve(Fun, X0, Options)", "Solve a system of nonlinear equations."),
    ("fminsearch", "fminsearch(Fun, X0, Options)", "Derivative-free unconstrained minimisation."),
    ("fminbnd", "fminbnd(Fun, Lower, Upper, Options)", "Minimise a scalar function on a bounded interval."),
    ("fminunc", "fminunc(Fun, X0, Options)", "Unconstrained minimisation."),
    ("fmincon", "fmincon(Fun, X0, A, B, Aeq, Beq, LB, UB, Nonlcon, Options)", "Constrained nonlinear minimisation."),
    ("lsqnonlin", "lsqnonlin(Fun, X0, LB, UB, Options)", "Solve nonlinear least-squares problems."),
    ("lsqcurvefit", "lsqcurvefit(Fun, X0, XData, YData, LB, UB, Options)", "Fit a nonlinear curve to data."),
    ("linprog", "linprog(F, A, B, Aeq, Beq, LB, UB, Options)", "Solve a linear program."),
    ("quadprog", "quadprog(H, F, A, B, Aeq, Beq, LB, UB, X0, Options)", "Solve a convex quadratic program."),
    ("ga", "ga(FitnessFcn, Nvars, A, B, Aeq, Beq, LB, UB, Options)", "Genetic algorithm for bounded optimisation."),
    ("patternsearch", "patternsearch(Fun, X0, A, B, Aeq, Beq, LB, UB, Options)", "Derivative-free pattern-search minimisation."),
    ("optimset", "optimset(Name, Value, ...)", "Create or update an optimisation options struct."),
    ("optimoptions", "optimoptions(Solver, Name, Value, ...)", "Create solver-specific optimisation options."),
    ("fitlm", "fitlm(X, Y, Modelspec)", "Fit a linear regression model."),
    ("table", "table(Var1, Var2, ..., VariableNames, Value, ...)", "Create a table from column variables."),
    ("readtable", "readtable(File, Name, Value, ...)", "Read a text, CSV or spreadsheet file into a table."),
    ("writetable", "writetable(T, File, Name, Value, ...)", "Write a table to a text, CSV or spreadsheet file."),
    ("readmatrix", "readmatrix(File, Name, Value, ...)", "Read a numeric matrix from a file."),
    ("writematrix", "writematrix(A, File, Name, Value, ...)", "Write a numeric matrix to a file."),
    ("readcell", "readcell(File, Name, Value, ...)", "Read a file into a cell array."),
    ("writecell", "writecell(C, File, Name, Value, ...)", "Write a cell array to a file."),
    ("csvread", "csvread(File, Row, Col)", "Read a comma-separated numeric file."),
    ("csvwrite", "csvwrite(File, A)", "Write a numeric matrix as comma-separated values."),
    ("xlsread", "xlsread(File, Sheet, Range)", "Read numeric data from a spreadsheet."),
    ("xlswrite", "xlswrite(File, A, Sheet, Range)", "Write numeric data to a spreadsheet."),
    ("h5read", "h5read(File, Dataset, Start, Count)", "Read a numeric HDF5 dataset, optionally a hyperslab."),
    ("h5write", "h5write(File, Dataset, Data, Start, Count)", "Write a numeric HDF5 dataset or hyperslab."),
    ("h5create", "h5create(File, Dataset, Size, Name, Value, ...)", "Create an HDF5 dataset."),
    ("imread", "imread(File, Format)", "Read an image file."),
    ("imwrite", "imwrite(A, File, Format)", "Write an image file."),
    ("imshow", "imshow(A, DisplayRange)", "Display an image in a figure."),
    ("imresize", "imresize(A, Scale, Method)", "Resize an image with nearest, bilinear, bicubic or Lanczos interpolation."),
    ("rgb2gray", "rgb2gray(RGB)", "Convert an RGB image to grayscale."),
    ("imfilter", "imfilter(A, H, Boundary, Size)", "Apply a linear filter to an image."),
    ("edge", "edge(A, Method, Threshold)", "Detect edges in an image."),
    ("regionprops", "regionprops(BW, Properties)", "Measure connected-component properties."),
    ("fprintf", "fprintf(FormatSpec, A, ...)", "Write formatted text to the Command Window or a file."),
    ("sprintf", "sprintf(FormatSpec, A, ...)", "Format data into a character vector."),
    ("disp", "disp(X)", "Display a value without printing its name."),
    ("error", "error(Message, A, ...)", "Raise an error with a formatted message."),
    ("warning", "warning(Message, A, ...)", "Display a formatted warning."),
    ("input", "input(Prompt, Choice)", "Request input from the user."),
    ("strsplit", "strsplit(Text, Delimiter)", "Split text at delimiters."),
    ("strjoin", "strjoin(C, Delimiter)", "Join a cellstr with a delimiter."),
    ("regexp", "regexp(Text, Expression, Option)", "Match regular expressions."),
    ("regexprep", "regexprep(Text, Expression, Replace)", "Replace regular-expression matches."),
    ("contains", "contains(Text, Pattern)", "True where text contains a pattern."),
    ("startsWith", "startsWith(Text, Prefix)", "True where text starts with a prefix."),
    ("endsWith", "endsWith(Text, Suffix)", "True where text ends with a suffix."),
    ("upper", "upper(Text)", "Convert text to uppercase."),
    ("lower", "lower(Text)", "Convert text to lowercase."),
    ("num2str", "num2str(A, Format)", "Convert numbers to a character vector."),
    ("str2num", "str2num(Text)", "Convert a character vector to numbers."),
    ("mat2str", "mat2str(A, Precision)", "Convert a matrix to a string that evaluates back to it."),
    ("eval", "eval(Expression)", "Evaluate an M-code expression string."),
    ("feval", "feval(Fun, A, ...)", "Evaluate a function handle or function name."),
    ("cellfun", "cellfun(Fun, C, UniformOutput)", "Apply a function to every cell."),
    ("arrayfun", "arrayfun(Fun, A, UniformOutput)", "Apply a function element-wise to an array."),
    ("struct", "struct(Field, Value, ...)", "Create a structure array."),
    ("fieldnames", "fieldnames(S)", "List field names of a struct or object."),
    ("getfield", "getfield(S, Field)", "Read a structure field."),
    ("setfield", "setfield(S, Field, Value)", "Set a structure field."),
    ("isfield", "isfield(S, Field)", "True when a structure has the named field."),
    ("class", "class(Value)", "Return the class name of a value."),
    ("isa", "isa(Value, ClassName)", "True when a value has the named class."),
    ("validateattributes", "validateattributes(A, Classes, Attributes)", "Validate array class and attribute constraints."),
    ("inputParser", "inputParser()", "Create an input parser object."),
    ("addRequired", "addRequired(Parser, Name, Validator)", "Add a required input to an input parser."),
    ("addOptional", "addOptional(Parser, Name, Default, Validator)", "Add an optional positional input."),
    ("addParameter", "addParameter(Parser, Name, Default, Validator)", "Add a name-value parameter."),
    ("parse", "parse(Parser, Inputs, ...)", "Parse and validate the supplied inputs."),
    ("tic", "tic()", "Start a stopwatch timer."),
    ("toc", "toc(StartTime)", "Read the stopwatch timer."),
    ("save", "save(File, Variables, Format)", "Save workspace variables to a MAT-file."),
    ("load", "load(File, Variables)", "Load variables from a MAT-file."),
    ("clear", "clear(Variables)", "Remove variables from the workspace."),
    ("maxNumCompThreads", "maxNumCompThreads(N)", "Set or query the number of parallel CPU workers."),
    ("parpool", "parpool(Workers)", "Create a parallel pool with the requested worker count."),
    ("gpuArray", "gpuArray(A)", "Move an array to the optional GPU backend."),
    ("gather", "gather(A)", "Copy a GPU array back to the CPU."),
];

/// A call signature found at the cursor.
struct ParamHint {
    name: String,
    doc: String,
    arg: usize,
    params: Vec<String>,
}

/// Locate the innermost function call whose argument list contains
/// `byte_pos`. Returns the callee name and the 1-based argument index.
fn call_hint(text: &str, byte_pos: usize) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    let end = byte_pos.min(bytes.len());
    let mut stack: Vec<(u8, usize, usize)> = Vec::new();
    let mut i = 0usize;
    let mut in_string = false;
    while i < end {
        let b = bytes[i];
        if in_string {
            if b == b'\'' {
                if bytes.get(i + 1) == Some(&b'\'') {
                    i += 2;
                    continue;
                }
                in_string = false;
            }
            i += 1;
            continue;
        }
        match b {
            b'%' => {
                while i < end && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'\'' => {
                let prev = (0..i).rev().find(|&j| !bytes[j].is_ascii_whitespace()).map(|j| bytes[j]);
                if !prev.is_some_and(|p| p.is_ascii_alphanumeric() || matches!(p, b')' | b']' | b'}' | b'\'' | b'.')) {
                    in_string = true;
                }
            }
            b'(' => stack.push((b'(', i, 1)),
            b'[' | b'{' => stack.push((b, i, 0)),
            b')' => {
                while let Some((open, _, _)) = stack.pop() {
                    if open == b'(' {
                        break;
                    }
                }
            }
            b']' | b'}' => {
                while let Some((open, _, _)) = stack.pop() {
                    if open == b'[' || open == b'{' {
                        break;
                    }
                }
            }
            b',' => {
                if let Some(paren) = stack.iter().rposition(|(o, _, _)| *o == b'(') {
                    let bracket_above = stack[paren + 1..].iter().any(|(o, _, _)| *o == b'[' || *o == b'{');
                    if !bracket_above {
                        if let Some((_, _, arg)) = stack.get_mut(paren) {
                            *arg += 1;
                        }
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    let (_, open, arg) = *stack.iter().rev().find(|(o, _, _)| *o == b'(')?;
    let mut j = open;
    while j > 0 && bytes.get(j - 1).is_some_and(u8::is_ascii_whitespace) {
        j -= 1;
    }
    let name_end = j;
    while j > 0 && (bytes[j - 1].is_ascii_alphanumeric() || bytes[j - 1] == b'_' || bytes[j - 1] == b'.') {
        j -= 1;
    }
    if j == name_end {
        return None;
    }
    Some((String::from_utf8_lossy(&bytes[j..name_end]).to_string(), arg))
}

/// Builtin signature table lookup, or the signature of a local function in
/// the buffer. The latter lets hints work for user code without evaluating
/// it.
fn signature_at(name: &str, text: &str) -> Option<(String, String, Vec<String>)> {
    if let Some((_, display, doc)) = SIGNATURES.iter().find(|(n, _, _)| n.eq_ignore_ascii_case(name)) {
        let params = display
            .split_once('(')
            .and_then(|(_, rest)| rest.rsplit_once(')'))
            .map(|(args, _)| args.split(',').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect())
            .unwrap_or_default();
        return Some(((*display).to_string(), (*doc).to_string(), params));
    }
    for line in text.lines() {
        let t = line.trim_start();
        if !t.starts_with("function") || !t["function".len()..].starts_with(|c: char| c.is_whitespace() || c == '[') {
            continue;
        }
        let rest = t["function".len()..].trim_start();
        let rest = rest.split_once('=').map(|(_, r)| r.trim_start()).unwrap_or(rest);
        let Some((fname, tail)) = rest.split_once('(') else { continue };
        if !fname.trim().eq_ignore_ascii_case(name) {
            continue;
        }
        let Some((args, _)) = tail.rsplit_once(')') else { continue };
        let params: Vec<String> = args.split(',').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect();
        return Some((format!("{}({})", fname.trim(), params.join(", ")), "Local function defined in this buffer.".to_string(), params));
    }
    None
}

fn param_hint(name: &str, arg: usize, text: &str) -> Option<ParamHint> {
    let (_signature, doc, params) = signature_at(name, text)?;
    Some(ParamHint { name: name.to_string(), doc, arg: arg.max(1), params })
}

/// `highlight` plus find-bar marks: every match gets a light-yellow
/// background, the current one a stronger amber — like the Editor's find
/// highlighting. `pause_line` (1-based) paints the debugger's stopped line
/// a distinct amber.
fn highlight_matches(
    text: &str,
    default_color: egui::Color32,
    matches: &[(usize, usize)],
    cur: Option<(usize, usize)>,
    pause_line: Option<usize>,
) -> egui::text::LayoutJob {
    let mut job = highlight(text, default_color);
    if matches.is_empty() {
        paint_debug_line(&mut job.sections, text, pause_line);
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
    paint_debug_line(&mut job.sections, text, pause_line);
    job
}

/// Amber background on the debugger's stopped line (1-based; `None` = not
/// paused) — distinct from the red breakpoint dots and paler find marks.
fn paint_debug_line(secs: &mut [egui::text::LayoutSection], text: &str, pause_line: Option<usize>) {
    let Some(ln) = pause_line else { return };
    let starts = line_start_offsets(text);
    let Some(&a) = starts.get(ln.saturating_sub(1)) else { return };
    let b = starts.get(ln).copied().unwrap_or(text.len());
    let bg = egui::Color32::from_rgb(0xF7, 0xD7, 0x6B);
    for sec in secs.iter_mut().filter(|s| s.byte_range.start.0 >= a && s.byte_range.end.0 <= b) {
        sec.format.background = bg;
    }
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

    #[test]
    fn call_hint_tracks_argument_index() {
        let text = "plot(x, y)";
        assert_eq!(call_hint(text, 5), Some(("plot".into(), 1)));
        assert_eq!(call_hint(text, 6), Some(("plot".into(), 1)));
        assert_eq!(call_hint(text, 7), Some(("plot".into(), 2)));
        assert_eq!(call_hint("zeros(3)", 7), Some(("zeros".into(), 1)));
        assert_eq!(call_hint("A(:, 2)", 6), Some(("A".into(), 2)));
        assert!(param_hint("A", 2, "A(:, 2)").is_none());
    }

    #[test]
    fn call_hint_ignores_nested_commas_and_strings() {
        assert_eq!(call_hint("f(g(1, 2), 3)", 11), Some(("f".into(), 2)));
        assert_eq!(call_hint("disp('a, b')", 8), Some(("disp".into(), 1)));
        assert_eq!(call_hint("x = [1, 2]; plot(x, 3)", 20), Some(("plot".into(), 2)));
    }

    #[test]
    fn signature_table_and_local_functions() {
        let (sig, _, params) = signature_at("plot", "").unwrap();
        assert!(sig.starts_with("plot("));
        assert_eq!(params.len(), 3);
        let text = "function [a, b] = pair(x, y)\na = x; b = y;\nend\n";
        let (sig, _, params) = signature_at("pair", text).unwrap();
        assert_eq!(sig, "pair(x, y)");
        assert_eq!(params, vec!["x", "y"]);
        assert!(signature_at("no_such_function", text).is_none());
    }
}
