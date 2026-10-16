//! MATLAB MAT-file v5 binary reader/writer (public format spec).
//!
//! Layout: a 128-byte header (text + endian indicator "MI"), then tagged
//! data elements `[type:u32][size:u32][payload]` padded to 8 bytes; payloads
//! of 4 bytes or fewer use the small-element form `[type:u16][size:u16][data]`.
//! Each variable is a `miMATRIX` (array flags, dimensions, name, data) wrapped
//! in a `miCOMPRESSED` (zlib) element. Matrices are stored column-major —
//! which matches `matcore::Matrix` directly.

use matcore::{CMatrix, CNum, Matrix};

use crate::interp::{CellArray, StructArray};
use crate::Value;

// mi* data types (subset we read/write).
const MI_INT8: u32 = 1;
const MI_UINT8: u32 = 2;
const MI_INT32: u32 = 5;
const MI_UINT32: u32 = 6;
const MI_DOUBLE: u32 = 9;
const MI_MATRIX: u32 = 14;
const MI_COMPRESSED: u32 = 15;
const MI_UTF8: u32 = 16;
const MI_UTF16: u32 = 17;

// mx* array classes.
const MX_CELL: u32 = 1;
const MX_STRUCT: u32 = 2;
const MX_CHAR: u32 = 4;
const MX_SPARSE: u32 = 5;
const MX_DOUBLE: u32 = 6;
const MX_UINT8: u32 = 9;

/// Cap on sub-elements read from a foreign cell/struct — corrupt dims
/// must not drive unbounded allocation.
const MAX_NESTED: usize = 10_000_000;

const FLAG_COMPLEX: u32 = 0x0800;

