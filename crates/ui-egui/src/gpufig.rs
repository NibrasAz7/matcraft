//! Interactive wgpu 3-D figure viewport (gaps.md G26).
//!
//! Figures holding `plot3`/`mesh`/`surf`/`quiver3`/`contour3` series get a
//! live GPU viewport inside the figure dock instead of the static SVG
//! bitmap: drag orbits the camera (writing `fig.view` back to the session,
//! like the 2-D path writes `xlim`), scroll zooms, Shift/right-drag pans.
//! Geometry and the orthographic projection mirror [`matplot::to_svg`]'s
//! 3-D path so the viewport and `saveas` exports agree.
//!
//! egui's callback render pass has no depth attachment, so surf faces are
//! painter-sorted far→near per frame on the CPU — the same ordering rule
//! `to_svg` uses — while lines draw last without a depth test (a `mesh`
//! wireframe is its own geometry and box edges stay readable).

use std::sync::Arc;

use egui_wgpu::wgpu;
use matplot::{Figure, SeriesKind};

/// One vertex: normalized position ([-1,1]³ data space) + RGBA colour.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vert {
    pos: [f32; 3],
    col: [f32; 4],
}

/// A triangle plus its normalized-space centroid for painter sorting.
struct Tri {
    v: [Vert; 3],
    c: [f64; 3],
}

/// Screen-space viewport state. `az`/`el` live in `fig.view` (session
/// state); `zoom`/`pan` stay in the app.
#[derive(Clone, Copy)]
pub struct View3d {
    pub zoom: f32,
    pub pan: [f32; 2],
}

impl Default for View3d {
    fn default() -> Self {
        View3d { zoom: 1.0, pan: [0.0, 0.0] }
    }
}

/// Pre-built geometry for one figure revision — line vertex list, sortable
/// triangle list, and the data limits (for tick labels).
pub struct Geom {
    pub id: usize,
    lines: Vec<Vert>,
    tris: Vec<Tri>,
    pub bounds: [f64; 6],
}

static GEOM_ID: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);

fn vert(x: f64, y: f64, z: f64, c: matplot::Rgb) -> Vert {
    Vert { pos: [x as f32, y as f32, z as f32], col: [c.0 as f32 / 255.0, c.1 as f32 / 255.0, c.2 as f32 / 255.0, 1.0] }
}

