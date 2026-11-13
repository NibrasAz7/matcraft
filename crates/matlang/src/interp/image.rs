//! Image-processing builtins: colour conversion, resizing, filtering,
//! edge detection, connected components and region measurements.
//!
//! Images are ordinary `Matrix` values: M×N grayscale, M×N×3 RGB, or a
//! 1-channel M×N×1 page. Kernels are 2-D matrices.

use super::*;

#[derive(Clone, Copy)]
enum Boundary {
    Zero,
    Replicate,
    Symmetric,
    Circular,
}

fn image_shape(m: &Matrix) -> Result<(usize, usize, usize), LangError> {
    let dims = m.dims();
    if dims.len() == 2 {
        return Ok((dims[0], dims[1], 1));
    }
    if dims.len() == 3 && dims[2] == 3 {
        return Ok((dims[0], dims[1], 3));
    }
    if dims.len() == 3 && dims[2] == 1 {
        return Ok((dims[0], dims[1], 1));
    }
    Err(LangError::new("image must be MxN grayscale or MxNx3 RGB", 0))
}

fn image_get(m: &Matrix, r: usize, c: usize, ch: usize, channels: usize) -> f64 {
    if channels == 1 {
        m.get(r, c).unwrap_or(0.0)
    } else {
        m.get_nd(&[r, c, ch]).unwrap_or(0.0)
    }
}

fn build_image(rows: usize, cols: usize, channels: usize, data: Vec<f64>) -> Result<Matrix, LangError> {
    if channels == 1 {
        Matrix::from_col_major(rows, cols, data).map_err(lang)
    } else {
        Matrix::from_col_major_nd(&[rows, cols, channels], data).map_err(lang)
    }
}

fn gray_matrix(m: &Matrix) -> Result<Matrix, LangError> {
    let (rows, cols, channels) = image_shape(m)?;
    if channels == 3 {
        return Err(LangError::new("a grayscale image is required", 0));
    }
    if m.dims().len() == 2 {
        Ok(m.clone())
    } else {
        Matrix::from_col_major(rows, cols, m.data().to_vec()).map_err(lang)
    }
}

fn rgb2gray_matrix(m: &Matrix) -> Result<Matrix, LangError> {
    let (rows, cols, channels) = image_shape(m)?;
    if channels == 1 {
        return gray_matrix(m);
    }
    let mut out = Matrix::zeros(rows, cols);
    for r in 0..rows {
        for c in 0..cols {
            let v = 0.298_936 * image_get(m, r, c, 0, 3) + 0.587_043 * image_get(m, r, c, 1, 3) + 0.114_021 * image_get(m, r, c, 2, 3);
            let _ = out.set(r, c, v);
        }
    }
    Ok(out)
}

#[derive(Clone, Copy, PartialEq)]
enum Interp {
    Nearest,
    Bilinear,
    Bicubic,
    Lanczos,
}

/// Keys cubic convolution, `a = -0.5` (MATLAB's `imresize` bicubic kernel).
fn cubic_weight(x: f64) -> f64 {
    let x = x.abs();
    if x <= 1.0 {
        1.5 * x * x * x - 2.5 * x * x + 1.0
    } else if x < 2.0 {
        -0.5 * x * x * x + 2.5 * x * x - 4.0 * x + 2.0
    } else {
        0.0
    }
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-12 {
        1.0
    } else {
        (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x)
    }
}

fn lanczos_weight(x: f64) -> f64 {
    let x = x.abs();
    if x < 3.0 {
        sinc(x) * sinc(x / 3.0)
    } else {
        0.0
    }
}

fn resize_channel(src: &Matrix, rows: usize, cols: usize, out_rows: usize, out_cols: usize, method: Interp) -> Vec<f64> {
    let mut out = Vec::with_capacity(out_rows.saturating_mul(out_cols));
    let max_r = rows.saturating_sub(1) as f64;
    let max_c = cols.saturating_sub(1) as f64;
    // Column-major output: build_image consumes flat data in that order.
    for c in 0..out_cols {
        let sx = (c as f64 + 0.5) * cols as f64 / out_cols as f64 - 0.5;
        for r in 0..out_rows {
            let sy = (r as f64 + 0.5) * rows as f64 / out_rows as f64 - 0.5;
            match method {
                Interp::Nearest => {
                    let rr = sy.round().clamp(0.0, max_r) as usize;
                    let cc = sx.round().clamp(0.0, max_c) as usize;
                    out.push(src.get(rr, cc).unwrap_or(0.0));
                }
                Interp::Bilinear => {
                    let y0 = sy.floor().max(0.0);
                    let x0 = sx.floor().max(0.0);
                    let y1 = (y0 + 1.0).min(max_r);
                    let x1 = (x0 + 1.0).min(max_c);
                    let fy = sy - y0;
                    let fx = sx - x0;
                    let g = |yy: f64, xx: f64| src.get(yy as usize, xx as usize).unwrap_or(0.0);
                    let top = g(y0, x0) * (1.0 - fx) + g(y0, x1) * fx;
                    let bot = g(y1, x0) * (1.0 - fx) + g(y1, x1) * fx;
                    out.push(top * (1.0 - fy) + bot * fy);
                }
                Interp::Bicubic | Interp::Lanczos => {
                    // Downscaling widens the kernel (anti-aliasing); upscaling
                    // keeps the nominal support.
                    let sc = (out_cols as f64 / cols as f64).min(out_rows as f64 / rows as f64).clamp(1e-6, 1.0);
                    let (radius, weight): (isize, &dyn Fn(f64) -> f64) = if method == Interp::Lanczos {
                        ((3.0 / sc).ceil() as isize, &lanczos_weight)
                    } else {
                        ((2.0 / sc).ceil() as isize, &cubic_weight)
                    };
                    let y0 = sy.floor();
                    let x0 = sx.floor();
                    let mut acc = 0.0f64;
                    let mut wsum = 0.0f64;
                    for j in -radius + 1..=radius {
                        let yy = y0 + j as f64;
                        let wy = weight((sy - yy) * sc);
                        let yi = yy.clamp(0.0, max_r) as usize;
                        for i in -radius + 1..=radius {
                            let xx = x0 + i as f64;
                            let wx = weight((sx - xx) * sc);
                            let xi = xx.clamp(0.0, max_c) as usize;
                            let w = wx * wy;
                            acc += src.get(yi, xi).unwrap_or(0.0) * w;
                            wsum += w;
                        }
                    }
                    out.push(if wsum.abs() > 1e-12 { acc / wsum } else { 0.0 });
                }
            }
        }
    }
    out
}

