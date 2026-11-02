//! Optional GPU acceleration (`--features gpu`).
//!
//! The default build is CPU-only. With the `gpu` feature the `gpuArray`
//! wrapper runs matrix multiplication through a wgpu compute shader and
//! `gather` copies the result back. All other operations fall back to CPU.

use super::*;

#[cfg(all(feature = "gpu", not(target_arch = "wasm32")))]
mod backend {
    use super::*;
    use wgpu::util::DeviceExt;

    pub fn available() -> bool {
        let instance = wgpu::Instance::default();
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).is_ok()
    }

    pub fn device_name() -> String {
        let instance = wgpu::Instance::default();
        match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) {
            Ok(adapter) => adapter.get_info().name,
            Err(_) => "no GPU adapter".into(),
        }
    }

    pub fn matmul(a: &Matrix, b: &Matrix) -> Result<Matrix, LangError> {
        let (m, k, n) = (a.rows(), a.cols(), b.cols());
        if a.cols() != b.rows() {
            return Err(LangError::new("gpuArray mtimes: inner dimensions must agree", 0));
        }
        if m.saturating_mul(k).saturating_mul(n) > 50_000_000 {
            return Err(LangError::new("gpuArray: matrix is too large for the GPU path", 0));
        }
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .map_err(|e| LangError::new(format!("gpuArray: no adapter ({e})"), 0))?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { label: Some("matcraft-gpu"), ..Default::default() }))
                .map_err(|e| LangError::new(format!("gpuArray: device request failed ({e})"), 0))?;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("matcraft-matmul"),
            source: wgpu::ShaderSource::Wgsl(
                r#"
struct Dims { m: u32, k: u32, n: u32, pad: u32 };
@group(0) @binding(0) var<storage, read> a: array<f32>;
@group(0) @binding(1) var<storage, read> b: array<f32>;
@group(0) @binding(2) var<storage, read_write> c: array<f32>;
@group(0) @binding(3) var<uniform> dims: Dims;
@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let r = gid.x;
    let col = gid.y;
    if (r >= dims.m || col >= dims.n) { return; }
    var s = 0.0;
    for (var i = 0u; i < dims.k; i = i + 1u) {
        s = s + a[i * dims.m + r] * b[col * dims.k + i];
    }
    c[col * dims.m + r] = s;
}
"#
                .into(),
            ),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("matcraft-matmul"),
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let a32: Vec<f32> = a.data().iter().map(|&x| x as f32).collect();
        let b32: Vec<f32> = b.data().iter().map(|&x| x as f32).collect();
        let a_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("a"),
            contents: bytemuck::cast_slice(&a32),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let b_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("b"),
            contents: bytemuck::cast_slice(&b32),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let out_size = (m * n * 4) as u64;
        let c_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c"),
            size: out_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let dims = [m as u32, k as u32, n as u32, 0];
        let dim_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("dims"),
            contents: bytemuck::cast_slice(&dims),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("matcraft-matmul"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: a_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: b_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: c_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: dim_buf.as_entire_binding() },
            ],
        });
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: out_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("matcraft") });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("matcraft"), timestamp_writes: None });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups((m as u32).div_ceil(8), (n as u32).div_ceil(8), 1);
        }
        encoder.copy_buffer_to_buffer(&c_buf, 0, &staging, 0, out_size);
        queue.submit(Some(encoder.finish()));
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| LangError::new(format!("gpuArray: poll failed ({e})"), 0))?;
        let _ = rx.recv();
        let data = slice.get_mapped_range().map_err(|e| LangError::new(format!("gpuArray: map failed ({e})"), 0))?;
        let values: Vec<f64> = bytemuck::cast_slice::<u8, f32>(&data).iter().map(|&x| x as f64).collect();
        drop(data);
        staging.unmap();
        Matrix::from_col_major(m, n, values).map_err(lang)
    }
}

#[cfg(not(all(feature = "gpu", not(target_arch = "wasm32"))))]
mod backend {
    use super::*;

    pub fn available() -> bool {
        false
    }

    pub fn device_name() -> String {
        "CPU fallback (build with --features gpu for wgpu)".into()
    }

    pub fn matmul(a: &Matrix, b: &Matrix) -> Result<Matrix, LangError> {
        a.matmul(b).map_err(lang)
    }
}

pub(super) fn available() -> bool {
    backend::available()
}

pub(super) fn device_name() -> String {
    backend::device_name()
}

pub(super) fn matmul(a: &Matrix, b: &Matrix) -> Result<Matrix, LangError> {
    backend::matmul(a, b)
}
