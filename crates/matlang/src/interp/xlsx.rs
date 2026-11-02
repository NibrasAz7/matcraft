//! MATLAB-facing `.xlsx` builtins: `xlsread`, `xlswrite`, `readcell`,
//! `writecell`, `readtable`, `writetable`, plus `height`/`width`/`istable`.

use std::collections::HashSet;
use std::path::PathBuf;

use super::*;
use crate::xlsx::{self, Cell, Grid, Sheet, Workbook};

#[derive(Clone)]
enum SheetSel {
    Name(String),
    Index(usize),
}

struct Range {
    r1: usize,
    c1: usize,
    r2: usize,
    c2: usize,
}

fn path_arg(v: &Value, what: &str) -> Result<PathBuf, LangError> {
    match v {
        Value::Str(s) => Ok(PathBuf::from(s)),
        _ => Err(LangError::new(format!("{what} must be a file name string"), 0)),
    }
}

fn sheet_sel(v: Option<&Value>) -> Result<Option<SheetSel>, LangError> {
    match v {
        None => Ok(None),
        Some(Value::Str(s)) => Ok(Some(SheetSel::Name(s.clone()))),
        Some(x) => {
            let n = x.scalar()?;
            if n < 1.0 || n.fract() != 0.0 {
                return Err(LangError::new("sheet index must be a positive integer", 0));
            }
            Ok(Some(SheetSel::Index(n as usize)))
        }
    }
}

fn parse_range(s: &str) -> Option<Range> {
    let s = s.trim();
    let (a, b) = match s.find(':') {
        Some(pos) => (s.get(..pos)?, s.get(pos.saturating_add(1)..)?),
        None => (s, s),
    };
    let (r1, c1) = xlsx::parse_ref(a)?;
    let (r2, c2) = xlsx::parse_ref(b)?;
    // A real worksheet name such as "Sheet1" is also letter+digits; reject
    // references outside Excel's grid so names are not mistaken for ranges.
    if r1 > 1_048_576 || r2 > 1_048_576 || c1 > 16_384 || c2 > 16_384 {
        return None;
    }
    Some(Range { r1: r1.min(r2), c1: c1.min(c2), r2: r1.max(r2), c2: c1.max(c2) })
}

fn range_arg(v: Option<&Value>) -> Result<Option<Range>, LangError> {
    match v {
        None => Ok(None),
        Some(Value::Str(s)) => {
            let r = parse_range(s).ok_or_else(|| LangError::new(format!("invalid cell range '{s}'"), 0))?;
            Ok(Some(r))
        }
        Some(_) => Err(LangError::new("range must be a string such as 'A1:C3'", 0)),
    }
}

fn select_sheet(wb: &Workbook, sel: &Option<SheetSel>) -> Result<usize, LangError> {
    match sel {
        None => Ok(0),
        Some(SheetSel::Name(name)) => {
            wb.sheets.iter().position(|s| s.name == *name).ok_or_else(|| LangError::new(format!("worksheet '{name}' not found"), 0))
        }
        Some(SheetSel::Index(i)) => {
            if *i >= 1 && *i <= wb.sheets.len() {
                Ok(i.saturating_sub(1))
            } else {
                Err(LangError::new(format!("worksheet index {i} is out of range"), 0))
            }
        }
    }
}

fn ensure_sheet(wb: &mut Workbook, sel: &Option<SheetSel>) -> Result<usize, LangError> {
    match sel {
        None => {
            if wb.sheets.is_empty() {
                wb.sheets.push(Sheet { name: "Sheet1".into(), grid: Grid::default() });
            }
            Ok(0)
        }
        Some(SheetSel::Name(name)) => {
            if let Some(i) = wb.sheets.iter().position(|s| s.name == *name) {
                return Ok(i);
            }
            wb.sheets.push(Sheet { name: name.clone(), grid: Grid::default() });
            Ok(wb.sheets.len().saturating_sub(1))
        }
        Some(SheetSel::Index(i)) => {
            if *i >= 1 && *i <= wb.sheets.len() {
                Ok(i.saturating_sub(1))
            } else if *i == wb.sheets.len().saturating_add(1) {
                wb.sheets.push(Sheet { name: format!("Sheet{}", wb.sheets.len().saturating_add(1)), grid: Grid::default() });
                Ok(wb.sheets.len().saturating_sub(1))
            } else {
                Err(LangError::new(format!("worksheet index {i} is out of range"), 0))
            }
        }
    }
}

