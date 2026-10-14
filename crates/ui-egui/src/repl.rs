//! The command window: scrollback, input line, history.

use std::collections::VecDeque;

const MAX_HISTORY: usize = 500;
const MAX_SCROLLBACK: usize = 1_000_000;

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

    /// Draw the command window. Returns a line to evaluate on Enter.
    pub fn ui(&mut self, ui: &mut egui::Ui, words: &[String]) -> Option<String> {
        // Bound the scroll area: inside a content-sized panel an expanding
        // MATLAB puts the `>>` prompt right after the last output line: the
        // prompt lives inside the scroll area and scrolls up with the text
        // (a separate bottom-pinned input leaves a dead gap and no longer
        // looks like the Command Window).
        let mut line_out = None;
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
            ui.add(egui::Label::new(egui::RichText::new(tail).monospace()).selectable(true).wrap());

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
                    line_out = Some(std::mem::take(&mut self.input));
                    self.want_focus = true;
                }
            });
        });
        line_out
    }
}

impl Default for Repl {
    fn default() -> Self {
        Self::new()
    }
}