fn push_tag(out: &mut Vec<u8>, ty: u32, payload: &[u8]) {
    if payload.len() <= 4 {
        // Small data element: type+size packed in one u32, data in the next.
        out.extend_from_slice(&(ty as u16).to_le_bytes());
        out.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        let mut p = [0u8; 4];
        p[..payload.len()].copy_from_slice(payload);
        out.extend_from_slice(&p);
    } else {
        out.extend_from_slice(&ty.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(payload);
        let pad = (8 - payload.len() % 8) % 8;
        out.resize(out.len() + pad, 0);
    }
}

/// Serialize one variable as a `miMATRIX` byte blob (pre-compression).
fn matrix_element(name: &str, class: u32, flags: u32, dims: &[u32], data_ty: u32, data: &[u8]) -> Vec<u8> {
    matrix_element_nz(name, class, flags, 0, dims, data_ty, data)
}

/// `matrix_element` with an explicit nzmax word (mxSPARSE uses it).
fn matrix_element_nz(name: &str, class: u32, flags: u32, nzmax: u32, dims: &[u32], data_ty: u32, data: &[u8]) -> Vec<u8> {
    let mut m = Vec::new();
    let mut af = Vec::with_capacity(8);
    af.extend_from_slice(&(class | flags).to_le_bytes());
    af.extend_from_slice(&nzmax.to_le_bytes());
    push_tag(&mut m, MI_UINT32, &af);
    let mut dv = Vec::with_capacity(dims.len() * 4);
    for d in dims {
        dv.extend_from_slice(&d.to_le_bytes());
    }
    push_tag(&mut m, MI_INT32, &dv);
    push_tag(&mut m, MI_INT8, name.as_bytes());
    push_tag(&mut m, data_ty, data);
    m
}

/// mxSPARSE element: flags carry nzmax, then `ir` (0-based row indices),
/// `jc` (column pointers, len cols+1), `pr` (values) — the MAT v5
/// sparse layout.
fn sparse_element(name: &str, s: &matcore::SparseMat) -> Vec<u8> {
    let mut ir = Vec::with_capacity(s.nnz() * 4);
    let mut jc = Vec::with_capacity((s.cols() + 1) * 4);
    let mut pr = Vec::with_capacity(s.nnz() * 8);
    let mut counts = vec![0u32; s.cols() + 1];
    for ((r, c), v) in s.entries() {
        ir.extend_from_slice(&(r as u32).to_le_bytes());
        pr.extend_from_slice(&v.to_le_bytes());
        counts[c + 1] += 1;
    }
    let mut run = 0u32;
    for c in 0..=s.cols() {
        jc.extend_from_slice(&run.to_le_bytes());
        run += counts.get(c + 1).copied().unwrap_or(0);
    }
    let mut m = Vec::new();
    let mut af = Vec::with_capacity(8);
    af.extend_from_slice(&MX_SPARSE.to_le_bytes());
    af.extend_from_slice(&(s.nnz() as u32).to_le_bytes());
    push_tag(&mut m, MI_UINT32, &af);
    push_tag(&mut m, MI_INT32, &[s.rows() as u32, s.cols() as u32].iter().flat_map(|d| d.to_le_bytes()).collect::<Vec<u8>>());
    push_tag(&mut m, MI_INT8, name.as_bytes());
    push_tag(&mut m, MI_INT32, &ir);
    push_tag(&mut m, MI_INT32, &jc);
    push_tag(&mut m, MI_DOUBLE, &pr);
    m
}

/// Serialize one value as a `miMATRIX` payload (the pre-compression body).
/// Shared by top-level variables and nested cell/struct elements — nested
/// elements carry an empty name, per the MAT v5 spec.
fn value_element(name: &str, val: &Value) -> Result<Vec<u8>, String> {
    match val {
        Value::Num(x) => Ok(matrix_element(name, MX_DOUBLE, 0, &[1, 1], MI_DOUBLE, &x.to_le_bytes())),
        Value::Mat(m) => {
            let dims = [m.rows() as u32, m.cols() as u32];
            let mut data = Vec::with_capacity(m.numel() * 8);
            // `Matrix::data` is already column-major — direct dump.
            for x in m.data() {
                data.extend_from_slice(&x.to_le_bytes());
            }
            Ok(matrix_element(name, MX_DOUBLE, 0, &dims, MI_DOUBLE, &data))
        }
        Value::Str(s) => Ok(matrix_element(name, MX_CHAR, 0, &[1, s.chars().count() as u32], MI_UTF8, s.as_bytes())),
        Value::Cplx(m) => {
            // Complex arrays: complex flag set + two miDOUBLE elements
            // (real part, then imaginary part), per the MAT v5 spec.
            let mut pr = Vec::with_capacity(m.numel() * 8);
            let mut pi = Vec::with_capacity(m.numel() * 8);
            for z in m.data() {
                pr.extend_from_slice(&z.re.to_le_bytes());
                pi.extend_from_slice(&z.im.to_le_bytes());
            }
            let mut el = matrix_element(name, MX_DOUBLE, FLAG_COMPLEX, &[m.rows() as u32, m.cols() as u32], MI_DOUBLE, &pr);
            push_tag(&mut el, MI_DOUBLE, &pi);
            Ok(el)
        }
        Value::Sparse(s) => Ok(sparse_element(name, s)),
        Value::Cell(c) => cell_element(name, c),
        Value::Struct(s) => struct_element(name, s),
        Value::Func(_) => Err(format!("cannot save '{name}' — function handles are not supported in .mat v5")),
    }
}

/// mxCELL element: flags/dims/name, then one nested miMATRIX per element
/// in column-major order (MAT v5 spec §mxCELL_CLASS).
fn cell_element(name: &str, c: &crate::CellArray) -> Result<Vec<u8>, String> {
    let mut m = Vec::new();
    let mut af = Vec::with_capacity(8);
    af.extend_from_slice(&MX_CELL.to_le_bytes());
    af.extend_from_slice(&0u32.to_le_bytes());
    push_tag(&mut m, MI_UINT32, &af);
    let (rows, cols) = c.shape();
    let mut dv = Vec::with_capacity(8);
    dv.extend_from_slice(&(rows as u32).to_le_bytes());
    dv.extend_from_slice(&(cols as u32).to_le_bytes());
    push_tag(&mut m, MI_INT32, &dv);
    push_tag(&mut m, MI_INT8, name.as_bytes());
    for i in 0..c.numel() {
        let v = c.get_linear(i).cloned().unwrap_or_else(|| Value::Mat(Matrix::zeros(0, 0)));
        let inner = value_element("", &v)?;
        push_tag(&mut m, MI_MATRIX, &inner);
    }
    Ok(m)
}

/// mxSTRUCT element: flags/dims/name, then the Field Name Length element
/// (miINT32 — longest field name + 1, room for the NUL terminator), the
/// Field Names element (miINT8 — each name NUL-padded to fieldnamelen),
/// then field values as nested miMATRIX elements — for each element in
/// column order, one matrix per field (MAT v5 spec §mxSTRUCT_CLASS).
fn struct_element(name: &str, s: &crate::interp::StructArray) -> Result<Vec<u8>, String> {
    let mut m = Vec::new();
    let mut af = Vec::with_capacity(8);
    af.extend_from_slice(&MX_STRUCT.to_le_bytes());
    af.extend_from_slice(&0u32.to_le_bytes());
    push_tag(&mut m, MI_UINT32, &af);
    let mut dv = Vec::with_capacity(8);
    dv.extend_from_slice(&(s.rows() as u32).to_le_bytes());
    dv.extend_from_slice(&(s.cols() as u32).to_le_bytes());
    push_tag(&mut m, MI_INT32, &dv);
    push_tag(&mut m, MI_INT8, name.as_bytes());
    let fields = s.field_names();
    // Spec: fieldnamelen = longest field name + 1 (NUL). MATLAB writes 32;
    // any consistent value is legal — keep it tight.
    let fnlen = fields.iter().map(|f| f.len() + 1).max().unwrap_or(0).max(1) as u32;
    push_tag(&mut m, MI_INT32, &fnlen.to_le_bytes());
    let mut fnb = Vec::with_capacity(fnlen as usize * fields.len());
    for f in &fields {
        let mut b = f.clone().into_bytes();
        b.resize(fnlen as usize, 0);
        fnb.extend_from_slice(&b);
    }
    push_tag(&mut m, MI_INT8, &fnb);
    for i in 0..s.numel() {
        for f in &fields {
            let v = s.field_at(i, f).cloned().unwrap_or_else(|| Value::Mat(Matrix::zeros(0, 0)));
            let inner = value_element("", &v)?;
            push_tag(&mut m, MI_MATRIX, &inner);
        }
    }
    Ok(m)
}

/// Write all `vars` to `path` in MAT v5 binary form.
pub fn write(path: &str, vars: &[(String, Value)]) -> Result<usize, String> {
    let mut out = Vec::new();
    let mut desc = format!("MATLAB 5.0 MAT-file, Platform: GLNXA64, Created by MatCraft {}", env!("CARGO_PKG_VERSION"));
    desc.truncate(116);
    desc.push_str(&" ".repeat(116 - desc.len()));
    out.extend_from_slice(desc.as_bytes());
    out.extend_from_slice(&[0u8; 8]); // subsystem data offset
    out.extend_from_slice(&[0x00, 0x01]); // version 0x0100
    out.extend_from_slice(b"MI"); // endian indicator: MI = little-endian

    for (name, val) in vars {
        let inner = value_element(name, val)?;
        // For miMATRIX the byte count includes its own 8-byte padding.
        let mut inner = inner;
        inner.resize(inner.len() + (8 - inner.len() % 8) % 8, 0);
        let mut elem = Vec::with_capacity(inner.len() + 8);
        push_tag(&mut elem, MI_MATRIX, &inner);
        let mut comp = Vec::new();
        {
            use std::io::Write;
            let mut enc = flate2::write::ZlibEncoder::new(&mut comp, flate2::Compression::default());
            enc.write_all(&elem).map_err(|e| format!("zlib: {e}"))?;
            enc.try_finish().map_err(|e| format!("zlib: {e}"))?;
        }
        push_tag(&mut out, MI_COMPRESSED, &comp);
    }
    std::fs::write(path, &out).map_err(|e| format!("cannot write {path}: {e}"))?;
    Ok(vars.len())
}

// ---------- reading ----------

struct Rd<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Rd<'a> {
    fn u32(&mut self) -> Option<u32> {
        let b = self.buf.get(self.pos..self.pos + 4)?;
        self.pos += 4;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let b = self.buf.get(self.pos..self.pos + n)?;
        self.pos += n;
        Some(b)
    }
    /// Read one data element; returns (type, payload). Handles the
    /// small-element form and 8-byte padding.
    fn element(&mut self) -> Option<(u32, &'a [u8])> {
        let tag = self.u32()?;
        let ty = tag & 0xFFFF;
        let small_n = tag >> 16;
        if small_n != 0 && small_n <= 4 && !matches!(ty, 0) {
            // Small data element: payload is the next 4 bytes inline.
            let p = self.bytes(4)?;
            return Some((ty, &p[..small_n as usize]));
        }
        let n = self.u32()? as usize;
        let p = self.bytes(n)?;
        let pad = (8 - n % 8) % 8;
        let _ = self.bytes(pad);
        Some((tag, p))
    }
}