fn bounds(grid: &Grid, range: &Option<Range>) -> (usize, usize, usize, usize) {
    match range {
        Some(r) => {
            let rows = r.r2.saturating_sub(r.r1).saturating_add(1);
            let cols = r.c2.saturating_sub(r.c1).saturating_add(1);
            (r.r1.saturating_sub(1), r.c1.saturating_sub(1), rows, cols)
        }
        None => (0, 0, grid.rows, grid.cols),
    }
}

fn cell_to_value(cell: Cell) -> Value {
    match cell {
        Cell::Empty => Value::Mat(Matrix::zeros(0, 0)),
        Cell::Number(v) => mat_or_num(Matrix::scalar(v)),
        Cell::Text(s) => Value::Str(s),
        Cell::Bool(b) => Value::Num(if b { 1.0 } else { 0.0 }),
    }
}

fn value_to_cell(v: &Value) -> Result<Cell, LangError> {
    match v {
        Value::Num(x) => Ok(Cell::Number(*x)),
        Value::Mat(m) if m.numel() == 0 => Ok(Cell::Empty),
        Value::Mat(m) if m.numel() == 1 => Ok(Cell::Number(m.data().first().copied().unwrap_or(0.0))),
        Value::Str(s) => Ok(Cell::Text(s.clone())),
        _ => Err(LangError::new("xlsx writes accept numeric scalars, strings or empty values per cell", 0)),
    }
}

fn numeric_grid(grid: &Grid, r0: usize, c0: usize, rows: usize, cols: usize) -> Result<Matrix, LangError> {
    check_elems(rows.saturating_mul(cols))?;
    let mut out = Matrix::zeros(rows, cols);
    for r in 0..rows {
        for c in 0..cols {
            let v = match grid.get(r0.saturating_add(r), c0.saturating_add(c)) {
                Cell::Number(v) => v,
                Cell::Bool(b) => {
                    if b {
                        1.0
                    } else {
                        0.0
                    }
                }
                _ => f64::NAN,
            };
            let _ = out.set(r, c, v);
        }
    }
    Ok(out)
}

fn text_grid(grid: &Grid, r0: usize, c0: usize, rows: usize, cols: usize) -> Result<CellArray, LangError> {
    if rows.saturating_mul(cols) > MAX_CELL_ELEMS {
        return Err(LangError::new("allocation exceeds limit", 0));
    }
    let empty = Value::Mat(Matrix::zeros(0, 0));
    let mut items = vec![empty; rows.saturating_mul(cols)];
    for r in 0..rows {
        for c in 0..cols {
            if let Cell::Text(s) = grid.get(r0.saturating_add(r), c0.saturating_add(c)) {
                if let Some(slot) = items.get_mut(c.saturating_mul(rows).saturating_add(r)) {
                    *slot = Value::Str(s);
                }
            }
        }
    }
    Ok(CellArray::from_col_major(rows, cols, items))
}

fn raw_grid(grid: &Grid, r0: usize, c0: usize, rows: usize, cols: usize) -> Result<CellArray, LangError> {
    if rows.saturating_mul(cols) > MAX_CELL_ELEMS {
        return Err(LangError::new("allocation exceeds limit", 0));
    }
    let mut items = Vec::with_capacity(rows.saturating_mul(cols));
    for c in 0..cols {
        for r in 0..rows {
            items.push(cell_to_value(grid.get(r0.saturating_add(r), c0.saturating_add(c))));
        }
    }
    Ok(CellArray::from_col_major(rows, cols, items))
}

fn apply_value(grid: &mut Grid, data: &Value, r0: usize, c0: usize) -> Result<(), LangError> {
    match data {
        Value::Cell(ca) => {
            for r in 0..ca.shape().0 {
                for c in 0..ca.shape().1 {
                    if let Some(v) = ca.get(r, c) {
                        grid.set(r0.saturating_add(r), c0.saturating_add(c), value_to_cell(v)?)?;
                    }
                }
            }
        }
        Value::Mat(m) => {
            for r in 0..m.rows() {
                for c in 0..m.cols() {
                    grid.set(r0.saturating_add(r), c0.saturating_add(c), Cell::Number(m.get(r, c).unwrap_or(f64::NAN)))?;
                }
            }
        }
        Value::Num(x) => {
            grid.set(r0, c0, Cell::Number(*x))?;
        }
        Value::Str(s) => {
            grid.set(r0, c0, Cell::Text(s.clone()))?;
        }
        _ => return Err(LangError::new("xlsx data must be a matrix, cell array or scalar/string", 0)),
    }
    Ok(())
}

