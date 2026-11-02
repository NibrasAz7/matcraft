//! Minimal NetCDF classic (CDF-1/CDF-2) reader and CDF-1 writer.
//!
//! Supports numeric variables, dimensions, global/variable attributes
//! (read and skipped), big-endian storage and non-record variables. NetCDF-4
//! files are HDF5 containers and are rejected here (use the HDF5 builtins).

use crate::LangError;

const TAG_DIMENSION: u32 = 10;
const TAG_VARIABLE: u32 = 11;
const TAG_ATTRIBUTE: u32 = 12;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum NcType {
    Byte,
    Char,
    Short,
    Int,
    Float,
    Double,
}

impl NcType {
    fn from_u32(v: u32) -> Option<Self> {
        match v {
            1 => Some(NcType::Byte),
            2 => Some(NcType::Char),
            3 => Some(NcType::Short),
            4 => Some(NcType::Int),
            5 => Some(NcType::Float),
            6 => Some(NcType::Double),
            _ => None,
        }
    }

    fn code(self) -> u32 {
        match self {
            NcType::Byte => 1,
            NcType::Char => 2,
            NcType::Short => 3,
            NcType::Int => 4,
            NcType::Float => 5,
            NcType::Double => 6,
        }
    }

    fn size(self) -> usize {
        match self {
            NcType::Byte | NcType::Char => 1,
            NcType::Short => 2,
            NcType::Int | NcType::Float => 4,
            NcType::Double => 8,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct NcVar {
    pub name: String,
    pub dim_names: Vec<String>,
    pub shape: Vec<usize>,
    pub kind: NcType,
    /// Row-major numeric payload.
    pub data: Vec<f64>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct NetCdf {
    pub dims: Vec<(String, usize)>,
    pub vars: Vec<NcVar>,
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
    version: u8,
}

impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self {
        Reader { b, pos: 0, version: 1 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], LangError> {
        let s = self.b.get(self.pos..self.pos.saturating_add(n)).ok_or_else(|| LangError::new("netcdf: truncated file", 0))?;
        self.pos = self.pos.saturating_add(n);
        Ok(s)
    }

    fn u32(&mut self) -> Result<u32, LangError> {
        let s = self.take(4)?;
        Ok(u32::from_be_bytes([*s.first().unwrap_or(&0), *s.get(1).unwrap_or(&0), *s.get(2).unwrap_or(&0), *s.get(3).unwrap_or(&0)]))
    }

    fn u64(&mut self) -> Result<u64, LangError> {
        let s = self.take(8)?;
        let mut a = [0u8; 8];
        for (i, slot) in a.iter_mut().enumerate() {
            *slot = *s.get(i).unwrap_or(&0);
        }
        Ok(u64::from_be_bytes(a))
    }

    fn size(&mut self) -> Result<usize, LangError> {
        if self.version == 2 {
            Ok(self.u64()? as usize)
        } else {
            Ok(self.u32()? as usize)
        }
    }

    fn name(&mut self) -> Result<String, LangError> {
        let n = self.u32()? as usize;
        if n > 1_000_000 {
            return Err(LangError::new("netcdf: name too long", 0));
        }
        let bytes = self.take(n)?;
        let pad = (4 - n % 4) % 4;
        let _ = self.take(pad)?;
        Ok(String::from_utf8_lossy(bytes).to_string())
    }

    fn skip(&mut self, n: usize) -> Result<(), LangError> {
        let _ = self.take(n)?;
        Ok(())
    }
}

fn type_size(t: NcType) -> usize {
    t.size()
}

fn skip_absent_count(r: &mut Reader<'_>) {
    if r.b.get(r.pos..r.pos.saturating_add(4)) == Some(&[0, 0, 0, 0]) {
        r.pos = r.pos.saturating_add(4);
    }
}

fn skip_attrs(r: &mut Reader<'_>) -> Result<(), LangError> {
    let tag = r.u32()?;
    if tag == 0 {
        skip_absent_count(r);
        return Ok(());
    }
    if tag != TAG_ATTRIBUTE {
        return Err(LangError::new("netcdf: expected attribute list", 0));
    }
    let n = r.u32()? as usize;
    for _ in 0..n {
        let _ = r.name()?;
        let kind = NcType::from_u32(r.u32()?).ok_or_else(|| LangError::new("netcdf: unknown attribute type", 0))?;
        let count = r.u32()? as usize;
        let bytes = count.saturating_mul(type_size(kind));
        r.skip(bytes)?;
        r.skip((4 - bytes % 4) % 4)?;
    }
    Ok(())
}

pub(crate) fn read(path: &std::path::Path) -> Result<NetCdf, LangError> {
    let bytes = std::fs::read(path).map_err(|e| LangError::new(format!("netcdf: cannot read {}: {e}", path.display()), 0))?;
    read_bytes(&bytes)
}

pub(crate) fn read_bytes(bytes: &[u8]) -> Result<NetCdf, LangError> {
    if bytes.get(0..3) != Some(b"CDF") {
        return Err(LangError::new("netcdf: not a classic NetCDF file (NetCDF-4/HDF5 is not supported here)", 0));
    }
    let version = *bytes.get(3).ok_or_else(|| LangError::new("netcdf: truncated header", 0))?;
    if version != 1 && version != 2 {
        return Err(LangError::new("netcdf: unsupported CDF version", 0));
    }
    let mut r = Reader::new(bytes);
    r.version = version;
    r.pos = 4;
    let _numrecs = if version == 2 { r.u64()? } else { r.u32()? as u64 };
    let mut dims: Vec<(String, usize)> = Vec::new();
    let tag = r.u32()?;
    if tag == TAG_DIMENSION {
        let n = r.u32()? as usize;
        for _ in 0..n {
            let name = r.name()?;
            let size = r.size()?;
            dims.push((name, size));
        }
    } else if tag != 0 {
        return Err(LangError::new("netcdf: bad dimension list", 0));
    } else {
        skip_absent_count(&mut r);
    }
    skip_attrs(&mut r)?;
    let tag = r.u32()?;
    if tag == 0 {
        skip_absent_count(&mut r);
        return Ok(NetCdf { dims, vars: Vec::new() });
    }
    if tag != TAG_VARIABLE {
        return Err(LangError::new("netcdf: bad variable list", 0));
    }
    let nvars = r.u32()? as usize;
    let mut raw_vars = Vec::with_capacity(nvars);
    for _ in 0..nvars {
        let name = r.name()?;
        let nd = r.u32()? as usize;
        let mut ids = Vec::with_capacity(nd);
        for _ in 0..nd {
            ids.push(r.u32()? as usize);
        }
        skip_attrs(&mut r)?;
        let kind = NcType::from_u32(r.u32()?).ok_or_else(|| LangError::new("netcdf: unknown variable type", 0))?;
        let _vsize = r.u32()?;
        let begin = if version == 2 { r.u64()? as usize } else { r.u32()? as usize };
        raw_vars.push((name, ids, kind, begin));
    }
    let mut out = NetCdf { dims, vars: Vec::new() };
    for (name, ids, kind, begin) in raw_vars {
        let mut shape = Vec::with_capacity(ids.len());
        let mut dim_names = Vec::with_capacity(ids.len());
        for id in &ids {
            let (dn, ds) = out.dims.get(*id).ok_or_else(|| LangError::new("netcdf: variable references a missing dimension", 0))?;
            if *ds == 0 {
                return Err(LangError::new("netcdf: record variables are not supported yet", 0));
            }
            dim_names.push(dn.clone());
            shape.push(*ds);
        }
        let count = shape.iter().fold(1usize, |a, &b| a.saturating_mul(b));
        let bytes_needed = count.saturating_mul(kind.size());
        let raw = bytes.get(begin..begin.saturating_add(bytes_needed)).ok_or_else(|| LangError::new("netcdf: truncated variable data", 0))?;
        let mut data = Vec::with_capacity(count);
        for i in 0..count {
            let off = i.saturating_mul(kind.size());
            let v = match kind {
                NcType::Byte | NcType::Char => raw.get(off).copied().unwrap_or(0) as i8 as f64,
                NcType::Short => {
                    let a = raw.get(off).copied().unwrap_or(0);
                    let b = raw.get(off + 1).copied().unwrap_or(0);
                    i16::from_be_bytes([a, b]) as f64
                }
                NcType::Int | NcType::Float => {
                    let a = [
                        raw.get(off).copied().unwrap_or(0),
                        raw.get(off + 1).copied().unwrap_or(0),
                        raw.get(off + 2).copied().unwrap_or(0),
                        raw.get(off + 3).copied().unwrap_or(0),
                    ];
                    if kind == NcType::Float {
                        f32::from_be_bytes(a) as f64
                    } else {
                        i32::from_be_bytes(a) as f64
                    }
                }
                NcType::Double => {
                    let mut a = [0u8; 8];
                    for (j, slot) in a.iter_mut().enumerate() {
                        *slot = raw.get(off + j).copied().unwrap_or(0);
                    }
                    f64::from_be_bytes(a)
                }
            };
            data.push(v);
        }
        out.vars.push(NcVar { name, dim_names, shape, kind, data });
    }
    Ok(out)
}

pub(crate) fn write(path: &std::path::Path, nc: &NetCdf) -> Result<(), LangError> {
    let bytes = write_bytes(nc)?;
    crate::write_file(path, &bytes).map_err(|e| LangError::new(format!("netcdf: cannot write {}: {e}", path.display()), 0))
}

fn name_bytes(name: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let b = name.as_bytes();
    out.extend_from_slice(&(b.len() as u32).to_be_bytes());
    out.extend_from_slice(b);
    let pad = (4 - b.len() % 4) % 4;
    out.extend(std::iter::repeat_n(0u8, pad));
    out
}

fn build_header(nc: &NetCdf, begins: &[usize]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"CDF\x01");
    out.extend_from_slice(&0u32.to_be_bytes()); // numrecs
    out.extend_from_slice(&TAG_DIMENSION.to_be_bytes());
    out.extend_from_slice(&(nc.dims.len() as u32).to_be_bytes());
    for (name, size) in &nc.dims {
        out.extend_from_slice(&name_bytes(name));
        out.extend_from_slice(&(*size as u32).to_be_bytes());
    }
    out.extend_from_slice(&TAG_ATTRIBUTE.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&TAG_VARIABLE.to_be_bytes());
    out.extend_from_slice(&(nc.vars.len() as u32).to_be_bytes());
    for (i, var) in nc.vars.iter().enumerate() {
        out.extend_from_slice(&name_bytes(&var.name));
        out.extend_from_slice(&(var.shape.len() as u32).to_be_bytes());
        for d in &var.dim_names {
            if let Some(id) = nc.dims.iter().position(|(n, _)| n == d) {
                out.extend_from_slice(&(id as u32).to_be_bytes());
            }
        }
        out.extend_from_slice(&TAG_ATTRIBUTE.to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&var.kind.code().to_be_bytes());
        let bytes = var.data.len().saturating_mul(var.kind.size());
        let padded = bytes.saturating_add((4 - bytes % 4) % 4);
        out.extend_from_slice(&(padded as u32).to_be_bytes());
        out.extend_from_slice(&(begins.get(i).copied().unwrap_or(0) as u32).to_be_bytes());
    }
    out
}

pub(crate) fn write_bytes(nc: &NetCdf) -> Result<Vec<u8>, LangError> {
    let mut probe = vec![0usize; nc.vars.len()];
    let header0 = build_header(nc, &probe);
    let mut begins = Vec::with_capacity(nc.vars.len());
    let mut pos = header0.len().saturating_add((4 - header0.len() % 4) % 4);
    for var in &nc.vars {
        begins.push(pos);
        let bytes = var.data.len().saturating_mul(var.kind.size());
        pos = pos.saturating_add(bytes).saturating_add((4 - bytes % 4) % 4);
    }
    if pos > u32::MAX as usize {
        return Err(LangError::new("netcdf: CDF-1 output exceeds 4 GB", 0));
    }
    probe = begins;
    let header = build_header(nc, &probe);
    let mut out = header;
    let pad = (4 - out.len() % 4) % 4;
    out.extend(std::iter::repeat_n(0u8, pad));
    for var in &nc.vars {
        let start = out.len();
        for &v in &var.data {
            match var.kind {
                NcType::Double => out.extend_from_slice(&v.to_be_bytes()),
                NcType::Float => out.extend_from_slice(&(v as f32).to_be_bytes()),
                NcType::Int => out.extend_from_slice(&(v as i32).to_be_bytes()),
                NcType::Short => out.extend_from_slice(&(v as i16).to_be_bytes()),
                NcType::Byte | NcType::Char => out.push(v as i8 as u8),
            }
        }
        let bytes = out.len().saturating_sub(start);
        let pad = (4 - bytes % 4) % 4;
        out.extend(std::iter::repeat_n(0u8, pad));
    }
    Ok(out)
}
