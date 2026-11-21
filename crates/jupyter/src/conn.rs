//! Kernel connection file — the JSON `jupyter` writes when launching a
//! kernel: transport, ports per channel and the HMAC signing key.

use serde_json::Value;

/// Parsed kernel connection file.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectionInfo {
    /// `"tcp"` is the only transport Jupyter launches today.
    pub transport: String,
    pub ip: String,
    /// Signing key (empty when the client disabled signing).
    pub key: Vec<u8>,
    /// e.g. `"hmac-sha256"`.
    pub signature_scheme: String,
    pub kernel_name: String,
    pub shell_port: u16,
    pub iopub_port: u16,
    pub stdin_port: u16,
    pub control_port: u16,
    pub hb_port: u16,
}

impl ConnectionInfo {
    /// Parse the JSON contents of a connection file.
    pub fn parse(json_text: &str) -> Result<Self, String> {
        let v: Value = serde_json::from_str(json_text).map_err(|e| format!("connection file: {e}"))?;
        let get = |k: &str| -> Result<&Value, String> { v.get(k).ok_or_else(|| format!("connection file: missing '{k}'")) };
        let str_of = |k: &str| -> Result<String, String> {
            get(k).and_then(|v| v.as_str().ok_or_else(|| format!("'{k}' is not a string"))).map(str::to_string)
        };
        let port_of = |k: &str| -> Result<u16, String> {
            let n = get(k)?.as_u64().ok_or_else(|| format!("'{k}' is not a number"))?;
            u16::try_from(n).map_err(|_| format!("'{k}' port {n} out of range"))
        };
        let scheme = str_of("signature_scheme")?;
        if !scheme.is_empty() && !scheme.eq_ignore_ascii_case("hmac-sha256") {
            return Err(format!("signature scheme '{scheme}' unsupported (need hmac-sha256)"));
        }
        Ok(ConnectionInfo {
            transport: str_of("transport").unwrap_or_else(|_| "tcp".into()),
            ip: str_of("ip")?,
            key: str_of("key")?.into_bytes(),
            signature_scheme: scheme,
            kernel_name: str_of("kernel_name").unwrap_or_default(),
            shell_port: port_of("shell_port")?,
            iopub_port: port_of("iopub_port")?,
            stdin_port: port_of("stdin_port")?,
            control_port: port_of("control_port")?,
            hb_port: port_of("hb_port")?,
        })
    }

    /// Read and parse a connection file from disk.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn read(path: &std::path::Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&text)
    }

    /// `transport://ip:port` socket address for one channel.
    pub fn addr(&self, port: u16) -> String {
        format!("{}://{}:{}", self.transport, self.ip, port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_connection_file() {
        let ci = ConnectionInfo::parse(
            r#"{"transport":"tcp","ip":"127.0.0.1","key":"abc-def",
                "signature_scheme":"hmac-sha256","kernel_name":"matcraft",
                "shell_port":60101,"iopub_port":60102,"stdin_port":60103,
                "control_port":60104,"hb_port":60105}"#,
        )
        .unwrap();
        assert_eq!(ci.shell_port, 60101);
        assert_eq!(ci.addr(ci.iopub_port), "tcp://127.0.0.1:60102");
        assert_eq!(ci.key, b"abc-def");
        assert!(ConnectionInfo::parse("{}").is_err());
        assert!(ConnectionInfo::parse("{\"ip\":\"x\",\"key\":\"\",\"signature_scheme\":\"hmac-sha512\"}").is_err());
    }
}