fn apply_grid(grid: &mut Grid, src: &Grid, r0: usize, c0: usize) -> Result<(), LangError> {
    for r in 0..src.rows {
        for c in 0..src.cols {
            let cell = src.get(r, c);
            if !matches!(cell, Cell::Empty) {
                grid.set(r0.saturating_add(r), c0.saturating_add(c), cell)?;
            }
        }
    }
    Ok(())
}

fn sanitize_names(names: &[String]) -> Vec<String> {
    let mut used: HashSet<String> = HashSet::new();
    let mut out = Vec::with_capacity(names.len());
    for (idx, name) in names.iter().enumerate() {
        let mut s: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
        if s.is_empty() {
            s = format!("Var{}", idx.saturating_add(1));
        }
        if s.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
            s.insert(0, 'x');
        }
        if matches!(s.as_str(), "Properties" | "end" | "VariableNames") {
            s.insert_str(0, "x_");
        }
        let mut candidate = s.clone();
        let mut suffix = 2usize;
        while used.contains(&candidate) {
            candidate = format!("{s}_{suffix}");
            suffix = suffix.saturating_add(1);
        }
        used.insert(candidate.clone());
        out.push(candidate);
    }
    out
}

fn cellstr(items: Vec<String>) -> Value {
    Value::Cell(CellArray::row(items.into_iter().map(Value::Str).collect()))
}

fn preserved_names(names: &[String]) -> Vec<String> {
    let mut used: HashSet<String> = HashSet::new();
    let mut out = Vec::with_capacity(names.len());
    for (idx, name) in names.iter().enumerate() {
        let mut s = if name.trim().is_empty() { format!("Var{}", idx.saturating_add(1)) } else { name.clone() };
        if s == "Properties" {
            s = "x_Properties".into();
        }
        let mut candidate = s.clone();
        let mut suffix = 2usize;
        while used.contains(&candidate) {
            candidate = format!("{s}_{suffix}");
            suffix = suffix.saturating_add(1);
        }
        used.insert(candidate.clone());
        out.push(candidate);
    }
    out
}

fn table_from_grid(grid: &Grid, range: &Option<Range>, read_names: bool, preserve_names: bool) -> Result<Value, LangError> {
    let (r0, c0, rows, cols) = bounds(grid, range);
    if rows == 0 || cols == 0 {
        let mut props = StructArray::new();
        let properties = table_properties(vec![], 0);
        props.set_field(0, "Properties", properties)?;
        return Ok(Value::Object(MObject::owned("table".into(), props)));
    }
    let has_header = read_names && (0..cols).any(|c| matches!(grid.get(r0, c0.saturating_add(c)), Cell::Text(ref s) if !s.trim().is_empty()));
    let data_start = if has_header { 1usize } else { 0usize };
    let data_rows = rows.saturating_sub(data_start);
    let mut names = Vec::with_capacity(cols);
    for c in 0..cols {
        let name = if has_header {
            match grid.get(r0, c0.saturating_add(c)) {
                Cell::Text(s) if !s.trim().is_empty() => s,
                Cell::Number(v) => format!("{v}"),
                _ => format!("Var{}", c.saturating_add(1)),
            }
        } else {
            format!("Var{}", c.saturating_add(1))
        };
        names.push(name);
    }
    let fields = if preserve_names { preserved_names(&names) } else { sanitize_names(&names) };
    let mut props = StructArray::new();
    for (c, field) in fields.iter().enumerate() {
        let mut numeric = true;
        for r in data_start..rows {
            match grid.get(r0.saturating_add(r), c0.saturating_add(c)) {
                Cell::Number(_) | Cell::Bool(_) | Cell::Empty => {}
                Cell::Text(_) => numeric = false,
            }
        }
        let value = if numeric {
            let mut col = Matrix::zeros(data_rows, 1);
            for r in 0..data_rows {
                let v = match grid.get(r0.saturating_add(data_start).saturating_add(r), c0.saturating_add(c)) {
                    Cell::Number(v) => v,
                    Cell::Bool(b) => {
                        if b {
                            1.0
                        } else {
                            0.0
                        }
                    }
                    _ => f64::NAN,
                };
                let _ = col.set(r, 0, v);
            }
            Value::Mat(col)
        } else {
            let mut items = Vec::with_capacity(data_rows);
            for r in 0..data_rows {
                items.push(cell_to_value(grid.get(r0.saturating_add(data_start).saturating_add(r), c0.saturating_add(c))));
            }
            Value::Cell(CellArray::col(items))
        };
        props.set_field(0, field, value)?;
    }
    let properties = table_properties(names, data_rows);
    props.set_field(0, "Properties", properties)?;
    Ok(Value::Object(MObject::owned("table".into(), props)))
}

