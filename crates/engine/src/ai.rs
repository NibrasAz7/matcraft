//! AI assistant state — the `ai.*` command family.
//!
//! Two provider kinds, mirroring the sibling apps' craft-ai spec: a BYO
//! OpenAI-compatible endpoint (OpenAI, Azure, Ollama, LM Studio — any
//! `/v1/chat/completions`) or a one-click local GGUF model from the curated
//! registry. Everything slow (handshake, download, load, generation) runs on
//! a worker thread and reports back over one channel; `drain` applies the
//! results. Callers poll progress through `ai.status` / `ui.state`'s `ai`
//! section — the same "smooth informed" flow PdfCraft uses.
//!
//! MatCraft's killer AI use is code help, not document translation: `ai.ask`
//! answers a prompt (optionally storing it in a workspace char variable) and
//! `ai.comments_translate` translates only the `%` comments of a `.m`
//! buffer, keeping code byte-identical because it is never sent.
//!
//! The whole module is native-only — craft-ai's probe/fetch don't build on
//! wasm, so `ai.*` commands are absent there (the panel is hidden too).

use std::sync::mpsc::{channel, Receiver, Sender};

use serde_json::{json, Value as Json};

use craft_ai::fetch::{self, Event};
use craft_ai::{by_id, AiError, GenOpts, OpenAiCompat, Provider, ProviderInfo, Segment};

/// Which backend is configured.
#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Kind {
    /// BYO OpenAI-compatible endpoint.
    Api,
    /// A downloaded local model.
    Local,
}

/// The last "Translate .m comments" result — kept for the panel's preview.
#[derive(Clone, Debug)]
pub struct CommentTranslation {
    pub lang: String,
    /// The buffer with its comment text replaced; code is untouched.
    pub text: String,
}

/// Messages worker threads report back on.
enum Msg {
    /// `provider.test()` finished — a handshake, no user text.
    Tested(Result<ProviderInfo, AiError>),
    /// Download progress event, forwarded.
    Progress(Event),
    /// GGUF + tokenizer are on disk; carries the model id.
    Downloaded(Result<String, AiError>),
    /// A local model finished loading into memory.
    #[cfg(feature = "ai-local")]
    Loaded(Result<Box<dyn Provider>, AiError>),
    /// `ai.ask` finished: the provider comes back with the answer text;
    /// `var`, when set, also receives it in the workspace.
    Answered { var: Option<String>, result: Result<(Box<dyn Provider>, String), AiError> },
    /// `ai.comments_translate` finished: provider + the translated buffer.
    Translated(Result<(Box<dyn Provider>, String), AiError>),
}

/// The whole AI state: persisted config, the live provider, in-flight job
/// bookkeeping and results. `provider` is `Some` only while ready to
/// generate — it is moved into the worker for a job and returned when done.
pub struct Ai {
    pub kind: Kind,
    pub endpoint: String,
    pub model: String,
    /// Kept in memory only unless `remember_token` is on.
    pub token: String,
    pub remember_token: bool,
    /// Target language for comment translation (free text, e.g. "French").
    pub lang: String,
    /// Registry id of the picked local model.
    pub local_id: String,
    /// Registry id of the downloaded+loadable local model.
    pub local_ready: Option<String>,
    pub caps: Option<craft_ai::probe::MachineCaps>,
    provider: Option<Box<dyn Provider>>,
    rx: Option<Receiver<Msg>>,
    /// One-line narration of the current job ("Downloading… 412/986 MB").
    pub busy: Option<String>,
    /// Download progress (done, total, bytes/sec).
    pub dl: Option<(u64, u64, u64)>,
    /// Result of the last `ai.provider.test` (shown beside the button).
    pub test_result: Option<Result<String, String>>,
    /// The panel's prompt box contents (runtime only, never persisted).
    pub prompt: String,
    /// The last `ai.ask`/`ai.complete` answer.
    pub last_answer: Option<String>,
    /// The last comment translation (panel preview / Apply source).
    pub last_translate: Option<CommentTranslation>,
    /// Rolling activity log for the curious.
    pub log: Vec<String>,
    pub log_open: bool,
}

