//! MATLAB-facing HDF5 builtins: `h5read`, `h5write`, `h5create`, `h5disp`.

use std::path::PathBuf;

use super::*;
use crate::hdf5::{H5Dataset, H5File};

fn path_arg(v: &Value, what: &str) -> Result<PathBuf, LangError> {
    match v {
        Value::Str(s) => Ok(PathBuf::from(s)),
        _ => Err(LangError::new(format!("{what} must be a file name string"), 0)),
    }
}

fn dataset_name(v: &Value) -> Result<String, LangError> {
    match v {
        Value::Str(s) => Ok(s.trim_start_matches('/').to_string()),
        _ => Err(LangError::new("dataset name must be a string", 0)),
    }
}

/// MatCraft extension: `h5create(..., 'SuperblockVersion', 2|3)` chooses
/// the superblock format for new files (v2 is the default).
fn superblock_version_option(argv: &[Value], at: usize) -> Result<Option<u8>, LangError> {
    let mut i = at;
    while i + 1 < argv.len() {
        if let Value::Str(key) = &argv[i] {
            if key.eq_ignore_ascii_case("SuperblockVersion") {
                let m = argv[i + 1].to_numeric()?;
                let v = m.as_scalar().ok_or_else(|| LangError::new("SuperblockVersion must be scalar", 0))?.round();
                if v != 2.0 && v != 3.0 {
                    return Err(LangError::new("SuperblockVersion must be 2 or 3", 0));
                }
                return Ok(Some(v as u8));
            }
        }
        i += 1;
    }
    Ok(None)
}

struct Selection {
    start: Vec<usize>,
    count: Vec<usize>,
    stride: Vec<usize>,
}