fn table_properties(names: Vec<String>, rows: usize) -> Value {
    let mut p = StructArray::new();
    let _ = p.set_field(0, "VariableNames", cellstr(names));
    let _ = p.set_field(0, "RowNames", Value::Cell(CellArray::row(Vec::new())));
    let _ = p.set_field(0, "DimensionNames", cellstr(vec!["Row".into(), "Variables".into()]));
    let _ = p.set_field(0, "NumRows", Value::Num(rows as f64));
    Value::Struct(p)
}

fn table_variable_names(v: &Value) -> Vec<String> {
    match v {
        Value::Object(o) if o.class_name == "table" => o
            .prop("Properties")
            .and_then(|p| match p {
                Value::Struct(s) => s.field_at(0, "VariableNames").cloned(),
                _ => None,
            })
            .and_then(|names| match names {
                Value::Cell(c) => Some(
                    c.items
                        .iter()
                        .filter_map(|v| match v {
                            Value::Str(s) => Some(s.clone()),
                            _ => None,
                        })
                        .collect(),
                ),
                _ => None,
            })
            .unwrap_or_default(),
        Value::Struct(s) => s.field_names().into_iter().filter(|n| n != "Properties").collect(),
        _ => Vec::new(),
    }
}

fn table_field(v: &Value, name: &str) -> Option<Value> {
    match v {
        Value::Object(o) => o.prop(name),
        Value::Struct(s) => s.field_at(0, name).cloned(),
        _ => None,
    }
}

fn field_len(v: &Value) -> usize {
    match v {
        Value::Num(_) | Value::Str(_) => 1,
        Value::Mat(m) => m.numel(),
        Value::Cell(c) => c.numel(),
        _ => 0,
    }
}

fn field_value(v: &Value, i: usize) -> Result<Cell, LangError> {
    match v {
        Value::Num(x) => Ok(Cell::Number(*x)),
        Value::Mat(m) => Ok(Cell::Number(m.get_linear(i).unwrap_or(f64::NAN))),
        Value::Cell(c) => c.get_linear(i).map(value_to_cell).unwrap_or(Ok(Cell::Empty)),
        Value::Str(s) => Ok(Cell::Text(s.clone())),
        _ => Err(LangError::new("unsupported table column type", 0)),
    }
}

fn is_text_table(path: &std::path::Path, file_type: Option<&str>) -> bool {
    if let Some(t) = file_type {
        let t = t.trim().to_ascii_lowercase();
        if t == "text" {
            return true;
        }
        if t == "spreadsheet" {
            return false;
        }
    }
    matches!(path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(), Some("csv" | "tsv" | "tab" | "txt" | "dat"))
}

fn delimiter_char(s: &str) -> Option<char> {
    match s.trim() {
        "\\t" | "tab" => Some('\t'),
        "\\n" => Some('\n'),
        "\\s" => Some(' '),
        other => other.chars().next(),
    }
}

fn text_delimiter(path: &std::path::Path, explicit: Option<char>, text: &str) -> char {
    if let Some(c) = explicit {
        return c;
    }
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("tsv" | "tab") => '\t',
        Some("csv") => ',',
        _ => {
            let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
            let candidates = [('\t', first.matches('\t').count()), (',', first.matches(',').count()), (';', first.matches(';').count())];
            candidates.iter().max_by_key(|(_, n)| *n).filter(|(_, n)| *n > 0).map(|(c, _)| *c).unwrap_or(' ')
        }
    }
}

