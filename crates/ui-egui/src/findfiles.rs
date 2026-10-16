//! Find in Files — search `*.m` files under the current folder (Ctrl+Shift+F),
//! MATLAB's "Find Files" tool. Results list `file:line: preview`; clicking a
//! hit opens the file in the editor at that line.

use std::path::{Path, PathBuf};

/// One matching line.
pub struct FindHit {
    pub path: PathBuf,
    /// 1-based line number.
    pub line: usize,
    /// The matched line, trimmed (capped at 160 chars).
    pub preview: String,
}

/// State of the Find in Files window.
#[derive(Default)]
pub struct FindFiles {
    pub open: bool,
    pub query: String,
    /// Case-sensitive matching.
    pub case: bool,
    pub hits: Vec<FindHit>,
    /// A search ran at least once (drives the "(no matches)" hint).
    pub searched: bool,
    /// The last search hit a cap — shown as a note.
    pub truncated: bool,
    /// Focus the query field next frame.
    pub want_focus: bool,
}

const MAX_FILES: usize = 20_000;
const MAX_HITS: usize = 1_000;
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_DEPTH: usize = 24;

/// ASCII case-insensitive substring test.
fn contains_ci(hay: &str, needle: &str) -> bool {
    let n: Vec<char> = needle.chars().collect();
    if n.is_empty() {
        return false;
    }
    hay.char_indices().any(|(i, _)| {
        let mut it = hay[i..].chars();
        n.iter().all(|&b| it.next().is_some_and(|a| a.eq_ignore_ascii_case(&b)))
    })
}

/// Search every `*.m` file under `root` (recursive, hidden dirs and `target`
/// skipped, depth/size/hit caps keep it bounded). Returns hits and whether
/// the walk or the hit list was truncated.
pub fn search(root: &Path, query: &str, case: bool) -> (Vec<FindHit>, bool) {
    let mut hits = Vec::new();
    let mut truncated = false;
    let mut files = 0usize;
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    'walk: while let Some((dir, depth)) = stack.pop() {
        if depth > MAX_DEPTH {
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.filter_map(|e| e.ok()) {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let p = e.path();
            let Ok(meta) = e.metadata() else { continue };
            if meta.is_dir() {
                if name != "target" {
                    stack.push((p, depth + 1));
                }
                continue;
            }
            if !name.ends_with(".m") {
                continue;
            }
            files += 1;
            if files > MAX_FILES {
                truncated = true;
                break 'walk;
            }
            if meta.len() > MAX_FILE_BYTES {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&p) else { continue };
            for (i, l) in text.lines().enumerate() {
                let found = if case { l.contains(query) } else { contains_ci(l, query) };
                if found {
                    hits.push(FindHit { path: p.clone(), line: i + 1, preview: l.trim().chars().take(160).collect() });
                    if hits.len() >= MAX_HITS {
                        truncated = true;
                        break 'walk;
                    }
                }
            }
        }
    }
    (hits, truncated)
}

/// The window's contents: query row, results list. Returns `(path, line)`
/// when a hit is clicked.
pub fn ui(ui: &mut egui::Ui, ff: &mut FindFiles, cwd: &Path) -> Option<(PathBuf, usize)> {
    let mut out = None;
    let mut go = false;
    ui.horizontal(|ui| {
        ui.label("Find:");
        let r = ui.add(egui::TextEdit::singleline(&mut ff.query).desired_width(220.0).id(egui::Id::new("find-files-query")));
        if std::mem::take(&mut ff.want_focus) {
            r.request_focus();
        }
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            go = true;
        }
        if ui.selectable_label(ff.case, "Aa").on_hover_text("match case").clicked() {
            ff.case = !ff.case;
        }
        if ui.button("Search").clicked() {
            go = true;
        }
    });
    ui.weak(format!("*.m files under {}", cwd.display()));
    if go && !ff.query.is_empty() {
        let (hits, truncated) = search(cwd, &ff.query, ff.case);
        ff.hits = hits;
        ff.truncated = truncated;
        ff.searched = true;
    }
    ui.separator();
    if ff.truncated {
        ui.weak("(result list truncated)");
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for h in &ff.hits {
            let name = h.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| h.path.display().to_string());
            let label = format!("{}:{}: {}", name, h.line, h.preview);
            let r = ui.selectable_label(false, egui::RichText::new(label).monospace().size(11.5));
            let r = r.on_hover_text(format!("{}:{}", h.path.display(), h.line));
            if r.clicked() {
                out = Some((h.path.clone(), h.line));
            }
        }
        if ff.searched && ff.hits.is_empty() {
            ui.weak("(no matches)");
        }
        if !ff.searched {
            ui.weak("type a query and press Enter");
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_finds_hits_in_m_files_only() {
        let dir = std::env::temp_dir().join("matcraft_findfiles_test");
        let sub = dir.join("sub");
        let _ = std::fs::create_dir_all(&sub);
        let a = dir.join("alpha.m");
        let b = sub.join("beta.m");
        let c = dir.join("note.txt");
        std::fs::write(&a, "x = 1;\nplot(x);\n").unwrap();
        std::fs::write(&b, "% comment\ny = plot(1:3);\n").unwrap();
        std::fs::write(&c, "plot in a txt file\n").unwrap();
        let (hits, truncated) = search(&dir, "plot", true);
        assert!(!truncated);
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().all(|h| h.path.extension().is_some_and(|e| e == "m")));
        let a_hit = hits.iter().find(|h| h.path == a).unwrap();
        assert_eq!(a_hit.line, 2);
        // Case-insensitive.
        let (ci, _) = search(&dir, "PLOT", false);
        assert_eq!(ci.len(), 2);
        let (cs_hits, _) = search(&dir, "PLOT", true);
        assert!(cs_hits.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
