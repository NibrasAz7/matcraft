//! `containers.Map` — a small handle-style associative container.
//!
//! Keys and values live in parallel cell columns sorted by a canonical key
//! string, so lookups are binary searches and insertion stays deterministic.

use super::*;

fn canonical(v: &Value) -> String {
    match v {
        Value::Str(s) => format!("s:{s}"),
        Value::Num(x) => format!("n:{x}"),
        Value::Mat(m) if m.numel() == 1 => format!("n:{}", m.data().first().copied().unwrap_or(0.0)),
        other => format!("o:{}", fmt_value(other, false)),
    }
}

fn cell_items(v: Option<Value>) -> Vec<Value> {
    match v {
        Some(Value::Cell(c)) => c.items,
        _ => Vec::new(),
    }
}

fn col(items: Vec<Value>) -> Value {
    Value::Cell(CellArray::col(items))
}

fn rebuild(o: &MObject, keys: Vec<Value>, values: Vec<Value>) {
    let mut o = o.clone();
    let mut pairs: Vec<(String, Value, Value)> = keys.into_iter().zip(values).map(|(k, v)| (canonical(&k), k, v)).collect();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    let canon: Vec<Value> = pairs.iter().map(|(c, _, _)| Value::Str(c.clone())).collect();
    let keys: Vec<Value> = pairs.iter().map(|(_, k, _)| k.clone()).collect();
    let values: Vec<Value> = pairs.iter().map(|(_, _, v)| v.clone()).collect();
    let count = keys.len() as f64;
    o.set_prop("_canon", col(canon));
    o.set_prop("_keys", col(keys.clone()));
    o.set_prop("_values", col(values.clone()));
    o.set_prop("keys", col(keys));
    o.set_prop("values", col(values));
    o.set_prop("Count", Value::Num(count));
}

fn base_map(key_type: &str, value_type: &str) -> Value {
    let mut props = StructArray::new();
    let _ = props.set_field(0, "KeyType", Value::Str(key_type.to_string()));
    let _ = props.set_field(0, "ValueType", Value::Str(value_type.to_string()));
    let o = MObject::new("containers.Map".into(), props, true);
    rebuild(&o, Vec::new(), Vec::new());
    Value::Object(o)
}

fn values_from(v: &Value, n: usize) -> Vec<Value> {
    match v {
        Value::Cell(c) => c.items.clone(),
        Value::Mat(m) => m.data().iter().map(|&x| Value::Num(x)).collect(),
        Value::Num(x) => vec![Value::Num(*x)],
        Value::Str(s) => vec![Value::Str(s.clone())],
        _ if n == 1 => vec![v.clone()],
        _ => Vec::new(),
    }
}

pub(super) fn new_from_args(argv: &[Value]) -> Result<Value, LangError> {
    let mut key_type = "char".to_string();
    let mut value_type = "any".to_string();
    let mut key_arg: Option<&Value> = None;
    let mut value_arg: Option<&Value> = None;
    let mut i = 0usize;
    while i < argv.len() {
        if let Value::Str(s) = &argv[i] {
            if s.eq_ignore_ascii_case("KeyType") || s.eq_ignore_ascii_case("ValueType") {
                let val = argv.get(i + 1).and_then(|v| match v {
                    Value::Str(x) => Some(x.clone()),
                    _ => None,
                });
                if s.eq_ignore_ascii_case("KeyType") {
                    if let Some(v) = val {
                        key_type = v;
                    }
                } else if let Some(v) = val {
                    value_type = v;
                }
                i += 2;
                continue;
            }
        }
        if key_arg.is_none() {
            key_arg = Some(&argv[i]);
        } else if value_arg.is_none() {
            value_arg = Some(&argv[i]);
        }
        i += 1;
    }
    let map = base_map(&key_type, &value_type);
    let Value::Object(o) = &map else {
        return Err(LangError::new("containers.Map: internal constructor error", 0));
    };
    if let (Some(k), Some(v)) = (key_arg, value_arg) {
        let keys = match k {
            Value::Cell(c) => c.items.clone(),
            Value::Mat(m) => m.data().iter().map(|&x| Value::Num(x)).collect(),
            Value::Str(s) => vec![Value::Str(s.clone())],
            Value::Num(x) => vec![Value::Num(*x)],
            _ => return Err(LangError::new("containers.Map: unsupported key array", 0)),
        };
        let n = keys.len();
        let vals: Vec<Value> = values_from(v, n).into_iter().take(n).collect();
        if vals.len() != n {
            return Err(LangError::new("containers.Map: keys and values must have the same length", 0));
        }
        rebuild(o, keys, vals);
    }
    Ok(map)
}

pub(super) fn lookup(o: &MObject, key: &Value) -> Option<Value> {
    let needle = canonical(key);
    let canon = cell_items(o.prop("_canon"));
    let idx = canon.iter().position(|v| matches!(v, Value::Str(s) if s == &needle))?;
    cell_items(o.prop("_values")).get(idx).cloned()
}

pub(super) fn get(o: &MObject, key: &Value) -> Result<Value, LangError> {
    lookup(o, key).ok_or_else(|| LangError::new(format!("containers.Map: key not found: {}", fmt_value(key, false)), 0))
}

pub(super) fn is_key(o: &MObject, key: &Value) -> bool {
    lookup(o, key).is_some()
}

pub(super) fn set(o: &MObject, key: Value, value: Value) {
    let mut keys = cell_items(o.prop("_keys"));
    let mut values = cell_items(o.prop("_values"));
    let needle = canonical(&key);
    let pos = keys.iter().position(|k| canonical(k) == needle);
    match pos {
        Some(i) => {
            if let Some(slot) = values.get_mut(i) {
                *slot = value;
            }
        }
        None => {
            keys.push(key);
            values.push(value);
        }
    }
    rebuild(o, keys, values);
}

pub(super) fn remove(o: &MObject, key: &Value) -> Option<Value> {
    let needle = canonical(key);
    let keys = cell_items(o.prop("_keys"));
    let values = cell_items(o.prop("_values"));
    let pos = keys.iter().position(|k| canonical(k) == needle)?;
    let mut keys = keys;
    let mut values = values;
    if pos < values.len() {
        let removed = values.remove(pos);
        keys.remove(pos);
        rebuild(o, keys, values);
        Some(removed)
    } else {
        None
    }
}

pub(super) fn keys(o: &MObject) -> Value {
    o.prop("keys").unwrap_or_else(|| Value::Cell(CellArray::new(0, 1)))
}

pub(super) fn values(o: &MObject) -> Value {
    o.prop("values").unwrap_or_else(|| Value::Cell(CellArray::new(0, 1)))
}

pub(super) fn count(o: &MObject, key: &Value) -> usize {
    usize::from(is_key(o, key))
}

pub(super) fn len(o: &MObject) -> usize {
    cell_items(o.prop("_keys")).len()
}
