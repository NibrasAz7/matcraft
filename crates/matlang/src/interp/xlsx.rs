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

fn table_rows_of(v: &Value) -> Result<usize, LangError> {
    match v {
        Value::Object(o) if o.class_name == "table" => Ok(o
            .prop("Properties")
            .and_then(|p| match p {
                Value::Struct(s) => s.field_at(0, "NumRows").cloned(),
                _ => None,
            })
            .and_then(|x| x.scalar().ok())
            .unwrap_or(0.0)
            .max(0.0) as usize),
        _ => Err(LangError::new("expected a table", 0)),
    }
}

fn table_names_of(v: &Value) -> Vec<String> {
    table_variable_names(v)
}

fn variable_at(v: &Value, row: usize) -> Value {
    match v {
        Value::Num(x) => Value::Num(*x),
        Value::Mat(m) if m.numel() == 1 => Value::Num(m.data().first().copied().unwrap_or(0.0)),
        Value::Mat(m) => Value::Num(m.get_linear(row).unwrap_or(f64::NAN)),
        Value::Cell(c) => c.get_linear(row).cloned().unwrap_or_else(|| Value::Mat(Matrix::zeros(0, 0))),
        Value::Str(s) => Value::Str(s.clone()),
        other => other.clone(),
    }
}

fn table2cell_value(v: &Value) -> Result<Value, LangError> {
    let names = table_names_of(v);
    let rows = table_rows_of(v)?;
    let mut items = Vec::with_capacity(rows.saturating_mul(names.len()));
    for name in &names {
        let col = table_field(v, name).ok_or_else(|| LangError::new(format!("table is missing variable '{name}'"), 0))?;
        for r in 0..rows {
            items.push(variable_at(&col, r));
        }
    }
    Ok(Value::Cell(CellArray::from_col_major(rows, names.len(), items)))
}

fn column_from_cells(items: &[Value], rows: usize) -> Value {
    let numeric = items.iter().all(|x| matches!(x, Value::Num(_) | Value::Mat(_)));
    if numeric {
        let mut m = Matrix::zeros(rows, 1);
        for (r, item) in items.iter().take(rows).enumerate() {
            let x = match item {
                Value::Num(x) => *x,
                Value::Mat(mm) => mm.data().first().copied().unwrap_or(f64::NAN),
                _ => f64::NAN,
            };
            let _ = m.set(r, 0, x);
        }
        Value::Mat(m)
    } else {
        Value::Cell(CellArray::col(items.iter().take(rows).cloned().collect()))
    }
}

fn cell2table_value(c: &Value, names: Vec<String>) -> Result<Value, LangError> {
    let (rows, cols, items) = match c {
        Value::Cell(c) => (c.rows, c.cols, c.items.clone()),
        Value::Mat(m) => (m.rows(), m.cols(), m.data().iter().map(|&x| Value::Num(x)).collect()),
        _ => return Err(LangError::new("cell2table needs a cell array", 0)),
    };
    let names = if names.len() == cols { names } else { (1..=cols).map(|i| format!("Var{i}")).collect() };
    let mut props = StructArray::new();
    for (c, name) in names.iter().enumerate() {
        let mut col = Vec::with_capacity(rows);
        for r in 0..rows {
            col.push(items.get(c * rows + r).cloned().unwrap_or_else(|| Value::Mat(Matrix::zeros(0, 0))));
        }
        props.set_field(0, name, column_from_cells(&col, rows))?;
    }
    props.set_field(0, "Properties", table_properties(names, rows))?;
    Ok(Value::Object(MObject::owned("table".into(), props)))
}

fn struct2table_value(v: &Value) -> Result<Value, LangError> {
    let s = match v {
        Value::Struct(s) => s,
        _ => return Err(LangError::new("struct2table needs a struct", 0)),
    };
    let names = s.field_names();
    let multi = s.rows().saturating_mul(s.cols()) > 1;
    let rows = if multi {
        s.rows().saturating_mul(s.cols())
    } else {
        names.iter().map(|n| table_field(v, n).map(|x| field_len(&x)).unwrap_or(0)).max().unwrap_or(0).max(1)
    };
    let mut props = StructArray::new();
    for name in &names {
        let value = table_field(v, name).ok_or_else(|| LangError::new(format!("struct is missing field '{name}'"), 0))?;
        let col = if multi {
            let vals = s.field_values(name).unwrap_or_default();
            column_from_cells(&vals, rows)
        } else if match &value {
            Value::Mat(m) => m.rows(),
            Value::Cell(c) => c.rows,
            _ => 1,
        } == rows
        {
            value
        } else if let Value::Mat(m) = &value {
            if m.numel() == 1 {
                let mut out = Matrix::zeros(rows, 1);
                for r in 0..rows {
                    let _ = out.set(r, 0, m.data().first().copied().unwrap_or(0.0));
                }
                Value::Mat(out)
            } else {
                value
            }
        } else {
            value
        };
        props.set_field(0, name, col)?;
    }
    props.set_field(0, "Properties", table_properties(names, rows))?;
    Ok(Value::Object(MObject::owned("table".into(), props)))
}