fn boundary_index(i: isize, n: usize, mode: Boundary) -> Option<usize> {
    if n == 0 {
        return None;
    }
    let n = n as isize;
    match mode {
        Boundary::Zero => {
            if i < 0 || i >= n {
                None
            } else {
                Some(i as usize)
            }
        }
        Boundary::Replicate => Some(i.clamp(0, n - 1) as usize),
        Boundary::Symmetric => {
            let mut x = i;
            while x < 0 || x >= n {
                if x < 0 {
                    x = -x - 1;
                } else if x >= n {
                    x = 2 * n - x - 1;
                }
            }
            Some(x as usize)
        }
        Boundary::Circular => Some(i.rem_euclid(n) as usize),
    }
}

fn filter_channel(src: &Matrix, h: &Matrix, boundary: Boundary, full: bool, conv: bool) -> Result<Matrix, LangError> {
    let rows = src.rows();
    let cols = src.cols();
    let kh = h.rows();
    let kw = h.cols();
    let (orows, ocols) = if full { (rows.saturating_add(kh).saturating_sub(1), cols.saturating_add(kw).saturating_sub(1)) } else { (rows, cols) };
    let ar = kh.saturating_sub(1) / 2;
    let ac = kw.saturating_sub(1) / 2;
    let mut out = Matrix::zeros(orows, ocols);
    for r in 0..orows {
        for c in 0..ocols {
            let mut acc = 0.0;
            for kr in 0..kh {
                for kc in 0..kw {
                    let (hr, hc) = if conv { (kh.saturating_sub(1).saturating_sub(kr), kw.saturating_sub(1).saturating_sub(kc)) } else { (kr, kc) };
                    let sr = if full { r as isize - hr as isize } else { r as isize - hr as isize + ar as isize };
                    let sc = if full { c as isize - hc as isize } else { c as isize - hc as isize + ac as isize };
                    let (Some(sr), Some(sc)) = (boundary_index(sr, rows, boundary), boundary_index(sc, cols, boundary)) else { continue };
                    acc += src.get(sr, sc).unwrap_or(0.0) * h.get(kr, kc).unwrap_or(0.0);
                }
            }
            let _ = out.set(r, c, acc);
        }
    }
    Ok(out)
}

fn resize_image(m: &Matrix, out_rows: usize, out_cols: usize, method: Interp) -> Result<Matrix, LangError> {
    let (rows, cols, channels) = image_shape(m)?;
    let mut data = Vec::with_capacity(out_rows.saturating_mul(out_cols).saturating_mul(channels));
    for ch in 0..channels {
        let plane = if channels == 1 {
            let mut p = Matrix::zeros(rows, cols);
            for r in 0..rows {
                for c in 0..cols {
                    let _ = p.set(r, c, m.get(r, c).unwrap_or(0.0));
                }
            }
            p
        } else {
            let mut p = Matrix::zeros(rows, cols);
            for r in 0..rows {
                for c in 0..cols {
                    let _ = p.set(r, c, image_get(m, r, c, ch, channels));
                }
            }
            p
        };
        data.extend(resize_channel(&plane, rows, cols, out_rows, out_cols, method));
    }
    build_image(out_rows, out_cols, channels, data)
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(|a, b| a.total_cmp(b));
    let n = values.len();
    if n == 0 {
        return f64::NAN;
    }
    if n % 2 == 1 {
        values.get(n / 2).copied().unwrap_or(f64::NAN)
    } else {
        let a = values.get(n / 2 - 1).copied().unwrap_or(0.0);
        let b = values.get(n / 2).copied().unwrap_or(0.0);
        (a + b) / 2.0
    }
}

fn binary_matrix(m: &Matrix) -> Result<Matrix, LangError> {
    let g = gray_matrix(m)?;
    let mut out = Matrix::zeros(g.rows(), g.cols());
    for r in 0..g.rows() {
        for c in 0..g.cols() {
            let _ = out.set(r, c, if g.get(r, c).unwrap_or(0.0) != 0.0 { 1.0 } else { 0.0 });
        }
    }
    Ok(out)
}

fn otsu_threshold(m: &Matrix) -> Result<f64, LangError> {
    let g = rgb2gray_matrix(m)?;
    let maxv = g.data().iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let scale = if maxv > 1.0 { 255.0 } else { 1.0 };
    let mut hist = [0usize; 256];
    for &v in g.data() {
        let bin = ((v / scale).clamp(0.0, 1.0) * 255.0).round() as usize;
        if let Some(slot) = hist.get_mut(bin.min(255)) {
            *slot += 1;
        }
    }
    let total: usize = hist.iter().sum();
    if total == 0 {
        return Ok(0.0);
    }
    let sum_all: f64 = hist.iter().enumerate().map(|(i, &h)| i as f64 * h as f64).sum();
    let mut w_b = 0.0f64;
    let mut sum_b = 0.0f64;
    let mut best = -1.0f64;
    let mut threshold = 0.0f64;
    for (i, &h) in hist.iter().enumerate() {
        w_b += h as f64;
        if w_b == 0.0 {
            continue;
        }
        let w_f = total as f64 - w_b;
        if w_f == 0.0 {
            break;
        }
        sum_b += i as f64 * h as f64;
        let m_b = sum_b / w_b;
        let m_f = (sum_all - sum_b) / w_f;
        let var = w_b * w_f * (m_b - m_f) * (m_b - m_f);
        if var > best {
            best = var;
            threshold = (i as f64 + 1.0) / 256.0;
        }
    }
    Ok(threshold)
}

