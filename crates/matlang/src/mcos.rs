//! Minimal MCOS (MATLAB Class Object System) subsystem support for MAT v5.
//!
//! MATLAB stores "opaque" objects as `mxOPAQUE_CLASS` variables whose real
//! fields live in a subsystem appended to the file. This module parses the
//! documented FileWrapper/MCOS metadata regions and can encode scalar
//! objects with non-object properties for `-mcos` saves. The format is
//! reverse-engineered from public descriptions; unsupported regions are
//! reported rather than guessed.

use crate::interp::{CellArray, MObject, StructArray};
use crate::matfile::{
    matrix_element, push_tag, value_element, MI_COMPRESSED, MI_DOUBLE, MI_INT32, MI_INT8, MI_MATRIX, MI_UINT32, MI_UINT8, MI_UTF8, MX_CELL, MX_CHAR,
    MX_DOUBLE, MX_INT32, MX_OPAQUE, MX_STRUCT, MX_UINT32, MX_UINT8,
};
use crate::Value;
use matcore::Matrix;

const MCOS_MAGIC: u32 = 0xdd00_0000;
const FWRAP_VERSION: u32 = 4;

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
        let b = self.buf.get(self.pos..self.pos.checked_add(n)?)?;
        self.pos += n;
        Some(b)
    }
    fn element(&mut self) -> Option<(u32, &'a [u8])> {
        let tag = self.u32()?;
        let ty = tag & 0xFFFF;
        let small = tag >> 16;
        if small != 0 && small <= 4 && ty != 0 {
            let p = self.bytes(4)?;
            return Some((ty, &p[..small as usize]));
        }
        let n = self.u32()? as usize;
        let p = self.bytes(n)?;
        let pad = (8 - n % 8) % 8;
        let _ = self.bytes(pad)?;
        Some((ty, p))
    }
}

#[derive(Clone)]
enum Raw {
    Num(Matrix),
    Str(String),
    Cell { rows: usize, cols: usize, items: Vec<Raw> },
    Struct(RawStruct),
    Bytes(Vec<u8>),
    U32(Vec<u32>),
    Opaque { type_system: String, class: String, data: Box<Raw> },
}

#[derive(Clone, Default)]
struct RawStruct {
    rows: usize,
    cols: usize,
    fields: Vec<(String, Vec<Raw>)>,
}

impl RawStruct {
    fn field(&self, name: &str) -> Option<&Raw> {
        self.fields.iter().find(|(n, _)| n == name).and_then(|(_, v)| v.first())
    }
}

fn read_name(p: &[u8]) -> String {
    let end = p.iter().position(|&b| b == 0).unwrap_or(p.len());
    String::from_utf8_lossy(p.get(..end).unwrap_or(&[])).to_string()
}

fn parse_raw_element(r: &mut Rd<'_>) -> Result<Raw, String> {
    let (ty, payload) = r.element().ok_or("mcos: truncated element")?;
    match ty {
        MI_MATRIX => parse_matrix(payload).map(|(_, v)| v),
        MI_COMPRESSED => {
            let mut dec = Vec::new();
            {
                use std::io::Read;
                let mut z = flate2::read::ZlibDecoder::new(payload);
                z.read_to_end(&mut dec).map_err(|e| format!("mcos: zlib: {e}"))?;
            }
            let mut inner = Rd { buf: &dec, pos: 0 };
            parse_raw_element(&mut inner)
        }
        _ => Err(format!("mcos: expected miMATRIX, found mi{ty}")),
    }
}

