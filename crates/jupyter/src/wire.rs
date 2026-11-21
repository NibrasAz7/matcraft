//! Jupyter wire messages: the multipart frame model and HMAC-SHA256
//! signing shared by every channel (shell, control, iopub, stdin).
//!
//! On the wire a message is `[idents…, "<IDS|MSG>", hmac, header,
//! parent_header, metadata, content, extra…]` where the four JSON frames
//! are signed as the HMAC of their concatenated UTF-8 bytes.

use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;

/// Frame separating ZMQ routing identities from the signed message body.
pub const DELIM: &[u8] = b"<IDS|MSG>";
const PROTOCOL: &str = "5.3";

/// HMAC-SHA256 over the concatenated JSON frames, hex-encoded.
pub fn hmac_sign(key: &[u8], frames: &[&[u8]]) -> String {
    if key.is_empty() {
        return String::new();
    }
    let mut mac = match <Hmac<Sha256> as Mac>::new_from_slice(key) {
        Ok(m) => m,
        Err(_) => return String::new(),
    };
    for f in frames {
        mac.update(f);
    }
    hex::encode(mac.finalize().into_bytes())
}

/// One logical Jupyter message.
#[derive(Debug, Clone)]
pub struct Message {
    pub msg_id: String,
    pub session: String,
    pub username: String,
    pub msg_type: String,
    pub content: Value,
}

impl Message {
    /// Serialise to header JSON.
    fn header(&self) -> Value {
        serde_json::json!({
            "msg_id": self.msg_id,
            "username": self.username,
            "session": self.session,
            "date": "",
            "msg_type": self.msg_type,
            "version": PROTOCOL,
        })
    }
}

/// A message with routing identities and parent linkage — the unit a
/// transport ferries. `parent` is the full JSON header of the request
/// being answered (Jupyter correlates replies through it).
#[derive(Debug, Clone)]
pub struct WireMessage {
    /// ZMQ routing identities copied from the request (empty on iopub).
    pub idents: Vec<Vec<u8>>,
    /// The channel this message belongs to (`shell`, `iopub`, …).
    pub channel: &'static str,
    pub message: Message,
    /// Raw JSON header of the request this replies to (for `parent_header`).
    pub parent_header: Value,
    /// Raw header frame of the originating request, kept so the transport
    /// can echo routing info unchanged.
    pub orig_header: Value,
}

impl WireMessage {
    /// Frames for the transport: `[idents…, DELIM, sig, header, parent,
    /// metadata, content]`.
    pub fn frames(&self, key: &[u8]) -> Vec<Vec<u8>> {
        let header = self.message.header().to_string();
        let parent = self.parent_header.to_string();
        let metadata = "{}".to_string();
        let content = self.message.content.to_string();
        let sig = hmac_sign(key, &[header.as_bytes(), parent.as_bytes(), metadata.as_bytes(), content.as_bytes()]);
        let mut out = self.idents.clone();
        out.push(DELIM.to_vec());
        out.push(sig.into_bytes());
        out.push(header.into_bytes());
        out.push(parent.into_bytes());
        out.push(metadata.into_bytes());
        out.push(content.into_bytes());
        out
    }

    /// Reassemble a request from its wire frames (routing idents first,
    /// then the signed section). Signature is verified when `key` is set.
    pub fn parse(frames: &[Vec<u8>], channel: &'static str, key: &[u8]) -> Result<Self, String> {
        let pos = frames.iter().position(|f| f == DELIM).ok_or("message missing <IDS|MSG> delimiter")?;
        let signed = frames.get(pos + 1..).unwrap_or(&[]);
        if signed.len() < 5 {
            return Err("message has fewer than 5 signed frames".into());
        }
        let (sig, header, parent, _meta, content) = (&signed[0], &signed[1], &signed[2], &signed[3], &signed[4]);
        if !key.is_empty() {
            let expect = hmac_sign(key, &[header, parent, _meta, content]);
            if expect != String::from_utf8_lossy(sig) {
                return Err("message signature mismatch".into());
            }
        }
        let hj: Value = serde_json::from_slice(header).map_err(|e| format!("header: {e}"))?;
        let cj: Value = serde_json::from_slice(content).map_err(|e| format!("content: {e}"))?;
        let s = |k: &str| hj.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        Ok(WireMessage {
            idents: frames.get(..pos).unwrap_or(&[]).to_vec(),
            channel,
            message: Message { msg_id: s("msg_id"), session: s("session"), username: s("username"), msg_type: s("msg_type"), content: cj },
            parent_header: serde_json::json!({}),
            orig_header: hj,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn frames_roundtrip_with_signature() {
        let m = WireMessage {
            idents: vec![b"router".to_vec()],
            channel: "shell",
            message: Message {
                msg_id: "m1".into(),
                session: "s1".into(),
                username: "u".into(),
                msg_type: "kernel_info_request".into(),
                content: json!({}),
            },
            parent_header: json!({}),
            orig_header: json!({}),
        };
        let key = b"secret";
        let frames = m.frames(key);
        let back = WireMessage::parse(&frames, "shell", key).unwrap();
        assert_eq!(back.idents, vec![b"router".to_vec()]);
        assert_eq!(back.message.msg_type, "kernel_info_request");
        // Tampered content fails the signature check.
        let mut bad = frames.clone();
        let n = bad.len() - 1;
        bad[n] = b"{\"x\":1}".to_vec();
        assert!(WireMessage::parse(&bad, "shell", key).is_err());
        // Empty key skips verification.
        assert!(WireMessage::parse(&bad, "shell", b"").is_ok());
    }
}
