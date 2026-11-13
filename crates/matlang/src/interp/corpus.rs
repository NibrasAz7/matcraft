//! Corpus class helpers: `inputParser`, `string`/`strings`, `table`.
//!
//! `inputParser` is a handle object so `addParameter(p, ...)` and
//! `parse(p, ...)` mutate the object created by `p = inputParser`, matching
//! MATLAB's handle-class behaviour. Tables are ordinary `table` objects with
//! `Properties.VariableNames`, `RowNames` and `NumRows`.

use super::*;

fn empty_value() -> Value {
    Value::Mat(Matrix::zeros(0, 0))
}

fn cell_values(v: &Value) -> Vec<Value> {
    match v {
        Value::Cell(c) => c.items.clone(),
        _ => Vec::new(),
    }
}

fn cell_strings(v: &Value) -> Vec<String> {
    match v {
        Value::Cell(c) => c
            .items
            .iter()
            .filter_map(|x| match x {
                Value::Str(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        Value::Str(s) => vec![s.clone()],
        _ => Vec::new(),
    }
}

fn cell_of(values: Vec<Value>) -> Value {
    Value::Cell(CellArray::row(values))
}

fn parser_prop(o: &MObject, name: &str) -> Vec<Value> {
    o.prop(name).map(|v| cell_values(&v)).unwrap_or_default()
}

fn parser_names(o: &MObject, name: &str) -> Vec<String> {
    o.prop(name).map(|v| cell_strings(&v)).unwrap_or_default()
}

fn set_parser_cells(o: &mut MObject, name: &str, values: Vec<Value>) {
    o.set_prop(name, cell_of(values));
}

fn parser_new() -> Value {
    let mut props = StructArray::new();
    let _ = props.set_field(0, "RequiredNames", cell_of(Vec::new()));
    let _ = props.set_field(0, "RequiredValidators", cell_of(Vec::new()));
    let _ = props.set_field(0, "OptionalNames", cell_of(Vec::new()));
    let _ = props.set_field(0, "OptionalDefaults", cell_of(Vec::new()));
    let _ = props.set_field(0, "OptionalValidators", cell_of(Vec::new()));
    let _ = props.set_field(0, "ParamNames", cell_of(Vec::new()));
    let _ = props.set_field(0, "ParamDefaults", cell_of(Vec::new()));
    let _ = props.set_field(0, "ParamValidators", cell_of(Vec::new()));
    let _ = props.set_field(0, "Results", Value::Struct(StructArray::new()));
    let _ = props.set_field(0, "Unmatched", cell_of(Vec::new()));
    let _ = props.set_field(0, "KeepUnmatched", Value::Num(0.0));
    let _ = props.set_field(0, "PartialMatching", Value::Num(1.0));
    let _ = props.set_field(0, "CaseSensitive", Value::Num(0.0));
    Value::Object(MObject::new("inputParser".to_string(), props, true))
}

fn parser_add(o: &mut MObject, kind: u8, name: &str, default: Option<Value>, validator: Option<Value>) {
    let (names_field, defaults_field, validators_field) = match kind {
        0 => ("RequiredNames", "", "RequiredValidators"),
        1 => ("OptionalNames", "OptionalDefaults", "OptionalValidators"),
        _ => ("ParamNames", "ParamDefaults", "ParamValidators"),
    };
    let mut names = parser_prop(o, names_field);
    names.push(Value::Str(name.to_string()));
    set_parser_cells(o, names_field, names);
    let mut validators = parser_prop(o, validators_field);
    validators.push(validator.unwrap_or_else(empty_value));
    set_parser_cells(o, validators_field, validators);
    if !defaults_field.is_empty() {
        let mut defaults = parser_prop(o, defaults_field);
        defaults.push(default.unwrap_or_else(empty_value));
        set_parser_cells(o, defaults_field, defaults);
    }
}

fn parameter_index(names: &[String], needle: &str, case_sensitive: bool, partial: bool) -> Option<usize> {
    let norm = |s: &str| if case_sensitive { s.to_string() } else { s.to_ascii_lowercase() };
    let needle = norm(needle);
    if let Some(i) = names.iter().position(|n| norm(n) == needle) {
        return Some(i);
    }
    if partial && !needle.is_empty() {
        let hits: Vec<usize> = names.iter().enumerate().filter(|(_, n)| norm(n).starts_with(&needle)).map(|(i, _)| i).collect();
        if hits.len() == 1 {
            return hits.first().copied();
        }
    }
    None
}

impl Session {
    fn parser_validate(&mut self, validator: &Value, value: &Value, name: &str) -> Result<(), LangError> {
        if matches!(validator, Value::Mat(m) if m.numel() == 0) {
            return Ok(());
        }
        let out = self.call_feval_target(validator, vec![value.clone()], 1)?;
        let ok = out.first().map(|v| v.truthy()).transpose()?.unwrap_or(true);
        if !ok {
            return Err(LangError::new(format!("inputParser: value for '{name}' failed validation"), 0));
        }
        Ok(())
    }

    pub(super) fn corpus_builtin(&mut self, name: &str, argv: Vec<Value>, _nout: usize) -> Result<Vec<Value>, LangError> {
        match name {
            "inputParser" => Ok(vec![parser_new()]),
            "addRequired" | "addOptional" | "addParameter" | "addParamValue" | "addparameter" => {
                let mut o = match argv.first() {
                    Some(Value::Object(o)) if o.class_name == "inputParser" => o.clone(),
                    _ => return Err(LangError::new(format!("{name} needs an inputParser object"), 0)),
                };
                let key = match argv.get(1) {
                    Some(Value::Str(s)) => s.clone(),
                    _ => return Err(LangError::new(format!("{name} needs a parameter name"), 0)),
                };
                match name {
                    "addRequired" => parser_add(&mut o, 0, &key, None, argv.get(2).cloned()),
                    "addOptional" => {
                        let default = argv.get(2).cloned().ok_or_else(|| LangError::new("addOptional needs a default", 0))?;
                        parser_add(&mut o, 1, &key, Some(default), argv.get(3).cloned());
                    }
                    _ => {
                        let default = argv.get(2).cloned().ok_or_else(|| LangError::new("addParameter needs a default", 0))?;
                        parser_add(&mut o, 2, &key, Some(default), argv.get(3).cloned());
                    }
                }
                Ok(vec![])
            }
            "parse" => {
                let mut o = match argv.first() {
                    Some(Value::Object(o)) if o.class_name == "inputParser" => o.clone(),
                    _ => return Err(LangError::new("parse needs an inputParser object", 0)),
                };
                let required = parser_names(&o, "RequiredNames");
                let required_validators = parser_prop(&o, "RequiredValidators");
                let optional = parser_names(&o, "OptionalNames");
                let optional_defaults = parser_prop(&o, "OptionalDefaults");
                let optional_validators = parser_prop(&o, "OptionalValidators");
                let params = parser_names(&o, "ParamNames");
                let param_defaults = parser_prop(&o, "ParamDefaults");
                let param_validators = parser_prop(&o, "ParamValidators");
                let case_sensitive = o.prop("CaseSensitive").and_then(|v| v.scalar().ok()).unwrap_or(0.0) != 0.0;
                let partial = o.prop("PartialMatching").and_then(|v| v.scalar().ok()).unwrap_or(1.0) != 0.0;
                let keep_unmatched = o.prop("KeepUnmatched").and_then(|v| v.scalar().ok()).unwrap_or(0.0) != 0.0;
                let mut results = StructArray::new();
                for (i, pname) in params.iter().enumerate() {
                    if let Some(v) = param_defaults.get(i) {
                        results.set_field(0, pname, v.clone())?;
                    }
                }
                let mut pos = 1usize;
                for (i, rname) in required.iter().enumerate() {
                    let value =
                        argv.get(pos).cloned().ok_or_else(|| LangError::new(format!("inputParser: missing required argument '{rname}'"), 0))?;
                    if let Some(val) = required_validators.get(i) {
                        self.parser_validate(val, &value, rname)?;
                    }
                    results.set_field(0, rname, value)?;
                    pos += 1;
                }
                for (i, oname) in optional.iter().enumerate() {
                    let next_is_param = match argv.get(pos) {
                        Some(Value::Str(s)) => parameter_index(&params, s, case_sensitive, partial).is_some(),
                        _ => false,
                    };
                    let value = if !next_is_param && pos < argv.len() {
                        let v = argv.get(pos).cloned().unwrap_or_else(empty_value);
                        pos += 1;
                        v
                    } else {
                        optional_defaults.get(i).cloned().unwrap_or_else(empty_value)
                    };
                    if let Some(val) = optional_validators.get(i) {
                        self.parser_validate(val, &value, oname)?;
                    }
                    results.set_field(0, oname, value)?;
                }
                let mut unmatched = Vec::new();
                while pos < argv.len() {
                    let key = match argv.get(pos) {
                        Some(Value::Str(s)) => s.clone(),
                        _ => return Err(LangError::new("inputParser: expected a parameter name", 0)),
                    };
                    let value = argv.get(pos + 1).cloned().ok_or_else(|| LangError::new(format!("inputParser: missing value for '{key}'"), 0))?;
                    pos += 2;
                    match parameter_index(&params, &key, case_sensitive, partial) {
                        Some(i) => {
                            if let Some(val) = param_validators.get(i) {
                                self.parser_validate(val, &value, &params[i])?;
                            }
                            results.set_field(0, &params[i], value)?;
                        }
                        None if keep_unmatched => {
                            unmatched.push(Value::Str(key));
                            unmatched.push(value);
                        }
                        None => return Err(LangError::new(format!("inputParser: unknown parameter '{key}'"), 0)),
                    }
                }
                o.set_prop("Results", Value::Struct(results));
                o.set_prop("Unmatched", cell_of(unmatched));
                Ok(vec![])
            }
            "string" => {
                let value = argv.first().cloned().unwrap_or_else(empty_value);
                Ok(vec![string_value(&value)?])
            }
            "strings" => {
                let (rows, cols) = string_dims(&argv)?;
                check_elems(rows.saturating_mul(cols))?;
                let mut c = CellArray::new(rows, cols);
                for i in 0..rows.saturating_mul(cols) {
                    c.set_linear(i, Value::Str(String::new()));
                }
                Ok(vec![Value::Cell(c)])
            }
            "isstring" => {
                let yes = match argv.first() {
                    Some(Value::Str(_)) => true,
                    Some(Value::Cell(c)) => c.items.iter().all(|v| matches!(v, Value::Str(_))),
                    _ => false,
                };
                Ok(vec![Value::Num(if yes { 1.0 } else { 0.0 })])
            }
            "table2array" => Ok(vec![table2array(argv.first())?]),
            _ => Err(LangError::new(format!("{name}: not a corpus builtin"), 0)),
        }
    }

    pub(super) fn table_call(&mut self, args: &[crate::ast::Expr]) -> Result<Option<Value>, LangError> {
        let mut values = Vec::new();
        let mut default_names = Vec::new();
        let mut explicit_names: Option<Vec<String>> = None;
        let mut i = 0usize;
        while i < args.len() {
            if let crate::ast::Expr::Str(key) = &args[i] {
                if key.eq_ignore_ascii_case("VariableNames") {
                    let v = args.get(i + 1).ok_or_else(|| LangError::new("table: VariableNames needs a value", 0))?;
                    let nv = self.eval(v)?;
                    explicit_names = Some(cell_strings(&nv));
                    i += 2;
                    continue;
                }
                if key.eq_ignore_ascii_case("RowNames") {
                    i += 2;
                    continue;
                }
            }
            let value = self.eval(&args[i])?;
            let name = match &args[i] {
                crate::ast::Expr::Ident(n) if !n.is_empty() => n.clone(),
                _ => format!("Var{}", values.len() + 1),
            };
            default_names.push(name);
            values.push(value);
            i += 1;
        }
        let names = explicit_names.unwrap_or(default_names);
        if names.len() != values.len() {
            return Err(LangError::new("table: VariableNames must match the number of variables", 0));
        }
        let rows = values.first().map(table_rows).unwrap_or(0);
        let mut props = StructArray::new();
        for (name, value) in names.iter().zip(values) {
            props.set_field(0, name, value)?;
        }
        let mut p = StructArray::new();
        p.set_field(0, "VariableNames", cell_of(names.iter().cloned().map(Value::Str).collect()))?;
        p.set_field(0, "RowNames", cell_of(Vec::new()))?;
        p.set_field(0, "DimensionNames", cell_of(vec![Value::Str("Row".into()), Value::Str("Variables".into())]))?;
        p.set_field(0, "NumRows", Value::Num(rows as f64))?;
        props.set_field(0, "Properties", Value::Struct(p))?;
        Ok(Some(Value::Object(MObject::owned("table".to_string(), props))))
    }
}

fn table_rows(v: &Value) -> usize {
    match v {
        Value::Mat(m) => {
            if m.rows() > 1 || m.cols() == 1 {
                m.rows()
            } else {
                1
            }
        }
        Value::Cell(c) => c.rows,
        Value::Struct(s) => s.rows,
        _ => 1,
    }
}

fn string_value(v: &Value) -> Result<Value, LangError> {
    match v {
        Value::Str(s) => Ok(Value::Str(s.clone())),
        Value::Num(x) => Ok(Value::Str(crate::interp::fmt_num(*x, false))),
        Value::Mat(m) if m.numel() == 1 => Ok(Value::Str(crate::interp::fmt_num(m.data().first().copied().unwrap_or(0.0), false))),
        Value::Mat(m) => {
            let mut c = CellArray::new(m.rows(), m.cols());
            for r in 0..m.rows() {
                for col in 0..m.cols() {
                    c.set(r, col, Value::Str(crate::interp::fmt_num(m.get(r, col).unwrap_or(0.0), false)));
                }
            }
            Ok(Value::Cell(c))
        }
        Value::Cell(c) => {
            let mut out = CellArray::new(c.rows, c.cols);
            for (i, item) in c.items.iter().enumerate() {
                let s = match item {
                    Value::Str(s) => s.clone(),
                    Value::Num(x) => crate::interp::fmt_num(*x, false),
                    other => fmt_value(other, false).to_string(),
                };
                out.set_linear(i, Value::Str(s));
            }
            Ok(Value::Cell(out))
        }
        _ => Err(LangError::new("string: unsupported value", 0)),
    }
}

fn string_dims(argv: &[Value]) -> Result<(usize, usize), LangError> {
    match argv {
        [] => Ok((1, 1)),
        [v] => {
            let m = v.to_numeric()?;
            if m.numel() == 2 {
                Ok((m.data().first().copied().unwrap_or(0.0).max(0.0) as usize, m.data().get(1).copied().unwrap_or(0.0).max(0.0) as usize))
            } else {
                let n = m.data().first().copied().unwrap_or(0.0).max(0.0) as usize;
                Ok((n, n))
            }
        }
        [r, c, ..] => Ok((r.scalar()?.max(0.0) as usize, c.scalar()?.max(0.0) as usize)),
    }
}

fn table2array(v: Option<&Value>) -> Result<Value, LangError> {
    let o = match v {
        Some(Value::Object(o)) if o.class_name == "table" => o,
        _ => return Err(LangError::new("table2array needs a table", 0)),
    };
    let names = o
        .prop("Properties")
        .and_then(|p| match p {
            Value::Struct(s) => s.field_at(0, "VariableNames").cloned(),
            _ => None,
        })
        .map(|x| cell_strings(&x))
        .unwrap_or_default();
    let mut mat: Option<Matrix> = None;
    for name in names {
        let value = o.prop(&name).ok_or_else(|| LangError::new(format!("table variable '{name}' is missing"), 0))?;
        let m = value.to_matrix()?;
        mat = Some(match mat {
            None => m,
            Some(prev) => {
                if prev.rows() != m.rows() {
                    return Err(LangError::new("table2array: variables have different heights", 0));
                }
                Matrix::hcat(&prev, &m).map_err(lang)?
            }
        });
    }
    Ok(mat_or_num(mat.unwrap_or_else(|| Matrix::zeros(0, 0))))
}