impl Session {
    pub(super) fn image_builtin(&mut self, name: &str, argv: Vec<Value>, _nout: usize) -> Result<Vec<Value>, LangError> {
        match name {
            "rgb2gray" => {
                let m = argv.first().ok_or_else(|| LangError::new("rgb2gray needs an image", 0))?.to_numeric()?;
                Ok(vec![mat_or_num(rgb2gray_matrix(&m)?)])
            }
            "imresize" => {
                let m = argv.first().ok_or_else(|| LangError::new("imresize needs an image", 0))?.to_numeric()?;
                let (rows, cols, _) = image_shape(&m)?;
                let spec = argv.get(1).ok_or_else(|| LangError::new("imresize needs a scale or output size", 0))?.to_numeric()?;
                let mut method = String::from("bicubic");
                for x in argv.iter().skip(2) {
                    if let Value::Str(s) = x {
                        method = s.to_ascii_lowercase();
                    }
                }
                let interp = match method.as_str() {
                    "nearest" => Interp::Nearest,
                    "bilinear" | "triangle" => Interp::Bilinear,
                    "bicubic" | "cubic" | "antialiased" => Interp::Bicubic,
                    "lanczos" | "lanczos2" | "lanczos3" => Interp::Lanczos,
                    other => return Err(LangError::new(format!("imresize: unsupported method '{other}'"), 0)),
                };
                let (out_rows, out_cols) = if spec.numel() == 1 {
                    let s = spec.data().first().copied().unwrap_or(1.0).max(0.0);
                    ((rows as f64 * s).round().max(1.0) as usize, (cols as f64 * s).round().max(1.0) as usize)
                } else {
                    let r = spec.data().first().copied().unwrap_or(rows as f64).max(1.0) as usize;
                    let c = spec.data().get(1).copied().unwrap_or(cols as f64).max(1.0) as usize;
                    (r, c)
                };
                check_elems(out_rows.saturating_mul(out_cols).saturating_mul(3))?;
                Ok(vec![mat_or_num(resize_image(&m, out_rows, out_cols, interp)?)])
            }
            "imfilter" => {
                let m = argv.first().ok_or_else(|| LangError::new("imfilter needs an image", 0))?.to_numeric()?;
                let h = argv.get(1).ok_or_else(|| LangError::new("imfilter needs a kernel", 0))?.to_numeric()?;
                if h.rows() == 0 || h.cols() == 0 {
                    return Err(LangError::new("imfilter kernel must not be empty", 0));
                }
                let mut boundary = Boundary::Zero;
                let mut full = false;
                let mut conv = false;
                for x in argv.iter().skip(2) {
                    if let Value::Str(s) = x {
                        match s.to_ascii_lowercase().as_str() {
                            "replicate" => boundary = Boundary::Replicate,
                            "symmetric" => boundary = Boundary::Symmetric,
                            "circular" => boundary = Boundary::Circular,
                            "full" => full = true,
                            "same" => full = false,
                            "conv" => conv = true,
                            "corr" => conv = false,
                            _ => {}
                        }
                    }
                }
                let (rows, cols, channels) = image_shape(&m)?;
                let (orows, ocols) = if full {
                    (rows.saturating_add(h.rows()).saturating_sub(1), cols.saturating_add(h.cols()).saturating_sub(1))
                } else {
                    (rows, cols)
                };
                let mut data = Vec::with_capacity(orows.saturating_mul(ocols).saturating_mul(channels));
                for ch in 0..channels {
                    let mut plane = Matrix::zeros(rows, cols);
                    for r in 0..rows {
                        for c in 0..cols {
                            let _ = plane.set(r, c, image_get(&m, r, c, ch, channels));
                        }
                    }
                    let filtered = filter_channel(&plane, &h, boundary, full, conv)?;
                    data.extend_from_slice(filtered.data());
                }
                let out = build_image(orows, ocols, channels, data)?;
                Ok(vec![mat_or_num(out)])
            }
            "medfilt2" => {
                let m = gray_matrix(&argv.first().ok_or_else(|| LangError::new("medfilt2 needs an image", 0))?.to_numeric()?)?;
                let (kh, kw) = match argv.get(1) {
                    Some(v) => {
                        let mm = v.to_numeric()?;
                        if mm.numel() == 1 {
                            let s = mm.data().first().copied().unwrap_or(3.0).max(1.0) as usize;
                            (s, s)
                        } else {
                            (mm.data().first().copied().unwrap_or(3.0).max(1.0) as usize, mm.data().get(1).copied().unwrap_or(3.0).max(1.0) as usize)
                        }
                    }
                    None => (3, 3),
                };
                let ar = kh / 2;
                let ac = kw / 2;
                let mut out = Matrix::zeros(m.rows(), m.cols());
                for r in 0..m.rows() {
                    for c in 0..m.cols() {
                        let mut vals = Vec::with_capacity(kh.saturating_mul(kw));
                        for kr in 0..kh {
                            for kc in 0..kw {
                                let sr = r as isize + kr as isize - ar as isize;
                                let sc = c as isize + kc as isize - ac as isize;
                                let v = if sr >= 0 && sc >= 0 && (sr as usize) < m.rows() && (sc as usize) < m.cols() {
                                    m.get(sr as usize, sc as usize).unwrap_or(0.0)
                                } else {
                                    0.0
                                };
                                vals.push(v);
                            }
                        }
                        let _ = out.set(r, c, median(&mut vals));
                    }
                }
                Ok(vec![mat_or_num(out)])
            }
            "edge" => {
                let m = rgb2gray_matrix(&argv.first().ok_or_else(|| LangError::new("edge needs an image", 0))?.to_numeric()?)?;
                let method = match argv.get(1) {
                    Some(Value::Str(s)) => s.to_ascii_lowercase(),
                    _ => "sobel".into(),
                };
                if method == "canny" {
                    let out = canny_edges(&m, argv.get(2), argv.get(3))?;
                    return Ok(vec![mat_or_num(out)]);
                }
                let (hx, hy) = match method.as_str() {
                    "prewitt" => (
                        Matrix::from_row_major(3, 3, &[-1.0, 0.0, 1.0, -1.0, 0.0, 1.0, -1.0, 0.0, 1.0]).map_err(lang)?,
                        Matrix::from_row_major(3, 3, &[-1.0, -1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0]).map_err(lang)?,
                    ),
                    "roberts" => (
                        Matrix::from_row_major(2, 2, &[1.0, 0.0, 0.0, -1.0]).map_err(lang)?,
                        Matrix::from_row_major(2, 2, &[0.0, 1.0, -1.0, 0.0]).map_err(lang)?,
                    ),
                    _ => (
                        Matrix::from_row_major(3, 3, &[-1.0, 0.0, 1.0, -2.0, 0.0, 2.0, -1.0, 0.0, 1.0]).map_err(lang)?,
                        Matrix::from_row_major(3, 3, &[-1.0, -2.0, -1.0, 0.0, 0.0, 0.0, 1.0, 2.0, 1.0]).map_err(lang)?,
                    ),
                };
                let gx = filter_channel(&m, &hx, Boundary::Replicate, false, false)?;
                let gy = filter_channel(&m, &hy, Boundary::Replicate, false, false)?;
                let mag = gx.elemwise(&gy, |a, b| (a * a + b * b).sqrt()).map_err(lang)?;
                let maxv = mag.data().iter().copied().fold(0.0f64, f64::max);
                let th = match argv.get(2) {
                    Some(v) => {
                        let t = v.to_numeric()?;
                        if t.numel() > 1 {
                            t.data().get(1).copied().unwrap_or(0.1 * maxv)
                        } else {
                            t.data().first().copied().unwrap_or(0.1 * maxv)
                        }
                    }
                    None => 0.1 * maxv,
                };
                let th = if th > 0.0 { th } else { f64::INFINITY };
                let mut out = Matrix::zeros(m.rows(), m.cols());
                for r in 0..m.rows() {
                    for c in 0..m.cols() {
                        let _ = out.set(r, c, if mag.get(r, c).unwrap_or(0.0) >= th { 1.0 } else { 0.0 });
                    }
                }
                Ok(vec![mat_or_num(out)])
            }
            "bwlabel" => {
                let bw = binary_matrix(&argv.first().ok_or_else(|| LangError::new("bwlabel needs a binary image", 0))?.to_numeric()?)?;
                let conn = argv.get(1).map(|v| v.scalar()).transpose()?.unwrap_or(8.0) as usize;
                if conn != 4 && conn != 8 {
                    return Err(LangError::new("bwlabel connectivity must be 4 or 8", 0));
                }
                let (labels, count) = label_components(&bw, conn)?;
                Ok(vec![mat_or_num(labels), Value::Num(count as f64)])
            }
            "imdilate" | "imerode" | "imopen" | "imclose" => {
                let m = argv.first().ok_or_else(|| LangError::new(format!("{name} needs an image"), 0))?.to_numeric()?;
                let se = structuring_element(argv.get(1))?;
                let out = match name {
                    "imdilate" => morph_image(&m, &se, true)?,
                    "imerode" => morph_image(&m, &se, false)?,
                    "imopen" => {
                        let eroded = morph_image(&m, &se, false)?;
                        morph_image(&eroded, &se, true)?
                    }
                    _ => {
                        let dilated = morph_image(&m, &se, true)?;
                        morph_image(&dilated, &se, false)?
                    }
                };
                Ok(vec![mat_or_num(out)])
            }
            "imrotate" => {
                let m = argv.first().ok_or_else(|| LangError::new("imrotate needs an image", 0))?.to_numeric()?;
                let angle = argv.get(1).ok_or_else(|| LangError::new("imrotate needs an angle", 0))?.scalar()?;
                let mut bilinear = true;
                let mut crop = false;
                for v in argv.iter().skip(2) {
                    if let Value::Str(s) = v {
                        match s.to_ascii_lowercase().as_str() {
                            "nearest" => bilinear = false,
                            "bilinear" | "bicubic" => bilinear = true,
                            "crop" => crop = true,
                            "loose" => crop = false,
                            _ => {}
                        }
                    }
                }
                Ok(vec![mat_or_num(rotate_image(&m, angle, bilinear, crop)?)])
            }
            "imcrop" => {
                let m = argv.first().ok_or_else(|| LangError::new("imcrop needs an image", 0))?.to_numeric()?;
                let rect = match argv.get(1) {
                    Some(v) => Some(v.to_numeric()?),
                    None => None,
                };
                Ok(vec![mat_or_num(crop_image(&m, rect.as_ref())?)])
            }
            "strel" => Ok(vec![mat_or_num(strel_matrix(&argv)?)]),
            "regionprops" => self.regionprops_call(&argv),
            "graythresh" => {
                let m = argv.first().ok_or_else(|| LangError::new("graythresh needs an image", 0))?.to_numeric()?;
                Ok(vec![Value::Num(otsu_threshold(&m)?)])
            }
            "imbinarize" => {
                let m = argv.first().ok_or_else(|| LangError::new("imbinarize needs an image", 0))?.to_numeric()?;
                let t = match argv.get(1) {
                    Some(Value::Num(v)) => *v,
                    Some(Value::Mat(mm)) if mm.numel() == 1 => mm.data().first().copied().unwrap_or(0.0),
                    _ => otsu_threshold(&m)?,
                };
                let g = rgb2gray_matrix(&m)?;
                let mut out = Matrix::zeros(g.rows(), g.cols());
                for r in 0..g.rows() {
                    for c in 0..g.cols() {
                        let _ = out.set(r, c, if g.get(r, c).unwrap_or(0.0) > t { 1.0 } else { 0.0 });
                    }
                }
                Ok(vec![mat_or_num(out)])
            }
            _ => Err(LangError::new(format!("{name}: not an image builtin"), 0)),
        }
    }