fn parse_matrix(payload: &[u8]) -> Result<(String, Raw), String> {
    let mut r = Rd { buf: payload, pos: 0 };
    let (ft, fp) = r.element().ok_or("mcos: missing array flags")?;
    if ft != MI_UINT32 || fp.len() < 8 {
        return Err("mcos: bad array flags".into());
    }
    let class = u32::from_le_bytes([fp[0], fp[1], fp[2], fp[3]]) & 0xFF;
    let mut dims: Vec<usize> = Vec::new();
    if class != MX_OPAQUE {
        let (dt, dp) = r.element().ok_or("mcos: missing dimensions")?;
        if dt != MI_INT32 {
            return Err("mcos: bad dimensions".into());
        }
        for c in dp.as_chunks::<4>().0 {
            dims.push(u32::from_le_bytes([c[0], c[1], c[2], c[3]]) as usize);
        }
    }
    let (nt, np) = r.element().ok_or("mcos: missing name")?;
    if nt != MI_INT8 && nt != MI_UINT8 {
        return Err("mcos: bad name".into());
    }
    let name = read_name(np);
    let rows = dims.first().copied().unwrap_or(1);
    let cols = dims.get(1).copied().unwrap_or(1);
    let count = dims.iter().fold(1usize, |a, &d| a.saturating_mul(d));
    if count > 100_000_000 {
        return Err("mcos: array is too large".into());
    }
    let value = match class {
        MX_DOUBLE => {
            let (dt, dp) = r.element().ok_or("mcos: missing double data")?;
            if dt != MI_DOUBLE || dp.len() < count * 8 {
                return Err("mcos: bad double data".into());
            }
            let mut data = Vec::with_capacity(count);
            for c in dp.as_chunks::<8>().0.iter().take(count) {
                data.push(f64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]));
            }
            Raw::Num(Matrix::from_col_major_nd(&dims, data).map_err(|e| format!("mcos: {e}"))?)
        }
        MX_CHAR => {
            let (dt, dp) = r.element().ok_or("mcos: missing char data")?;
            let text = match dt {
                MI_UTF8 | MI_INT8 | MI_UINT8 => String::from_utf8_lossy(dp).to_string(),
                _ => return Err(format!("mcos: unsupported char storage mi{dt}")),
            };
            Raw::Str(text.trim_end_matches('\0').to_string())
        }
        MX_CELL => {
            let n = rows.saturating_mul(cols);
            let mut items = Vec::with_capacity(n);
            for _ in 0..n {
                items.push(parse_raw_element(&mut r)?);
            }
            Raw::Cell { rows, cols, items }
        }
        MX_STRUCT => Raw::Struct(parse_struct(&mut r, rows, cols, None)?),
        MX_UINT8 => {
            let (dt, dp) = r.element().ok_or("mcos: missing uint8 data")?;
            if dt != MI_UINT8 {
                return Err("mcos: bad uint8 data".into());
            }
            let _ = dims;
            Raw::Bytes(dp.to_vec())
        }
        MX_INT32 => {
            let (dt, dp) = r.element().ok_or("mcos: missing int32 data")?;
            if dt != MI_INT32 {
                return Err("mcos: bad int32 data".into());
            }
            let mut v = Vec::with_capacity(dp.len() / 4);
            for c in dp.as_chunks::<4>().0 {
                v.push(i32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f64);
            }
            Raw::Num(Matrix::col_vector(v))
        }
        MX_UINT32 => {
            let (dt, dp) = r.element().ok_or("mcos: missing uint32 data")?;
            if dt != MI_UINT32 {
                return Err("mcos: bad uint32 data".into());
            }
            let mut v = Vec::with_capacity(dp.len() / 4);
            for c in dp.as_chunks::<4>().0 {
                v.push(u32::from_le_bytes([c[0], c[1], c[2], c[3]]));
            }
            Raw::U32(v)
        }
        MX_OPAQUE => {
            let (tt, tp) = r.element().ok_or("mcos: missing type system")?;
            if tt != MI_INT8 && tt != MI_UINT8 && tt != MI_UTF8 {
                return Err("mcos: bad type system".into());
            }
            let type_system = read_name(tp);
            let (ct, cp) = r.element().ok_or("mcos: missing class name")?;
            if ct != MI_INT8 && ct != MI_UINT8 && ct != MI_UTF8 {
                return Err("mcos: bad class name".into());
            }
            let class_name = read_name(cp);
            let data = Box::new(parse_raw_element(&mut r)?);
            Raw::Opaque { type_system, class: class_name, data }
        }
        other => return Err(format!("mcos: unsupported array class {other}")),
    };
    Ok((name, value))
}