impl Default for Ai {
    fn default() -> Self {
        Self {
            kind: Kind::Api,
            endpoint: "http://localhost:11434/v1".into(),
            model: String::new(),
            token: String::new(),
            remember_token: false,
            lang: "English".into(),
            local_id: String::new(),
            local_ready: None,
            caps: None,
            provider: None,
            rx: None,
            busy: None,
            dl: None,
            test_result: None,
            prompt: String::new(),
            last_answer: None,
            last_translate: None,
            log: Vec::new(),
            log_open: false,
        }
    }
}

impl Ai {
    /// What survives a restart: fields, not the live provider or jobs. The
    /// token is written only when `remember_token` is on — and the UI's
    /// checkbox says it is stored unencrypted. An empty string (not null)
    /// keeps the value TOML-friendly for craft-settings.
    pub fn persist_json(&self) -> Json {
        json!({
            "kind": self.kind,
            "endpoint": self.endpoint,
            "model": self.model,
            "remember_token": self.remember_token,
            "token": if self.remember_token { self.token.as_str() } else { "" },
            "lang": self.lang,
            "local_id": self.local_id,
        })
    }

    /// Restore `persist_json` output; malformed values are ignored, as
    /// everywhere else in settings.
    pub fn restore(&mut self, v: &Json) {
        if let Ok(k) = serde_json::from_value::<Kind>(v.get("kind").cloned().unwrap_or(Json::Null)) {
            self.kind = k;
        }
        for (key, slot) in [
            ("endpoint", &mut self.endpoint),
            ("model", &mut self.model),
            ("token", &mut self.token),
            ("lang", &mut self.lang),
            ("local_id", &mut self.local_id),
        ] {
            if let Some(s) = v.get(key).and_then(Json::as_str) {
                *slot = s.to_string();
            }
        }
        if let Some(b) = v.get("remember_token").and_then(Json::as_bool) {
            self.remember_token = b;
        }
    }

    /// One line in the rolling activity log (also surfaced in `ai.status`).
    pub fn note(&mut self, line: impl Into<String>) {
        self.log.push(line.into());
        const KEEP: usize = 400;
        if self.log.len() > KEEP {
            self.log.drain(..self.log.len() - KEEP);
        }
    }

    /// Is a provider ready to generate right now?
    pub fn ready(&self) -> bool {
        self.provider.is_some()
    }

    /// The active provider's display name, or none.
    pub fn provider_name(&self) -> Option<String> {
        self.provider.as_ref().map(|p| p.name())
    }

    pub fn provider_is_local(&self) -> bool {
        self.provider.as_ref().is_some_and(|p| p.is_local())
    }

    /// Forget the provider (and only it — settings stay for next time).
    pub fn reset_provider(&mut self) {
        self.provider = None;
        self.note("Provider cleared — pick or configure one below");
    }

    /// Adopt the stored endpoint/model/token as the active provider.
    /// Returns false (and notes why) when either required field is empty.
    pub fn configure_api(&mut self) -> bool {
        if self.endpoint.trim().is_empty() || self.model.trim().is_empty() {
            self.note("Endpoint and model are both required");
            return false;
        }
        self.provider =
            Some(Box::new(OpenAiCompat::new(self.endpoint.clone(), (!self.token.is_empty()).then(|| self.token.clone()), self.model.clone())));
        self.note(format!("Provider ready: {} — code and text will be sent to {}", self.model, self.endpoint));
        true
    }

    fn busy_with(&mut self, what: &str) {
        self.busy = Some(what.to_string());
        self.note(what);
    }

    /// Start a worker job; results arrive on `rx` and land in `drain`. The
    /// UI repaints on a short timer while `busy` is set — there is no
    /// egui::Context down here (layering), only channels.
    fn spawn(&mut self, job: impl FnOnce(Sender<Msg>) + Send + 'static) {
        let (tx, rx) = channel::<Msg>();
        self.rx = Some(rx);
        std::thread::spawn(move || {
            job(tx);
        });
    }

    /// `ai.provider.test` — a handshake only, never user text. Validates
    /// synchronously (an empty endpoint fails cleanly, no network).
    pub fn test_endpoint(&mut self) -> Result<(), String> {
        if self.endpoint.trim().is_empty() {
            return Err("no AI endpoint configured".to_string());
        }
        let mut p = OpenAiCompat::new(self.endpoint.clone(), (!self.token.is_empty()).then(|| self.token.clone()), self.model.clone());
        self.test_result = None;
        self.busy_with("Testing the connection…");
        self.spawn(move |tx| {
            let _ = tx.send(Msg::Tested(p.test()));
        });
        Ok(())
    }