    fn regionprops_call(&mut self, argv: &[Value]) -> Result<Vec<Value>, LangError> {
        let labels = argv.first().ok_or_else(|| LangError::new("regionprops needs a label image", 0))?.to_numeric()?;
        let mut props: Vec<String> = Vec::new();
        for arg in argv.iter().skip(1) {
            match arg {
                Value::Str(s) => props.push(s.to_ascii_lowercase()),
                Value::Cell(c) => {
                    for v in c.items.iter() {
                        if let Value::Str(s) = v {
                            props.push(s.to_ascii_lowercase());
                        }
                    }
                }
                _ => {}
            }
        }
        if props.is_empty() {
            props.push("area".into());
        }
        if props.iter().any(|p| p == "all") {
            props = vec![
                "area".into(),
                "centroid".into(),
                "boundingbox".into(),
                "pixelidxlist".into(),
                "pixellist".into(),
                "extent".into(),
                "equivdiameter".into(),
                "perimeter".into(),
                "majoraxislength".into(),
                "minoraxislength".into(),
                "orientation".into(),
                "convexarea".into(),
                "solidity".into(),
                "eulernumber".into(),
                "filledarea".into(),
            ];
        }
        let max_label = labels.data().iter().copied().filter(|v| v.is_finite() && *v > 0.0).fold(0usize, |m, v| m.max(v as usize));
        let mut out = StructArray::with_dims(1, max_label);
        for field in props.iter() {
            for label in 1..=max_label {
                let mut rr = Vec::new();
                let mut cc = Vec::new();
                for c in 0..labels.cols() {
                    for r in 0..labels.rows() {
                        if labels.get(r, c).unwrap_or(0.0).round() as usize == label {
                            rr.push(r);
                            cc.push(c);
                        }
                    }
                }
                let area = rr.len();
                let val = match field.as_str() {
                    "area" => Value::Num(area as f64),
                    "centroid" => {
                        if area == 0 {
                            Value::Mat(Matrix::row_vector(vec![f64::NAN, f64::NAN]))
                        } else {
                            let cx = cc.iter().sum::<usize>() as f64 / area as f64 + 1.0;
                            let cy = rr.iter().sum::<usize>() as f64 / area as f64 + 1.0;
                            Value::Mat(Matrix::row_vector(vec![cx, cy]))
                        }
                    }
                    "boundingbox" => {
                        if area == 0 {
                            Value::Mat(Matrix::row_vector(vec![f64::NAN; 4]))
                        } else {
                            let minr = *rr.iter().min().unwrap_or(&0);
                            let maxr = *rr.iter().max().unwrap_or(&0);
                            let minc = *cc.iter().min().unwrap_or(&0);
                            let maxc = *cc.iter().max().unwrap_or(&0);
                            Value::Mat(Matrix::row_vector(vec![
                                minc as f64 + 0.5,
                                minr as f64 + 0.5,
                                (maxc - minc + 1) as f64,
                                (maxr - minr + 1) as f64,
                            ]))
                        }
                    }
                    "pixelidxlist" => {
                        let vals: Vec<f64> = rr.iter().zip(cc.iter()).map(|(&r, &c)| (c * labels.rows() + r + 1) as f64).collect();
                        Value::Mat(Matrix::col_vector(vals))
                    }
                    "pixellist" => {
                        let mut m = Matrix::zeros(area, 2);
                        for (i, (&r, &c)) in rr.iter().zip(cc.iter()).enumerate() {
                            let _ = m.set(i, 0, c as f64 + 1.0);
                            let _ = m.set(i, 1, r as f64 + 1.0);
                        }
                        Value::Mat(m)
                    }
                    "extent" => {
                        if area == 0 {
                            Value::Num(f64::NAN)
                        } else {
                            let minr = *rr.iter().min().unwrap_or(&0);
                            let maxr = *rr.iter().max().unwrap_or(&0);
                            let minc = *cc.iter().min().unwrap_or(&0);
                            let maxc = *cc.iter().max().unwrap_or(&0);
                            Value::Num(area as f64 / ((maxr - minr + 1) * (maxc - minc + 1)) as f64)
                        }
                    }
                    "equivdiameter" => Value::Num((4.0 * area as f64 / std::f64::consts::PI).sqrt()),
                    "perimeter" => {
                        let mut per = 0.0;
                        for (&r, &c) in rr.iter().zip(cc.iter()) {
                            let boundary = [(-1isize, 0isize), (1, 0), (0, -1), (0, 1)].iter().any(|(dr, dc)| {
                                let nr = r as isize + dr;
                                let nc = c as isize + dc;
                                nr < 0
                                    || nc < 0
                                    || nr >= labels.rows() as isize
                                    || nc >= labels.cols() as isize
                                    || labels.get(nr as usize, nc as usize).unwrap_or(0.0).round() as usize != label
                            });
                            if boundary {
                                per += 1.0;
                            }
                        }
                        Value::Num(per)
                    }
                    "majoraxislength" | "minoraxislength" | "orientation" => {
                        if area == 0 {
                            Value::Num(f64::NAN)
                        } else {
                            let xs: Vec<f64> = cc.iter().map(|&c| c as f64 + 1.0).collect();
                            let ys: Vec<f64> = rr.iter().map(|&r| r as f64 + 1.0).collect();
                            let mx = xs.iter().sum::<f64>() / area as f64;
                            let my = ys.iter().sum::<f64>() / area as f64;
                            let (mut cxx, mut cyy, mut cxy) = (0.0, 0.0, 0.0);
                            for i in 0..area {
                                let dx = xs[i] - mx;
                                let dy = ys[i] - my;
                                cxx += dx * dx;
                                cyy += dy * dy;
                                cxy += dx * dy;
                            }
                            cxx /= area as f64;
                            cyy /= area as f64;
                            cxy /= area as f64;
                            let tr = cxx + cyy;
                            let det = cxx * cyy - cxy * cxy;
                            let disc = (tr * tr / 4.0 - det).max(0.0).sqrt();
                            let l1 = tr / 2.0 + disc;
                            let l2 = (tr / 2.0 - disc).max(0.0);
                            match field.as_str() {
                                "majoraxislength" => Value::Num(4.0 * l1.sqrt()),
                                "minoraxislength" => Value::Num(4.0 * l2.sqrt()),
                                _ => {
                                    let mut ang = 0.5 * (2.0 * cxy).atan2(cxx - cyy).to_degrees();
                                    if ang < 0.0 {
                                        ang += 180.0;
                                    }
                                    Value::Num(ang)
                                }
                            }
                        }
                    }
                    "convexarea" | "solidity" => {
                        if area == 0 {
                            Value::Num(f64::NAN)
                        } else {
                            let mut xs = Vec::with_capacity(area * 4);
                            let mut ys = Vec::with_capacity(area * 4);
                            for (&r, &c) in rr.iter().zip(cc.iter()) {
                                for dx in [-0.5, 0.5] {
                                    for dy in [-0.5, 0.5] {
                                        xs.push(c as f64 + 1.0 + dx);
                                        ys.push(r as f64 + 1.0 + dy);
                                    }
                                }
                            }
                            let hull = convex_hull_area(&xs, &ys);
                            if field == "convexarea" {
                                Value::Num(hull)
                            } else {
                                Value::Num(if hull > 0.0 { area as f64 / hull } else { f64::NAN })
                            }
                        }
                    }
                    "eulernumber" | "filledarea" => {
                        // Approximate: treat the region as hole-free; filled
                        // area equals the pixel count.
                        Value::Num(if field == "eulernumber" { 1.0 } else { area as f64 })
                    }
                    _ => Value::Mat(Matrix::zeros(0, 0)),
                };
                let field_name = match field.as_str() {
                    "area" => "Area",
                    "centroid" => "Centroid",
                    "boundingbox" => "BoundingBox",
                    "pixelidxlist" => "PixelIdxList",
                    "pixellist" => "PixelList",
                    "extent" => "Extent",
                    "equivdiameter" => "EquivDiameter",
                    "perimeter" => "Perimeter",
                    "majoraxislength" => "MajorAxisLength",
                    "minoraxislength" => "MinorAxisLength",
                    "orientation" => "Orientation",
                    "convexarea" => "ConvexArea",
                    "solidity" => "Solidity",
                    "eulernumber" => "EulerNumber",
                    "filledarea" => "FilledArea",
                    other => other,
                };
                out.set_field(label.saturating_sub(1), field_name, val)?;
            }
        }
        Ok(vec![Value::Struct(out)])
    }
}