/// Flatten a figure's 3-D series into normalized-space GPU vertex lists —
/// the same vertex order and colouring rules as `to_svg`'s 3-D path.
pub fn build_geom(fig: &Figure) -> Geom {
    let mut b = [f64::INFINITY; 3];
    let mut t = [f64::NEG_INFINITY; 3];
    for s in fig.series.iter().filter(|s| s.is_3d()) {
        for (i, src) in [&s.x, &s.y, &s.z].iter().enumerate() {
            for &v in src.iter() {
                if v.is_finite() {
                    b[i] = b[i].min(v);
                    t[i] = t[i].max(v);
                }
            }
        }
    }
    let (x0, x1) = matplot::limits(b[0], t[0], fig.xlim);
    let (y0, y1) = matplot::limits(b[1], t[1], fig.ylim);
    let (z0, z1) = matplot::limits(b[2], t[2], fig.zlim);
    let (cl, ch) = matplot::caxis(fig);
    let cspan = (ch - cl).abs().max(f64::EPSILON);
    let norm = |v: f64, lo: f64, hi: f64| (v - (lo + hi) / 2.0) / (hi - lo).abs().max(f64::EPSILON) * 2.0;
    let n3 = |p: (f64, f64, f64)| (norm(p.0, x0, x1), norm(p.1, y0, y1), norm(p.2, z0, z1));

    let mut lines: Vec<Vert> = Vec::new();
    let mut tris: Vec<Tri> = Vec::new();
    let mut seg = |a: (f64, f64, f64), bq: (f64, f64, f64), c: matplot::Rgb| {
        let (a, bq) = (n3(a), n3(bq));
        lines.push(vert(a.0, a.1, a.2, c));
        lines.push(vert(bq.0, bq.1, bq.2, c));
    };

    for s in fig.series.iter().filter(|s| s.is_3d()) {
        match s.kind {
            SeriesKind::Line3 => {
                let col = s.spec.color.unwrap_or(matplot::CYCLE[0]);
                for i in 1..s.x.len().min(s.y.len()).min(s.z.len()) {
                    let (px, py, pz) = (s.x[i - 1], s.y[i - 1], s.z[i - 1]);
                    let (x, y, z) = (s.x[i], s.y[i], s.z[i]);
                    if [px, py, pz, x, y, z].iter().all(|v| v.is_finite()) {
                        seg((px, py, pz), (x, y, z), col);
                    }
                }
            }
            SeriesKind::Quiver3 => {
                let col = s.spec.color.unwrap_or(matplot::CYCLE[0]);
                for i in 0..s.x.len().min(s.y.len()).min(s.z.len()).min(s.vectors.len()) {
                    let (u, v, w) = s.vectors[i];
                    seg((s.x[i], s.y[i], s.z[i]), (s.x[i] + u, s.y[i] + v, s.z[i] + w), col);
                }
            }
            SeriesKind::Contour3 | SeriesKind::Contour3Base => {
                let base = z0 + 0.01 * (z1 - z0).max(f64::EPSILON);
                let col = s.spec.color.unwrap_or(matplot::CYCLE[0]);
                for lv in matplot::contour_levels(s, cl, ch) {
                    let z = if s.kind == SeriesKind::Contour3 { lv } else { base };
                    for (a, bq) in matplot::contour_segments(s, lv) {
                        seg((a.0, a.1, z), (bq.0, bq.1, z), col);
                    }
                }
            }
            SeriesKind::Mesh | SeriesKind::Surf => {
                let (rows, cols) = s.zshape;
                let vtx = |r: usize, c: usize| -> Option<(f64, f64, f64)> {
                    let i = c * rows + r;
                    match (s.x.get(i), s.y.get(i), s.z.get(i)) {
                        (Some(&x), Some(&y), Some(&z)) if x.is_finite() && y.is_finite() && z.is_finite() => Some((x, y, z)),
                        _ => None,
                    }
                };
                if s.kind == SeriesKind::Mesh {
                    for r in 0..rows {
                        for c in 0..cols.saturating_sub(1) {
                            if let (Some(a), Some(bq)) = (vtx(r, c), vtx(r, c + 1)) {
                                seg(a, bq, fig.cmap.map(((a.2 + bq.2) / 2.0 - cl) / cspan));
                            }
                        }
                    }
                    for c in 0..cols {
                        for r in 0..rows.saturating_sub(1) {
                            if let (Some(a), Some(bq)) = (vtx(r, c), vtx(r + 1, c)) {
                                seg(a, bq, fig.cmap.map(((a.2 + bq.2) / 2.0 - cl) / cspan));
                            }
                        }
                    }
                } else {
                    // Surf: quads → two triangles, flat per-face colour by
                    // mean z; `lighting flat` Lambert-shades the face.
                    for r in 0..rows.saturating_sub(1) {
                        for c in 0..cols.saturating_sub(1) {
                            let vs = [vtx(r, c), vtx(r + 1, c), vtx(r + 1, c + 1), vtx(r, c + 1)];
                            if !vs.iter().all(Option::is_some) {
                                continue;
                            }
                            let corners: [(f64, f64, f64); 4] = vs.map(|v| v.unwrap_or((0.0, 0.0, 0.0)));
                            let mz = corners.iter().map(|v| v.2).sum::<f64>() / 4.0;
                            let mut color = fig.cmap.map((mz - cl) / cspan);
                            if fig.lighting {
                                let (a, bq, cq) = (corners[0], corners[1], corners[2]);
                                let u = (bq.0 - a.0, bq.1 - a.1, bq.2 - a.2);
                                let v = (cq.0 - a.0, cq.1 - a.1, cq.2 - a.2);
                                let n = (u.1 * v.2 - u.2 * v.1, u.2 * v.0 - u.0 * v.2, u.0 * v.1 - u.1 * v.0);
                                let len = (n.0 * n.0 + n.1 * n.1 + n.2 * n.2).sqrt().max(1e-12);
                                let light = (0.4f64, 0.4f64, 0.8f64);
                                let ll = (light.0 * light.0 + light.1 * light.1 + light.2 * light.2).sqrt();
                                let d = ((n.0 * light.0 + n.1 * light.1 + n.2 * light.2) / (len * ll)).abs();
                                color = color.scale(0.45 + 0.55 * d);
                            }
                            for tri in [[0, 1, 2], [0, 2, 3]] {
                                let ps = tri.map(|k| n3(corners[k]));
                                tris.push(Tri {
                                    v: ps.map(|p| vert(p.0, p.1, p.2, color)),
                                    c: [
                                        (ps[0].0 + ps[1].0 + ps[2].0) / 3.0,
                                        (ps[0].1 + ps[1].1 + ps[2].1) / 3.0,
                                        (ps[0].2 + ps[1].2 + ps[2].2) / 3.0,
                                    ],
                                });
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Axes box wireframe (12 edges, same corner indexing as `to_svg`).
    let boxc = matplot::box_corners(x0, x1, y0, y1, z0, z1);
    let gray = matplot::Rgb(0xcc, 0xcc, 0xcc);
    for a in 0..8 {
        for &bix in &[a ^ 4, a ^ 2, a ^ 1] {
            if a < bix {
                seg(boxc[a], boxc[bix], gray);
            }
        }
    }
    Geom { id: GEOM_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed), lines, tris, bounds: [x0, x1, y0, y1, z0, z1] }
}

/// The `to_svg` orthonormal camera basis for `(az, el)` in degrees —
/// `r`/`s` are screen right/up, `c` the view direction.
pub fn view_basis(az: f64, el: f64) -> ([f64; 3], [f64; 3], [f64; 3]) {
    let (az, el) = (az.to_radians(), el.to_radians());
    let c = [el.cos() * az.sin(), -el.cos() * az.cos(), el.sin()];
    let hl = (c[0] * c[0] + c[1] * c[1]).sqrt();
    let (rx, ry) = if hl > 1e-9 { (-c[1] / hl, c[0] / hl) } else { (1.0, 0.0) };
    let r = [rx, ry, 0.0];
    let s = [-c[2] * ry, c[2] * rx, c[0] * ry - c[1] * rx];
    (r, s, c)
}

/// Project a normalized point through the camera basis → `(u, v, depth)`.
pub fn project(az: f64, el: f64, n: (f64, f64, f64)) -> (f64, f64, f64) {
    let (r, s, c) = view_basis(az, el);
    (r[0] * n.0 + r[1] * n.1 + r[2] * n.2, s[0] * n.0 + s[1] * n.1 + s[2] * n.2, c[0] * n.0 + c[1] * n.1 + c[2] * n.2)
}

/// NDC `(x, y)` of a normalized point under the same transform the
/// callback's uniform applies — used to overlay tick labels in egui.
pub fn to_ndc(az: f64, el: f64, v: View3d, aspect: f32, n: (f64, f64, f64)) -> (f32, f32) {
    let m = view_matrix(az, el, v, aspect);
    let x = m[0][0] * n.0 as f32 + m[1][0] * n.1 as f32 + m[2][0] * n.2 as f32 + m[3][0];
    let y = m[0][1] * n.0 as f32 + m[1][1] * n.1 as f32 + m[2][1] * n.2 as f32 + m[3][1];
    (x, y)
}

/// Clip-space matrix (column-major, wgpu layout): ortho projection + fit +
/// zoom/pan + depth remap into [0, 1].
fn view_matrix(az: f64, el: f64, v: View3d, aspect: f32) -> [[f32; 4]; 4] {
    let (mut u0, mut u1, mut v0, mut v1, mut d0, mut d1) =
        (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
    for i in 0..8 {
        let n = (((i >> 2) & 1) as f64 * 2.0 - 1.0, ((i >> 1) & 1) as f64 * 2.0 - 1.0, (i & 1) as f64 * 2.0 - 1.0);
        let p = project(az, el, n);
        u0 = u0.min(p.0);
        u1 = u1.max(p.0);
        v0 = v0.min(p.1);
        v1 = v1.max(p.1);
        d0 = d0.min(p.2);
        d1 = d1.max(p.2);
    }
    let us = (u1 - u0).max(f64::EPSILON);
    let vs = (v1 - v0).max(f64::EPSILON);
    let ds = (d1 - d0).max(f64::EPSILON);
    let dm = (d0 + d1) / 2.0;
    let aspect = f64::from(aspect).max(1e-3);
    let z = f64::from(v.zoom).max(0.02);
    let kx = 0.9 * z * (2.0 / us).min(2.0 / (vs * aspect));
    let ky = 0.9 * z * (2.0 * aspect / us).min(2.0 / vs);
    let kd = 0.9 / ds;
    let rows = [
        [view_basis(az, el).0[0] * kx, view_basis(az, el).0[1] * kx, 0.0, f64::from(v.pan[0])],
        {
            let s = view_basis(az, el).1;
            [-s[0] * ky, -s[1] * ky, -s[2] * ky, f64::from(v.pan[1])]
        },
        {
            let c = view_basis(az, el).2;
            [c[0] * kd, c[1] * kd, c[2] * kd, 0.5 - dm * kd]
        },
        [0.0, 0.0, 0.0, 1.0],
    ];
    let mut m = [[0.0f32; 4]; 4];
    for (row, rr) in rows.iter().enumerate() {
        for (col, x) in rr.iter().enumerate() {
            m[col][row] = *x as f32;
        }
    }
    m
}

/// Shared GPU state — pipelines, uniform buffer, bind group, and cached
/// vertex buffers keyed by `Geom::id` — stored once in
/// `RenderState.callback_resources` by the app.
struct GpuPipes {
    line_pipe: wgpu::RenderPipeline,
    tri_pipe: wgpu::RenderPipeline,
    bind: wgpu::BindGroup,
    ubuf: wgpu::Buffer,
    verts: std::collections::HashMap<usize, VBufs>,
}

struct VBufs {
    line: wgpu::Buffer,
    tri: wgpu::Buffer,
    nline: u32,
    ntri: u32,
}

/// Create the shared pipeline state — called once per `RenderState` by
/// [`insert_pipes`].
fn new_pipes(device: &wgpu::Device, format: wgpu::TextureFormat) -> GpuPipes {
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("gpufig"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        }],
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("gpufig"),
        source: wgpu::ShaderSource::Wgsl(
            r#"
struct U { m: mat4x4<f32> };
@group(0) @binding(0) var<uniform> u: U;
struct In { @location(0) p: vec3<f32>, @location(1) c: vec4<f32> };
struct Out { @builtin(position) p: vec4<f32>, @location(0) c: vec4<f32> };
@vertex fn vs(i: In) -> Out { var o: Out; o.p = u.m * vec4<f32>(i.p, 1.0); o.c = i.c; return o; }
@fragment fn fs(i: Out) -> @location(0) vec4<f32> { return i.c; }
"#
            .into(),
        ),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("gpufig"),
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });
    let attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];
    let mk = |tri: bool| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gpufig"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vert>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attrs,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::REPLACE), write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: if tri { wgpu::PrimitiveTopology::TriangleList } else { wgpu::PrimitiveTopology::LineList },
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    };
    let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gpufig"),
        size: 64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("gpufig"),
        layout: &bgl,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() }],
    });
    GpuPipes { line_pipe: mk(false), tri_pipe: mk(true), bind, ubuf, verts: std::collections::HashMap::new() }
}