    /// `ai.detect` — probe once, pick the suggested model. `None` when no
    /// usable cache dir exists on this system.
    pub fn detect(&mut self) -> Option<&craft_ai::probe::MachineCaps> {
        let dir = craft_ai::probe::models_dir()?;
        let caps = craft_ai::probe::probe(&dir);
        if self.local_id.is_empty() {
            if let Some(m) = craft_ai::suggest(&caps) {
                self.local_id = m.id.to_string();
                self.note(format!(
                    "Suggested {} — fits your {:.0} GB RAM ({:.0} GB free disk; est. ~{:.0} tok/s)",
                    m.name,
                    caps.ram_gb,
                    caps.disk_free_gb,
                    caps.est_tokens_per_sec(m.size_mb),
                ));
            }
        }
        self.caps = Some(caps);
        self.caps.as_ref()
    }

    /// `ai.model.download` — resumable fetch of the picked (or given) model,
    /// tokenizer included. Existing files are verified and reused.
    pub fn download(&mut self, id: Option<&str>) -> Result<String, String> {
        if self.busy.is_some() {
            return Err(format!("busy — wait for the current job first ({})", self.busy.as_deref().unwrap_or("")));
        }
        let id = id.map(str::to_string).unwrap_or_else(|| self.local_id.clone());
        let entry = by_id(&id).ok_or_else(|| format!("unknown model `{id}` — `ai.models.list` has the ids"))?;
        let Some(dir) = craft_ai::probe::models_dir() else {
            return Err("no usable cache folder on this system".to_string());
        };
        self.local_id = id.to_string();
        self.busy_with(&format!("Downloading {} ({:.0} MB)…", entry.name, entry.size_mb));
        self.dl = Some((0, 0, 0));
        self.spawn(move |tx| {
            let report = |e: &Event| {
                let _ = tx.send(Msg::Progress(e.clone()));
            };
            let out = fetch::download(entry, &dir, report, None).and_then(|_| fetch::download_tokenizer(entry, &dir));
            let _ = tx.send(Msg::Downloaded(out.map(|_| entry.id.to_string())));
        });
        Ok(id)
    }

    /// `ai.model.load` — move a downloaded model into memory (the slow part).
    /// Needs the `ai-local` feature (candle); without it this fails with a
    /// clean error — the model still downloads fine.
    #[cfg(feature = "ai-local")]
    pub fn load(&mut self, id: Option<&str>) -> Result<String, String> {
        if self.busy.is_some() {
            return Err(format!("busy — wait for the current job first ({})", self.busy.as_deref().unwrap_or("")));
        }
        let id = id
            .map(str::to_string)
            .or_else(|| self.local_ready.clone())
            .or_else(|| self.downloaded_ids().first().cloned())
            .ok_or_else(|| "no downloaded model — run `ai.model.download` first".to_string())?;
        let Some(entry) = by_id(&id) else {
            return Err(format!("unknown model `{id}`"));
        };
        let Some(dir) = craft_ai::probe::models_dir() else {
            return Err("no usable cache folder on this system".to_string());
        };
        if !fetch::is_downloaded(entry, &dir) {
            return Err(format!("`{id}` is not downloaded yet"));
        }
        self.busy_with(&format!("Loading {} — this takes a moment…", entry.name));
        self.spawn(move |tx| {
            let gguf = dir.join(entry.file);
            let tok = fetch::tokenizer_path(entry, &dir);
            let r = craft_ai::local::LocalGguf::load(entry, &gguf, &tok).map(|m| Box::new(m) as Box<dyn Provider>);
            let _ = tx.send(Msg::Loaded(r));
        });
        Ok(id)
    }

    /// Without `ai-local` (the default build) loading is a clean error.
    #[cfg(not(feature = "ai-local"))]
    pub fn load(&mut self, _id: Option<&str>) -> Result<String, String> {
        Err("this build has no local inference (compiled without the `ai-local` feature — candle) — the model still downloads fine".to_string())
    }