fn read_matrix(buf: &[u8]) -> Result<(String, Value), String> {
    let mut r = Rd { buf, pos: 0 };
    // Array flags
    let (t, p) = r.element().ok_or("truncated array flags")?;
    if t != MI_UINT32 || p.len() < 4 {
        return Err("bad array flags".into());
    }
    let flags = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
    let class = flags & 0xFF;
    let is_complex = flags & FLAG_COMPLEX != 0;
    // Dimensions
    let (t, p) = r.element().ok_or("truncated dims")?;
    if t != MI_INT32 || p.len() < 8 {
        return Err("bad dims".into());
    }
    let nd = p.len() / 4;
    let mut dims = Vec::with_capacity(nd);
    for i in 0..nd {
        dims.push(u32::from_le_bytes([p[i * 4], p[i * 4 + 1], p[i * 4 + 2], p[i * 4 + 3]]) as usize);
    }
    if dims.len() > 2 {
        return Err("N-D arrays not supported yet".into());
    }
    let rows = dims.first().copied().unwrap_or(1);
    let cols = dims.get(1).copied().unwrap_or(1);
    // Name
    let (t, p) = r.element().ok_or("truncated name")?;
    if t != MI_INT8 {
        return Err("bad name".into());
    }
    let name = String::from_utf8_lossy(p).into_owned();
    // Data
    let read_doubles = |r: &mut Rd<'_>, n: usize| -> Result<(u32, Vec<f64>), String> {
        let (t, p) = r.element().ok_or("truncated data")?;
        if t != MI_DOUBLE || p.len() < n * 8 {
            return Err("double payload mismatch".into());
        }
        let mut data = Vec::with_capacity(n);
        for i in 0..n {
            let b = p.get(i * 8..i * 8 + 8).ok_or("double payload short")?;
            data.push(f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]));
        }
        Ok((t, data))
    };
    let (t, p) = r.element().ok_or("truncated data")?;
    match class {
        MX_DOUBLE => {
            if t != MI_DOUBLE || p.len() < rows * cols * 8 {
                return Err("double payload mismatch".into());
            }
            let mut data = Vec::with_capacity(rows * cols);
            for i in 0..rows * cols {
                let b = p.get(i * 8..i * 8 + 8).ok_or("double payload short")?;
                data.push(f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]));
            }
            if is_complex {
                // Imaginary part follows as a second miDOUBLE element.
                let (_t2, im) = read_doubles(&mut r, rows * cols)?;
                if im.len() != data.len() {
                    return Err("complex imag payload mismatch".into());
                }
                let cd: Vec<CNum> = data.into_iter().zip(im).map(|(re, im)| CNum::new(re, im)).collect();
                let m = CMatrix::from_col_major(rows, cols, cd).map_err(|e| format!("bad dims: {e}"))?;
                return Ok((name, Value::Cplx(m)));
            }
            let m = Matrix::from_col_major(rows, cols, data).map_err(|e| format!("bad dims: {e}"))?;
            Ok((name, if m.is_scalar() { Value::Num(m.as_scalar().unwrap_or(0.0)) } else { Value::Mat(m) }))
        }
        MX_CHAR => {
            let s = match t {
                MI_UTF8 | MI_INT8 | MI_UINT8 => String::from_utf8_lossy(p).into_owned(),
                MI_UTF16 => {
                    let u: Vec<u16> = p.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
                    String::from_utf16_lossy(&u)
                }
                _ => return Err(format!("unsupported char storage mi{t}")),
            };
            Ok((name, Value::Str(s)))
        }
        MX_UINT8 => {
            // e.g. logical arrays stored as uint8 with the logical flag.
            let data: Vec<f64> = p.iter().take(rows * cols).map(|&b| f64::from(b)).collect();
            let m = Matrix::from_col_major(rows, cols, data).map_err(|e| format!("bad dims: {e}"))?;
            Ok((name, Value::Mat(m)))
        }
        MX_SPARSE => {
            // Sparse payload: `ir` row indices, `jc` column pointers
            // (len cols+1), `pr` values. The first element read above is ir.
            if t != MI_INT32 {
                return Err("sparse ir payload mismatch".into());
            }
            let ir: Vec<u32> = p.as_chunks::<4>().0.iter().map(|b| u32::from_le_bytes(*b)).collect();
            let (_t2, pj) = r.element().ok_or("truncated sparse jc")?;
            if pj.len() < (cols + 1) * 4 {
                return Err("sparse jc payload mismatch".into());
            }
            let jc: Vec<u32> = pj.as_chunks::<4>().0.iter().take(cols + 1).map(|b| u32::from_le_bytes(*b)).collect();
            let nnz = ir.len();
            let (_t3, pp) = r.element().ok_or("truncated sparse pr")?;
            if pp.len() < nnz * 8 {
                return Err("sparse pr payload mismatch".into());
            }
            let pr: Vec<f64> = pp.as_chunks::<8>().0.iter().take(nnz).map(|b| f64::from_le_bytes(*b)).collect();
            // Expand CSC back to triplets for SparseMat.
            let mut is = Vec::with_capacity(nnz);
            let mut js = Vec::with_capacity(nnz);
            let mut k = 0usize;
            for c in 0..cols {
                let end = jc.get(c + 1).copied().unwrap_or(0) as usize;
                while k < end.min(nnz) {
                    is.push(f64::from(ir.get(k).copied().unwrap_or(0)) + 1.0);
                    js.push((c + 1) as f64);
                    k += 1;
                }
            }
            let s = matcore::SparseMat::from_triplets(rows, cols, &is, &js, &pr).map_err(|e| format!("bad sparse data: {e}"))?;
            Ok((name, Value::Sparse(s)))
        }
        MX_CELL => {
            // After the name: one nested miMATRIX per cell element, in
            // column-major order. (t, p) read above is the first of them.
            let n = rows.saturating_mul(cols);
            if n > MAX_NESTED {
                return Err("cell array too large".into());
            }
            let mut items = Vec::with_capacity(n);
            let mut first = Some((t, p));
            for _ in 0..n {
                let (et, ep) = match first.take() {
                    Some(x) => x,
                    None => r.element().ok_or("truncated cell contents")?,
                };
                if et != MI_MATRIX {
                    return Err("cell element is not an miMATRIX".into());
                }
                items.push(read_matrix(ep)?.1);
            }
            Ok((name, Value::Cell(CellArray::from_col_major(rows, cols, items))))
        }
        MX_STRUCT => {
            // (t, p) read above is the Field Name Length element (miINT32).
            if t != MI_INT32 || p.len() < 4 {
                return Err("bad struct field-name length".into());
            }
            let fnlen = u32::from_le_bytes([p[0], p[1], p[2], p[3]]) as usize;
            if fnlen == 0 || fnlen > 65536 {
                return Err("bad struct field-name length".into());
            }
            let (ft, fp) = r.element().ok_or("truncated struct field names")?;
            if !matches!(ft, MI_INT8 | MI_UINT8 | MI_UTF8) {
                return Err("bad struct field names".into());
            }
            let nfields = fp.len() / fnlen;
            let mut names = Vec::with_capacity(nfields);
            for i in 0..nfields {
                let chunk = fp.get(i * fnlen..(i + 1) * fnlen).unwrap_or(&[]);
                let end = chunk.iter().position(|&b| b == 0).unwrap_or(chunk.len());
                names.push(String::from_utf8_lossy(&chunk[..end]).into_owned());
            }
            let n = rows.saturating_mul(cols);
            if n.saturating_mul(nfields) > MAX_NESTED {
                return Err("struct array too large".into());
            }
            // Field values: per element (column order), one miMATRIX per
            // field, in field-name order.
            let mut vals: Vec<Vec<Value>> = (0..nfields).map(|_| Vec::with_capacity(n)).collect();
            for _el in 0..n {
                for f in 0..nfields {
                    let (et, ep) = r.element().ok_or("truncated struct field data")?;
                    if et != MI_MATRIX {
                        return Err("struct field element is not an miMATRIX".into());
                    }
                    let (_, v) = read_matrix(ep)?;
                    if let Some(slot) = vals.get_mut(f) {
                        slot.push(v);
                    }
                }
            }
            let mut sa = StructArray::with_dims(rows, cols);
            for (f, fname) in names.iter().enumerate() {
                sa.add_field(fname);
                if let Some(field_vals) = vals.get_mut(f) {
                    for (i, v) in field_vals.drain(..).enumerate() {
                        sa.set_field(i, fname, v).map_err(|e| e.message)?;
                    }
                }
            }
            Ok((name, Value::Struct(sa)))
        }
        _ => Err(format!("unsupported array class {class}")),
    }
}