fn convex_hull_area(xs: &[f64], ys: &[f64]) -> f64 {
    let mut pts: Vec<(f64, f64)> = xs.iter().copied().zip(ys.iter().copied()).collect();
    pts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    pts.dedup();
    if pts.len() < 3 {
        return 0.0;
    }
    let cross = |o: (f64, f64), a: (f64, f64), b: (f64, f64)| -> f64 { (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0) };
    let mut lower: Vec<(f64, f64)> = Vec::new();
    for &p in &pts {
        while lower.len() >= 2 {
            let n = lower.len();
            if cross(lower[n - 2], lower[n - 1], p) <= 0.0 {
                lower.pop();
            } else {
                break;
            }
        }
        lower.push(p);
    }
    let mut upper: Vec<(f64, f64)> = Vec::new();
    for &p in pts.iter().rev() {
        while upper.len() >= 2 {
            let n = upper.len();
            if cross(upper[n - 2], upper[n - 1], p) <= 0.0 {
                upper.pop();
            } else {
                break;
            }
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    if lower.len() < 3 {
        return 0.0;
    }
    let mut area = 0.0;
    for i in 0..lower.len() {
        let a = lower[i];
        let b = lower[(i + 1) % lower.len()];
        area += a.0 * b.1 - b.0 * a.1;
    }
    area.abs() / 2.0
}

fn structuring_element(v: Option<&Value>) -> Result<Matrix, LangError> {
    match v {
        None => Ok(Matrix::ones(3, 3)),
        Some(v) => {
            let m = v.to_numeric()?;
            if m.rows() == 0 || m.cols() == 0 {
                return Err(LangError::new("structuring element must not be empty", 0));
            }
            if !m.data().iter().any(|v| *v != 0.0) {
                return Err(LangError::new("structuring element needs at least one nonzero entry", 0));
            }
            Ok(m)
        }
    }
}

fn morph_image(m: &Matrix, se: &Matrix, dilate: bool) -> Result<Matrix, LangError> {
    let (rows, cols, channels) = image_shape(m)?;
    let kh = se.rows();
    let kw = se.cols();
    let ar = kh / 2;
    let ac = kw / 2;
    let mut offsets: Vec<(isize, isize)> = Vec::new();
    for kr in 0..kh {
        for kc in 0..kw {
            if se.get(kr, kc).unwrap_or(0.0) != 0.0 {
                offsets.push((kr as isize - ar as isize, kc as isize - ac as isize));
            }
        }
    }
    check_elems(rows.saturating_mul(cols).saturating_mul(channels))?;
    let mut data = Vec::with_capacity(rows.saturating_mul(cols).saturating_mul(channels));
    for ch in 0..channels {
        for c in 0..cols {
            for r in 0..rows {
                let mut acc = if dilate { f64::NEG_INFINITY } else { f64::INFINITY };
                for &(dr, dc) in &offsets {
                    let rr = r as isize + dr;
                    let cc = c as isize + dc;
                    if rr < 0 || cc < 0 || rr >= rows as isize || cc >= cols as isize {
                        continue;
                    }
                    let v = image_get(m, rr as usize, cc as usize, ch, channels);
                    acc = if dilate { acc.max(v) } else { acc.min(v) };
                }
                data.push(if acc.is_finite() { acc } else { 0.0 });
            }
        }
    }
    build_image(rows, cols, channels, data)
}

fn rotate_image(m: &Matrix, angle_deg: f64, bilinear: bool, crop: bool) -> Result<Matrix, LangError> {
    let (rows, cols, channels) = image_shape(m)?;
    if rows == 0 || cols == 0 {
        return Err(LangError::new("imrotate needs a nonempty image", 0));
    }
    let theta = angle_deg.to_radians();
    let (out_rows, out_cols) = if crop {
        (rows, cols)
    } else {
        let c = theta.cos().abs();
        let s = theta.sin().abs();
        let h = (rows as f64 * c + cols as f64 * s).round().max(1.0) as usize;
        let w = (cols as f64 * c + rows as f64 * s).round().max(1.0) as usize;
        (h, w)
    };
    check_elems(out_rows.saturating_mul(out_cols).saturating_mul(channels))?;
    let cos = theta.cos();
    let sin = theta.sin();
    let (cx, cy) = ((cols as f64 - 1.0) / 2.0, (rows as f64 - 1.0) / 2.0);
    let (ox, oy) = ((out_cols as f64 - 1.0) / 2.0, (out_rows as f64 - 1.0) / 2.0);
    let sample = |x: f64, y: f64, ch: usize| -> f64 {
        if x < -0.5 || y < -0.5 || x > cols as f64 - 0.5 || y > rows as f64 - 0.5 {
            return 0.0;
        }
        image_get(
            m,
            y.round().clamp(0.0, rows.saturating_sub(1) as f64) as usize,
            x.round().clamp(0.0, cols.saturating_sub(1) as f64) as usize,
            ch,
            channels,
        )
    };
    let sample_xy = |x: f64, y: f64, ch: usize| -> f64 {
        if x < -0.5 || y < -0.5 || x > cols as f64 - 0.5 || y > rows as f64 - 0.5 {
            return 0.0;
        }
        let x0 = x.floor().max(0.0);
        let y0 = y.floor().max(0.0);
        let x1 = (x0 + 1.0).min(cols.saturating_sub(1) as f64);
        let y1 = (y0 + 1.0).min(rows.saturating_sub(1) as f64);
        let fx = (x - x0).clamp(0.0, 1.0);
        let fy = (y - y0).clamp(0.0, 1.0);
        let g = |yy: f64, xx: f64| image_get(m, yy as usize, xx as usize, ch, channels);
        let top = g(y0, x0) * (1.0 - fx) + g(y0, x1) * fx;
        let bot = g(y1, x0) * (1.0 - fx) + g(y1, x1) * fx;
        top * (1.0 - fy) + bot * fy
    };
    let mut data = Vec::with_capacity(out_rows.saturating_mul(out_cols).saturating_mul(channels));
    for ch in 0..channels {
        for c in 0..out_cols {
            for r in 0..out_rows {
                let dx = c as f64 - ox;
                let dy = r as f64 - oy;
                let sx = dx * cos - dy * sin + cx;
                let sy = dx * sin + dy * cos + cy;
                data.push(if bilinear { sample_xy(sx, sy, ch) } else { sample(sx, sy, ch) });
            }
        }
    }
    build_image(out_rows, out_cols, channels, data)
}

fn crop_image(m: &Matrix, rect: Option<&Matrix>) -> Result<Matrix, LangError> {
    let (rows, cols, channels) = image_shape(m)?;
    let (x0, y0, w, h) = match rect {
        None => (1.0, 1.0, cols as f64, rows as f64),
        Some(r) if r.numel() >= 4 => (
            r.data().first().copied().unwrap_or(1.0),
            r.data().get(1).copied().unwrap_or(1.0),
            r.data().get(2).copied().unwrap_or(1.0),
            r.data().get(3).copied().unwrap_or(1.0),
        ),
        _ => return Err(LangError::new("imcrop rect must be [xmin ymin width height]", 0)),
    };
    if w <= 0.0 || h <= 0.0 {
        return Err(LangError::new("imcrop width and height must be positive", 0));
    }
    let x0 = x0.round().max(1.0) as usize;
    let y0 = y0.round().max(1.0) as usize;
    if x0 > cols || y0 > rows {
        return Err(LangError::new("imcrop rectangle starts outside the image", 0));
    }
    let out_cols = (w.round().max(1.0) as usize).min(cols.saturating_sub(x0).saturating_add(1));
    let out_rows = (h.round().max(1.0) as usize).min(rows.saturating_sub(y0).saturating_add(1));
    check_elems(out_rows.saturating_mul(out_cols).saturating_mul(channels))?;
    let mut data = Vec::with_capacity(out_rows.saturating_mul(out_cols).saturating_mul(channels));
    for ch in 0..channels {
        for c in 0..out_cols {
            for r in 0..out_rows {
                data.push(image_get(m, y0.saturating_sub(1).saturating_add(r), x0.saturating_sub(1).saturating_add(c), ch, channels));
            }
        }
    }
    build_image(out_rows, out_cols, channels, data)
}

fn strel_matrix(argv: &[Value]) -> Result<Matrix, LangError> {
    let shape = match argv.first() {
        Some(Value::Str(s)) => s.to_ascii_lowercase(),
        Some(_) => return Err(LangError::new("strel needs a shape name", 0)),
        None => "square".to_string(),
    };
    let params = argv.get(1).map(|v| v.to_numeric()).transpose()?;
    let p = |i: usize, d: f64| params.as_ref().and_then(|m| m.data().get(i).copied()).unwrap_or(d);
    match shape.as_str() {
        "square" => {
            let n = p(0, 3.0).round().max(1.0) as usize;
            check_elems(n.saturating_mul(n))?;
            Ok(Matrix::ones(n, n))
        }
        "rectangle" => {
            let r = p(0, 3.0).round().max(1.0) as usize;
            let c = p(1, 3.0).round().max(1.0) as usize;
            check_elems(r.saturating_mul(c))?;
            Ok(Matrix::ones(r, c))
        }
        "disk" => {
            let r = p(0, 3.0).round().max(0.0) as isize;
            let n = (2 * r + 1) as usize;
            check_elems(n.saturating_mul(n))?;
            let mut m = Matrix::zeros(n, n);
            for dy in -r..=r {
                for dx in -r..=r {
                    if (dx * dx + dy * dy) as f64 <= r as f64 * r as f64 + 0.5 {
                        let _ = m.set((dy + r) as usize, (dx + r) as usize, 1.0);
                    }
                }
            }
            Ok(m)
        }
        "diamond" => {
            let r = p(0, 3.0).round().max(0.0) as isize;
            let n = (2 * r + 1) as usize;
            check_elems(n.saturating_mul(n))?;
            let mut m = Matrix::zeros(n, n);
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs() + dy.abs() <= r {
                        let _ = m.set((dy + r) as usize, (dx + r) as usize, 1.0);
                    }
                }
            }
            Ok(m)
        }
        "line" => {
            let len = p(0, 3.0).round().max(1.0) as isize;
            let angle = p(1, 0.0).to_radians();
            let mut m = Matrix::zeros(len as usize, len as usize);
            let center = (len - 1) as f64 / 2.0;
            let half = (len - 1) / 2;
            for t in -half..=half {
                let x = (center + t as f64 * angle.cos()).round();
                let y = (center - t as f64 * angle.sin()).round();
                if x >= 0.0 && y >= 0.0 && x < len as f64 && y < len as f64 {
                    let _ = m.set(y as usize, x as usize, 1.0);
                }
            }
            let _ = m.set(center as usize, center as usize, 1.0);
            Ok(m)
        }
        "arbitrary" => {
            let m = params.ok_or_else(|| LangError::new("strel('arbitrary', ...) needs a matrix", 0))?;
            if m.rows() == 0 || m.cols() == 0 {
                return Err(LangError::new("strel neighborhood must not be empty", 0));
            }
            Ok(m.map(|v| if v != 0.0 { 1.0 } else { 0.0 }))
        }
        other => Err(LangError::new(format!("strel: unsupported shape '{other}'"), 0)),
    }
}