fn struct2cell_value(v: &Value) -> Result<Value, LangError> {
    let s = match v {
        Value::Struct(s) => s,
        _ => return Err(LangError::new("struct2cell needs a struct", 0)),
    };
    let names = s.field_names();
    let mut items = Vec::with_capacity(names.len());
    for name in names {
        items.push(
            s.field_values(&name)
                .and_then(|mut x| if x.len() == 1 { x.pop() } else { Some(Value::Cell(CellArray::col(x))) })
                .unwrap_or_else(|| Value::Mat(Matrix::zeros(0, 0))),
        );
    }
    Ok(Value::Cell(CellArray::col(items)))
}

fn cell2struct_value(c: &Value, fields: &[String], dim: usize) -> Result<Value, LangError> {
    let ca = match c {
        Value::Cell(c) => c,
        _ => return Err(LangError::new("cell2struct needs a cell array", 0)),
    };
    let (groups, elems) = if dim == 1 { (ca.rows, ca.cols) } else { (ca.cols, ca.rows) };
    if fields.len() != groups {
        return Err(LangError::new("cell2struct: field count must match the selected dimension", 0));
    }
    if elems <= 1 {
        let mut props = StructArray::new();
        for (g, name) in fields.iter().enumerate() {
            let item = if dim == 1 { ca.get(g, 0) } else { ca.get(0, g) };
            props.set_field(0, name, item.cloned().unwrap_or_else(|| Value::Mat(Matrix::zeros(0, 0))))?;
        }
        return Ok(Value::Struct(props));
    }
    let mut props = StructArray::with_dims(elems, 1);
    for e in 0..elems {
        for (g, name) in fields.iter().enumerate() {
            let item = if dim == 1 { ca.get(g, e) } else { ca.get(e, g) };
            props.set_field(e, name, item.cloned().unwrap_or_else(|| Value::Mat(Matrix::zeros(0, 0))))?;
        }
    }
    Ok(Value::Struct(props))
}