    /// Models already sitting in the cache (for the "load again" path).
    pub fn downloaded_ids(&self) -> Vec<String> {
        craft_ai::probe::models_dir()
            .map(|dir| craft_ai::MODELS.iter().filter(|m| fetch::is_downloaded(m, &dir)).map(|m| m.id.to_string()).collect())
            .unwrap_or_default()
    }

    /// `ai.complete` — one synchronous completion. Blocks the caller (fine
    /// for CLI/agents; the UI's Ask button uses the async `ask` instead).
    pub fn complete(&mut self, prompt: &str, max_tokens: Option<u32>) -> Result<String, AiError> {
        let p = self.provider.as_mut().ok_or(AiError::NotConfigured)?;
        let mut opts = GenOpts::default();
        if let Some(m) = max_tokens {
            opts.max_tokens = m;
        }
        let text = p.complete(prompt, &opts)?;
        self.last_answer = Some(text.clone());
        Ok(text)
    }

    /// `ai.ask` — async completion: the provider moves onto a worker, the
    /// answer lands in `last_answer` (and `var`'s workspace char) on `drain`.
    pub fn ask(&mut self, prompt: String, var: Option<String>, max_tokens: Option<u32>) -> Result<(), String> {
        let Some(mut provider) = self.provider.take() else {
            return Err("no AI provider configured — set one up with `ai.provider.set`".to_string());
        };
        if self.busy.is_some() {
            self.provider = Some(provider);
            return Err(format!("busy — wait for the current job first ({})", self.busy.as_deref().unwrap_or("")));
        }
        self.busy_with("Asking the AI…");
        self.spawn(move |tx| {
            let mut opts = GenOpts::default();
            if let Some(m) = max_tokens {
                opts.max_tokens = m;
            }
            let r = provider.complete(&prompt, &opts);
            let out = r.map(move |text| (provider, text));
            let _ = tx.send(Msg::Answered { var, result: out });
        });
        Ok(())
    }

    /// `ai.translate` — synchronous text translation via the segmented path
    /// (URLs, placeholders and number-runs are never sent).
    pub fn translate(&mut self, text: &str, lang: &str) -> Result<String, AiError> {
        let p = self.provider.as_mut().ok_or(AiError::NotConfigured)?;
        let segs = craft_ai::segment(text, &craft_ai::PdfGrammar);
        let parts = craft_ai::translate_segments(p.as_mut(), &segs, lang)?;
        Ok(parts.concat())
    }

    /// `ai.comments_translate` — async: translate only the `%` comments of a
    /// `.m` buffer; the result lands in `last_translate` on `drain`.
    pub fn translate_comments(&mut self, text: String, lang: Option<String>) -> Result<(), String> {
        let Some(mut provider) = self.provider.take() else {
            return Err("no AI provider configured — set one up with `ai.provider.set`".to_string());
        };
        if self.busy.is_some() {
            self.provider = Some(provider);
            return Err(format!("busy — wait for the current job first ({})", self.busy.as_deref().unwrap_or("")));
        }
        let lang = lang.unwrap_or_else(|| self.lang.clone());
        let n = mcode_segments(&text).iter().filter(|s| matches!(s, Segment::Text(_))).count();
        if n == 0 {
            self.provider = Some(provider);
            return Err("the buffer has no `%` comments to translate".to_string());
        }
        self.busy_with(&format!("Translating {n} comment(s) to {lang}…"));
        self.spawn(move |tx| {
            let segs = mcode_segments(&text);
            let out = craft_ai::translate_segments(provider.as_mut(), &segs, &lang).map(|parts| (provider, parts.concat()));
            let _ = tx.send(Msg::Translated(out));
        });
        Ok(())
    }

