//! The command window: scrollback, input line, history.

use std::collections::VecDeque;

const MAX_HISTORY: usize = 500;
const MAX_SCROLLBACK: usize = 1_000_000;

/// A `file.m (line n)` target detected in the scrollback — rendered as a
/// hyperlink that opens the file in an editor tab at that line.
pub struct FileLink {
    /// Byte range within the scanned text that forms the link.
    pub start: usize,
    pub end: usize,
    /// The `.m` file as printed (name or path).
    pub file: String,
    /// 1-based line to open at; 0 means no line was printed.
    pub line: usize,
}

/// What [`Repl::ui`] reports back to the app.
#[derive(Default)]
pub struct ReplOut {
    /// A submitted command line (Enter pressed).
    pub line: Option<String>,
    /// `(file, line)` — a scrollback file link was clicked (`line` 0 = none).
    pub open: Option<(String, usize)>,
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '.' | '/' | '\\' | ':' | '-' | '~')
}

/// `line 12` / `(line 12)` at the start of `rest` → `Some(12)`.
fn parse_line_marker(rest: &str) -> Option<usize> {
    let r = rest.trim_start_matches([' ', '\t']);
    let r = r.strip_prefix('(').unwrap_or(r).trim_start_matches([' ', '\t']);
    let r = r.strip_prefix("line")?.trim_start_matches([' ', '\t']);
    let digits: String = r.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

/// First `line <digits>` word in `line` → `(byte_start, byte_end, n)`.
/// Used for `Error: line 3: …` messages that carry no file name.
fn find_bare_line_no(line: &str) -> Option<(usize, usize, usize)> {
    let mut i = 0;
    while let Some(m) = line.get(i..)?.find("line") {
        let s = i + m;
        let bound_ok = s == 0 || line[..s].chars().next_back().is_some_and(|c| !c.is_alphanumeric() && c != '_');
        let after = s + 4;
        let rest = line.get(after..).unwrap_or("");
        let r = rest.trim_start_matches([' ', '\t']);
        let digits_len = r.bytes().take_while(|b| b.is_ascii_digit()).count();
        if bound_ok && digits_len > 0 {
            let n: usize = r.get(..digits_len)?.parse().ok()?;
            return Some((s, after + (rest.len() - r.len()) + digits_len, n));
        }
        i = after;
    }
    None
}

/// `>> run 'path'` (the script/editor echo) → the quoted path, `''` unescaped.
fn run_target(echo: &str) -> Option<&str> {
    let rest = echo.trim_start_matches('>').trim_start().strip_prefix("run")?.trim_start();
    let rest = rest.strip_prefix('\'')?;
    let mut end = 0;
    while end < rest.len() {
        match rest.get(end..)?.find('\'') {
            Some(p) if rest.as_bytes().get(end + p + 1) == Some(&b'\'') => end += p + 2,
            Some(p) => return Some(&rest[..end + p]),
            None => return None,
        }
    }
    None
}

/// Scan scrollback text for file links: `foo.m` / `foo.m (line 12)` tokens on
/// output lines (everything but `>> ` echoes), plus bare `line N` markers on
/// `Error`/`Warning` lines — those resolve against the file named by the most
/// recent `>> run 'path'` echo, like MATLAB's stack links.
pub fn find_links(text: &str) -> Vec<FileLink> {
    let mut out: Vec<FileLink> = Vec::new();
    let mut last_run: Option<String> = None;
    let mut base = 0usize;
    for line in text.split_inclusive('\n') {
        let t = line.trim_start();
        if t.starts_with(">>") {
            last_run = run_target(t).map(str::to_string);
            base += line.len();
            continue;
        }
        let had_m = out.len();
        let mut i = 0;
        while let Some(m) = line.get(i..).and_then(|s| s.find(".m")) {
            let dot = i + m;
            let end = dot + 2;
            let mut start = dot;
            for (bi, c) in line[..dot].char_indices().rev() {
                if is_name_char(c) {
                    start = bi;
                } else {
                    break;
                }
            }
            // The file name must end at `.m` — `foo.mx` is not a link.
            let name_ends = line[end..].chars().next().is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
            if start < dot && name_ends {
                let line_no = parse_line_marker(&line[end..]).unwrap_or(0);
                out.push(FileLink { start: base + start, end: base + end, file: line[start..end].to_string(), line: line_no });
            }
            i = end;
        }
        // `Error: line 3: …` with no file of its own → the last run file.
        if out.len() == had_m && (t.starts_with("Error") || t.starts_with("Warning")) {
            if let (Some(f), Some((s, e, n))) = (&last_run, find_bare_line_no(line)) {
                out.push(FileLink { start: base + s, end: base + e, file: f.clone(), line: n });
            }
        }
        base += line.len();
    }
    out
}

/// One submitted command, timestamped (seconds since the Unix epoch) for the
/// Command History panel's date headers.
#[derive(Clone)]
pub struct HistEntry {
    pub cmd: String,
    pub epoch: u64,
}

pub struct Repl {
    /// Everything printed so far (echo + output + errors).
    pub scrollback: String,
    /// The line being typed.
    pub input: String,
    history: VecDeque<HistEntry>,
    hist_idx: Option<usize>,
    /// Restore input focus after a submit / panel click.
    want_focus: bool,
}

impl Repl {
    pub fn new() -> Self {
        Repl { scrollback: String::new(), input: String::new(), history: VecDeque::new(), hist_idx: None, want_focus: true }
    }

    pub fn push(&mut self, text: &str) {
        self.scrollback.push_str(text);
        if !text.ends_with('\n') {
            self.scrollback.push('\n');
        }
        // Cap the scrollback: keep the tail.
        if self.scrollback.len() > MAX_SCROLLBACK {
            let mut cut = self.scrollback.len() - MAX_SCROLLBACK;
            while cut < self.scrollback.len() && !self.scrollback.is_char_boundary(cut) {
                cut += 1;
            }
            self.scrollback.drain(..cut);
        }
    }

    fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let next = match self.hist_idx {
            None => self.history.len().saturating_sub(1),
            Some(i) => i.saturating_sub(1),
        };
        if let Some(s) = self.history.get(next) {
            self.input = s.cmd.clone();
            self.hist_idx = Some(next);
        }
    }

    fn history_next(&mut self) {
        let Some(i) = self.hist_idx else { return };
        if i + 1 < self.history.len() {
            if let Some(s) = self.history.get(i + 1) {
                self.input = s.cmd.clone();
                self.hist_idx = Some(i + 1);
            }
        } else {
            self.input.clear();
            self.hist_idx = None;
        }
    }

    /// Command-history entries, oldest first.
    pub fn entries(&self) -> &VecDeque<HistEntry> {
        &self.history
    }

    /// Load a command into the input line (Command History click) and focus it.
    pub fn load_input(&mut self, cmd: &str) {
        self.input = cmd.to_string();
        self.hist_idx = None;
        self.want_focus = true;
    }

    /// `clc` — clear the scrollback (history is kept, like MATLAB).
    pub fn clear(&mut self) {
        self.scrollback.clear();
    }

    /// Record a submitted command in the history (the app calls this for every
    /// executed line, whatever its source — typed, replayed, control channel).
    pub fn record(&mut self, cmd: &str) {
        // The clock is only read natively; wasm stores 0 (no history dates).
        #[cfg(not(target_arch = "wasm32"))]
        #[allow(clippy::disallowed_methods)]
        let epoch = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        #[cfg(target_arch = "wasm32")]
        let epoch = 0;
        self.history.push_back(HistEntry { cmd: cmd.to_string(), epoch });
        if self.history.len() > MAX_HISTORY {
            self.history.pop_front();
        }
        self.hist_idx = None;
    }

    /// Tab-completion: extend the identifier under the cursor against
    /// `words` (builtins + workspace vars + user functions). One match
    /// completes it; several extend to the common prefix — or, when the
    /// prefix is already the full common stem, list candidates like MATLAB.
    fn complete(&mut self, words: &[String]) {
        let start = self.input.rfind(|c: char| !(c.is_alphanumeric() || c == '_')).map(|i| i + 1).unwrap_or(0);
        let prefix = self.input[start..].to_string();
        if prefix.is_empty() {
            return;
        }
        let mut matches: Vec<&String> = words.iter().filter(|w| w.starts_with(prefix.as_str()) && w.as_str() != prefix).collect();
        if matches.is_empty() {
            return;
        }
        if matches.len() == 1 {
            self.input.replace_range(start.., matches.swap_remove(0));
            return;
        }
        matches.sort();
        let common = matches.iter().skip(1).fold(matches[0].clone(), |acc, w| {
            let n = acc.bytes().zip(w.bytes()).take_while(|(a, b)| a == b).count();
            acc[..n].to_string()
        });
        if common.len() > prefix.len() {
            self.input.replace_range(start.., &common);
        } else {
            self.push(&format!("{}\n", matches.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("   ")));
        }
    }

    /// Draw the command window. Reports a line to evaluate on Enter and a
    /// `(file, line)` target when a scrollback file link was clicked.
    pub fn ui(&mut self, ui: &mut egui::Ui, words: &[String]) -> ReplOut {
        // Bound the scroll area: inside a content-sized panel an expanding
        // MATLAB puts the `>>` prompt right after the last output line: the
        // prompt lives inside the scroll area and scrolls up with the text
        // (a separate bottom-pinned input leaves a dead gap and no longer
        // looks like the Command Window).
        let mut out = ReplOut::default();
        egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            // The label's galley is cached by egui, but the TextEdit version
            // walked every row each frame — show only the tail anyway so a
            // long session stays cheap.
            const RENDER_TAIL: usize = 64 * 1024;
            let tail = if self.scrollback.len() > RENDER_TAIL {
                let cut = self.scrollback.len() - RENDER_TAIL;
                let start = self.scrollback[..cut].rfind('\n').map(|i| i + 1).unwrap_or(cut);
                &self.scrollback[start..]
            } else {
                &self.scrollback
            };
            let links = find_links(tail);
            let mut job = scrollback_job(tail, ui.visuals().text_color(), &links);
            job.wrap.max_width = ui.available_width();
            let galley = ui.fonts_mut(|f| f.layout_job(job));
            let resp = ui.add(egui::Label::new(galley.clone()).selectable(true));
            // File links: the galley knows where every glyph landed, so each
            // link's rows get an invisible click target (LayoutJob sections
            // can't be clicked on their own).
            let galley_pos = resp.rect.min;
            let row_starts: Vec<usize> = galley
                .rows
                .iter()
                .scan(0usize, |acc, r| {
                    let s = *acc;
                    *acc += r.char_count_including_newline().0;
                    Some(s)
                })
                .collect();
            for (li, link) in links.iter().enumerate() {
                let cs = tail[..link.start].chars().count();
                let ce = cs + tail[link.start..link.end].chars().count();
                for (ri, rect) in link_rects(&galley, galley_pos, &row_starts, cs, ce).into_iter().enumerate() {
                    let tip = if link.line > 0 { format!("open {} at line {}", link.file, link.line) } else { format!("open {}", link.file) };
                    let r = ui.interact(rect, resp.id.with(("errlink", li, ri)), egui::Sense::click()).on_hover_text(tip);
                    if r.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if r.clicked() {
                        out.open = Some((link.file.clone(), link.line));
                    }
                }
            }

            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(">>").monospace().strong());
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.input)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .hint_text("type M-code, Enter to run, Up/Down for history"),
                );
                if resp.has_focus() {
                    if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
                        self.history_prev();
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
                        self.history_next();
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Tab)) {
                        self.complete(words);
                    }
                }
                if self.want_focus {
                    resp.request_focus();
                    self.want_focus = false;
                }
                let entered = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if entered && !self.input.trim().is_empty() {
                    // Echo and history recording happen in `MatApp::run_line`, so
                    // commands from every source (typing, history replay, control
                    // channel) are echoed and logged identically.
                    out.line = Some(std::mem::take(&mut self.input));
                    self.want_focus = true;
                }
            });
        });
        out
    }
}

