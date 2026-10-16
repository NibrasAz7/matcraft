//! `matcraft-web` (L7) — MatCraft Studio in the browser (`wasm32-unknown-unknown`).
//!
//! Runs [`matcraft_ui_egui::MatApp`] inside `eframe::WebRunner`.
//! Build with `trunk build --release` from `apps/matcraft-web` (or verify
//! with `cargo xtask wasm`).
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(target_arch = "wasm32")]
fn main() {
    use matcraft_ui_egui::{MatApp, MatAppOptions};
    use wasm_bindgen::JsCast as _;

    wasm_bindgen_futures::spawn_local(async {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        let Some(canvas) = document.get_element_by_id("matcraft_canvas").and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok()) else {
            return;
        };
        let options = eframe::WebOptions::default();
        let result = eframe::WebRunner::new().start(canvas, options, Box::new(|cc| Ok(Box::new(MatApp::new(cc, MatAppOptions::default()))))).await;
        if let Some(el) = document.get_element_by_id("matcraft_loading") {
            if result.is_ok() {
                el.remove();
            } else {
                el.set_inner_html("<p>MatCraft requires a browser with WebGPU or WebGL2 enabled.</p>");
            }
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let _ = matcraft_ui_egui::MatAppOptions::default();
    let _ = matcraft_engine::Session::new();
    let _ = egui::Color32::BLACK;
    let _ = serde_json::Value::Null;
    eprintln!("matcraft-web targets wasm32-unknown-unknown; build with `trunk build --release` or check with `cargo xtask wasm`");
}