/// Parse a delimited text file into the same cell grid the xlsx reader uses.
/// Handles quoted fields (including embedded delimiters/newlines and doubled
/// quotes) and decides per column whether values are numeric.
fn delimited_grid(text: &str, delim: char) -> Result<Grid, LangError> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut row_started = false;
    let mut iter = text.chars().peekable();
    while let Some(ch) = iter.next() {
        if quoted {
            if ch == '"' {
                if iter.peek() == Some(&'"') {
                    iter.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            } else {
                field.push(ch);
            }
            row_started = true;
            continue;
        }
        if ch == '"' && field.trim().is_empty() {
            field.clear();
            quoted = true;
            row_started = true;
            continue;
        }
        if ch == delim {
            row.push(std::mem::take(&mut field));
            row_started = true;
            continue;
        }
        if ch == '\r' || ch == '\n' {
            if ch == '\r' && iter.peek() == Some(&'\n') {
                iter.next();
            }
            row.push(std::mem::take(&mut field));
            if row_started || row.iter().any(|f| !f.trim().is_empty()) {
                rows.push(std::mem::take(&mut row));
            } else {
                row.clear();
            }
            row_started = false;
            continue;
        }
        field.push(ch);
        row_started = true;
    }
    if quoted {
        return Err(LangError::new("readtable: unterminated quoted field", 0));
    }
    if row_started || !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if rows.len().saturating_mul(cols) > MAX_CELL_ELEMS {
        return Err(LangError::new("readtable: text table exceeds the cell limit", 0));
    }
    let mut grid = Grid::default();
    for (r, fields) in rows.iter().enumerate() {
        for (c, field) in fields.iter().enumerate() {
            let trimmed = field.trim();
            let cell = if trimmed.is_empty() {
                Cell::Empty
            } else if let Ok(v) = trimmed.parse::<f64>() {
                // Keep identifiers such as "0012" textual.
                let leading_zero = trimmed.len() > 1
                    && trimmed.starts_with('0')
                    && !trimmed.starts_with("0.")
                    && !trimmed.starts_with("0e")
                    && !trimmed.starts_with("0E");
                if leading_zero {
                    Cell::Text(trimmed.to_string())
                } else {
                    Cell::Number(v)
                }
            } else {
                Cell::Text(field.clone())
            };
            grid.set(r, c, cell)?;
        }
    }
    Ok(grid)
}

fn grid_to_delimited(grid: &Grid, delim: char) -> String {
    let quote = |s: &str| -> String {
        if s.contains(delim) || s.contains('"') || s.contains('\n') || s.contains('\r') {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.to_string()
        }
    };
    let mut out = String::new();
    for r in 0..grid.rows {
        for c in 0..grid.cols {
            if c > 0 {
                out.push(delim);
            }
            match grid.get(r, c) {
                Cell::Empty => {}
                Cell::Number(v) => {
                    if v.is_finite() {
                        out.push_str(&format!("{v}"));
                    } else if v.is_nan() {
                        out.push_str("NaN");
                    } else if v > 0.0 {
                        out.push_str("Inf");
                    } else {
                        out.push_str("-Inf");
                    }
                }
                Cell::Bool(b) => out.push_str(if b { "1" } else { "0" }),
                Cell::Text(s) => out.push_str(&quote(&s)),
            }
        }
        out.push('\n');
    }
    out
}

fn table_grid(v: &Value, write_names: bool) -> Result<Grid, LangError> {
    let names = table_variable_names(v);
    let fields = sanitize_names(&names);
    if names.is_empty() {
        return Ok(Grid::default());
    }
    let mut columns = Vec::with_capacity(fields.len());
    for (name, field) in names.iter().zip(fields.iter()) {
        let col =
            table_field(v, field).or_else(|| table_field(v, name)).ok_or_else(|| LangError::new(format!("table is missing variable '{name}'"), 0))?;
        columns.push(col);
    }
    let mut grid = Grid::default();
    let start: usize = if write_names { 1 } else { 0 };
    if write_names {
        for (c, name) in names.iter().enumerate() {
            grid.set(0, c, Cell::Text(name.clone()))?;
        }
    }
    for (c, col) in columns.iter().enumerate() {
        for r in 0..field_len(col) {
            grid.set(start.saturating_add(r), c, field_value(col, r)?)?;
        }
    }
    Ok(grid)
}