fn gaussian_kernel(sigma: f64) -> Vec<f64> {
    let radius = (3.0 * sigma).ceil().max(1.0) as isize;
    let mut kernel = Vec::with_capacity((2 * radius + 1) as usize);
    let mut sum = 0.0f64;
    for i in -radius..=radius {
        let v = (-(i as f64) * (i as f64) / (2.0 * sigma * sigma)).exp();
        kernel.push(v);
        sum += v;
    }
    if sum > 0.0 {
        for v in &mut kernel {
            *v /= sum;
        }
    }
    kernel
}

fn gaussian_blur(src: &Matrix, kernel: &[f64]) -> Matrix {
    let rows = src.rows();
    let cols = src.cols();
    let radius = (kernel.len().saturating_sub(1) / 2) as isize;
    let mut tmp = Matrix::zeros(rows, cols);
    for r in 0..rows {
        for c in 0..cols {
            let mut acc = 0.0f64;
            for (k, &w) in kernel.iter().enumerate() {
                let cc = (c as isize + k as isize - radius).clamp(0, cols.saturating_sub(1) as isize) as usize;
                acc += src.get(r, cc).unwrap_or(0.0) * w;
            }
            let _ = tmp.set(r, c, acc);
        }
    }
    let mut out = Matrix::zeros(rows, cols);
    for r in 0..rows {
        for c in 0..cols {
            let mut acc = 0.0f64;
            for (k, &w) in kernel.iter().enumerate() {
                let rr = (r as isize + k as isize - radius).clamp(0, rows.saturating_sub(1) as isize) as usize;
                acc += tmp.get(rr, c).unwrap_or(0.0) * w;
            }
            let _ = out.set(r, c, acc);
        }
    }
    out
}

fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    let pos = (p.clamp(0.0, 100.0) / 100.0) * (v.len().saturating_sub(1)) as f64;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    let t = pos - lo as f64;
    let a = v.get(lo).copied().unwrap_or(0.0);
    let b = v.get(hi).copied().unwrap_or(a);
    a * (1.0 - t) + b * t
}

fn canny_edges(g: &Matrix, thresh: Option<&Value>, sigma: Option<&Value>) -> Result<Matrix, LangError> {
    let sigma = sigma.map(|v| v.scalar()).transpose()?.unwrap_or(2f64.sqrt());
    if sigma.is_nan() || sigma <= 0.0 {
        return Err(LangError::new("edge: Canny sigma must be positive", 0));
    }
    let kernel = gaussian_kernel(sigma);
    let blur = gaussian_blur(g, &kernel);
    let hx = Matrix::from_row_major(3, 3, &[-1.0, 0.0, 1.0, -2.0, 0.0, 2.0, -1.0, 0.0, 1.0]).map_err(lang)?;
    let hy = Matrix::from_row_major(3, 3, &[-1.0, -2.0, -1.0, 0.0, 0.0, 0.0, 1.0, 2.0, 1.0]).map_err(lang)?;
    let gx = filter_channel(&blur, &hx, Boundary::Replicate, false, false)?;
    let gy = filter_channel(&blur, &hy, Boundary::Replicate, false, false)?;
    let rows = g.rows();
    let cols = g.cols();
    let mut nms = Matrix::zeros(rows, cols);
    for r in 1..rows.saturating_sub(1) {
        for c in 1..cols.saturating_sub(1) {
            let x = gx.get(r, c).unwrap_or(0.0);
            let y = gy.get(r, c).unwrap_or(0.0);
            let m = (x * x + y * y).sqrt();
            let theta = y.atan2(x);
            let (dy, dx) = (theta.sin(), theta.cos());
            let a = sample_magnitude(&gx, &gy, r as f64 - dy, c as f64 - dx);
            let b = sample_magnitude(&gx, &gy, r as f64 + dy, c as f64 + dx);
            if m >= a && m >= b {
                let _ = nms.set(r, c, m);
            }
        }
    }
    let (low, high) = match thresh {
        Some(v) => {
            let t = v.to_numeric()?;
            if t.numel() == 0 {
                auto_canny_thresholds(&nms)
            } else if t.numel() == 1 {
                let h = t.data().first().copied().unwrap_or(0.0);
                (0.4 * h, h)
            } else {
                let l = t.data().first().copied().unwrap_or(0.0);
                let h = t.data().get(1).copied().unwrap_or(l);
                if l <= h {
                    (l, h)
                } else {
                    (h, l)
                }
            }
        }
        None => auto_canny_thresholds(&nms),
    };
    if high.is_nan() || high <= 0.0 {
        return Ok(Matrix::zeros(rows, cols));
    }
    let mut out = Matrix::zeros(rows, cols);
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            if nms.get(r, c).unwrap_or(0.0) >= high {
                let _ = out.set(r, c, 1.0);
                stack.push((r, c));
            }
        }
    }
    while let Some((r, c)) = stack.pop() {
        for dr in -1..=1 {
            for dc in -1..=1 {
                if dr == 0 && dc == 0 {
                    continue;
                }
                let rr = r as isize + dr;
                let cc = c as isize + dc;
                if rr < 0 || cc < 0 || rr >= rows as isize || cc >= cols as isize {
                    continue;
                }
                let (rr, cc) = (rr as usize, cc as usize);
                if out.get(rr, cc).unwrap_or(0.0) == 0.0 && nms.get(rr, cc).unwrap_or(0.0) >= low {
                    let _ = out.set(rr, cc, 1.0);
                    stack.push((rr, cc));
                }
            }
        }
    }
    Ok(out)
}

