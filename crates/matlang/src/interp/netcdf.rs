//! MATLAB-facing NetCDF classic builtins: `ncread`, `ncwrite`, `ncinfo`,
//! `ncdisp`.

use std::path::PathBuf;

use super::*;
use crate::netcdf::{NcType, NcVar, NetCdf};

fn path_arg(v: &Value) -> Result<PathBuf, LangError> {
    match v {
        Value::Str(s) => Ok(PathBuf::from(s)),
        _ => Err(LangError::new("netcdf file name must be a string", 0)),
    }
}

fn var_name(v: &Value) -> Result<String, LangError> {
    match v {
        Value::Str(s) => Ok(s.clone()),
        _ => Err(LangError::new("netcdf variable name must be a string", 0)),
    }
}

struct Sel {
    start: Vec<usize>,
    count: Vec<usize>,
    stride: Vec<usize>,
}

fn selection(argv: &[Value], at: usize, shape: &[usize]) -> Result<Option<Sel>, LangError> {
    if argv.len() <= at {
        return Ok(None);
    }
    let parse = |v: Option<&Value>, default: usize| -> Result<Vec<usize>, LangError> {
        match v {
            None => Ok(vec![default.max(1); shape.len()]),
            Some(x) => {
                let m = x.to_numeric()?;
                if m.numel() == 1 {
                    Ok(vec![m.data().first().copied().unwrap_or(default as f64).max(1.0) as usize; shape.len()])
                } else if m.numel() == shape.len() {
                    Ok(m.data().iter().map(|v| v.max(1.0) as usize).collect())
                } else {
                    Err(LangError::new("start/count/stride must be scalar or rank long", 0))
                }
            }
        }
    };
    let start = parse(argv.get(at), 1)?;
    let count = match argv.get(at + 1) {
        Some(v) => {
            let m = v.to_numeric()?;
            if m.numel() == 1 {
                vec![m.data().first().copied().unwrap_or(1.0).max(1.0) as usize; shape.len()]
            } else if m.numel() == shape.len() {
                m.data().iter().map(|v| v.max(1.0) as usize).collect()
            } else {
                return Err(LangError::new("count must be scalar or rank long", 0));
            }
        }
        None => (0..shape.len())
            .map(|i| shape.get(i).copied().unwrap_or(0).saturating_sub(start.get(i).copied().unwrap_or(1)).saturating_add(1))
            .collect(),
    };
    let stride = parse(argv.get(at + 2), 1)?;
    for i in 0..shape.len() {
        let s = start.get(i).copied().unwrap_or(1).saturating_sub(1);
        let c = count.get(i).copied().unwrap_or(1);
        let st = stride.get(i).copied().unwrap_or(1);
        let last = s.saturating_add(c.saturating_sub(1).saturating_mul(st));
        if last >= shape.get(i).copied().unwrap_or(0) {
            return Err(LangError::new("netcdf: subset exceeds the variable dimensions", 0));
        }
    }
    Ok(Some(Sel { start, count, stride }))
}

fn read_matrix(var: &NcVar, sel: &Option<Sel>) -> Result<Matrix, LangError> {
    let shape = &var.shape;
    if shape.len() > 2 {
        return Err(LangError::new("netcdf: only 1-D and 2-D variables are supported", 0));
    }
    let (start, count, stride) = match sel {
        Some(s) => (s.start.clone(), s.count.clone(), s.stride.clone()),
        None => (vec![1; shape.len()], shape.clone(), vec![1; shape.len()]),
    };
    let rows = count.first().copied().unwrap_or(1);
    if shape.len() <= 1 {
        let mut out = Vec::with_capacity(rows);
        for r in 0..rows {
            let idx = start.first().copied().unwrap_or(1).saturating_sub(1).saturating_add(r.saturating_mul(stride.first().copied().unwrap_or(1)));
            out.push(var.data.get(idx).copied().unwrap_or(f64::NAN));
        }
        return Ok(Matrix::col_vector(out));
    }
    let cols = count.get(1).copied().unwrap_or(1);
    check_elems(rows.saturating_mul(cols))?;
    let mut out = Matrix::zeros(rows, cols);
    for r in 0..rows {
        for c in 0..cols {
            let ri = start.first().copied().unwrap_or(1).saturating_sub(1).saturating_add(r.saturating_mul(stride.first().copied().unwrap_or(1)));
            let ci = start.get(1).copied().unwrap_or(1).saturating_sub(1).saturating_add(c.saturating_mul(stride.get(1).copied().unwrap_or(1)));
            let src = ri.saturating_mul(shape.get(1).copied().unwrap_or(0)).saturating_add(ci);
            let _ = out.set(r, c, var.data.get(src).copied().unwrap_or(f64::NAN));
        }
    }
    Ok(out)
}