fn parse_struct(r: &mut Rd<'_>, rows: usize, cols: usize, class_name: Option<String>) -> Result<RawStruct, String> {
    let _ = class_name;
    let (flt, flp) = r.element().ok_or("mcos: missing field-name length")?;
    if flt != MI_INT32 || flp.len() < 4 {
        return Err(format!("mcos: bad field-name length (type {flt}, bytes {})", flp.len()));
    }
    let fnlen = u32::from_le_bytes([flp[0], flp[1], flp[2], flp[3]]) as usize;
    if fnlen > 4096 {
        return Err(format!("mcos: bad field-name length (type {flt}, bytes {})", flp.len()));
    }
    if fnlen == 0 {
        return Ok(RawStruct { rows, cols, fields: Vec::new() });
    }
    let (nt, np) = r.element().ok_or("mcos: missing field names")?;
    if nt != MI_INT8 && nt != MI_UINT8 {
        return Err("mcos: bad field names".into());
    }
    let nfields = np.len() / fnlen;
    let mut names = Vec::with_capacity(nfields);
    for i in 0..nfields {
        let chunk = np.get(i * fnlen..(i + 1) * fnlen).unwrap_or(&[]);
        names.push(read_name(chunk));
    }
    let n = rows.saturating_mul(cols);
    if n.saturating_mul(nfields) > 10_000_000 {
        return Err("mcos: struct is too large".into());
    }
    let mut values: Vec<Vec<Raw>> = (0..nfields).map(|_| Vec::with_capacity(n)).collect();
    for _ in 0..n {
        for (f, slot) in values.iter_mut().enumerate() {
            let _ = f;
            slot.push(parse_raw_element(r)?);
        }
    }
    Ok(RawStruct { rows, cols, fields: names.into_iter().zip(values).collect() })
}

impl Raw {
    fn to_value(&self) -> Result<Value, String> {
        match self {
            Raw::Num(m) => Ok(if m.is_scalar() { Value::Num(m.as_scalar().unwrap_or(0.0)) } else { Value::Mat(m.clone()) }),
            Raw::Str(s) => Ok(Value::Str(s.clone())),
            Raw::Cell { rows, cols, items } => {
                let vals = items.iter().map(Raw::to_value).collect::<Result<Vec<_>, _>>()?;
                Ok(Value::Cell(CellArray::from_col_major(*rows, *cols, vals)))
            }
            Raw::Struct(s) => {
                let mut sa = StructArray::with_dims(s.rows.max(1), s.cols.max(1));
                for (name, values) in &s.fields {
                    sa.add_field(name);
                    for (i, v) in values.iter().enumerate() {
                        sa.set_field(i, name, v.to_value()?).map_err(|e| e.message)?;
                    }
                }
                Ok(Value::Struct(sa))
            }
            Raw::Bytes(b) => Ok(Value::Mat(Matrix::row_vector(b.iter().map(|&x| f64::from(x)).collect()))),
            Raw::U32(v) => Ok(Value::Mat(Matrix::col_vector(v.iter().map(|&x| f64::from(x)).collect()))),
            Raw::Opaque { data, .. } => data.to_value(),
        }
    }
}

/// Parsed MCOS subsystem.
pub(crate) struct Ctx {
    names: Vec<String>,
    classes: Vec<u32>,
    objects: Vec<u32>,
    obj_props: Vec<u32>,
    dyn_props: Vec<u32>,
    prop_vals: Vec<Raw>,
    defaults: Vec<Raw>,
}