fn selection(argv: &[Value], at: usize, shape: &[usize]) -> Result<Option<Selection>, LangError> {
    if argv.len() <= at {
        return Ok(None);
    }
    let parse = |v: Option<&Value>, default: f64| -> Result<Vec<usize>, LangError> {
        match v {
            None => Ok(vec![default.max(1.0) as usize; shape.len()]),
            Some(x) => {
                let m = x.to_numeric()?;
                if m.numel() == 1 {
                    Ok(vec![m.data().first().copied().unwrap_or(default).max(1.0) as usize; shape.len()])
                } else if m.numel() == shape.len() {
                    Ok(m.data().iter().map(|v| v.max(1.0) as usize).collect())
                } else {
                    Err(LangError::new("start/count/stride must be scalar or rank long", 0))
                }
            }
        }
    };
    let start = parse(argv.get(at), 1.0)?;
    let mut count = match argv.get(at + 1) {
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
    let stride = parse(argv.get(at + 2), 1.0)?;
    for i in 0..shape.len() {
        let s = start.get(i).copied().unwrap_or(1).saturating_sub(1);
        let c = count.get(i).copied().unwrap_or(1);
        let st = stride.get(i).copied().unwrap_or(1);
        let last = s.saturating_add(c.saturating_sub(1).saturating_mul(st));
        if last >= shape.get(i).copied().unwrap_or(0) {
            return Err(LangError::new("hdf5: subset exceeds the dataset dimensions", 0));
        }
    }
    if count.is_empty() {
        count = vec![1];
    }
    Ok(Some(Selection { start, count, stride }))
}

fn read_matrix(ds: &H5Dataset, sel: &Option<Selection>) -> Result<Matrix, LangError> {
    let shape = &ds.dims;
    if shape.is_empty() || shape.len() > 2 {
        return Err(LangError::new("hdf5: only 1-D and 2-D numeric datasets are supported", 0));
    }
    let (start, count, stride) = match sel {
        Some(s) => (s.start.clone(), s.count.clone(), s.stride.clone()),
        None => (vec![1; shape.len()], shape.clone(), vec![1; shape.len()]),
    };
    let rows = count.first().copied().unwrap_or(1);
    if shape.len() == 1 {
        let mut out = Vec::with_capacity(rows);
        for r in 0..rows {
            let idx = start.first().copied().unwrap_or(1).saturating_sub(1).saturating_add(r.saturating_mul(stride.first().copied().unwrap_or(1)));
            out.push(ds.data.get(idx).copied().unwrap_or(f64::NAN));
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
            let _ = out.set(r, c, ds.data.get(src).copied().unwrap_or(f64::NAN));
        }
    }
    Ok(out)
}

fn matrix_to_dataset(name: String, m: &Matrix) -> H5Dataset {
    let dims = if m.cols() == 1 { vec![m.rows()] } else { vec![m.rows(), m.cols()] };
    let mut data = Vec::with_capacity(m.numel());
    if dims.len() == 1 {
        for r in 0..m.rows() {
            data.push(m.get(r, 0).unwrap_or(0.0));
        }
    } else {
        for r in 0..m.rows() {
            for c in 0..m.cols() {
                data.push(m.get(r, c).unwrap_or(0.0));
            }
        }
    }
    H5Dataset { name, dims, data }
}

fn shape_arg(v: &Value) -> Result<Vec<usize>, LangError> {
    let m = v.to_numeric()?;
    if m.numel() == 0 {
        return Err(LangError::new("h5create needs a size", 0));
    }
    let dims: Vec<usize> = m.data().iter().map(|v| v.max(0.0) as usize).collect();
    if dims.is_empty() || dims.len() > 2 {
        return Err(LangError::new("h5create supports 1-D and 2-D sizes", 0));
    }
    Ok(dims)
}

impl Session {
    pub(super) fn hdf5_builtin(&mut self, name: &str, argv: Vec<Value>, _nout: usize) -> Result<Vec<Value>, LangError> {
        match name {
            "h5read" => {
                let path = path_arg(argv.first().ok_or_else(|| LangError::new("h5read needs a file", 0))?, "file")?;
                let ds_name = dataset_name(argv.get(1).ok_or_else(|| LangError::new("h5read needs a dataset", 0))?)?;
                let file = crate::hdf5::read(&path)?;
                let ds = file
                    .datasets
                    .iter()
                    .find(|d| d.name == ds_name)
                    .ok_or_else(|| LangError::new(format!("h5read: dataset '{ds_name}' not found"), 0))?;
                let sel = selection(&argv, 2, &ds.dims)?;
                Ok(vec![mat_or_num(read_matrix(ds, &sel)?)])
            }
            "h5write" => {
                let path = path_arg(argv.first().ok_or_else(|| LangError::new("h5write needs a file", 0))?, "file")?;
                let ds_name = dataset_name(argv.get(1).ok_or_else(|| LangError::new("h5write needs a dataset", 0))?)?;
                let data = argv.get(2).ok_or_else(|| LangError::new("h5write needs data", 0))?.to_numeric()?;
                let mut file = if path.exists() { crate::hdf5::read(&path)? } else { H5File::default() };
                let new_ds = matrix_to_dataset(ds_name.clone(), &data);
                if let Some(pos) = file.datasets.iter().position(|d| d.name == ds_name) {
                    let shape = file.datasets[pos].dims.clone();
                    if argv.len() > 3 {
                        let sel = selection(&argv, 3, &shape)?.ok_or_else(|| LangError::new("h5write: bad selection", 0))?;
                        let rows = sel.count.first().copied().unwrap_or(1);
                        let cols = if shape.len() > 1 { sel.count.get(1).copied().unwrap_or(1) } else { 1 };
                        if data.numel() != rows.saturating_mul(cols) {
                            return Err(LangError::new("h5write: data size does not match count", 0));
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
                                if let Some(slot) = file.datasets[pos].data.get_mut(src) {
                                    *slot = data.get(r, c).unwrap_or(0.0);
                                }
                            }
                        }
                    } else {
                        file.datasets[pos] = new_ds;
                    }
                } else {
                    file.datasets.push(new_ds);
                }
                crate::hdf5::write(&path, &file)?;
                Ok(vec![])
            }
            "h5create" => {
                let path = path_arg(argv.first().ok_or_else(|| LangError::new("h5create needs a file", 0))?, "file")?;
                let ds_name = dataset_name(argv.get(1).ok_or_else(|| LangError::new("h5create needs a dataset", 0))?)?;
                let dims = shape_arg(argv.get(2).ok_or_else(|| LangError::new("h5create needs a size", 0))?)?;
                let mut file = if path.exists() { crate::hdf5::read(&path)? } else { H5File::default() };
                if file.datasets.iter().any(|d| d.name == ds_name) {
                    return Err(LangError::new("h5create: dataset already exists", 0));
                }
                let count = dims.iter().fold(1usize, |a, &b| a.saturating_mul(b));
                file.datasets.push(H5Dataset { name: ds_name, dims, data: vec![0.0; count] });
                if let Some(v) = superblock_version_option(&argv, 3)? {
                    file.superblock_version = v;
                }
                crate::hdf5::write(&path, &file)?;
                Ok(vec![])
            }
            "h5disp" => {
                let path = path_arg(argv.first().ok_or_else(|| LangError::new("h5disp needs a file", 0))?, "file")?;
                let file = crate::hdf5::read(&path)?;
                if let Some(v) = argv.get(1) {
                    let name = dataset_name(v)?;
                    for ds in &file.datasets {
                        if ds.name == name {
                            self.print_buf.push_str(&format!(
                                "/{}\n    Size: {}\n    Datatype: double\n",
                                ds.name,
                                ds.dims.iter().map(usize::to_string).collect::<Vec<_>>().join("x")
                            ));
                        }
                    }
                } else {
                    for ds in &file.datasets {
                        self.print_buf.push_str(&format!(
                            "/{}\n    Size: {}\n    Datatype: double\n",
                            ds.name,
                            ds.dims.iter().map(usize::to_string).collect::<Vec<_>>().join("x")
                        ));
                    }
                }
                Ok(vec![])
            }
            _ => Err(LangError::new(format!("{name}: not an HDF5 builtin"), 0)),
        }
    }
}