fn sample_magnitude(gx: &Matrix, gy: &Matrix, r: f64, c: f64) -> f64 {
    let rows = gx.rows();
    let cols = gx.cols();
    if rows == 0 || cols == 0 {
        return 0.0;
    }
    let r = r.clamp(0.0, rows.saturating_sub(1) as f64);
    let c = c.clamp(0.0, cols.saturating_sub(1) as f64);
    let r0 = r.floor().min(rows.saturating_sub(1) as f64);
    let c0 = c.floor().min(cols.saturating_sub(1) as f64);
    let r1 = (r0 + 1.0).min(rows.saturating_sub(1) as f64);
    let c1 = (c0 + 1.0).min(cols.saturating_sub(1) as f64);
    let fr = r - r0;
    let fc = c - c0;
    let g = |rr: f64, cc: f64| {
        let x = gx.get(rr as usize, cc as usize).unwrap_or(0.0);
        let y = gy.get(rr as usize, cc as usize).unwrap_or(0.0);
        (x * x + y * y).sqrt()
    };
    let top = g(r0, c0) * (1.0 - fc) + g(r0, c1) * fc;
    let bot = g(r1, c0) * (1.0 - fc) + g(r1, c1) * fc;
    top * (1.0 - fr) + bot * fr
}

fn auto_canny_thresholds(nms: &Matrix) -> (f64, f64) {
    let vals: Vec<f64> = nms.data().iter().copied().filter(|v| *v > 0.0).collect();
    let high = percentile(&vals, 70.0);
    let high = if high > 0.0 { high } else { vals.iter().copied().fold(0.0f64, f64::max) * 0.1 };
    (0.4 * high, high)
}

fn label_components(bw: &Matrix, conn: usize) -> Result<(Matrix, usize), LangError> {
    let rows = bw.rows();
    let cols = bw.cols();
    check_elems(rows.saturating_mul(cols))?;
    let mut labels = Matrix::zeros(rows, cols);
    let mut next = 0usize;
    let neighbors: &[(isize, isize)] =
        if conn == 4 { &[(-1, 0), (1, 0), (0, -1), (0, 1)] } else { &[(-1, -1), (-1, 0), (-1, 1), (0, -1), (0, 1), (1, -1), (1, 0), (1, 1)] };
    for r0 in 0..rows {
        for c0 in 0..cols {
            if bw.get(r0, c0).unwrap_or(0.0) == 0.0 || labels.get(r0, c0).unwrap_or(0.0) != 0.0 {
                continue;
            }
            next += 1;
            let mut stack = vec![(r0, c0)];
            let _ = labels.set(r0, c0, next as f64);
            while let Some((r, c)) = stack.pop() {
                for &(dr, dc) in neighbors {
                    let nr = r as isize + dr;
                    let nc = c as isize + dc;
                    if nr < 0 || nc < 0 || nr >= rows as isize || nc >= cols as isize {
                        continue;
                    }
                    let (nr, nc) = (nr as usize, nc as usize);
                    if bw.get(nr, nc).unwrap_or(0.0) != 0.0 && labels.get(nr, nc).unwrap_or(0.0) == 0.0 {
                        let _ = labels.set(nr, nc, next as f64);
                        stack.push((nr, nc));
                    }
                }
            }
        }
    }
    Ok((labels, next))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_two_blocks() {
        let mut bw = Matrix::zeros(4, 5);
        for r in 0..2 {
            for c in 0..2 {
                let _ = bw.set(r, c, 1.0);
            }
        }
        let _ = bw.set(2, 3, 1.0);
        let (l, n) = label_components(&bw, 8).unwrap();
        assert_eq!(n, 2);
        assert_eq!(l.get(0, 0), Some(1.0));
        assert_eq!(l.get(2, 3), Some(2.0));
    }

    #[test]
    fn otsu_bimodal() {
        let m = Matrix::from_row_major(1, 4, &[0.0, 0.0, 1.0, 1.0]).unwrap();
        let t = otsu_threshold(&m).unwrap();
        assert!(t > 0.0 && t < 1.0, "threshold = {t}");
    }
}