/// Screen rects covering the character range `[cs, ce)` of a galley painted
/// at `galley_pos` — one rect per visual row the range touches. `row_starts`
/// is the prefix sum of `char_count_including_newline` per row.
fn link_rects(galley: &egui::Galley, galley_pos: egui::Pos2, row_starts: &[usize], cs: usize, ce: usize) -> Vec<egui::Rect> {
    let mut out = Vec::new();
    for (ri, row) in galley.rows.iter().enumerate() {
        let rs = row_starts.get(ri).copied().unwrap_or(0);
        let re = rs + row.char_count_excluding_newline().0;
        if rs > ce {
            break;
        }
        let (a, b) = (cs.max(rs), ce.min(re));
        if b > a {
            let x0 = row.pos.x + row.x_offset((a - rs).into());
            let x1 = row.pos.x + row.x_offset((b - rs).into());
            out.push(egui::Rect::from_min_max(galley_pos + egui::vec2(x0, row.pos.y), galley_pos + egui::vec2(x1, row.pos.y + row.height())));
        }
    }
    out
}

/// Colour the scrollback like the Command Window: `Error …` blocks in red
/// (indented continuation lines stay red), `Warning …` in dark orange, the
/// rest in the default text colour. [`FileLink`] spans are drawn as blue
/// underlined hyperlinks like MATLAB's `file (line n)` error links.
fn scrollback_job(text: &str, default_color: egui::Color32, links: &[FileLink]) -> egui::text::LayoutJob {
    let font = egui::FontId::monospace(13.0);
    let err = egui::Color32::from_rgb(0xB0, 0x00, 0x00);
    let warn = egui::Color32::from_rgb(0xB4, 0x5F, 0x06);
    let link_color = egui::Color32::from_rgb(0x00, 0x72, 0xBD);
    let link_fmt =
        egui::text::TextFormat { font_id: font.clone(), color: link_color, underline: egui::Stroke::new(1.0, link_color), ..Default::default() };
    let mut job = egui::text::LayoutJob::default();
    let mut in_error = false;
    let mut base = 0usize;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let color = if trimmed.starts_with(">> ") || trimmed.starts_with(">>\n") {
            in_error = false;
            default_color
        } else if trimmed.starts_with("Error") {
            in_error = true;
            err
        } else if trimmed.starts_with("Warning") {
            in_error = false;
            warn
        } else if in_error && !line.starts_with(|c: char| c.is_alphanumeric()) && !trimmed.is_empty() {
            err
        } else {
            in_error = false;
            default_color
        };
        let fmt = egui::text::TextFormat { font_id: font.clone(), color, ..Default::default() };
        // Split the line at link spans.
        let mut pos = 0usize;
        for link in links.iter().filter(|l| l.start >= base && l.end <= base + line.len()) {
            let (s, e) = (link.start - base, link.end - base);
            if s > pos {
                job.append(&line[pos..s], 0.0, fmt.clone());
            }
            job.append(&line[s..e], 0.0, link_fmt.clone());
            pos = e;
        }
        if pos < line.len() {
            job.append(&line[pos..], 0.0, fmt);
        }
        base += line.len();
    }
    job
}

