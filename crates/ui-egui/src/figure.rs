//! Figure rendering: rasterize `matplot` SVG into an egui texture / PNG.
//!
//! The figure model stays headless — `matplot::to_svg` is the single source
//! of truth, and this module turns that SVG into pixels for the Studio
//! figure dock, `ui.render` and PNG capture. Fonts resolve through a
//! `fontdb::Database` loaded once (system fonts, native only).

use resvg::{tiny_skia, usvg};

/// SVG → pixels, with a shared font database.
pub struct Rasterizer {
    fontdb: std::sync::Arc<usvg::fontdb::Database>,
}

impl Rasterizer {
    pub fn new() -> Self {
        let mut db = usvg::fontdb::Database::new();
        #[cfg(not(target_arch = "wasm32"))]
        db.load_system_fonts();
        Rasterizer { fontdb: std::sync::Arc::new(db) }
    }

    /// Rasterize an SVG string at `scale` (1.0 = SVG pixels).
    pub fn svg_to_image(&self, svg: &str, scale: f32) -> Option<egui::ColorImage> {
        let opt = usvg::Options { fontdb: self.fontdb.clone(), ..Default::default() };
        let tree = usvg::Tree::from_str(svg, &opt).ok()?;
        let size = tree.size();
        let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        let w = (size.width() * scale).ceil().max(1.0) as u32;
        let h = (size.height() * scale).ceil().max(1.0) as u32;
        // Cap the bitmap: figure sizes are ours, but guard anyway.
        if w.saturating_mul(h) > 16_000_000 {
            return None;
        }
        let mut pixmap = tiny_skia::Pixmap::new(w, h)?;
        resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
        // tiny-skia yields premultiplied RGBA.
        Some(egui::ColorImage::from_rgba_premultiplied([w as usize, h as usize], pixmap.data()))
    }

    /// Rasterize a figure's SVG to PNG bytes.
    pub fn svg_to_png(&self, svg: &str, scale: f32) -> Result<Vec<u8>, String> {
        let img = self.svg_to_image(svg, scale).ok_or_else(|| "figure does not rasterize".to_string())?;
        encode_png(&img)
    }
}

impl Default for Rasterizer {
    fn default() -> Self {
        Self::new()
    }
}

/// RGBA image → PNG bytes.
pub fn encode_png(img: &egui::ColorImage) -> Result<Vec<u8>, String> {
    let (w, h) = (img.width() as u32, img.height() as u32);
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|e| e.to_string())?;
        let mut bytes = Vec::with_capacity(img.pixels.len() * 4);
        for p in &img.pixels {
            bytes.extend_from_slice(&p.to_array());
        }
        writer.write_image_data(&bytes).map_err(|e| e.to_string())?;
    }
    Ok(out)
}
