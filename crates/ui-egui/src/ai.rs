//! The AI panel — provider setup (BYO API endpoint or a one-click local
//! model), then code help: an "Ask" prompt box, and "Translate comments"
//! which rewrites only the `%` comments of the current editor buffer.
//!
//! All state lives in `matcraft_engine::Session::ai`; this module is just
//! the renderer. Every button runs the matching `ai.*` engine command — the
//! slow ones return `{started: true}` and report progress through
//! `ui.state`'s `ai` section, which the panel reads back next frame.

#![cfg(not(target_arch = "wasm32"))]

use egui::{Color32, RichText};

use serde_json::json;

use crate::MatApp;

/// Green for "ready", red for failures — the hues status notices use.
const OK: Color32 = Color32::from_rgb(0x2F, 0x9E, 0x44);
const ERR: Color32 = Color32::from_rgb(0xE0, 0x3E, 0x3E);

/// The languages offered in the picker; the field stays free-text.
const LANGUAGES: &[&str] = &[
    "English",
    "Arabic",
    "Chinese (Simplified)",
    "Chinese (Traditional)",
    "Dutch",
    "French",
    "German",
    "Hindi",
    "Italian",
    "Japanese",
    "Korean",
    "Polish",
    "Portuguese",
    "Russian",
    "Spanish",
    "Turkish",
    "Ukrainian",
];

fn card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::NONE
        .fill(ui.visuals().faint_bg_color)
        .stroke(egui::Stroke::new(1.0, ui.visuals().window_stroke.color))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::same(8))
        .show(ui, add);
}

/// Run an `ai.*` command; failures are noted into the panel's activity log
/// (the reply itself isn't echoed — the panel narrates instead).
fn run(app: &mut MatApp, id: &str, params: serde_json::Value) {
    let out = app.exec_command(id, params);
    if out.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        if let Some(e) = out.get("error").and_then(serde_json::Value::as_str) {
            app.session.ai_mut().note(format!("{id}: {e}"));
        }
    }
}

enum Action {
    Test,
    UseApi,
    Detect,
    Download,
    LoadLocal,
    ResetProvider,
    Ask,
    TranslateComments,
    ApplyTranslation,
}