    /// Apply worker results — called once per frame by the app (and by
    /// headless callers polling `ai.status`). `lang` is the workspace for
    /// `ai.ask`'s `var` injection. Returns true when a message landed.
    pub fn drain(&mut self, lang: &mut matlang::Session) -> bool {
        let mut got = false;
        loop {
            let msg = match self.rx.as_ref().map(|rx| rx.try_recv()) {
                Some(Ok(m)) => m,
                _ => break,
            };
            got = true;
            match msg {
                Msg::Tested(Ok(info)) => {
                    self.busy = None;
                    let mut line = info.label;
                    if !info.models.is_empty() {
                        line = format!("{line} — models: {}", info.models.iter().take(6).cloned().collect::<Vec<_>>().join(", "));
                    }
                    self.test_result = Some(Ok(line.clone()));
                    self.note(format!("Connection OK: {line}"));
                }
                Msg::Tested(Err(e)) => {
                    self.busy = None;
                    self.test_result = Some(Err(e.to_string()));
                    self.note(format!("Connection failed: {e}"));
                }
                Msg::Progress(Event::Started { total, resumed_from }) => {
                    self.dl = Some((resumed_from, total, 0));
                }
                Msg::Progress(Event::Progress { done, total, bytes_per_sec }) => {
                    self.dl = Some((done, total, bytes_per_sec));
                    self.busy = Some(match total {
                        0 => format!("Downloading… {:.0} MB", done as f64 / 1e6),
                        t => format!("Downloading… {:.0}/{:.0} MB ({:.0}%)", done as f64 / 1e6, t as f64 / 1e6, done as f64 / t as f64 * 100.0),
                    });
                }
                Msg::Progress(Event::Verifying) => self.busy = Some("Verifying checksum…".into()),
                Msg::Progress(Event::Done { verified }) => {
                    self.note(if verified { "Checksum verified" } else { "Download finished (no checksum available — unverified)" });
                }
                Msg::Downloaded(Ok(id)) => {
                    self.busy = None;
                    self.dl = None;
                    self.local_ready = Some(id);
                    self.note("Download complete — ready to load".to_string());
                }
                Msg::Downloaded(Err(e)) => {
                    self.busy = None;
                    self.dl = None;
                    self.note(format!("Download failed: {e}"));
                }
                #[cfg(feature = "ai-local")]
                Msg::Loaded(Ok(p)) => {
                    self.busy = None;
                    self.note(format!("Local model ready: {} — nothing leaves this PC", p.name()));
                    self.provider = Some(p);
                }
                #[cfg(feature = "ai-local")]
                Msg::Loaded(Err(e)) => {
                    self.busy = None;
                    self.note(format!("Model failed to load: {e}"));
                }
                Msg::Answered { var, result } => {
                    self.busy = None;
                    match result {
                        Ok((p, text)) => {
                            self.provider = Some(p);
                            self.note(format!("AI answered ({} chars)", text.chars().count()));
                            if let Some(v) = var {
                                lang.set(v.clone(), matlang::Value::Str(text.clone()));
                                self.note(format!("Answer stored in `{v}`"));
                            }
                            self.last_answer = Some(text);
                        }
                        Err(e) => self.note(format!("Ask failed: {e}")),
                    }
                }
                Msg::Translated(result) => {
                    self.busy = None;
                    match result {
                        Ok((p, text)) => {
                            self.provider = Some(p);
                            self.last_translate = Some(CommentTranslation { lang: self.lang.clone(), text });
                            self.note("Comment translation finished".to_string());
                        }
                        Err(e) => self.note(format!("Comment translation stopped: {e}")),
                    }
                }
            }
        }
        got
    }

    /// The curated registry with per-model download/fit state — shared by
    /// `ai.models.list` and `ai.status`.
    pub fn models_json(&self) -> Vec<Json> {
        let downloaded = self.downloaded_ids();
        craft_ai::MODELS
            .iter()
            .map(|m| {
                json!({
                    "id": m.id,
                    "name": m.name,
                    "size_mb": m.size_mb,
                    "license": m.license,
                    "langs": m.langs,
                    "downloaded": downloaded.iter().any(|d| d == m.id),
                    "fits": self.caps.as_ref().map(|c| craft_ai::fits(c, m)),
                    "loaded": self.local_ready.as_deref() == Some(m.id) && self.provider_is_local(),
                })
            })
            .collect()
    }