impl Session {
    pub(super) fn xlsx_builtin(&mut self, name: &str, argv: Vec<Value>, nout: usize) -> Result<Vec<Value>, LangError> {
        match name {
            "xlsread" => self.xlsread_call(&argv, nout),
            "xlswrite" => self.xlswrite_call(&argv),
            "readcell" => self.readcell_call(&argv),
            "writecell" => self.writecell_call(&argv),
            "readtable" => self.readtable_call(&argv),
            "writetable" => self.writetable_call(&argv),
            "height" => Ok(vec![Value::Num(height_of(argv.first())? as f64)]),
            "width" => Ok(vec![Value::Num(width_of(argv.first())? as f64)]),
            "istable" => Ok(vec![Value::Num(matches!(argv.first(), Some(Value::Object(o)) if o.class_name == "table") as u8 as f64)]),
            _ => Err(LangError::new(format!("{name}: not an xlsx builtin"), 0)),
        }
    }

    fn xlsread_call(&mut self, argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
        let path = path_arg(argv.first().ok_or_else(|| LangError::new("xlsread needs a file name", 0))?, "file name")?;
        let mut sheet = sheet_sel(argv.get(1))?;
        let mut range = range_arg(argv.get(2))?;
        if range.is_none() {
            if let Some(SheetSel::Name(s)) = &sheet {
                if let Some(r) = parse_range(s) {
                    range = Some(r);
                    sheet = None;
                }
            }
        }
        let wb = xlsx::read_workbook(&path)?;
        let idx = select_sheet(&wb, &sheet)?;
        let grid = wb.sheets.get(idx).map(|s| &s.grid).ok_or_else(|| LangError::new("worksheet not found", 0))?;
        let (r0, c0, rows, cols) = bounds(grid, &range);
        let num = numeric_grid(grid, r0, c0, rows, cols)?;
        let txt = text_grid(grid, r0, c0, rows, cols)?;
        let raw = raw_grid(grid, r0, c0, rows, cols)?;
        let mut out = vec![mat_or_num(num), Value::Cell(txt), Value::Cell(raw)];
        out.truncate(nout.max(1));
        Ok(out)
    }

    fn xlswrite_call(&mut self, argv: &[Value]) -> Result<Vec<Value>, LangError> {
        let path = path_arg(argv.first().ok_or_else(|| LangError::new("xlswrite needs a file name", 0))?, "file name")?;
        let data = argv.get(1).ok_or_else(|| LangError::new("xlswrite needs data", 0))?;
        let sheet = sheet_sel(argv.get(2))?;
        let range = range_arg(argv.get(3))?;
        let mut wb = if path.exists() {
            xlsx::read_workbook(&path)?
        } else {
            Workbook { sheets: vec![Sheet { name: "Sheet1".into(), grid: Grid::default() }] }
        };
        let idx = ensure_sheet(&mut wb, &sheet)?;
        let (r0, c0) = range.as_ref().map(|r| (r.r1.saturating_sub(1), r.c1.saturating_sub(1))).unwrap_or((0, 0));
        if let Some(grid) = wb.sheets.get_mut(idx) {
            apply_value(&mut grid.grid, data, r0, c0)?;
        }
        xlsx::write_workbook(&path, &wb)?;
        Ok(vec![Value::Num(1.0), Value::Str(String::new())])
    }

    fn readcell_call(&mut self, argv: &[Value]) -> Result<Vec<Value>, LangError> {
        let path = path_arg(argv.first().ok_or_else(|| LangError::new("readcell needs a file name", 0))?, "file name")?;
        let sheet = sheet_sel(argv.get(1))?;
        let range = range_arg(argv.get(2))?;
        let wb = xlsx::read_workbook(&path)?;
        let idx = select_sheet(&wb, &sheet)?;
        let grid = wb.sheets.get(idx).map(|s| &s.grid).ok_or_else(|| LangError::new("worksheet not found", 0))?;
        let (r0, c0, rows, cols) = bounds(grid, &range);
        Ok(vec![Value::Cell(raw_grid(grid, r0, c0, rows, cols)?)])
    }