/// Idempotently insert the shared pipeline state into a `RenderState`'s
/// callback resources. Call once per frame before adding paint callbacks —
/// cheap after the first call.
pub fn insert_pipes(render_state: &egui_wgpu::RenderState) {
    let mut renderer = render_state.renderer.write();
    if renderer.callback_resources.get::<GpuPipes>().is_none() {
        renderer.callback_resources.insert(new_pipes(&render_state.device, render_state.target_format));
    }
}

/// One paint callback — carries its geometry plus the resolved camera.
pub struct GpuFig {
    pub geom: Arc<Geom>,
    pub az: f64,
    pub el: f64,
    pub view: View3d,
    pub aspect: f32,
}

impl egui_wgpu::CallbackTrait for GpuFig {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        _enc: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(pipes) = resources.get_mut::<GpuPipes>() else { return Vec::new() };
        if !pipes.verts.contains_key(&self.geom.id) {
            let cap = |vs: usize| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("gpufig verts"),
                    size: (vs.max(1) * std::mem::size_of::<Vert>()) as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            };
            if pipes.verts.len() >= 16 {
                pipes.verts.clear();
            }
            pipes.verts.insert(
                self.geom.id,
                VBufs {
                    line: cap(self.geom.lines.len()),
                    tri: cap(self.geom.tris.len() * 3),
                    nline: self.geom.lines.len() as u32,
                    ntri: (self.geom.tris.len() * 3) as u32,
                },
            );
        }
        let Some(vb) = pipes.verts.get(&self.geom.id) else { return Vec::new() };
        if !self.geom.lines.is_empty() {
            queue.write_buffer(&vb.line, 0, bytemuck::cast_slice(&self.geom.lines));
        }
        if !self.geom.tris.is_empty() {
            // Painter sort far→near by projected centroid depth (matches
            // `to_svg`): the order is view-dependent, so it happens per frame.
            let (_, _, cdir) = view_basis(self.az, self.el);
            let mut order: Vec<(f64, usize)> =
                self.geom.tris.iter().enumerate().map(|(i, t)| (cdir[0] * t.c[0] + cdir[1] * t.c[1] + cdir[2] * t.c[2], i)).collect();
            order.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            let mut flat: Vec<Vert> = Vec::with_capacity(self.geom.tris.len() * 3);
            for (_, i) in order {
                if let Some(t) = self.geom.tris.get(i) {
                    flat.extend_from_slice(&t.v);
                }
            }
            queue.write_buffer(&vb.tri, 0, bytemuck::cast_slice(&flat));
        }
        let m = view_matrix(self.az, self.el, self.view, self.aspect);
        let mut bytes = [0u8; 64];
        for (i, col) in m.iter().enumerate() {
            for (j, v) in col.iter().enumerate() {
                let off = i * 16 + j * 4;
                bytes[off..off + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
        queue.write_buffer(&pipes.ubuf, 0, &bytes);
        Vec::new()
    }

    fn paint(&self, _info: egui::PaintCallbackInfo, render_pass: &mut wgpu::RenderPass<'static>, resources: &egui_wgpu::CallbackResources) {
        let Some(pipes) = resources.get::<GpuPipes>() else { return };
        let Some(vb) = pipes.verts.get(&self.geom.id) else { return };
        if vb.ntri > 0 {
            render_pass.set_pipeline(&pipes.tri_pipe);
            render_pass.set_bind_group(0, &pipes.bind, &[]);
            render_pass.set_vertex_buffer(0, vb.tri.slice(..));
            render_pass.draw(0..vb.ntri, 0..1);
        }
        if vb.nline > 0 {
            render_pass.set_pipeline(&pipes.line_pipe);
            render_pass.set_bind_group(0, &pipes.bind, &[]);
            render_pass.set_vertex_buffer(0, vb.line.slice(..));
            render_pass.draw(0..vb.nline, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geom_collects_3d_series_and_box() {
        let mut f = Figure::new();
        // One 2×2 surf quad → 2 triangles; one plot3 segment → 2 line
        // verts; the 12-edge box adds 24 more.
        f.push_grid(
            SeriesKind::Surf,
            vec![1.0, 2.0, 1.0, 2.0],
            vec![1.0, 1.0, 2.0, 2.0],
            vec![0.0, 1.0, 1.0, 0.0],
            2,
            2,
            Vec::new(),
            matplot::LineSpec::new(),
        );
        f.push_xyz(vec![0.0, 1.0], vec![0.0, 1.0], vec![0.0, 1.0], matplot::LineSpec::new());
        assert!(matplot::is_3d_view(&f));
        let g = build_geom(&f);
        assert_eq!(g.tris.len(), 2);
        assert_eq!(g.lines.len(), 24 + 2);
    }

    #[test]
    fn view_matrix_and_ndc_stay_finite() {
        let m = view_matrix(-37.5, 30.0, View3d::default(), 1.0);
        assert!(m.iter().flatten().all(|v| v.is_finite()));
        let (x, y) = to_ndc(-37.5, 30.0, View3d::default(), 1.0, (0.0, 0.0, 0.0));
        assert!(x.abs() <= 1.0 && y.abs() <= 1.0);
        // Ortho basis at view(0,0): camera looks along -y, screen-right
        // is +x, so +x projects to +u.
        let (px, _, _) = project(0.0, 0.0, (1.0, 0.0, 0.0));
        assert!(px > 0.0);
    }
}