/// Read a MAT v5 file; returns every variable found.
pub fn read(bytes: &[u8]) -> Result<Vec<(String, Value)>, String> {
    if bytes.len() < 128 || !bytes.starts_with(b"MATLAB 5.0 MAT-file") {
        return Err("not a MAT-file v5".into());
    }
    let mut r = Rd { buf: &bytes[128..], pos: 0 };
    let mut vars = Vec::new();
    while let Some((t, p)) = r.element() {
        if t == MI_MATRIX {
            vars.push(read_matrix(p)?);
        } else if t == MI_COMPRESSED {
            let mut dec = Vec::new();
            {
                use std::io::Read;
                let mut z = flate2::read::ZlibDecoder::new(p);
                z.read_to_end(&mut dec).map_err(|e| format!("zlib: {e}"))?;
            }
            let mut ir = Rd { buf: &dec, pos: 0 };
            while let Some((it, ip)) = ir.element() {
                if it == MI_MATRIX {
                    vars.push(read_matrix(ip)?);
                }
            }
        }
    }
    Ok(vars)
}

/// Quick check: does `bytes` look like a MAT v5 file?
pub fn looks_like_v5(bytes: &[u8]) -> bool {
    bytes.len() >= 128 && bytes.starts_with(b"MATLAB 5.0 MAT-file")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(vars: Vec<(String, Value)>) -> Vec<(String, Value)> {
        // Unique temp path per call — tests run in parallel on one process.
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let tmp = std::env::temp_dir().join(format!(
            "matcraft_matfile_test_{}_{}.mat",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let p = tmp.to_string_lossy().to_string();
        write(&p, &vars).unwrap();
        let bytes = std::fs::read(&tmp).unwrap();
        read(&bytes).unwrap()
    }

    #[test]
    fn header_is_matlab5() {
        let tmp = std::env::temp_dir().join("matcraft_hdr.mat");
        write(&tmp.to_string_lossy(), &[]).unwrap();
        let b = std::fs::read(&tmp).unwrap();
        assert_eq!(&b[126..128], b"MI");
        assert!(b.starts_with(b"MATLAB 5.0 MAT-file"));
    }

    #[test]
    fn roundtrip_num_mat_str() {
        let vars = vec![
            ("a".to_string(), Value::Num(2.5)),
            ("m".to_string(), Value::Mat(Matrix::from_row_major(2, 2, &[1.0, 2.0, 3.0, 4.0]).unwrap())),
            ("s".to_string(), Value::Str("héllo".into())),
        ];
        let got = roundtrip(vars);
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].1, Value::Num(2.5));
        match &got[1].1 {
            Value::Mat(m) => {
                assert_eq!((m.rows(), m.cols()), (2, 2));
                assert_eq!(m.get(1, 0).unwrap_or(0.0), 3.0);
            }
            v => panic!("expected Mat, got {v:?}"),
        }
        assert_eq!(got[2].1, Value::Str("héllo".into()));
    }

    #[test]
    fn roundtrip_complex() {
        let cm = CMatrix::from_col_major(2, 2, vec![CNum::new(1.0, 2.0), CNum::new(5.0, 0.5), CNum::new(3.0, -4.0), CNum::new(0.0, -2.0)]).unwrap();
        let got = roundtrip(vec![("z".to_string(), Value::Cplx(cm))]);
        assert_eq!(got.len(), 1);
        match &got[0].1 {
            Value::Cplx(m) => {
                assert_eq!((m.rows(), m.cols()), (2, 2));
                assert_eq!(m.get(1, 0).unwrap_or(CNum::real(0.0)), CNum::new(5.0, 0.5));
                assert_eq!(m.get(1, 1).unwrap_or(CNum::real(0.0)), CNum::new(0.0, -2.0));
            }
            v => panic!("expected Cplx, got {v:?}"),
        }
    }

    #[test]
    fn roundtrip_sparse() {
        let s = matcore::SparseMat::from_triplets(3, 3, &[1.0, 2.0, 3.0], &[1.0, 3.0, 2.0], &[10.0, 20.0, 30.0]).unwrap();
        let got = roundtrip(vec![("s".to_string(), Value::Sparse(s))]);
        assert_eq!(got.len(), 1);
        match &got[0].1 {
            Value::Sparse(m) => {
                assert_eq!((m.rows(), m.cols()), (3, 3));
                assert_eq!(m.nnz(), 3);
                assert_eq!(m.get(0, 0), 10.0);
                assert_eq!(m.get(1, 2), 20.0);
                assert_eq!(m.get(2, 1), 30.0);
            }
            v => panic!("expected Sparse, got {v:?}"),
        }
    }
}