impl Ctx {
    pub(crate) fn parse(bytes: &[u8], offset: usize) -> Result<Self, String> {
        if offset == 0 || offset >= bytes.len() {
            return Err("mcos: bad subsystem offset".into());
        }
        let mut r = Rd { buf: bytes, pos: offset };
        let outer = parse_raw_element(&mut r)?;
        let Raw::Bytes(raw) = outer else {
            return Err("mcos: subsystem wrapper is not a uint8 array".into());
        };
        if raw.len() < 8 {
            return Err("mcos: subsystem header is truncated".into());
        }
        let mut inner = Rd { buf: &raw[8..], pos: 0 };
        let root = parse_raw_element(&mut inner)?;
        let Raw::Struct(root) = root else {
            return Err("mcos: subsystem payload is not a struct".into());
        };
        let Some(Raw::Opaque { type_system, class, data }) = root.field("MCOS") else {
            return Err("mcos: subsystem has no MCOS field".into());
        };
        if type_system != "MCOS" || class != "FileWrapper__" {
            return Err(format!("mcos: unsupported wrapper '{type_system}/{class}'"));
        }
        let Raw::Cell { items, .. } = data.as_ref() else {
            return Err("mcos: FileWrapper__ data is not a cell".into());
        };
        if items.len() < 4 {
            return Err("mcos: FileWrapper__ cell is too short".into());
        }
        let Raw::Bytes(meta) = items.first().ok_or("mcos: missing metadata")? else {
            return Err("mcos: metadata is not a uint8 array".into());
        };
        if meta.len() < 40 {
            return Err("mcos: metadata is truncated".into());
        }
        let version = u32_at(meta, 0)?;
        if version != FWRAP_VERSION {
            return Err(format!("mcos: unsupported FileWrapper version {version}"));
        }
        let num_names = u32_at(meta, 4)? as usize;
        if num_names > 100_000 {
            return Err("mcos: too many names".into());
        }
        let mut offsets = Vec::with_capacity(8);
        for i in 0..8 {
            offsets.push(u32_at(meta, 8 + i * 4)? as usize);
        }
        let names_end = *offsets.first().ok_or("mcos: no name offsets")?;
        if names_end > meta.len() || names_end < 40 {
            return Err("mcos: bad name region".into());
        }
        let names_bytes = meta.get(40..names_end).ok_or("mcos: truncated names")?;
        let mut names = Vec::with_capacity(num_names);
        let mut start = 0usize;
        while names.len() < num_names && start < names_bytes.len() {
            let end = names_bytes.get(start..).and_then(|s| s.iter().position(|&b| b == 0)).map(|p| start + p).unwrap_or(names_bytes.len());
            names.push(String::from_utf8_lossy(names_bytes.get(start..end).unwrap_or(&[])).to_string());
            start = end + 1;
        }
        if names.len() != num_names {
            return Err("mcos: name table is truncated".into());
        }
        let region = |i: usize| -> Result<&[u8], String> {
            let a = *offsets.get(i).ok_or("mcos: bad region offset")?;
            let b = *offsets.get(i + 1).ok_or("mcos: bad region offset")?;
            if a > b || b > meta.len() {
                return Err("mcos: bad metadata region".into());
            }
            meta.get(a..b).ok_or_else(|| "mcos: truncated metadata region".to_string())
        };
        let u32vec = |b: &[u8]| -> Vec<u32> { b.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect() };
        let classes = u32vec(region(0)?);
        let objects = u32vec(region(2)?);
        let obj_props = u32vec(region(3)?);
        let dyn_props = u32vec(region(4)?);
        if classes.len() % 4 != 0 || objects.len() % 6 != 0 {
            return Err("mcos: malformed class/object metadata".into());
        }
        let prop_vals = items.get(2..items.len().saturating_sub(3)).ok_or("mcos: missing field values")?.to_vec();
        let defaults = match items.last() {
            Some(Raw::Cell { items, .. }) => items.clone(),
            _ => Vec::new(),
        };
        Ok(Ctx { names, classes, objects, obj_props, dyn_props, prop_vals, defaults })
    }

    fn class_name(&self, class_id: u32) -> Result<String, String> {
        let base = (class_id as usize).checked_mul(4).ok_or("mcos: class id overflow")?;
        let ns = *self.classes.get(base).ok_or("mcos: unknown class id")?;
        let cn = *self.classes.get(base + 1).ok_or("mcos: unknown class id")?;
        let class = self.name(cn)?;
        if ns == 0 {
            Ok(class)
        } else {
            Ok(format!("{}.{}", self.name(ns)?, class))
        }
    }

    fn name(&self, idx: u32) -> Result<String, String> {
        if idx == 0 {
            return Ok(String::new());
        }
        self.names.get(idx as usize - 1).cloned().ok_or_else(|| "mcos: name index out of range".into())
    }

    fn object_meta(&self, object_id: u32) -> Result<&[u32], String> {
        let base = (object_id as usize).checked_mul(6).ok_or("mcos: object id overflow")?;
        let slice = self.objects.get(base..base + 6).ok_or("mcos: unknown object id")?;
        Ok(slice)
    }