    /// `ui.state`'s `ai` section — the headless `ai.status` payload. The
    /// token is never in it.
    pub fn status_json(&self) -> Json {
        json!({
            "kind": self.kind,
            "endpoint": self.endpoint,
            "model": self.model,
            "lang": self.lang,
            "local_id": self.local_id,
            "ready": self.ready(),
            "provider": self.provider_name(),
            "provider_is_local": self.provider_is_local(),
            "busy": self.busy,
            "download": self.dl.map(|(done, total, bps)| json!({"done": done, "total": total, "bytes_per_sec": bps})),
            "test_result": self.test_result.as_ref().map(|r| match r {
                Ok(m) => json!({"ok": m}),
                Err(m) => json!({"error": m}),
            }),
            "caps": self.caps.as_ref().map(|c| json!({
                "ram_gb": c.ram_gb, "threads": c.threads, "disk_free_gb": c.disk_free_gb, "avx2": c.avx2,
            })),
            "suggested": self.caps.as_ref().and_then(|c| craft_ai::suggest(c)).map(|m| m.id),
            "models": self.models_json(),
            "last_answer": self.last_answer,
            "translation": self.last_translate.as_ref().map(|t| json!({"lang": t.lang, "chars": t.text.chars().count()})),
            "log": self.log.iter().rev().take(20).rev().cloned().collect::<Vec<_>>(),
        })
    }
}

/// Split a `.m` buffer into translate segments: every `%` comment's text is
/// `Text`, all code (and the `%` marker itself, plus `%%` section breaks)
/// stays `Protected` and is never sent. A `%` inside a `'…'` string does
/// not start a comment — quote parity per line (transpose `'` after code
/// can only leave a comment untranslated, never corrupt code).
pub fn mcode_segments(text: &str) -> Vec<Segment> {
    let mut segs = Vec::new();
    for line in text.split_inclusive('\n') {
        let mut quotes = 0usize;
        let mut comment_at = None;
        // `%` and `'` are ASCII, so byte positions are char boundaries.
        for (i, b) in line.bytes().enumerate() {
            match b {
                b'\'' => quotes += 1,
                b'%' if quotes.is_multiple_of(2) => {
                    comment_at = Some(i);
                    break;
                }
                _ => {}
            }
        }
        let Some(mut i) = comment_at else {
            segs.push(Segment::Protected(line.to_string()));
            continue;
        };
        while line.as_bytes().get(i + 1) == Some(&b'%') {
            i += 1;
        }
        // The whitespace framing the comment text stays `Protected` too —
        // the provider trims replies, so a newline inside a `Text` segment
        // would never come back and the buffer would lose its line break.
        let rest = &line[i + 1..];
        let trimmed = rest.trim();
        if trimmed.is_empty() {
            segs.push(Segment::Protected(line.to_string()));
        } else {
            let lead = rest.len() - rest.trim_start().len();
            segs.push(Segment::Protected(line[..=i + lead].to_string()));
            segs.push(Segment::Text(trimmed.to_string()));
            segs.push(Segment::Protected(rest[lead + trimmed.len()..].to_string()));
        }
    }
    segs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcode_segments_keep_code_protected() {
        let segs = mcode_segments("% header\nx = 1 % trailing\nfprintf('%d%%')\n%% section\n");
        let concat: String = segs.iter().map(|s| s.source()).collect();
        assert_eq!(concat, "% header\nx = 1 % trailing\nfprintf('%d%%')\n%% section\n");
        let texts: Vec<&str> = segs
            .iter()
            .filter_map(|s| match s {
                Segment::Text(t) => Some(t.as_str()),
                Segment::Protected(_) => None,
            })
            .collect();
        // `%%` section breaks are comments too; `%` inside a string is not.
        assert_eq!(texts, vec!["header", "trailing", "section"]);
    }

    #[test]
    fn persist_omits_token_unless_remembered() {
        let mut ai = Ai { token: "secret".to_string(), ..Default::default() };
        assert_eq!(ai.persist_json()["token"], json!(""));
        ai.remember_token = true;
        assert_eq!(ai.persist_json()["token"], json!("secret"));
    }

    #[test]
    fn restore_roundtrips() {
        let ai = Ai {
            endpoint: "http://x/v1".into(),
            model: "m".into(),
            lang: "French".into(),
            local_id: "qwen2.5-0.5b-q4km".into(),
            ..Default::default()
        };
        let v = ai.persist_json();
        let mut back = Ai::default();
        back.restore(&v);
        assert_eq!(back.endpoint, "http://x/v1");
        assert_eq!(back.lang, "French");
        assert_eq!(back.local_id, "qwen2.5-0.5b-q4km");
    }
}