fn var_from_matrix(name: String, m: &Matrix) -> NcVar {
    let dim_names = vec!["dim1".to_string(), "dim2".to_string()];
    let shape = vec![m.rows(), m.cols()];
    let mut data = Vec::with_capacity(m.numel());
    for r in 0..m.rows() {
        for c in 0..m.cols() {
            data.push(m.get(r, c).unwrap_or(0.0));
        }
    }
    NcVar { name, dim_names, shape, kind: NcType::Double, data }
}

/// Give a new variable dimension names that exist in `nc`, reusing a
/// same-length dimension when one is available and creating `dimN`
/// otherwise. Keeps classic headers valid after importing a NetCDF-4 model
/// with real dimension names.
fn ensure_dimension_names(var: &mut NcVar, nc: &mut NetCdf) {
    for axis in 0..var.shape.len() {
        let size = var.shape.get(axis).copied().unwrap_or(1);
        let name = if let Some((name, _)) = nc.dims.iter().find(|(_, s)| *s == size) {
            name.clone()
        } else {
            let mut name = format!("dim{}", axis + 1);
            let mut suffix = 1usize;
            while nc.dims.iter().any(|(n, _)| n == &name) {
                name = format!("dim{}_{suffix}", axis + 1);
                suffix = suffix.saturating_add(1);
            }
            nc.dims.push((name.clone(), size));
            name
        };
        if let Some(slot) = var.dim_names.get_mut(axis) {
            *slot = name;
        }
    }
    var.dim_names.truncate(var.shape.len());
}

fn type_name(kind: NcType) -> &'static str {
    match kind {
        NcType::Byte => "int8",
        NcType::Char => "char",
        NcType::Short => "int16",
        NcType::Int => "int32",
        NcType::Float => "single",
        NcType::Double => "double",
    }
}

