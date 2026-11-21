//! `loadlibrary`/`calllib`/`libpointer` — plain-C FFI over `matffi`.
//!
//! Supported (native targets only): scalar `intN`/`uintN`/`bool`/`single`/
//! `double` arguments and returns, `cstring` in/out, and `Type*Ptr`
//! pointer arguments fed either from a `lib.pointer` object (with
//! write-back to its `Value` property) or a matrix copied in.
//! Not supported: structs-by-value, `long double`, varargs, callbacks
//! into M-code. Pointer arguments stay valid for the duration of the
//! `calllib` — a callee must not retain them.

use super::*;

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use super::*;
    use matffi::{RetKind, RetVal, Scalar};
    use std::collections::HashMap;

    /// `int8`–`uint64`/`bool`/`single`/`double`/`cstring`/`void`/`X*Ptr`.
    #[derive(Debug, Clone, PartialEq)]
    pub enum CType {
        Void,
        Bool,
        CStr,
        Int { bits: u8, signed: bool },
        F32,
        F64,
        Ptr(Box<CType>),
    }

    impl CType {
        /// Parses a MATLAB-ish type name: `double`, `single`, `int32`,
        /// `uint64`, `logical`/`bool`, `cstring`/`string`, `void`,
        /// `voidPtr`, `libpointer` and `X*Ptr`/`XPtr` pointer forms.
        pub fn parse(s: &str) -> Option<CType> {
            let t = s.trim().to_ascii_lowercase().replace(' ', "");
            let t = t.strip_prefix("lib.").unwrap_or(t.as_str());
            match t {
                "void" => Some(CType::Void),
                "logical" | "bool" | "_bool" => Some(CType::Bool),
                "cstring" | "string" | "char*" | "charptr" => Some(CType::CStr),
                "single" | "float" | "float32" => Some(CType::F32),
                "double" | "float64" => Some(CType::F64),
                "voidptr" | "libpointer" | "ptr" => Some(CType::Ptr(Box::new(CType::Void))),
                "int8" | "sint8" | "char" => Some(CType::Int { bits: 8, signed: true }),
                "int16" | "short" => Some(CType::Int { bits: 16, signed: true }),
                "int32" | "int" | "long" => Some(CType::Int { bits: 32, signed: true }),
                "int64" | "longlong" => Some(CType::Int { bits: 64, signed: true }),
                "uint8" | "byte" | "uchar" => Some(CType::Int { bits: 8, signed: false }),
                "uint16" | "ushort" => Some(CType::Int { bits: 16, signed: false }),
                "uint32" | "uint" | "ulong" => Some(CType::Int { bits: 32, signed: false }),
                "uint64" | "ulonglong" | "size_t" | "uintptr" => Some(CType::Int { bits: 64, signed: false }),
                _ => {
                    // `doublePtr`, `int32Ptr`, `uint8Ptr`, …
                    let elem = t.strip_suffix("ptr")?;
                    CType::parse(elem).filter(|e| !matches!(e, CType::Void | CType::Ptr(_))).map(|e| CType::Ptr(Box::new(e)))
                }
            }
        }

        fn is_ptr(&self) -> bool {
            matches!(self, CType::Ptr(_))
        }

        fn elem(&self) -> &CType {
            match self {
                CType::Ptr(e) => e,
                other => other,
            }
        }

        /// Canonical name — stored in a `lib.pointer`'s `DataType` prop.
        fn name(&self) -> String {
            match self {
                CType::Void => "void".into(),
                CType::Bool => "bool".into(),
                CType::CStr => "cstring".into(),
                CType::F32 => "single".into(),
                CType::F64 => "double".into(),
                CType::Int { bits, signed } => format!("{}int{bits}", if *signed { "" } else { "u" }),
                CType::Ptr(e) => format!("{}Ptr", e.name()),
            }
        }
    }

    /// Declared signature for one exported function: return + arg types.
    #[derive(Debug)]
    pub struct FnSig {
        pub ret: CType,
        pub args: Vec<CType>,
    }

    /// One loaded shared library plus its declared signatures.
    #[derive(Debug)]
    pub struct LibEntry {
        pub lib: matffi::Lib,
        pub sigs: HashMap<String, FnSig>,
    }

    /// A pointer argument's owned backing store — kept alive across the
    /// call, `scalar` points into it.
    enum PtrBuf {
        Bytes(Vec<u8>),
        CString(std::ffi::CString),
    }

    impl PtrBuf {
        fn addr(&self) -> u64 {
            match self {
                PtrBuf::Bytes(b) => b.as_ptr() as u64,
                PtrBuf::CString(s) => s.as_ptr() as u64,
            }
        }
    }

    /// What a marshalled argument produces.
    struct Prepared {
        scalar: Scalar,
        keep: Option<PtrBuf>,
        /// Index of the argv slot this arg came from (for `lib.pointer`
        /// write-back) plus the element type to decode with.
        writeback: Option<(usize, CType)>,
    }

    /// Bytes-per-element for buffer-backed types.
    fn elem_size(t: &CType) -> usize {
        match t {
            CType::Bool => 1,
            CType::CStr | CType::Int { .. } => 8,
            CType::F32 => 4,
            CType::F64 => 8,
            CType::Void | CType::Ptr(_) => 8,
        }
    }

    /// Encode a numeric `Value` as little-endian C elements. Buffers are
    /// bounded at 256 MiB — arguments are untrusted input.
    const MAX_BUF: usize = 256 * 1024 * 1024;

    fn encode_numeric(v: &Value, elem: &CType) -> Result<Vec<u8>, LangError> {
        let m = v.to_matrix().map_err(|_| LangError::new("calllib: pointer argument needs numeric data", 0))?;
        let n = m.numel().saturating_mul(elem_size(elem));
        if n > MAX_BUF {
            return Err(LangError::new("calllib: buffer argument too large", 0));
        }
        let mut out = Vec::with_capacity(n);
        match elem {
            CType::F64 => {
                for &x in m.data() {
                    out.extend_from_slice(&x.to_le_bytes());
                }
            }
            CType::F32 => {
                for &x in m.data() {
                    out.extend_from_slice(&(x as f32).to_le_bytes());
                }
            }
            CType::Bool => {
                for &x in m.data() {
                    out.push(u8::from(x != 0.0));
                }
            }
            CType::Int { bits, signed: _ } => {
                let mask: u64 = if *bits >= 64 { u64::MAX } else { (1u64 << bits) - 1 };
                for &x in m.data() {
                    out.extend_from_slice(&(x as i64 as u64 & mask).to_le_bytes());
                }
            }
            _ => return Err(LangError::new("calllib: unsupported pointer element type", 0)),
        }
        Ok(out)
    }

    /// Inverse of `encode_numeric` — rebuilds a `Value` (same dims as
    /// `orig`) from the buffer a callee may have written.
    fn decode_numeric(buf: &[u8], elem: &CType, orig: &Value) -> Result<Value, LangError> {
        let m = orig.to_matrix().map_err(|_| LangError::new("calllib: pointer argument needs numeric data", 0))?;
        let count = m.numel();
        let mut data = Vec::with_capacity(count);
        // Short tails read as zeros — never a length-mismatch copy.
        let take = |i: usize, sz: usize| -> [u8; 8] {
            let mut b = [0u8; 8];
            let start = i.saturating_mul(sz);
            if let Some(chunk) = buf.get(start..start.saturating_add(sz)) {
                b[..chunk.len()].copy_from_slice(chunk);
            }
            b
        };
        match elem {
            CType::F64 => {
                for i in 0..count {
                    data.push(f64::from_le_bytes(take(i, 8)));
                }
            }
            CType::F32 => {
                for i in 0..count {
                    let mut b = [0u8; 4];
                    b.copy_from_slice(&take(i, 4)[..4]);
                    data.push(f32::from_le_bytes(b) as f64);
                }
            }
            CType::Bool => {
                for i in 0..count {
                    data.push(f64::from(take(i, 1)[0] != 0));
                }
            }
            CType::Int { bits, signed } => {
                for i in 0..count {
                    let raw = u64::from_le_bytes(take(i, 8));
                    let v = match (bits, signed) {
                        (8, true) => raw as u8 as i8 as f64,
                        (16, true) => raw as u16 as i16 as f64,
                        (32, true) => raw as u32 as i32 as f64,
                        (_, true) => raw as i64 as f64,
                        (8, false) => f64::from(raw as u8),
                        (16, false) => f64::from(raw as u16),
                        (32, false) => f64::from(raw as u32),
                        (_, false) => raw as f64,
                    };
                    data.push(v);
                }
            }
            _ => return Err(LangError::new("calllib: unsupported pointer element type", 0)),
        }
        let mat = Matrix::from_col_major(m.rows(), m.cols(), data).map_err(lang)?;
        Ok(mat_or_num(mat))
    }

    /// Does `v` carry a `lib.pointer` object?
    fn as_libpointer(v: &Value) -> Option<&MObject> {
        match v {
            Value::Object(o) if o.class_name == "lib.pointer" => Some(o),
            _ => None,
        }
    }

    /// Marshal one positional argument for the call. `decl` is the
    /// declared type when a signature exists; `None` means guess from the
    /// value (`Num`→`double`, `Str`→`cstring`, `lib.pointer`→its type).
    fn prepare_arg(idx: usize, v: &Value, decl: Option<&CType>) -> Result<Prepared, LangError> {
        let num = |v: &Value| -> Result<f64, LangError> {
            match v {
                Value::Num(x) => Ok(*x),
                Value::Mat(m) if m.numel() == 1 => Ok(m.data().first().copied().unwrap_or(0.0)),
                _ => Err(LangError::new("calllib: expected a numeric scalar argument", 0)),
            }
        };
        match decl {
            // --------- declared types ---------
            Some(CType::Bool) => Ok(Prepared { scalar: Scalar::I(u64::from(num(v)? != 0.0)), keep: None, writeback: None }),
            Some(CType::Int { .. }) => Ok(Prepared { scalar: Scalar::I(num(v)? as i64 as u64), keep: None, writeback: None }),
            Some(CType::F32) => Ok(Prepared { scalar: Scalar::S(num(v)? as f32), keep: None, writeback: None }),
            Some(CType::F64) => Ok(Prepared { scalar: Scalar::F(num(v)?), keep: None, writeback: None }),
            Some(CType::CStr) => {
                let s = match v {
                    Value::Str(s) => s.clone(),
                    other => {
                        // char arrays arrive as Str; anything else is wrong.
                        return Err(LangError::new(format!("calllib: expected a string for cstring argument, got {other:?}"), 0));
                    }
                };
                let cs = std::ffi::CString::new(s).map_err(|_| LangError::new("calllib: cstring argument contains an interior NUL", 0))?;
                let buf = PtrBuf::CString(cs);
                Ok(Prepared { scalar: Scalar::I(buf.addr()), keep: Some(buf), writeback: None })
            }
            Some(t @ CType::Ptr(elem)) => {
                if let Some(ptr) = as_libpointer(v) {
                    // `lib.pointer` — encode its `Value` into a buffer per
                    // its own `DataType`, pass the address, decode back.
                    let elem = match ptr.prop("DataType").as_ref().and_then(|d| match d {
                        Value::Str(s) => CType::parse(s),
                        _ => None,
                    }) {
                        Some(e) if e.is_ptr() => e.elem().clone(),
                        Some(e) => e,
                        None => elem.as_ref().clone(),
                    };
                    let init = ptr.prop("Value").unwrap_or(Value::Num(0.0));
                    let buf = PtrBuf::Bytes(encode_numeric(&init, &elem)?);
                    Ok(Prepared { scalar: Scalar::I(buf.addr()), keep: Some(buf), writeback: Some((idx, elem)) })
                } else if let Value::Str(s) = v {
                    // `'text'` → `char*` out/in string buffer.
                    let cs =
                        std::ffi::CString::new(s.as_str()).map_err(|_| LangError::new("calllib: cstring argument contains an interior NUL", 0))?;
                    let buf = PtrBuf::CString(cs);
                    Ok(Prepared { scalar: Scalar::I(buf.addr()), keep: Some(buf), writeback: None })
                } else {
                    match t {
                        CType::Ptr(inner) if matches!(**inner, CType::Void) => {
                            // `voidPtr` — a numeric argument is a raw
                            // address; a matrix is copied into a buffer.
                            match v {
                                Value::Num(x) => Ok(Prepared { scalar: Scalar::I(*x as u64), keep: None, writeback: None }),
                                _ => {
                                    let buf = PtrBuf::Bytes(encode_numeric(v, &CType::F64)?);
                                    Ok(Prepared { scalar: Scalar::I(buf.addr()), keep: Some(buf), writeback: None })
                                }
                            }
                        }
                        _ => {
                            // `doublePtr`/… — copy the matrix in (no
                            // write-back: use a `lib.pointer` for outputs).
                            let buf = PtrBuf::Bytes(encode_numeric(v, t.elem())?);
                            Ok(Prepared { scalar: Scalar::I(buf.addr()), keep: Some(buf), writeback: None })
                        }
                    }
                }
            }
            Some(CType::Void) => Err(LangError::new("calllib: cannot pass a value for a void parameter", 0)),
            // --------- no signature: guess ---------
            None => match v {
                Value::Num(x) => Ok(Prepared { scalar: Scalar::F(*x), keep: None, writeback: None }),
                Value::Mat(m) if m.numel() == 1 => {
                    Ok(Prepared { scalar: Scalar::F(m.data().first().copied().unwrap_or(0.0)), keep: None, writeback: None })
                }
                Value::Str(s) => {
                    let cs =
                        std::ffi::CString::new(s.as_str()).map_err(|_| LangError::new("calllib: cstring argument contains an interior NUL", 0))?;
                    let buf = PtrBuf::CString(cs);
                    Ok(Prepared { scalar: Scalar::I(buf.addr()), keep: Some(buf), writeback: None })
                }
                Value::Object(o) if o.class_name == "lib.pointer" => {
                    let elem = o
                        .prop("DataType")
                        .and_then(|d| match d {
                            Value::Str(s) => CType::parse(&s),
                            _ => None,
                        })
                        .unwrap_or(CType::F64);
                    let init = o.prop("Value").unwrap_or(Value::Num(0.0));
                    let buf = PtrBuf::Bytes(encode_numeric(&init, &elem)?);
                    Ok(Prepared { scalar: Scalar::I(buf.addr()), keep: Some(buf), writeback: Some((idx, elem)) })
                }
                _ => Err(LangError::new("calllib: unsupported argument type (numbers, strings and lib.pointer are supported)", 0)),
            },
        }
    }

    /// Interpret the raw return register per the declared return type.
    fn ret_to_value(ret: &CType, raw: RetVal) -> Result<Value, LangError> {
        let u = match raw {
            RetVal::U64(u) => u,
            RetVal::Void => return Ok(Value::Mat(Matrix::zeros(0, 0))),
            _ => 0,
        };
        Ok(match ret {
            CType::Void => Value::Mat(Matrix::zeros(0, 0)),
            CType::Bool => Value::Num(f64::from(u != 0)),
            CType::Int { bits: 8, signed: true } => Value::Num(u as u8 as i8 as f64),
            CType::Int { bits: 16, signed: true } => Value::Num(u as u16 as i16 as f64),
            CType::Int { bits: 32, signed: true } => Value::Num(u as u32 as i32 as f64),
            CType::Int { signed: true, .. } => Value::Num(u as i64 as f64),
            CType::Int { bits: 8, signed: false } => Value::Num(f64::from(u as u8)),
            CType::Int { bits: 16, signed: false } => Value::Num(f64::from(u as u16)),
            CType::Int { bits: 32, signed: false } => Value::Num(f64::from(u as u32)),
            CType::Int { signed: false, .. } => Value::Num(u as f64),
            CType::CStr => match matffi::read_cstring(u as usize) {
                Some(s) => Value::Str(s),
                None => Value::Str(String::new()),
            },
            CType::F32 => match raw {
                RetVal::F32(x) => Value::Num(x as f64),
                _ => Value::Num(f64::from(f32::from_bits(u as u32))),
            },
            CType::F64 => match raw {
                RetVal::F64(x) => Value::Num(x),
                _ => Value::Num(f64::from_bits(u)),
            },
            CType::Ptr(_) => Value::Num(u as f64),
        })
    }

    /// Platform search candidates for `loadlibrary('name')` — the name as
    /// given, then platform decorations (`x.dll` / `libx.so` / `libx.dylib`).
    fn lib_candidates(name: &str) -> Vec<std::path::PathBuf> {
        let mut out = vec![std::path::PathBuf::from(name)];
        let p = std::path::Path::new(name);
        let has_ext = p.extension().is_some();
        let stem = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| name.to_string());
        let dir = p.parent().filter(|d| !d.as_os_str().is_empty());
        let mut push = |f: String| match dir {
            Some(d) => out.push(d.join(f)),
            None => out.push(std::path::PathBuf::from(f)),
        };
        if cfg!(windows) {
            if !has_ext {
                push(format!("{stem}.dll"));
            }
        } else if cfg!(target_os = "macos") {
            if !has_ext {
                push(format!("{stem}.dylib"));
            }
            let bare = stem.strip_prefix("lib").unwrap_or(&stem);
            push(format!("lib{bare}.dylib"));
        } else {
            if !has_ext {
                push(format!("{stem}.so"));
            }
            let bare = stem.strip_prefix("lib").unwrap_or(&stem);
            push(format!("lib{bare}.so"));
        }
        out
    }

    /// `loadlibrary` signature argument: a struct mapping function name →
    /// cellstr `{ret, arg1, …}` or a whitespace-separated signature string.
    fn parse_sigs(v: &Value) -> Result<HashMap<String, FnSig>, LangError> {
        let s = match v {
            Value::Struct(s) => s,
            _ => return Err(LangError::new("loadlibrary: signatures must be a struct of {retType,argTypes…} cellstrs", 0)),
        };
        let mut out = HashMap::new();
        for name in s.field_names() {
            let spec = s.field_at(0, &name);
            let parts: Vec<String> = match spec {
                Some(Value::Cell(c)) => c
                    .items
                    .iter()
                    .filter_map(|x| match x {
                        Value::Str(t) => Some(t.clone()),
                        _ => None,
                    })
                    .collect(),
                Some(Value::Str(t)) => t.split_whitespace().map(|x| x.to_string()).collect(),
                _ => Vec::new(),
            };
            if parts.is_empty() {
                return Err(LangError::new(format!("loadlibrary: bad signature for '{name}' — expected a cellstr like {{'double','double'}}"), 0));
            }
            let ret = CType::parse(&parts[0])
                .ok_or_else(|| LangError::new(format!("loadlibrary: unknown return type '{}' in signature for '{name}'", parts[0]), 0))?;
            let it = parts.iter().skip(1);
            let mut args = Vec::with_capacity(parts.len() - 1);
            for t in it {
                args.push(CType::parse(t).ok_or_else(|| LangError::new(format!("loadlibrary: unknown type '{t}' in signature for '{name}'"), 0))?);
            }
            out.insert(name, FnSig { ret, args });
        }
        Ok(out)
    }

    impl Session {
        pub(super) fn ffi_call_native(&mut self, name: &str, v: Vec<Value>) -> Result<Vec<Value>, LangError> {
            let sarg = |i: usize| -> Option<String> {
                match v.get(i) {
                    Some(Value::Str(s)) => Some(s.clone()),
                    _ => None,
                }
            };
            match name {
                "loadlibrary" => {
                    let raw = sarg(0).ok_or_else(|| LangError::new("loadlibrary needs a library name or path", 0))?;
                    let sigs = match v.get(1) {
                        Some(spec) => parse_sigs(spec)?,
                        None => HashMap::new(),
                    };
                    let mut last_err = String::new();
                    let mut loaded = None;
                    for cand in lib_candidates(&raw) {
                        match matffi::Lib::load(&cand) {
                            Ok(l) => {
                                loaded = Some(l);
                                break;
                            }
                            Err(e) => last_err = e.to_string(),
                        }
                    }
                    let lib = loaded.ok_or_else(|| LangError::new(format!("loadlibrary: {last_err}"), 0))?;
                    let alias = lib.alias();
                    if self.libs.borrow().contains_key(&alias) {
                        return Err(LangError::new(format!("loadlibrary: '{alias}' is already loaded"), 0));
                    }
                    self.libs.borrow_mut().insert(alias, LibEntry { lib, sigs });
                    Ok(vec![])
                }
                "unloadlibrary" => {
                    match sarg(0) {
                        Some(alias) => {
                            if self.libs.borrow_mut().remove(&alias).is_none() {
                                return Err(LangError::new(format!("unloadlibrary: '{alias}' is not loaded"), 0));
                            }
                        }
                        None => self.libs.borrow_mut().clear(),
                    }
                    Ok(vec![])
                }
                "libisloaded" => {
                    let alias = sarg(0).ok_or_else(|| LangError::new("libisloaded needs a library name", 0))?;
                    let yes = self.libs.borrow().contains_key(&alias);
                    Ok(vec![Value::Num(f64::from(yes))])
                }
                "libpointer" => {
                    let dt = sarg(0).ok_or_else(|| LangError::new("libpointer needs a data type", 0))?;
                    let elem = match CType::parse(&dt) {
                        Some(CType::Ptr(e)) => *e,
                        Some(CType::CStr) => CType::Int { bits: 8, signed: false },
                        Some(t) => t,
                        None => return Err(LangError::new(format!("libpointer: unknown data type '{dt}'"), 0)),
                    };
                    let init = v.get(1).cloned().unwrap_or_else(|| {
                        // `libpointer('doublePtr')` — MATLAB preallocates
                        // with a zero value.
                        Value::Num(0.0)
                    });
                    let mut props = StructArray::new();
                    let _ = props.set_field(0, "DataType", Value::Str(elem.name()));
                    let _ = props.set_field(0, "Value", init);
                    Ok(vec![Value::Object(MObject::new("lib.pointer".into(), props, true))])
                }
                "calllib" => {
                    let alias = sarg(0).ok_or_else(|| LangError::new("calllib needs a library name", 0))?;
                    let fname = sarg(1).ok_or_else(|| LangError::new("calllib needs a function name", 0))?;
                    // Trailing `'RetType',t` / `'ArgTypes',{…}` pairs are a
                    // MatCraft extension for libraries loaded without a
                    // signature struct.
                    let mut arg_end = v.len();
                    let mut ret_decl: Option<CType> = None;
                    let mut arg_decls: Option<Vec<CType>> = None;
                    while let Some(i) = arg_end.checked_sub(2) {
                        let key = match v.get(i) {
                            Some(Value::Str(k)) => k.clone(),
                            _ => break,
                        };
                        if key.eq_ignore_ascii_case("rettype") {
                            let t = match v.get(i + 1) {
                                Some(Value::Str(t)) => CType::parse(t),
                                _ => None,
                            }
                            .ok_or_else(|| LangError::new("calllib: 'RetType' needs a type name string", 0))?;
                            ret_decl = Some(t);
                            arg_end = i;
                        } else if key.eq_ignore_ascii_case("argtypes") {
                            let ts: Vec<CType> = match v.get(i + 1) {
                                Some(Value::Cell(c)) => c
                                    .items
                                    .iter()
                                    .map(|x| match x {
                                        Value::Str(t) => {
                                            CType::parse(t).ok_or_else(|| LangError::new(format!("calllib: 'ArgTypes' — unknown type '{t}'"), 0))
                                        }
                                        _ => Err(LangError::new("calllib: 'ArgTypes' needs a cellstr of type names", 0)),
                                    })
                                    .collect::<Result<_, _>>()?,
                                _ => return Err(LangError::new("calllib: 'ArgTypes' needs a cellstr of type names", 0)),
                            };
                            arg_decls = Some(ts);
                            arg_end = i;
                        } else {
                            break;
                        }
                    }
                    let args = v.get(2..arg_end).unwrap_or(&[]);
                    let (ret, decls, addr) = {
                        let libs = self.libs.borrow();
                        let entry = libs.get(&alias).ok_or_else(|| LangError::new(format!("calllib: '{alias}' is not loaded"), 0))?;
                        let (ret, decls) = match entry.sigs.get(&fname) {
                            Some(s) => (s.ret.clone(), Some(s.args.clone())),
                            // Undeclared: default return `int64` (the
                            // ctypes convention) — doubles need an explicit
                            // signature or `'RetType','double'`.
                            None => (ret_decl.unwrap_or(CType::Int { bits: 64, signed: true }), arg_decls),
                        };
                        let addr = entry.lib.sym_addr(&fname).map_err(|e| LangError::new(e.to_string(), 0))?;
                        (ret, decls, addr)
                    };
                    if let Some(d) = &decls {
                        if d.len() != args.len() {
                            return Err(LangError::new(format!("calllib: '{fname}' takes {} arguments, {} given", d.len(), args.len()), 0));
                        }
                    }
                    let mut preps = Vec::with_capacity(args.len());
                    for (i, a) in args.iter().enumerate() {
                        preps.push(prepare_arg(i, a, decls.as_deref().and_then(|d| d.get(i)))?);
                    }
                    let scalars: Vec<Scalar> = preps.iter().map(|p| p.scalar).collect();
                    let retspec = match ret {
                        CType::Void => RetKind::Void,
                        CType::F32 => RetKind::F32,
                        CType::F64 => RetKind::F64,
                        _ => RetKind::U64,
                    };
                    let raw = matffi::invoke(addr, retspec, &scalars).map_err(|e| LangError::new(e.to_string(), 0))?;
                    // Write pointer outputs back into the `lib.pointer`s'
                    // `Value` properties (handle semantics — cloned arg
                    // objects share the store).
                    for prep in &preps {
                        if let (Some((argi, elem)), Some(PtrBuf::Bytes(bytes))) = (&prep.writeback, &prep.keep) {
                            if let Some(Value::Object(o)) = args.get(*argi) {
                                let orig = o.prop("Value").unwrap_or(Value::Num(0.0));
                                let mut obj = o.clone();
                                obj.set_prop("Value", decode_numeric(bytes, elem, &orig)?);
                            }
                        }
                    }
                    Ok(vec![ret_to_value(&ret, raw)?])
                }
                "libfunctions" | "libfunctionsview" => {
                    let alias = sarg(0).ok_or_else(|| LangError::new(format!("{name} needs a library name"), 0))?;
                    let names = {
                        let libs = self.libs.borrow();
                        match libs.get(&alias) {
                            Some(e) => e.lib.exports().map_err(|e| LangError::new(e.to_string(), 0))?,
                            None => return Err(LangError::new(format!("{name}: '{alias}' is not loaded"), 0)),
                        }
                    };
                    if name == "libfunctionsview" {
                        self.print_buf.push_str(&format!("Functions in library {alias}:\n"));
                        for n in &names {
                            self.print_buf.push_str(&format!("  {n}\n"));
                        }
                    }
                    Ok(vec![Value::Cell(CellArray::col(names.into_iter().map(Value::Str).collect()))])
                }
                _ => Err(LangError::new(format!("unknown ffi builtin '{name}'"), 0)),
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use imp::LibEntry;

impl Session {
    /// `loadlibrary`-family dispatch — errors cleanly on wasm.
    pub(super) fn ffi_call(&mut self, name: &str, v: Vec<Value>) -> Result<Vec<Value>, LangError> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.ffi_call_native(name, v)
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = v;
            Err(LangError::new(format!("{name} is not available on wasm"), 0))
        }
    }
}