/// The AI panel body (called from a right-side `Panel` in `MatApp::ui`).
pub(crate) fn panel(app: &mut MatApp, ui: &mut egui::Ui) {
    let mut action: Option<Action> = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.label(RichText::new("AI").strong());
        ui.add_space(4.0);
        let ai = app.session.ai_mut();

        // ---- provider ------------------------------------------------
        if ai.ready() {
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("✓").color(OK).strong());
                    ui.label(RichText::new(ai.provider_name().unwrap_or_default()).strong());
                });
                ui.label(
                    RichText::new(if ai.provider_is_local() {
                        "Local — nothing leaves this PC"
                    } else {
                        "Cloud — code and text are sent to the endpoint"
                    })
                    .small()
                    .weak(),
                );
                if ui.link("Change provider").clicked() {
                    action = Some(Action::ResetProvider);
                }
            });
            ui.add_space(8.0);
        } else {
            // ---- BYOT card -------------------------------------------
            card(ui, |ui| {
                let ai = app.session.ai_mut();
                ui.label(RichText::new("Use an API provider").strong());
                ui.add_space(4.0);
                ui.label(RichText::new("OpenAI, Azure, Ollama, LM Studio — any OpenAI-compatible endpoint").small().weak());
                ui.add_space(6.0);
                egui::Grid::new("ai-byot").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                    ui.label("Endpoint:");
                    ui.add(egui::TextEdit::singleline(&mut ai.endpoint).desired_width(170.0).hint_text("https://api.openai.com/v1"));
                    ui.end_row();
                    ui.label("Model:");
                    ui.add(egui::TextEdit::singleline(&mut ai.model).desired_width(170.0).hint_text("gpt-4o-mini"));
                    ui.end_row();
                    ui.label("API token:");
                    ui.add(egui::TextEdit::singleline(&mut ai.token).desired_width(170.0).password(true).hint_text("optional for local endpoints"));
                    ui.end_row();
                });
                ui.checkbox(&mut ai.remember_token, "Remember token in app settings (stored unencrypted)");
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.add_enabled(ai.busy.is_none(), egui::Button::new("Test connection")).clicked() {
                        action = Some(Action::Test);
                    }
                    if ui.add_enabled(ai.busy.is_none(), egui::Button::new("Use this provider")).clicked() {
                        action = Some(Action::UseApi);
                    }
                });
                if let Some(res) = &ai.test_result {
                    let (txt, col) = match res {
                        Ok(m) => (m.clone(), OK),
                        Err(m) => (m.clone(), ERR),
                    };
                    ui.label(RichText::new(txt).small().color(col));
                }
            });
            ui.add_space(8.0);

            // ---- local model card --------------------------------------
            card(ui, |ui| {
                let ai = app.session.ai_mut();
                ui.label(RichText::new("Run on this PC").strong());
                ui.add_space(4.0);
                ui.label(RichText::new("One click downloads a curated open model — everything then runs offline").small().weak());
                ui.add_space(6.0);
                if let Some(c) = &ai.caps {
                    let suggested = craft_ai::suggest(c).map(|m| m.name).unwrap_or("none fits");
                    ui.label(
                        RichText::new(format!(
                            "This PC: {:.0} GB RAM · {} threads · {:.0} GB free · AVX2 {} · suggested: {suggested}",
                            c.ram_gb,
                            c.threads,
                            c.disk_free_gb,
                            if c.avx2 { "yes" } else { "no" },
                        ))
                        .small()
                        .weak(),
                    );
                }
                if ui.add_enabled(ai.busy.is_none(), egui::Button::new("Detect this PC")).clicked() {
                    action = Some(Action::Detect);
                }
                if ai.caps.is_some() {
                    ui.add_space(4.0);
                    let chosen = craft_ai::MODELS.iter().position(|m| m.id == ai.local_id).and_then(|i| craft_ai::MODELS.get(i));
                    egui::ComboBox::from_id_salt("ai-model")
                        .selected_text(chosen.map_or("Pick a model".to_string(), |m| m.name.to_string()))
                        .show_ui(ui, |ui| {
                            for m in craft_ai::MODELS {
                                let fits = ai.caps.as_ref().is_none_or(|c| craft_ai::fits(c, m));
                                ui.add_enabled_ui(fits, |ui| {
                                    ui.selectable_value(
                                        &mut ai.local_id,
                                        m.id.to_string(),
                                        format!("{} — {:.0} MB · {}", m.name, m.size_mb, m.license),
                                    )
                                    .on_hover_text(m.langs);
                                });
                            }
                        });
                    let downloaded = ai.downloaded_ids();
                    let picked_downloaded = downloaded.contains(&ai.local_id);
                    ui.add_space(4.0);
                    if let Some((done, total, _bps)) = ai.dl {
                        let frac = if total > 0 { done as f32 / total as f32 } else { 0.0 };
                        ui.add(egui::ProgressBar::new(frac).show_percentage());
                    }
                    ui.horizontal(|ui| {
                        if ui.add_enabled(ai.busy.is_none() && !ai.local_id.is_empty() && !picked_downloaded, egui::Button::new("Download")).clicked()
                        {
                            action = Some(Action::Download);
                        }
                        if ui.add_enabled(ai.busy.is_none() && picked_downloaded, egui::Button::new("Load model")).clicked() {
                            action = Some(Action::LoadLocal);
                        }
                    });
                }
            });
        }

        // ---- ask ------------------------------------------------------
        ui.add_space(10.0);
        let ai = app.session.ai_mut();
        ui.label(RichText::new("Ask about code").strong());
        ui.add_space(4.0);
        ui.add(
            egui::TextEdit::multiline(&mut ai.prompt)
                .desired_rows(3)
                .desired_width(f32::INFINITY)
                .hint_text("e.g. explain what det([1 2; 3 4]) returns, or write a function that …"),
        );
        ui.horizontal(|ui| {
            if ui.add_enabled(ai.ready() && ai.busy.is_none(), egui::Button::new("Ask")).clicked() {
                action = Some(Action::Ask);
            }
            ui.weak("the answer lands below and in ai.status");
        });

        // ---- translate comments ---------------------------------------
        ui.add_space(10.0);
        ui.label(RichText::new("Translate .m comments").strong());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("To:");
            egui::ComboBox::from_id_salt("ai-lang").selected_text(ai.lang.clone()).width(150.0).show_ui(ui, |ui| {
                for l in LANGUAGES {
                    ui.selectable_value(&mut ai.lang, (*l).to_string(), *l);
                }
            });
            if ui.add_enabled(ai.ready() && ai.busy.is_none(), egui::Button::new("Translate comments")).clicked() {
                action = Some(Action::TranslateComments);
            }
        });
        ui.weak("only `%` comment text is sent — code stays verbatim");

        // ---- status + results ------------------------------------------
        if let Some(busy) = &ai.busy {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(12.0));
                ui.label(RichText::new(busy).small().weak());
            });
        }
        if let Some(ans) = &ai.last_answer {
            ui.add_space(8.0);
            ui.separator();
            ui.label(RichText::new("Answer").strong());
            ui.add(egui::Label::new(RichText::new(ans)).selectable(true).wrap());
        }
        if let Some(tr) = &ai.last_translate {
            ui.add_space(8.0);
            ui.separator();
            ui.label(RichText::new(format!("Comments translated to {}", tr.lang)).strong());
            egui::ScrollArea::vertical().id_salt("ai-translated").max_height(180.0).show(ui, |ui| {
                ui.add(egui::Label::new(RichText::new(&tr.text).monospace().small()).selectable(true).wrap());
            });
            if ui.button("Apply to editor").on_hover_text("replace the editor buffer with this translation").clicked() {
                action = Some(Action::ApplyTranslation);
            }
        }

        // ---- activity log -----------------------------------------------
        ui.add_space(10.0);
        if ui.link(if ai.log_open { "Hide activity log" } else { "Activity log" }).clicked() {
            ai.log_open = !ai.log_open;
        }
        if ai.log_open {
            egui::Frame::NONE.fill(ui.visuals().panel_fill).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
                for line in &ai.log {
                    ui.label(RichText::new(line).small().weak());
                }
            });
        }
    });

    // Deferred actions (cannot borrow the session while the scroll closure owns it).
    match action {
        Some(Action::Test) => run(app, "ai.provider.test", json!({})),
        Some(Action::UseApi) => run(app, "ai.provider.set", json!({})),
        Some(Action::Detect) => run(app, "ai.detect", json!({})),
        Some(Action::Download) => run(app, "ai.model.download", json!({})),
        Some(Action::LoadLocal) => run(app, "ai.model.load", json!({})),
        Some(Action::ResetProvider) => run(app, "ai.provider.set", json!({"clear": true})),
        Some(Action::Ask) => {
            let prompt = app.session.ai().prompt.clone();
            run(app, "ai.ask", json!({"prompt": prompt}));
        }
        Some(Action::TranslateComments) => {
            let text = app.cur_editor().map(|e| e.text.clone());
            match text {
                Some(t) => run(app, "ai.comments_translate", json!({"text": t})),
                None => app.session.ai_mut().note("Open an editor buffer first"),
            }
        }
        Some(Action::ApplyTranslation) => {
            let translated = app.session.ai().last_translate.as_ref().map(|t| t.text.clone());
            match (translated, app.cur_editor_mut()) {
                (Some(text), Some(e)) => {
                    e.text = text;
                    e.dirty = true;
                }
                _ => app.session.ai_mut().note("No editor buffer to apply to"),
            }
        }
        None => {}
    }
}