impl Session {
    pub(super) fn netcdf_builtin(&mut self, name: &str, argv: Vec<Value>, _nout: usize) -> Result<Vec<Value>, LangError> {
        match name {
            "ncread" => {
                let path = path_arg(argv.first().ok_or_else(|| LangError::new("ncread needs a file", 0))?)?;
                let vname = var_name(argv.get(1).ok_or_else(|| LangError::new("ncread needs a variable", 0))?)?;
                let nc = crate::netcdf::read(&path)?;
                let var =
                    nc.vars.iter().find(|v| v.name == vname).ok_or_else(|| LangError::new(format!("ncread: variable '{vname}' not found"), 0))?;
                let sel = selection(&argv, 2, &var.shape)?;
                Ok(vec![mat_or_num(read_matrix(var, &sel)?)])
            }
            "ncwrite" => {
                let path = path_arg(argv.first().ok_or_else(|| LangError::new("ncwrite needs a file", 0))?)?;
                let vname = var_name(argv.get(1).ok_or_else(|| LangError::new("ncwrite needs a variable", 0))?)?;
                let data = argv.get(2).ok_or_else(|| LangError::new("ncwrite needs data", 0))?.to_numeric()?;
                let mut nc = if path.exists() { crate::netcdf::read(&path)? } else { NetCdf::default() };
                let new_var = var_from_matrix(vname.clone(), &data);
                if let Some(pos) = nc.vars.iter().position(|v| v.name == vname) {
                    if argv.len() > 3 {
                        let shape = nc.vars[pos].shape.clone();
                        let sel = selection(&argv, 3, &shape)?.ok_or_else(|| LangError::new("ncwrite: bad selection", 0))?;
                        let rows = sel.count.first().copied().unwrap_or(1);
                        let cols = if shape.len() > 1 { sel.count.get(1).copied().unwrap_or(1) } else { 1 };
                        if data.numel() != rows.saturating_mul(cols) {
                            return Err(LangError::new("ncwrite: data size does not match count", 0));
                        }
                        for r in 0..rows {
                            for c in 0..cols {
                                let ri = sel
                                    .start
                                    .first()
                                    .copied()
                                    .unwrap_or(1)
                                    .saturating_sub(1)
                                    .saturating_add(r.saturating_mul(sel.stride.first().copied().unwrap_or(1)));
                                let src = if shape.len() > 1 {
                                    let ci = sel
                                        .start
                                        .get(1)
                                        .copied()
                                        .unwrap_or(1)
                                        .saturating_sub(1)
                                        .saturating_add(c.saturating_mul(sel.stride.get(1).copied().unwrap_or(1)));
                                    ri.saturating_mul(shape.get(1).copied().unwrap_or(0)).saturating_add(ci)
                                } else {
                                    ri
                                };
                                if let Some(slot) = nc.vars[pos].data.get_mut(src) {
                                    *slot = data.get(r, c).unwrap_or(0.0);
                                }
                            }
                        }
                    } else {
                        let shape = nc.vars[pos].shape.clone();
                        let dim_names = nc.vars[pos].dim_names.clone();
                        let expected = shape.iter().fold(1usize, |a, &d| a.saturating_mul(d));
                        if data.numel() != expected {
                            return Err(LangError::new("ncwrite: replacement data size does not match the variable", 0));
                        }
                        let mut nv = new_var;
                        nv.shape = shape;
                        nv.dim_names = dim_names;
                        nc.vars[pos] = nv;
                    }
                } else {
                    let mut nv = new_var;
                    ensure_dimension_names(&mut nv, &mut nc);
                    nc.vars.push(nv);
                }
                crate::netcdf::write(&path, &nc)?;
                Ok(vec![Value::Num(0.0)])
            }
            "ncinfo" => {
                let path = path_arg(argv.first().ok_or_else(|| LangError::new("ncinfo needs a file", 0))?)?;
                let nc = crate::netcdf::read(&path)?;
                Ok(vec![ncinfo_value(&path, &nc)?])
            }
            "ncdisp" => {
                let path = path_arg(argv.first().ok_or_else(|| LangError::new("ncdisp needs a file", 0))?)?;
                let nc = crate::netcdf::read(&path)?;
                self.print_buf.push_str(&format!("Source: {}\n", path.display()));
                self.print_buf.push_str("Dimensions:\n");
                for (name, size) in &nc.dims {
                    self.print_buf.push_str(&format!("    {} = {}\n", name, size));
                }
                self.print_buf.push_str("Variables:\n");
                for v in &nc.vars {
                    let shape = v.shape.iter().map(usize::to_string).collect::<Vec<_>>().join("x");
                    self.print_buf.push_str(&format!("    {} ({})\n", v.name, shape));
                }
                Ok(vec![])
            }
            _ => Err(LangError::new(format!("{name}: not a NetCDF builtin"), 0)),
        }
    }
}

fn ncinfo_value(path: &std::path::Path, nc: &NetCdf) -> Result<Value, LangError> {
    let mut dims = StructArray::with_dims(1, nc.dims.len());
    for (i, (name, size)) in nc.dims.iter().enumerate() {
        dims.set_field(i, "Name", Value::Str(name.clone()))?;
        dims.set_field(i, "Length", Value::Num(*size as f64))?;
        dims.set_field(i, "Unlimited", Value::Num(0.0))?;
    }
    let mut vars = StructArray::with_dims(1, nc.vars.len());
    for (i, var) in nc.vars.iter().enumerate() {
        vars.set_field(i, "Name", Value::Str(var.name.clone()))?;
        vars.set_field(i, "Dimensions", Value::Cell(CellArray::row(var.dim_names.iter().cloned().map(Value::Str).collect())))?;
        vars.set_field(i, "Size", Value::Mat(Matrix::row_vector(var.shape.iter().map(|v| *v as f64).collect())))?;
        vars.set_field(i, "Datatype", Value::Str(type_name(var.kind).to_string()))?;
        vars.set_field(i, "Attributes", Value::Struct(StructArray::new()))?;
    }
    let mut info = StructArray::new();
    info.set_field(0, "Filename", Value::Str(path.to_string_lossy().to_string()))?;
    info.set_field(0, "Name", Value::Str("/".into()))?;
    info.set_field(0, "Dimensions", Value::Struct(dims))?;
    info.set_field(0, "Variables", Value::Struct(vars))?;
    info.set_field(0, "Attributes", Value::Struct(StructArray::new()))?;
    Ok(Value::Struct(info))
}