impl Default for Repl {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_file_and_line() {
        let t = ">> run 'solver.m'\nError in solver.m (line 12)\n";
        let links = find_links(t);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].file, "solver.m");
        assert_eq!(links[0].line, 12);
        assert_eq!(&t[links[0].start..links[0].end], "solver.m");
    }

    #[test]
    fn links_paths_and_parens() {
        let t = "Error in C:\\work\\tools\\fit.m (line 3)\n";
        let links = find_links(t);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].file, "C:\\work\\tools\\fit.m");
        assert_eq!(links[0].line, 3);
    }

    #[test]
    fn bare_line_no_uses_last_run_file() {
        let t = ">> run 'C:\\proj\\main.m'\nError: line 7: undefined function `nope`\n";
        let links = find_links(t);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].file, "C:\\proj\\main.m");
        assert_eq!(links[0].line, 7);
        // The link covers the `line 7` span.
        assert!(t[links[0].start..links[0].end].starts_with("line"));
    }

    #[test]
    fn echo_lines_and_plain_output_have_no_links() {
        let t = ">> run 'a.m'\n>> edit b.m\nplain output, no error\n";
        assert!(find_links(t).is_empty());
        // A new `>>` line resets the run target: the error no longer links.
        let t2 = ">> run 'a.m'\n>> x = 1\nError: line 4: bad\n";
        let l2 = find_links(t2);
        assert!(l2.is_empty() || l2.iter().all(|l| l.file != "a.m"));
    }

    #[test]
    fn scrollback_job_marks_links() {
        let t = "Error in foo.m (line 5)\n";
        let links = find_links(t);
        let job = scrollback_job(t, egui::Color32::BLACK, &links);
        // The `foo.m` span is a separate, underlined section.
        let link_sec = job.sections.iter().find(|s| &t[s.byte_range.start.0..s.byte_range.end.0] == "foo.m");
        assert!(link_sec.is_some());
        assert_eq!(link_sec.map(|s| s.format.underline), Some(egui::Stroke::new(1.0, egui::Color32::from_rgb(0x00, 0x72, 0xBD))));
    }
}