fn parse_name_list(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Cell(c)) => c
            .items
            .iter()
            .filter_map(|x| match x {
                Value::Str(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        Some(Value::Str(s)) => vec![s.clone()],
        _ => Vec::new(),
    }
}

fn addvars_table(t: &Value, argv: &[Value]) -> Result<Value, LangError> {
    let Value::Object(o) = t else { return Err(LangError::new("addvars needs a table", 0)) };
    let mut names = table_names_of(t);
    let mut values: Vec<Value> = Vec::new();
    let mut explicit: Option<Vec<String>> = None;
    let mut i = 1usize;
    while i < argv.len() {
        if let Value::Str(key) = &argv[i] {
            if key.eq_ignore_ascii_case("NewVariableNames") || key.eq_ignore_ascii_case("Before") || key.eq_ignore_ascii_case("After") {
                if key.eq_ignore_ascii_case("NewVariableNames") {
                    explicit = Some(parse_name_list(argv.get(i + 1)));
                }
                i += 2;
                continue;
            }
        }
        values.push(argv[i].clone());
        i += 1;
    }
    let new_names = explicit.unwrap_or_else(|| (0..values.len()).map(|k| format!("Var{}", names.len() + k + 1)).collect());
    let rows = values.first().map(field_len).unwrap_or_else(|| table_rows_of(t).unwrap_or(0)).max(1);
    let mut out = StructArray::new();
    for name in &names {
        out.set_field(0, name, o.prop(name).unwrap_or_else(|| Value::Mat(Matrix::zeros(0, 0))))?;
    }
    for (k, value) in values.iter().enumerate() {
        let name = new_names.get(k).cloned().unwrap_or_else(|| format!("Var{}", names.len() + k + 1));
        out.set_field(0, &name, value.clone())?;
        names.push(name);
    }
    out.set_field(0, "Properties", table_properties(names, rows))?;
    Ok(Value::Object(MObject::owned("table".into(), out)))
}

fn group_keys(v: &Value) -> Vec<String> {
    match v {
        Value::Mat(m) => (0..m.rows()).map(|r| format!("n:{}", m.get(r, 0).unwrap_or(f64::NAN))).collect(),
        Value::Cell(c) => (0..c.rows)
            .map(|r| match c.get(r, 0) {
                Some(Value::Str(s)) => format!("s:{s}"),
                Some(Value::Num(x)) => format!("n:{x}"),
                _ => String::new(),
            })
            .collect(),
        Value::Str(s) => vec![format!("s:{s}")],
        _ => Vec::new(),
    }
}

fn unique_groups(keys: &[String]) -> Vec<String> {
    let mut out: Vec<String> = keys.to_vec();
    out.sort();
    out.dedup();
    out
}

fn id_for_group(v: &Value, key: &str, keys: &[String]) -> Value {
    let rows: Vec<usize> = keys.iter().enumerate().filter(|(_, k)| *k == key).map(|(i, _)| i).collect();
    match v {
        Value::Mat(m) => Value::Num(m.get(rows.first().copied().unwrap_or(0), 0).unwrap_or(f64::NAN)),
        Value::Cell(c) => c.get(rows.first().copied().unwrap_or(0), 0).cloned().unwrap_or_else(|| Value::Mat(Matrix::zeros(0, 0))),
        Value::Str(s) => Value::Str(s.clone()),
        _ => Value::Str(key.to_string()),
    }
}

fn findgroups_value(argv: &[Value], nout: usize) -> Result<Vec<Value>, LangError> {
    if argv.is_empty() {
        return Err(LangError::new("findgroups needs grouping variables", 0));
    }
    let first = &argv[0];
    let keys = group_keys(first);
    if keys.is_empty() {
        return Err(LangError::new("findgroups: unsupported grouping variable", 0));
    }
    let groups = unique_groups(&keys);
    let mut g = Matrix::zeros(keys.len(), 1);
    for (r, k) in keys.iter().enumerate() {
        let idx = groups.iter().position(|x| x == k).unwrap_or(0);
        let _ = g.set(r, 0, (idx + 1) as f64);
    }
    let mut out = vec![Value::Mat(g)];
    if nout > 1 {
        let ids: Vec<Value> = groups.iter().map(|k| id_for_group(first, k, &keys)).collect();
        let id = match first {
            Value::Mat(_) => {
                let mut m = Matrix::zeros(ids.len(), 1);
                for (i, v) in ids.iter().enumerate() {
                    let x = v.scalar().unwrap_or(f64::NAN);
                    let _ = m.set(i, 0, x);
                }
                Value::Mat(m)
            }
            _ => Value::Cell(CellArray::col(ids)),
        };
        out.push(id);
    }
    Ok(out)
}

fn subset_matrix_rows(m: &Matrix, rows: &[usize]) -> Matrix {
    let mut out = Matrix::zeros(rows.len(), m.cols());
    for (i, &r) in rows.iter().enumerate() {
        for c in 0..m.cols() {
            let _ = out.set(i, c, m.get(r, c).unwrap_or(f64::NAN));
        }
    }
    out
}

fn splitapply_value(s: &mut Session, argv: &[Value]) -> Result<Value, LangError> {
    if argv.len() < 3 {
        return Err(LangError::new("splitapply needs (func, X, G)", 0));
    }
    let func = &argv[0];
    let x = &argv[1];
    let g = argv[2].to_matrix()?;
    let n = g.numel();
    let maxg = g.data().iter().copied().filter(|v| v.is_finite()).fold(0.0f64, f64::max).max(0.0) as usize;
    let mut results: Vec<Value> = Vec::new();
    for group in 1..=maxg {
        let rows: Vec<usize> = (0..n).filter(|&i| g.get_linear(i).unwrap_or(0.0) == group as f64).collect();
        if rows.is_empty() {
            continue;
        }
        let subset = match x {
            Value::Mat(m) => Value::Mat(subset_matrix_rows(m, &rows)),
            Value::Cell(c) => {
                let mut out = CellArray::new(rows.len(), c.cols);
                for (i, &r) in rows.iter().enumerate() {
                    for col in 0..c.cols {
                        if let Some(v) = c.get(r, col) {
                            out.set(i, col, v.clone());
                        }
                    }
                }
                Value::Cell(out)
            }
            Value::Num(_) | Value::Str(_) => x.clone(),
            _ => return Err(LangError::new("splitapply: unsupported data", 0)),
        };
        let mut got = s.call_feval_target(func, vec![subset], 1)?;
        if let Some(v) = got.pop() {
            results.push(v);
        }
    }
    if results.iter().all(|v| matches!(v, Value::Num(_) | Value::Mat(_))) {
        let total: usize = results.iter().map(|v| v.to_matrix().map(|m| m.rows()).unwrap_or(0)).sum();
        let cols = results.iter().filter_map(|v| v.to_matrix().map(|m| m.cols()).ok()).max().unwrap_or(1);
        let mut out = Matrix::zeros(total, cols);
        let mut r0 = 0;
        for v in &results {
            if let Ok(m) = v.to_matrix() {
                for r in 0..m.rows() {
                    for c in 0..m.cols() {
                        let _ = out.set(r0 + r, c, m.get(r, c).unwrap_or(0.0));
                    }
                }
                r0 += m.rows();
            }
        }
        Ok(mat_or_num(out))
    } else {
        Ok(Value::Cell(CellArray::col(results)))
    }
}

fn varfun_value(s: &mut Session, argv: &[Value]) -> Result<Value, LangError> {
    if argv.len() < 2 {
        return Err(LangError::new("varfun needs (func, table)", 0));
    }
    let func = &argv[0];
    let t = &argv[1];
    let names = table_names_of(t);
    let mut props = StructArray::new();
    for name in &names {
        let col = table_field(t, name).ok_or_else(|| LangError::new(format!("varfun: missing variable '{name}'"), 0))?;
        let out = s.call_feval_target(func, vec![col], 1)?.into_iter().next().unwrap_or_else(|| Value::Mat(Matrix::zeros(0, 0)));
        props.set_field(0, name, out)?;
    }
    props.set_field(0, "Properties", table_properties(names, table_rows_of(t)?))?;
    Ok(Value::Object(MObject::owned("table".into(), props)))
}

fn rowfun_value(s: &mut Session, argv: &[Value]) -> Result<Value, LangError> {
    if argv.len() < 2 {
        return Err(LangError::new("rowfun needs (func, table)", 0));
    }
    let func = &argv[0];
    let t = &argv[1];
    let names = table_names_of(t);
    let rows = table_rows_of(t)?;
    let mut out = Vec::with_capacity(rows);
    for r in 0..rows {
        let mut vals = Vec::with_capacity(names.len());
        for name in &names {
            let col = table_field(t, name).unwrap_or_else(|| Value::Mat(Matrix::zeros(0, 0)));
            vals.push(variable_at(&col, r));
        }
        let row = if vals.iter().all(|v| matches!(v, Value::Num(_) | Value::Mat(_))) {
            let mut m = Matrix::zeros(1, vals.len());
            for (c, v) in vals.iter().enumerate() {
                let _ = m.set(0, c, v.scalar().unwrap_or(f64::NAN));
            }
            Value::Mat(m)
        } else {
            Value::Cell(CellArray::row(vals))
        };
        let got = s.call_feval_target(func, vec![row], 1)?.into_iter().next().unwrap_or_else(|| Value::Mat(Matrix::zeros(0, 0)));
        out.push(got);
    }
    if out.iter().all(|v| matches!(v, Value::Num(_) | Value::Mat(_))) {
        let mut m = Matrix::zeros(out.len(), 1);
        for (i, v) in out.iter().enumerate() {
            let _ = m.set(i, 0, v.scalar().unwrap_or(f64::NAN));
        }
        Ok(mat_or_num(m))
    } else {
        Ok(Value::Cell(CellArray::col(out)))
    }
}

fn summary_value(t: &Value) -> Result<Value, LangError> {
    let names = table_names_of(t);
    let rows = table_rows_of(t)?;
    let mut m = std::collections::BTreeMap::new();
    m.insert("NumRows".to_string(), Value::Num(rows as f64));
    m.insert("NumVariables".to_string(), Value::Num(names.len() as f64));
    m.insert("VariableNames".to_string(), Value::Cell(CellArray::row(names.into_iter().map(Value::Str).collect())));
    Ok(Value::Struct(StructArray::from_map(m)))
}

fn removevars_value(t: &Value, argv: &[Value]) -> Result<Value, LangError> {
    let Value::Object(o) = t else { return Err(LangError::new("removevars needs a table", 0)) };
    let drop: Vec<String> = parse_name_list(argv.get(1));
    let names: Vec<String> = table_names_of(t).into_iter().filter(|n| !drop.iter().any(|d| d == n)).collect();
    let mut props = StructArray::new();
    for name in &names {
        props.set_field(0, name, o.prop(name).unwrap_or_else(|| Value::Mat(Matrix::zeros(0, 0))))?;
    }
    props.set_field(0, "Properties", table_properties(names, table_rows_of(t)?))?;
    Ok(Value::Object(MObject::owned("table".into(), props)))
}

fn renamevars_value(t: &Value, argv: &[Value]) -> Result<Value, LangError> {
    let Value::Object(o) = t else { return Err(LangError::new("renamevars needs a table", 0)) };
    let old = parse_name_list(argv.get(1));
    let new = parse_name_list(argv.get(2));
    let orig = table_names_of(t);
    let mut names = Vec::with_capacity(orig.len());
    let mut props = StructArray::new();
    for name in &orig {
        let out_name = old.iter().position(|x| x == name).and_then(|i| new.get(i)).cloned().unwrap_or_else(|| name.clone());
        props.set_field(0, &out_name, o.prop(name).unwrap_or_else(|| Value::Mat(Matrix::zeros(0, 0))))?;
        names.push(out_name);
    }
    props.set_field(0, "Properties", table_properties(names, table_rows_of(t)?))?;
    Ok(Value::Object(MObject::owned("table".into(), props)))
}

fn join_tables(t1: &Value, t2: &Value, key_opt: Option<String>) -> Result<Value, LangError> {
    let n1 = table_names_of(t1);
    let n2 = table_names_of(t2);
    let key = key_opt.or_else(|| n1.iter().find(|x| n2.contains(x)).cloned()).ok_or_else(|| LangError::new("join: no common key variable", 0))?;
    let k1 = table_field(t1, &key).ok_or_else(|| LangError::new(format!("join: missing key '{key}'"), 0))?;
    let k2 = table_field(t2, &key).ok_or_else(|| LangError::new(format!("join: missing key '{key}'"), 0))?;
    let r1 = table_rows_of(t1)?;
    let r2 = table_rows_of(t2)?;
    let canon = |v: Value| -> String {
        match v {
            Value::Str(s) => s,
            Value::Num(x) => format!("{x}"),
            Value::Mat(m) => format!("{}", m.data().first().copied().unwrap_or(f64::NAN)),
            _ => String::new(),
        }
    };
    let keys2: Vec<String> = (0..r2).map(|r| canon(variable_at(&k2, r))).collect();
    let mut matches: Vec<(usize, usize)> = Vec::new();
    for r in 0..r1 {
        let k = canon(variable_at(&k1, r));
        if let Some(j) = keys2.iter().position(|x| x == &k) {
            matches.push((r, j));
        }
    }
    let mut names = Vec::new();
    let mut props = StructArray::new();
    for name in &n1 {
        let col = table_field(t1, name).unwrap_or_else(|| Value::Mat(Matrix::zeros(0, 0)));
        let vals: Vec<Value> = matches.iter().map(|(r, _)| variable_at(&col, *r)).collect();
        props.set_field(0, name, column_from_cells(&vals, matches.len()))?;
        names.push(name.clone());
    }
    for name in &n2 {
        if name == &key {
            continue;
        }
        let out_name = if names.contains(name) { format!("{name}_2") } else { name.clone() };
        let col = table_field(t2, name).unwrap_or_else(|| Value::Mat(Matrix::zeros(0, 0)));
        let vals: Vec<Value> = matches.iter().map(|(_, r)| variable_at(&col, *r)).collect();
        props.set_field(0, &out_name, column_from_cells(&vals, matches.len()))?;
        names.push(out_name);
    }
    props.set_field(0, "Properties", table_properties(names, matches.len()))?;
    Ok(Value::Object(MObject::owned("table".into(), props)))
}

fn groupsummary_value(t: &Value, argv: &[Value]) -> Result<Value, LangError> {
    let group_name = parse_name_list(argv.get(1)).into_iter().next().ok_or_else(|| LangError::new("groupsummary needs a grouping variable", 0))?;
    let method = match argv.get(2) {
        Some(Value::Str(s)) => s.to_ascii_lowercase(),
        _ => "mean".to_string(),
    };
    let group_col = table_field(t, &group_name).ok_or_else(|| LangError::new(format!("groupsummary: missing '{group_name}'"), 0))?;
    let rows = table_rows_of(t)?;
    let keys: Vec<String> = (0..rows)
        .map(|r| match variable_at(&group_col, r) {
            Value::Str(s) => s,
            Value::Num(x) => format!("{x}"),
            _ => String::new(),
        })
        .collect();
    let mut groups: Vec<String> = keys.clone();
    groups.sort();
    groups.dedup();
    let names = table_names_of(t);
    let numeric: Vec<(String, Value)> = names
        .iter()
        .filter(|n| *n != &group_name)
        .filter_map(|n| table_field(t, n).map(|v| (n.clone(), v)))
        .filter(|(_, v)| matches!(v, Value::Mat(_)))
        .collect();
    let mut out_names = vec![group_name.clone(), "GroupCount".to_string()];
    for (n, _) in &numeric {
        out_names.push(format!("{method}_{n}"));
    }
    let mut props = StructArray::new();
    let group_vals: Vec<Value> = groups
        .iter()
        .map(|g| match &group_col {
            Value::Mat(_) => Value::Num(g.parse::<f64>().unwrap_or(f64::NAN)),
            _ => Value::Str(g.clone()),
        })
        .collect();
    props.set_field(0, &group_name, column_from_cells(&group_vals, groups.len()))?;
    let mut counts = Matrix::zeros(groups.len(), 1);
    for (gi, g) in groups.iter().enumerate() {
        let idx: Vec<usize> = keys.iter().enumerate().filter(|(_, k)| *k == g).map(|(i, _)| i).collect();
        let _ = counts.set(gi, 0, idx.len() as f64);
        for (n, col) in &numeric {
            let m = col.to_matrix()?;
            let vals: Vec<f64> = idx.iter().map(|&r| m.get_linear(r).unwrap_or(f64::NAN)).collect();
            let val = match method.as_str() {
                "sum" => vals.iter().sum(),
                "median" => {
                    let mut v = vals.clone();
                    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                    v.get(v.len() / 2).copied().unwrap_or(f64::NAN)
                }
                "std" => {
                    let mean = vals.iter().sum::<f64>() / vals.len().max(1) as f64;
                    (vals.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / (vals.len().saturating_sub(1)).max(1) as f64).sqrt()
                }
                "min" => vals.iter().copied().fold(f64::INFINITY, f64::min),
                "max" => vals.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                "nummissing" => vals.iter().filter(|x| x.is_nan()).count() as f64,
                _ => vals.iter().filter(|x| x.is_finite()).sum::<f64>() / vals.iter().filter(|x| x.is_finite()).count().max(1) as f64,
            };
            let mut existing =
                props.field_at(0, &format!("{method}_{n}")).and_then(|v| v.to_matrix().ok()).unwrap_or_else(|| Matrix::zeros(groups.len(), 1));
            let _ = existing.set(gi, 0, val);
            props.set_field(0, &format!("{method}_{n}"), Value::Mat(existing))?;
        }
    }
    props.set_field(0, "GroupCount", Value::Mat(counts))?;
    props.set_field(0, "Properties", table_properties(out_names, groups.len()))?;
    Ok(Value::Object(MObject::owned("table".into(), props)))
}

fn fillmissing_table(t: &Value, method: &str) -> Result<Value, LangError> {
    let Value::Object(o) = t else { return Err(LangError::new("fillmissing needs a table or matrix", 0)) };
    let names = table_names_of(t);
    let rows = table_rows_of(t)?;
    let mut props = StructArray::new();
    for name in &names {
        let col = o.prop(name).unwrap_or_else(|| Value::Mat(Matrix::zeros(0, 0)));
        let new = match &col {
            Value::Mat(m) => {
                let mut out = m.clone();
                let mut last = f64::NAN;
                for r in 0..out.numel() {
                    let v = out.get_linear(r).unwrap_or(f64::NAN);
                    if v.is_nan() {
                        let fill = match method {
                            "previous" => last,
                            "next" => (r..out.numel()).find_map(|i| out.get_linear(i).filter(|x| x.is_finite())).unwrap_or(f64::NAN),
                            "zero" | "constant" => 0.0,
                            _ => {
                                let vals: Vec<f64> = out.data().iter().copied().filter(|x| x.is_finite()).collect();
                                vals.iter().sum::<f64>() / vals.len().max(1) as f64
                            }
                        };
                        out.set_linear(r, fill);
                    } else {
                        last = v;
                    }
                }
                Value::Mat(out)
            }
            _ => col,
        };
        props.set_field(0, name, new)?;
    }
    props.set_field(0, "Properties", table_properties(names, rows))?;
    Ok(Value::Object(MObject::owned("table".into(), props)))
}

fn rmmissing_table(t: &Value) -> Result<Value, LangError> {
    let names = table_names_of(t);
    let rows = table_rows_of(t)?;
    let cols: Vec<Value> = names.iter().map(|n| table_field(t, n).unwrap_or_else(|| Value::Mat(Matrix::zeros(0, 0)))).collect();
    let keep: Vec<usize> = (0..rows)
        .filter(|&r| {
            cols.iter().all(|c| match c {
                Value::Mat(m) => m.get_linear(r).map(|x| x.is_finite()).unwrap_or(true),
                Value::Cell(c) => c.get(r, 0).map(|v| !matches!(v, Value::Str(s) if s.is_empty())).unwrap_or(true),
                _ => true,
            })
        })
        .collect();
    let mut props = StructArray::new();
    for (name, col) in names.iter().zip(&cols) {
        let vals: Vec<Value> = keep.iter().map(|&r| variable_at(col, r)).collect();
        props.set_field(0, name, column_from_cells(&vals, keep.len()))?;
    }
    props.set_field(0, "Properties", table_properties(names, keep.len()))?;
    Ok(Value::Object(MObject::owned("table".into(), props)))
}

fn setvartype_table(t: &Value, argv: &[Value]) -> Result<Value, LangError> {
    let Value::Object(o) = t else { return Err(LangError::new("setvartype needs a table", 0)) };
    let selected = parse_name_list(argv.get(1));
    let kind = match argv.get(2) {
        Some(Value::Str(s)) => s.to_ascii_lowercase(),
        _ => "double".to_string(),
    };
    let names = table_names_of(t);
    let mut out = StructArray::new();
    for name in &names {
        let value = o.prop(name).unwrap_or_else(|| Value::Mat(Matrix::zeros(0, 0)));
        let new = if selected.is_empty() || selected.iter().any(|s| s == name) {
            match kind.as_str() {
                "double" | "single" | "int8" | "int16" | "int32" | "int64" | "uint8" | "uint16" | "uint32" | "uint64" => match &value {
                    Value::Mat(_) | Value::Num(_) => value.clone(),
                    Value::Cell(c) => {
                        let mut m = Matrix::zeros(c.rows, 1);
                        for r in 0..c.rows {
                            let x = match c.get(r, 0) {
                                Some(Value::Str(s)) => s.trim().parse::<f64>().unwrap_or(f64::NAN),
                                Some(Value::Num(x)) => *x,
                                _ => f64::NAN,
                            };
                            let _ = m.set(r, 0, x);
                        }
                        Value::Mat(m)
                    }
                    _ => value.clone(),
                },
                "char" | "string" | "categorical" => match &value {
                    Value::Mat(m) => {
                        let mut items = Vec::with_capacity(m.numel());
                        for i in 0..m.numel() {
                            items.push(Value::Str(format!("{}", m.get_linear(i).unwrap_or(0.0))));
                        }
                        Value::Cell(CellArray::col(items))
                    }
                    _ => value.clone(),
                },
                "logical" => match &value {
                    Value::Mat(m) => {
                        let mut out = Matrix::zeros(m.rows(), m.cols());
                        for i in 0..m.numel() {
                            out.set_linear(i, if m.get_linear(i).unwrap_or(0.0) != 0.0 { 1.0 } else { 0.0 });
                        }
                        Value::Mat(out)
                    }
                    _ => value.clone(),
                },
                _ => value.clone(),
            }
        } else {
            value
        };
        out.set_field(0, name, new)?;
    }
    out.set_field(0, "Properties", table_properties(names, table_rows_of(t)?))?;
    Ok(Value::Object(MObject::owned("table".into(), out)))
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
            "table2cell" => Ok(vec![table2cell_value(argv.first().ok_or_else(|| LangError::new("table2cell needs a table", 0))?)?]),
            "cell2table" => {
                let c = argv.first().ok_or_else(|| LangError::new("cell2table needs a cell array", 0))?;
                let mut names = Vec::new();
                if let Some(Value::Str(k)) = argv.get(1) {
                    if k.eq_ignore_ascii_case("VariableNames") {
                        names = parse_name_list(argv.get(2));
                    }
                }
                Ok(vec![cell2table_value(c, names)?])
            }
            "struct2table" => Ok(vec![struct2table_value(argv.first().ok_or_else(|| LangError::new("struct2table needs a struct", 0))?)?]),
            "struct2cell" => Ok(vec![struct2cell_value(argv.first().ok_or_else(|| LangError::new("struct2cell needs a struct", 0))?)?]),
            "cell2struct" => {
                let c = argv.first().ok_or_else(|| LangError::new("cell2struct needs a cell array", 0))?;
                let fields = parse_name_list(argv.get(1));
                let dim = argv.get(2).and_then(|v| v.scalar().ok()).unwrap_or(1.0).max(1.0) as usize;
                Ok(vec![cell2struct_value(c, &fields, dim)?])
            }
            "addvars" => Ok(vec![addvars_table(argv.first().ok_or_else(|| LangError::new("addvars needs a table", 0))?, &argv)?]),
            "setvartype" | "convertvars" => {
                Ok(vec![setvartype_table(argv.first().ok_or_else(|| LangError::new("setvartype needs a table", 0))?, &argv)?])
            }
            "removevars" => Ok(vec![removevars_value(argv.first().ok_or_else(|| LangError::new("removevars needs a table", 0))?, &argv)?]),
            "renamevars" => Ok(vec![renamevars_value(argv.first().ok_or_else(|| LangError::new("renamevars needs a table", 0))?, &argv)?]),
            "summary" => Ok(vec![summary_value(argv.first().ok_or_else(|| LangError::new("summary needs a table", 0))?)?]),
            "join" | "innerjoin" | "outerjoin" => {
                let t1 = argv.first().ok_or_else(|| LangError::new("join needs two tables", 0))?;
                let t2 = argv.get(1).ok_or_else(|| LangError::new("join needs two tables", 0))?;
                let mut key = None;
                if let Some(Value::Str(k)) = argv.get(2) {
                    if k.eq_ignore_ascii_case("Keys") {
                        key = parse_name_list(argv.get(3)).into_iter().next();
                    }
                }
                Ok(vec![join_tables(t1, t2, key)?])
            }
            "groupsummary" => Ok(vec![groupsummary_value(argv.first().ok_or_else(|| LangError::new("groupsummary needs a table", 0))?, &argv)?]),
            "fillmissing" => {
                let method = match argv.get(1) {
                    Some(Value::Str(s)) => s.to_ascii_lowercase(),
                    _ => "previous".to_string(),
                };
                let source = argv.first().ok_or_else(|| LangError::new("fillmissing needs data", 0))?;
                match source {
                    Value::Object(o) if o.class_name == "table" => Ok(vec![fillmissing_table(source, &method)?]),
                    x => {
                        let m = x.to_matrix()?;
                        let mut out = m.clone();
                        let mut last = f64::NAN;
                        for r in 0..out.numel() {
                            let v = out.get_linear(r).unwrap_or(f64::NAN);
                            if v.is_nan() {
                                let fill = match method.as_str() {
                                    "previous" => last,
                                    "next" => (r..out.numel()).find_map(|i| out.get_linear(i).filter(|x| x.is_finite())).unwrap_or(f64::NAN),
                                    "zero" | "constant" => 0.0,
                                    _ => {
                                        let vals: Vec<f64> = out.data().iter().copied().filter(|x| x.is_finite()).collect();
                                        vals.iter().sum::<f64>() / vals.len().max(1) as f64
                                    }
                                };
                                out.set_linear(r, fill);
                            } else {
                                last = v;
                            }
                        }
                        Ok(vec![mat_or_num(out)])
                    }
                }
            }
            "standardizeMissing" => {
                let target = argv.get(1).cloned().unwrap_or(Value::Num(f64::NAN));
                let source = argv.first().ok_or_else(|| LangError::new("standardizeMissing needs data", 0))?;
                match source {
                    Value::Object(o) if o.class_name == "table" => {
                        let names = table_names_of(source);
                        let rows = table_rows_of(source)?;
                        let mut props = StructArray::new();
                        for name in &names {
                            let col = o.prop(name).unwrap_or_else(|| Value::Mat(Matrix::zeros(0, 0)));
                            let new = match (&col, &target) {
                                (Value::Mat(m), Value::Num(x)) => {
                                    let mut out = m.clone();
                                    for i in 0..out.numel() {
                                        if (out.get_linear(i).unwrap_or(f64::NAN) - x).abs() < 1e-12 {
                                            out.set_linear(i, f64::NAN);
                                        }
                                    }
                                    Value::Mat(out)
                                }
                                (Value::Cell(c), Value::Str(s)) => {
                                    let mut out = c.clone();
                                    for i in 0..out.numel() {
                                        if matches!(out.get_linear(i), Some(Value::Str(x)) if x == s) {
                                            out.set_linear(i, Value::Str(String::new()));
                                        }
                                    }
                                    Value::Cell(out)
                                }
                                _ => col,
                            };
                            props.set_field(0, name, new)?;
                        }
                        props.set_field(0, "Properties", table_properties(names, rows))?;
                        Ok(vec![Value::Object(MObject::owned("table".into(), props))])
                    }
                    x => {
                        let m = x.to_matrix()?;
                        let mut out = m.clone();
                        let target = target.scalar().unwrap_or(f64::NAN);
                        for i in 0..out.numel() {
                            if (out.get_linear(i).unwrap_or(f64::NAN) - target).abs() < 1e-12 {
                                out.set_linear(i, f64::NAN);
                            }
                        }
                        Ok(vec![mat_or_num(out)])
                    }
                }
            }
            "rmmissing" => {
                let source = argv.first().ok_or_else(|| LangError::new("rmmissing needs data", 0))?;
                match source {
                    Value::Object(o) if o.class_name == "table" => Ok(vec![rmmissing_table(source)?]),
                    x => {
                        let m = x.to_matrix()?;
                        let keep: Vec<usize> =
                            (0..m.rows()).filter(|&r| (0..m.cols()).all(|c| m.get(r, c).map(|x| x.is_finite()).unwrap_or(true))).collect();
                        Ok(vec![mat_or_num(subset_matrix_rows(&m, &keep))])
                    }
                }
            }
            "findgroups" => findgroups_value(&argv, nout),
            "splitapply" => Ok(vec![splitapply_value(self, &argv)?]),
            "varfun" => Ok(vec![varfun_value(self, &argv)?]),
            "rowfun" => Ok(vec![rowfun_value(self, &argv)?]),
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