    fn prop_block(&self, type_id: u32) -> Result<(usize, usize), String> {
        if type_id == 0 {
            return Ok((0, 0));
        }
        let mut off = 0usize;
        for _ in 1..type_id {
            let n = *self.obj_props.get(off).ok_or("mcos: bad property metadata")? as usize;
            off = off.saturating_add(1 + 3 * n);
            if off % 2 == 1 {
                off += 1;
            }
        }
        let n = *self.obj_props.get(off).ok_or("mcos: bad property metadata")? as usize;
        off += 1;
        Ok((off, n))
    }

    fn build_object(&self, object_id: u32) -> Result<Value, String> {
        let meta = self.object_meta(object_id)?;
        let class_id = meta.first().copied().unwrap_or(0);
        let class = self.class_name(class_id)?;
        let mut props = StructArray::new();
        if let Some(Raw::Struct(def)) = self.defaults.get(class_id as usize) {
            if def.rows.saturating_mul(def.cols) > 0 {
                for (name, values) in &def.fields {
                    if let Some(v) = values.first() {
                        props.add_field(name);
                        props.set_field(0, name, v.to_value()?).map_err(|e| e.message)?;
                    }
                }
            }
        }
        let type_id = if meta.get(3).copied().unwrap_or(0) != 0 { meta[3] } else { meta.get(4).copied().unwrap_or(0) };
        if type_id != 0 {
            let (off, nprops) = self.prop_block(type_id)?;
            for i in 0..nprops {
                let base = off + i * 3;
                let name_idx = *self.obj_props.get(base).ok_or("mcos: truncated property metadata")?;
                let ptype = *self.obj_props.get(base + 1).ok_or("mcos: truncated property metadata")?;
                let value_idx = *self.obj_props.get(base + 2).ok_or("mcos: truncated property metadata")?;
                let name = self.name(name_idx)?;
                let value = match ptype {
                    0 => Value::Str(self.name(value_idx)?),
                    1 => self.prop_vals.get(value_idx as usize).ok_or("mcos: property value out of range")?.to_value()?,
                    2 => Value::Num(value_idx as f64),
                    other => return Err(format!("mcos: unknown property type {other}")),
                };
                props.add_field(&name);
                props.set_field(0, &name, value).map_err(|e| e.message)?;
            }
        }
        let _ = &self.dyn_props;
        Ok(Value::Object(MObject::from_saved(class, props)))
    }

    fn resolve_opaque(&self, ty: &str, class_hint: &str, refs: &[u32]) -> Result<Value, String> {
        if ty != "MCOS" {
            return Err(format!("mcos: type system '{ty}' is not supported"));
        }
        let magic = *refs.first().ok_or("mcos: empty object reference")?;
        if magic != MCOS_MAGIC {
            return Err("mcos: bad object reference magic".into());
        }
        let ndims = *refs.get(1).ok_or("mcos: truncated object reference")? as usize;
        if ndims == 0 || ndims > 8 {
            return Err("mcos: bad object array rank".into());
        }
        let mut dims = Vec::with_capacity(ndims);
        for i in 0..ndims {
            dims.push(*refs.get(2 + i).ok_or("mcos: truncated object dimensions")? as usize);
        }
        let n = dims.iter().fold(1usize, |a, &d| a.saturating_mul(d));
        if n == 0 || n > 1_000_000 {
            return Err("mcos: bad object array size".into());
        }
        let base = 2 + ndims;
        let ids = refs.get(base..base + n).ok_or("mcos: truncated object ids")?;
        let class_id = *refs.get(base + n).ok_or("mcos: missing object class id")?;
        let class = if class_id != 0 {
            self.class_name(class_id)?
        } else if !class_hint.is_empty() {
            class_hint.to_string()
        } else {
            "mcOS".to_string()
        };
        let mut elements = Vec::with_capacity(n);
        for &id in ids {
            match self.build_object(id)? {
                Value::Object(o) => elements.push(o),
                _ => return Err("mcos: object metadata did not build an object".into()),
            }
        }
        if elements.len() == 1 {
            return Ok(Value::Object(elements.into_iter().next().ok_or("mcos: empty object")?));
        }
        let (rows, cols) = (dims.first().copied().unwrap_or(1), dims.get(1).copied().unwrap_or(1));
        Ok(Value::Object(MObject::array(class, elements, (rows, cols))))
    }