    fn writecell_call(&mut self, argv: &[Value]) -> Result<Vec<Value>, LangError> {
        let data = argv.first().ok_or_else(|| LangError::new("writecell needs a cell array", 0))?;
        if !matches!(data, Value::Cell(_)) {
            return Err(LangError::new("writecell needs a cell array", 0));
        }
        let path = path_arg(argv.get(1).ok_or_else(|| LangError::new("writecell needs a file name", 0))?, "file name")?;
        let sheet = sheet_sel(argv.get(2))?;
        let range = range_arg(argv.get(3))?;
        let mut wb = if path.exists() {
            xlsx::read_workbook(&path)?
        } else {
            Workbook { sheets: vec![Sheet { name: "Sheet1".into(), grid: Grid::default() }] }
        };
        let idx = ensure_sheet(&mut wb, &sheet)?;
        let (r0, c0) = range.as_ref().map(|r| (r.r1.saturating_sub(1), r.c1.saturating_sub(1))).unwrap_or((0, 0));
        if let Some(grid) = wb.sheets.get_mut(idx) {
            apply_value(&mut grid.grid, data, r0, c0)?;
        }
        xlsx::write_workbook(&path, &wb)?;
        Ok(vec![Value::Num(1.0), Value::Str(String::new())])
    }

    fn readtable_call(&mut self, argv: &[Value]) -> Result<Vec<Value>, LangError> {
        let path = path_arg(argv.first().ok_or_else(|| LangError::new("readtable needs a file name", 0))?, "file name")?;
        let mut sheet: Option<SheetSel> = None;
        let mut range: Option<Range> = None;
        let mut read_names = true;
        let mut delimiter: Option<char> = None;
        let mut file_type: Option<String> = None;
        let mut preserve_names = false;
        let mut i = 1usize;
        while i < argv.len() {
            if let Value::Str(key) = &argv[i] {
                match key.to_ascii_lowercase().as_str() {
                    "sheet" => {
                        sheet = sheet_sel(argv.get(i.saturating_add(1)))?;
                        i = i.saturating_add(2);
                        continue;
                    }
                    "range" => {
                        range = range_arg(argv.get(i.saturating_add(1)))?;
                        i = i.saturating_add(2);
                        continue;
                    }
                    "readvariablenames" => {
                        read_names = argv.get(i.saturating_add(1)).and_then(|v| v.truthy().ok()).unwrap_or(true);
                        i = i.saturating_add(2);
                        continue;
                    }
                    "delimiter" => {
                        delimiter = match argv.get(i.saturating_add(1)) {
                            Some(Value::Str(s)) => delimiter_char(s),
                            _ => None,
                        };
                        i = i.saturating_add(2);
                        continue;
                    }
                    "filetype" => {
                        file_type = match argv.get(i.saturating_add(1)) {
                            Some(Value::Str(s)) => Some(s.clone()),
                            _ => None,
                        };
                        i = i.saturating_add(2);
                        continue;
                    }
                    "variablenamingrule" => {
                        preserve_names = matches!(argv.get(i.saturating_add(1)), Some(Value::Str(s)) if s.eq_ignore_ascii_case("preserve"));
                        i = i.saturating_add(2);
                        continue;
                    }
                    "preservevariablenames" => {
                        preserve_names = argv.get(i.saturating_add(1)).and_then(|v| v.truthy().ok()).unwrap_or(true);
                        i = i.saturating_add(2);
                        continue;
                    }
                    "texttype" => {
                        i = i.saturating_add(2);
                        continue;
                    }
                    _ => {
                        if i == 1 {
                            if let Some(r) = parse_range(key) {
                                range = Some(r);
                            } else {
                                sheet = Some(SheetSel::Name(key.clone()));
                            }
                        }
                    }
                }
            }
            i = i.saturating_add(1);
        }
        if is_text_table(&path, file_type.as_deref()) {
            let text = std::fs::read_to_string(&path).map_err(|e| LangError::new(format!("readtable: cannot read {}: {e}", path.display()), 0))?;
            let delim = text_delimiter(&path, delimiter, &text);
            let grid = delimited_grid(&text, delim)?;
            return Ok(vec![table_from_grid(&grid, &range, read_names, preserve_names)?]);
        }
        let wb = xlsx::read_workbook(&path)?;
        let idx = select_sheet(&wb, &sheet)?;
        let grid = wb.sheets.get(idx).map(|s| &s.grid).ok_or_else(|| LangError::new("worksheet not found", 0))?;
        Ok(vec![table_from_grid(grid, &range, read_names, preserve_names)?])
    }

