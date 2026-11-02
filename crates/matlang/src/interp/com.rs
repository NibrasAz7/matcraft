//! Safe Windows COM automation bridge.
//!
//! The core stays `unsafe`-free: a long-lived PowerShell process owns the
//! COM object and speaks a tiny JSON-lines protocol over stdin/stdout.

use super::*;

#[cfg(all(windows, not(target_arch = "wasm32")))]
mod imp {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Child, ChildStdin, Command, Stdio};

    pub struct ComSession {
        child: Child,
        stdin: ChildStdin,
        stdout: BufReader<std::process::ChildStdout>,
        progid: String,
    }

    impl std::fmt::Debug for ComSession {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("ComSession").field("progid", &self.progid).finish()
        }
    }

    impl Drop for ComSession {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    impl ComSession {
        pub fn start(progid: &str) -> Result<Self, LangError> {
            if progid.is_empty() || !progid.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+')) {
                return Err(LangError::new("actxserver: invalid COM ProgID", 0));
            }
            if !progid.contains('.') {
                return Err(LangError::new("actxserver: COM ProgID must contain a '.'", 0));
            }
            let script = format!(
                r#"$ErrorActionPreference='Stop'
$prog='{progid}'
$obj=New-Object -ComObject $prog
Write-Output '{{"ok":true}}'
while ($true) {{
  $line = [Console]::In.ReadLine()
  if ($null -eq $line) {{ break }}
  try {{
    $c = $line | ConvertFrom-Json
    $done = $false
    $r = $null
    if ($c.op -eq 'release') {{ $done = $true }}
    elseif ($c.op -eq 'get') {{ $r = $obj.($c.name) }}
    elseif ($c.op -eq 'set') {{ $obj.($c.name) = $c.value }}
    elseif ($c.op -eq 'invoke') {{
      $m = $obj.PSObject.Methods[$c.name]
      if ($null -eq $m) {{ throw "member '$($c.name)' not found" }}
      $a = @()
      if ($null -ne $c.args) {{ $a = @($c.args) }}
      $r = $m.Invoke($a)
    }} else {{ throw 'unknown COM bridge operation' }}
    if (($c.op -eq 'get' -or $c.op -eq 'invoke') -and ($r -is [System.__ComObject])) {{
      throw "member '$($c.name)' returned a COM object; call a scalar-returning member"
    }}
    $out = @{{ok=$true; value=$r}} | ConvertTo-Json -Compress -Depth 12
    Write-Output $out
    if ($done) {{ break }}
  }} catch {{
    $out = @{{ok=$false; error=$_.Exception.Message}} | ConvertTo-Json -Compress
    Write-Output $out
  }}
}}
"#
            );
            let mut cmd = Command::new("powershell");
            cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &script])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
            }
            let mut child = cmd.spawn().map_err(|e| LangError::new(format!("actxserver: cannot start PowerShell: {e}"), 0))?;
            let stdin = child.stdin.take().ok_or_else(|| LangError::new("actxserver: COM bridge has no stdin", 0))?;
            let stdout = child.stdout.take().ok_or_else(|| LangError::new("actxserver: COM bridge has no stdout", 0))?;
            let mut session = ComSession { child, stdin, stdout: BufReader::new(stdout), progid: progid.into() };
            let mut ready = String::new();
            session.stdout.read_line(&mut ready).map_err(|e| LangError::new(format!("actxserver: cannot read COM bridge: {e}"), 0))?;
            let j: serde_json::Value =
                serde_json::from_str(ready.trim()).map_err(|_| LangError::new(format!("actxserver: COM ProgID '{progid}' is unavailable"), 0))?;
            if j.get("ok").and_then(|v| v.as_bool()) != Some(true) {
                return Err(LangError::new(format!("actxserver: COM ProgID '{progid}' is unavailable"), 0));
            }
            Ok(session)
        }

        fn request(&mut self, cmd: &serde_json::Value) -> Result<serde_json::Value, LangError> {
            let line = serde_json::to_string(cmd).map_err(|e| LangError::new(format!("actxserver: {e}"), 0))?;
            writeln!(self.stdin, "{line}").map_err(|e| LangError::new(format!("actxserver: COM bridge write failed: {e}"), 0))?;
            self.stdin.flush().map_err(|e| LangError::new(format!("actxserver: COM bridge flush failed: {e}"), 0))?;
            let mut resp = String::new();
            let n = self.stdout.read_line(&mut resp).map_err(|e| LangError::new(format!("actxserver: COM bridge read failed: {e}"), 0))?;
            if n == 0 {
                return Err(LangError::new("actxserver: COM bridge exited", 0));
            }
            let j: serde_json::Value =
                serde_json::from_str(resp.trim()).map_err(|e| LangError::new(format!("actxserver: invalid COM response: {e}"), 0))?;
            if j.get("ok").and_then(|v| v.as_bool()) != Some(true) {
                let msg = j.get("error").and_then(|v| v.as_str()).unwrap_or("unknown COM error");
                return Err(LangError::new(format!("actxserver: {msg}"), 0));
            }
            Ok(j.get("value").cloned().unwrap_or(serde_json::Value::Null))
        }

        pub fn get(&mut self, name: &str) -> Result<Value, LangError> {
            let j = self.request(&serde_json::json!({"op": "get", "name": name}))?;
            json_to_value(&j)
        }

        pub fn set(&mut self, name: &str, v: &Value) -> Result<(), LangError> {
            let j = value_to_json(v)?;
            let _ = self.request(&serde_json::json!({"op": "set", "name": name, "value": j}))?;
            Ok(())
        }

        pub fn invoke(&mut self, name: &str, args: &[Value]) -> Result<Value, LangError> {
            let a: Result<Vec<_>, _> = args.iter().map(value_to_json).collect();
            let j = self.request(&serde_json::json!({"op": "invoke", "name": name, "args": a?}))?;
            json_to_value(&j)
        }

        pub fn release(&mut self) {
            let _ = self.request(&serde_json::json!({"op": "release"}));
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[cfg(not(all(windows, not(target_arch = "wasm32"))))]
mod imp {
    use super::*;

    #[derive(Debug)]
    pub struct ComSession;

    impl ComSession {
        pub fn start(_progid: &str) -> Result<Self, LangError> {
            Err(LangError::new("actxserver: COM automation is only available on Windows", 0))
        }
        pub fn get(&mut self, _name: &str) -> Result<Value, LangError> {
            Err(LangError::new("actxserver: COM automation is only available on Windows", 0))
        }
        pub fn set(&mut self, _name: &str, _v: &Value) -> Result<(), LangError> {
            Err(LangError::new("actxserver: COM automation is only available on Windows", 0))
        }
        pub fn invoke(&mut self, _name: &str, _args: &[Value]) -> Result<Value, LangError> {
            Err(LangError::new("actxserver: COM automation is only available on Windows", 0))
        }
        pub fn release(&mut self) {}
    }
}

pub(super) use imp::ComSession;

fn json_to_value(j: &serde_json::Value) -> Result<Value, LangError> {
    match j {
        serde_json::Value::Null => Ok(Value::Mat(Matrix::zeros(0, 0))),
        serde_json::Value::Bool(b) => Ok(Value::Num(if *b { 1.0 } else { 0.0 })),
        serde_json::Value::Number(n) => Ok(Value::Num(n.as_f64().unwrap_or(f64::NAN))),
        serde_json::Value::String(s) => Ok(Value::Str(s.clone())),
        serde_json::Value::Array(a) => {
            if a.iter().all(|x| x.is_number()) {
                let data: Vec<f64> = a.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect();
                return Ok(Value::Mat(Matrix::row_vector(data)));
            }
            let nested_numeric = !a.is_empty()
                && a.iter().all(|row| row.as_array().is_some_and(|r| r.iter().all(|x| x.is_number())))
                && a.iter()
                    .filter_map(|row| row.as_array().map(Vec::len))
                    .all(|n| n == a.first().and_then(|r| r.as_array()).map(Vec::len).unwrap_or(0));
            if nested_numeric {
                let rows = a.len();
                let cols = a.first().and_then(|r| r.as_array()).map(Vec::len).unwrap_or(0);
                let mut data = Vec::with_capacity(rows.saturating_mul(cols));
                for c in 0..cols {
                    for row in a {
                        data.push(row.as_array().and_then(|r| r.get(c)).and_then(|x| x.as_f64()).unwrap_or(0.0));
                    }
                }
                return Matrix::from_col_major(rows, cols, data).map(Value::Mat).map_err(|e| LangError::new(format!("actxserver: {e}"), 0));
            }
            let items: Result<Vec<Value>, LangError> = a.iter().map(json_to_value).collect();
            Ok(Value::Cell(CellArray::row(items?)))
        }
        serde_json::Value::Object(_) => Err(LangError::new("actxserver: cannot convert a COM object result", 0)),
    }
}

fn value_to_json(v: &Value) -> Result<serde_json::Value, LangError> {
    Ok(match v {
        Value::Num(x) => serde_json::json!(x),
        Value::Mat(m) if m.numel() == 1 => serde_json::json!(m.as_scalar().unwrap_or(0.0)),
        Value::Mat(m) => serde_json::json!(m.data()),
        Value::Str(s) => serde_json::json!(s),
        Value::Cell(c) => {
            let items: Result<Vec<_>, _> = (0..c.numel()).map(|i| value_to_json(&c.get_linear(i).cloned().unwrap_or(Value::Num(0.0)))).collect();
            serde_json::json!(items?)
        }
        other => return Err(LangError::new(format!("actxserver: cannot pass {other:?} to COM"), 0)),
    })
}