    /// Resolve a top-level `mxOPAQUE_CLASS` payload into an object value.
    pub(crate) fn read_variable(&self, payload: &[u8]) -> Result<(String, Value), String> {
        let (name, raw) = parse_matrix(payload)?;
        let Raw::Opaque { type_system, class, data } = raw else {
            return Err("mcos: variable is not an opaque object".into());
        };
        let Raw::U32(refs) = data.as_ref() else {
            return Err("mcos: object reference is not a uint32 matrix".into());
        };
        let value = self.resolve_opaque(&type_system, &class, refs)?;
        Ok((name, value))
    }
}

fn u32_at(b: &[u8], off: usize) -> Result<u32, String> {
    let s = b.get(off..off + 4).ok_or("mcos: truncated integer")?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

// ---------- encoding ----------

pub(crate) struct Encoded {
    pub records: Vec<(String, Vec<u8>)>,
    pub subsystem: Vec<u8>,
}

struct NameTable {
    names: Vec<String>,
}

impl NameTable {
    fn index(&mut self, name: &str) -> u32 {
        if let Some(i) = self.names.iter().position(|n| n == name) {
            return i as u32 + 1;
        }
        self.names.push(name.to_string());
        self.names.len() as u32
    }
}

fn uint32_matrix(name: &str, values: &[u32], rows: usize, cols: usize) -> Vec<u8> {
    let mut data = Vec::with_capacity(values.len() * 4);
    for v in values {
        data.extend_from_slice(&v.to_le_bytes());
    }
    matrix_element(name, MX_UINT32, 0, &[rows as u32, cols as u32], MI_UINT32, &data)
}

fn uint8_matrix(name: &str, bytes: &[u8]) -> Vec<u8> {
    matrix_element(name, MX_UINT8, 0, &[1, bytes.len() as u32], MI_UINT8, bytes)
}

fn empty_cell(name: &str) -> Vec<u8> {
    matrix_element(name, MX_CELL, 0, &[0, 0], MI_INT8, &[])
}

fn empty_struct(name: &str, rows: usize, cols: usize) -> Vec<u8> {
    let mut m = Vec::new();
    let mut af = Vec::with_capacity(8);
    af.extend_from_slice(&MX_STRUCT.to_le_bytes());
    af.extend_from_slice(&0u32.to_le_bytes());
    push_tag(&mut m, MI_UINT32, &af);
    let mut dv = Vec::with_capacity(8);
    dv.extend_from_slice(&(rows as u32).to_le_bytes());
    dv.extend_from_slice(&(cols as u32).to_le_bytes());
    push_tag(&mut m, MI_INT32, &dv);
    push_tag(&mut m, MI_INT8, name.as_bytes());
    push_tag(&mut m, MI_INT32, &0u32.to_le_bytes());
    push_tag(&mut m, MI_INT8, &[]);
    m
}

fn struct_with_field(name: &str, field: &str, value: &[u8]) -> Result<Vec<u8>, String> {
    let mut m = Vec::new();
    let mut af = Vec::with_capacity(8);
    af.extend_from_slice(&MX_STRUCT.to_le_bytes());
    af.extend_from_slice(&0u32.to_le_bytes());
    push_tag(&mut m, MI_UINT32, &af);
    let mut dv = Vec::with_capacity(8);
    dv.extend_from_slice(&1u32.to_le_bytes());
    dv.extend_from_slice(&1u32.to_le_bytes());
    push_tag(&mut m, MI_INT32, &dv);
    push_tag(&mut m, MI_INT8, name.as_bytes());
    let fnlen = (field.len() + 1) as u32;
    push_tag(&mut m, MI_INT32, &fnlen.to_le_bytes());
    let mut fnb = field.as_bytes().to_vec();
    fnb.resize(fnlen as usize, 0);
    push_tag(&mut m, MI_INT8, &fnb);
    push_tag(&mut m, MI_MATRIX, value);
    Ok(m)
}

fn opaque_matrix(name: &str, type_system: &str, class: &str, inner: &[u8]) -> Vec<u8> {
    let mut m = Vec::new();
    let mut af = Vec::with_capacity(8);
    af.extend_from_slice(&MX_OPAQUE.to_le_bytes());
    af.extend_from_slice(&0u32.to_le_bytes());
    push_tag(&mut m, MI_UINT32, &af);
    push_tag(&mut m, MI_INT8, name.as_bytes());
    push_tag(&mut m, MI_INT8, type_system.as_bytes());
    push_tag(&mut m, MI_INT8, class.as_bytes());
    push_tag(&mut m, MI_MATRIX, inner);
    m
}

fn cell_matrix(items: &[Vec<u8>], rows: usize, cols: usize) -> Vec<u8> {
    let mut m = Vec::new();
    let mut af = Vec::with_capacity(8);
    af.extend_from_slice(&MX_CELL.to_le_bytes());
    af.extend_from_slice(&0u32.to_le_bytes());
    push_tag(&mut m, MI_UINT32, &af);
    let mut dv = Vec::with_capacity(8);
    dv.extend_from_slice(&(rows as u32).to_le_bytes());
    dv.extend_from_slice(&(cols as u32).to_le_bytes());
    push_tag(&mut m, MI_INT32, &dv);
    push_tag(&mut m, MI_INT8, b"");
    for item in items {
        push_tag(&mut m, MI_MATRIX, item);
    }
    m
}

fn finish_element(payload: Vec<u8>) -> Vec<u8> {
    let mut inner = payload;
    inner.resize(inner.len() + (8 - inner.len() % 8) % 8, 0);
    let mut out = Vec::with_capacity(inner.len() + 8);
    push_tag(&mut out, MI_MATRIX, &inner);
    out
}

/// Encode scalar objects as MCOS opaque records plus the subsystem block.
pub(crate) fn encode(vars: &[(String, Value)]) -> Result<Option<Encoded>, String> {
    let objects: Vec<(&String, &MObject)> = vars
        .iter()
        .filter_map(|(n, v)| match v {
            Value::Object(o) if o.class_name != "COM" => Some((n, o)),
            _ => None,
        })
        .collect();
    if objects.is_empty() {
        return Ok(None);
    }
    let mut names = NameTable { names: Vec::new() };
    let mut classes: Vec<u32> = vec![0, 0, 0, 0];
    let mut class_ids: Vec<String> = Vec::new();
    let mut object_meta: Vec<u32> = vec![0; 6];
    let mut prop_meta: Vec<u32> = Vec::new();
    let mut prop_vals: Vec<Vec<u8>> = Vec::new();
    let mut records = Vec::with_capacity(objects.len());
    for (name, obj) in &objects {
        if obj.is_array() {
            return Err(format!("cannot save '{name}' as MCOS - object arrays are not supported yet"));
        }
        let class_name = obj.class_name.clone();
        let class_id = if let Some(pos) = class_ids.iter().position(|c| c == &class_name) {
            pos as u32 + 1
        } else {
            let cname = class_name.rsplit('.').next().unwrap_or(&class_name).to_string();
            let cidx = names.index(&cname);
            let ns = class_name.rsplit_once('.').map(|(ns, _)| names.index(ns)).unwrap_or(0);
            classes.extend_from_slice(&[ns, cidx, 0, 0]);
            class_ids.push(class_name.clone());
            class_ids.len() as u32
        };
        let oid = object_meta.len() / 6;
        object_meta.extend_from_slice(&[class_id, 0, 0, 0, oid as u32, 0]);
        let props = obj.props_map();
        let fields: Vec<String> = obj.prop_names();
        prop_meta.push(fields.len() as u32);
        for f in &fields {
            let idx = names.index(f);
            prop_meta.extend_from_slice(&[idx, 1, prop_vals.len() as u32]);
            let value = props.field_at(0, f).cloned().unwrap_or_else(|| Value::Mat(Matrix::zeros(0, 0)));
            if matches!(value, Value::Object(_)) {
                return Err(format!("cannot save '{name}' as MCOS - nested object property '{f}' is not supported yet"));
            }
            prop_vals.push(value_element("", &value)?);
        }
        if prop_meta.len() % 2 == 1 {
            prop_meta.push(0);
        }
        let refs = vec![MCOS_MAGIC, 2, 1, 1, oid as u32, class_id];
        let refs_mat = uint32_matrix("", &refs, refs.len(), 1);
        let rec = opaque_matrix(name, "MCOS", &class_name, &refs_mat);
        records.push(((*name).clone(), finish_element(rec)));
    }
    // Metadata regions.
    let names_str = {
        let mut s = String::new();
        for n in &names.names {
            s.push_str(n);
            s.push('\0');
        }
        s.into_bytes()
    };
    let names_padded = names_str.len().div_ceil(8) * 8;
    let mut names_bytes = names_str;
    names_bytes.resize(names_padded, 0);
    let classes_bytes: Vec<u8> = classes.iter().flat_map(|v| v.to_le_bytes()).collect();
    let objects_bytes: Vec<u8> = object_meta.iter().flat_map(|v| v.to_le_bytes()).collect();
    let props_bytes: Vec<u8> = prop_meta.iter().flat_map(|v| v.to_le_bytes()).collect();
    let dyn_props: Vec<u32> = vec![0, 0];
    let dyn_bytes: Vec<u8> = dyn_props.iter().flat_map(|v| v.to_le_bytes()).collect();
    let region6: Vec<u8> = Vec::new();
    let region7 = [0u8; 8];
    let mut offsets = [0u32; 8];
    let mut pos = 40usize + names_bytes.len();
    offsets[0] = pos as u32;
    pos += classes_bytes.len();
    offsets[1] = pos as u32;
    offsets[2] = pos as u32; // no saveobj block
    pos += objects_bytes.len();
    offsets[3] = pos as u32;
    pos += props_bytes.len();
    offsets[4] = pos as u32;
    pos += dyn_bytes.len();
    offsets[5] = pos as u32;
    pos += region6.len();
    offsets[6] = pos as u32;
    pos += region7.len();
    offsets[7] = pos as u32;
    let mut metadata = Vec::with_capacity(pos);
    metadata.extend_from_slice(&FWRAP_VERSION.to_le_bytes());
    metadata.extend_from_slice(&(names.names.len() as u32).to_le_bytes());
    for o in offsets {
        metadata.extend_from_slice(&o.to_le_bytes());
    }
    metadata.extend_from_slice(&names_bytes);
    metadata.extend_from_slice(&classes_bytes);
    metadata.extend_from_slice(&objects_bytes);
    metadata.extend_from_slice(&props_bytes);
    metadata.extend_from_slice(&dyn_bytes);
    metadata.extend_from_slice(&region6);
    metadata.extend_from_slice(&region7);
    let nclasses = classes.len() / 4;
    let mut cell_items = Vec::with_capacity(prop_vals.len() + 5);
    cell_items.push(uint8_matrix("", &metadata));
    cell_items.push(empty_cell(""));
    cell_items.extend(prop_vals);
    let c3: Vec<Vec<u8>> = (0..nclasses).map(|_| empty_struct("", 1, 0)).collect();
    cell_items.push(cell_matrix(&c3, nclasses, 1));
    cell_items.push(matrix_element("", MX_INT32, 0, &[1, 1], MI_INT32, &0i32.to_le_bytes()));
    let defaults: Vec<Vec<u8>> = (0..nclasses).map(|_| empty_struct("", 1, 0)).collect();
    cell_items.push(cell_matrix(&defaults, nclasses, 1));
    let fwrap_cell = cell_matrix(&cell_items, cell_items.len(), 1);
    let fwrap = opaque_matrix("", "MCOS", "FileWrapper__", &fwrap_cell);
    let root = struct_with_field("", "MCOS", &fwrap)?;
    let mut raw = vec![0u8, 1, b'M', b'I', 0, 0, 0, 0];
    raw.extend_from_slice(&finish_element(root));
    let outer = uint8_matrix("", &raw);
    Ok(Some(Encoded { records, subsystem: finish_element(outer) }))
}