    fn writetable_call(&mut self, argv: &[Value]) -> Result<Vec<Value>, LangError> {
        let data = argv.first().ok_or_else(|| LangError::new("writetable needs a table", 0))?;
        let path = path_arg(argv.get(1).ok_or_else(|| LangError::new("writetable needs a file name", 0))?, "file name")?;
        let mut sheet: Option<SheetSel> = None;
        let mut range: Option<Range> = None;
        let mut write_names = true;
        let mut delimiter: Option<char> = None;
        let mut i = 2usize;
        while i.saturating_add(1) < argv.len() {
            if let Value::Str(key) = &argv[i] {
                match key.to_ascii_lowercase().as_str() {
                    "sheet" => sheet = sheet_sel(argv.get(i.saturating_add(1)))?,
                    "range" => range = range_arg(argv.get(i.saturating_add(1)))?,
                    "writevariablenames" => {
                        write_names = argv.get(i.saturating_add(1)).and_then(|v| v.truthy().ok()).unwrap_or(true);
                    }
                    "delimiter" => {
                        delimiter = match argv.get(i.saturating_add(1)) {
                            Some(Value::Str(s)) => delimiter_char(s),
                            _ => None,
                        };
                    }
                    _ => {}
                }
            }
            i = i.saturating_add(2);
        }
        let src = table_grid(data, write_names)?;
        if is_text_table(&path, None) {
            let delim = delimiter.unwrap_or_else(|| match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
                Some("tsv" | "tab") => '\t',
                _ => ',',
            });
            let body = grid_to_delimited(&src, delim);
            crate::write_file(&path, body.as_bytes()).map_err(|e| LangError::new(format!("writetable: cannot write {}: {e}", path.display()), 0))?;
            return Ok(vec![Value::Num(1.0), Value::Str(String::new())]);
        }
        let mut wb = if path.exists() {
            xlsx::read_workbook(&path)?
        } else {
            Workbook { sheets: vec![Sheet { name: "Sheet1".into(), grid: Grid::default() }] }
        };
        let idx = ensure_sheet(&mut wb, &sheet)?;
        let (r0, c0) = range.as_ref().map(|r| (r.r1.saturating_sub(1), r.c1.saturating_sub(1))).unwrap_or((0, 0));
        if let Some(grid) = wb.sheets.get_mut(idx) {
            apply_grid(&mut grid.grid, &src, r0, c0)?;
        }
        xlsx::write_workbook(&path, &wb)?;
        Ok(vec![Value::Num(1.0)])
    }
}

fn height_of(v: Option<&Value>) -> Result<usize, LangError> {
    let v = v.ok_or_else(|| LangError::new("height needs an argument", 0))?;
    match v {
        Value::Object(o) if o.class_name == "table" => {
            let mut rows = 0usize;
            for field in o.prop_names() {
                if field == "Properties" {
                    continue;
                }
                if let Some(val) = o.prop(&field) {
                    rows = rows.max(field_len(&val));
                }
            }
            Ok(rows)
        }
        Value::Struct(s) => Ok(s.field_names().iter().filter_map(|f| s.field_at(0, f)).map(field_len).max().unwrap_or(0)),
        Value::Mat(m) => Ok(m.rows()),
        Value::Cell(c) => Ok(c.shape().0),
        Value::Num(_) | Value::Str(_) => Ok(1),
        _ => Err(LangError::new("height is not defined for this value", 0)),
    }
}

fn width_of(v: Option<&Value>) -> Result<usize, LangError> {
    let v = v.ok_or_else(|| LangError::new("width needs an argument", 0))?;
    match v {
        Value::Object(o) if o.class_name == "table" => Ok(table_variable_names(v).len()),
        Value::Struct(s) => Ok(s.field_names().iter().filter(|f| *f != "Properties").count()),
        Value::Mat(m) => Ok(m.cols()),
        Value::Cell(c) => Ok(c.shape().1),
        Value::Num(_) | Value::Str(_) => Ok(1),
        _ => Err(LangError::new("width is not defined for this value", 0)),
    }
}
