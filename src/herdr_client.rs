use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

#[derive(Debug)]
pub struct SocketClient {
    socket_path: PathBuf,
    next_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationResult {
    pub shown: bool,
    pub reason: String,
}

/// Visible pane text plus the content revision used for stale-safe copy-mode jumps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisiblePaneSnapshot {
    pub text: String,
    pub revision: u64,
}

/// Scroll metrics from `pane.get`, including the revision at the time of the get.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneScrollSnapshot {
    pub offset_from_bottom: u64,
    pub max_offset_from_bottom: u64,
    pub viewport_rows: u64,
    pub revision: u64,
}

/// Identity of the viewport used to choose a jump cell (Herdr `pane.copy_mode_jump` params).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyModeJumpRequest {
    pub pane_id: String,
    pub viewport_row: u16,
    pub viewport_col: u16,
    pub revision: u64,
    pub offset_from_bottom: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyModeJumpResult {
    pub pane_id: String,
    pub viewport_row: u16,
    pub viewport_col: u16,
    pub revision: u64,
    pub offset_from_bottom: u64,
}

impl SocketClient {
    pub fn connect(socket_path: &Path) -> Result<Self> {
        UnixStream::connect(socket_path).with_context(|| {
            format!(
                "cannot connect to Herdr API socket at {}",
                socket_path.display()
            )
        })?;
        Ok(Self {
            socket_path: socket_path.to_path_buf(),
            next_id: 1,
        })
    }

    pub fn read_visible_pane(&mut self, pane_id: &str) -> Result<VisiblePaneSnapshot> {
        let result = self.call(
            "pane.read",
            json!({
                "pane_id": pane_id,
                "source": "visible",
                "format": "text",
                "strip_ansi": true
            }),
        )?;
        let actual_type = result["type"].as_str().unwrap_or("<missing>");
        if actual_type != "pane_read" {
            bail!("expected pane_read result, got {actual_type}");
        }
        let read = &result["read"];
        let text = read["text"].as_str().unwrap_or_default().to_string();
        let revision = read["revision"]
            .as_u64()
            .context("pane_read result did not include revision")?;
        Ok(VisiblePaneSnapshot { text, revision })
    }

    pub fn pane_scroll(&mut self, pane_id: &str) -> Result<PaneScrollSnapshot> {
        let result = self.call("pane.get", json!({ "pane_id": pane_id }))?;
        let actual_type = result["type"].as_str().unwrap_or("<missing>");
        if actual_type != "pane_info" {
            bail!("expected pane_info result, got {actual_type}");
        }
        let pane = &result["pane"];
        let scroll = pane
            .get("scroll")
            .context("pane_info result did not include scroll")?;
        Ok(PaneScrollSnapshot {
            offset_from_bottom: scroll["offset_from_bottom"]
                .as_u64()
                .context("scroll.offset_from_bottom missing")?,
            max_offset_from_bottom: scroll["max_offset_from_bottom"]
                .as_u64()
                .context("scroll.max_offset_from_bottom missing")?,
            viewport_rows: scroll["viewport_rows"]
                .as_u64()
                .context("scroll.viewport_rows missing")?,
            revision: pane["revision"]
                .as_u64()
                .context("pane_info result did not include revision")?,
        })
    }

    pub fn visible_pane_width(&mut self, pane_id: &str) -> Result<usize> {
        let result = self.call("pane.layout", json!({ "pane_id": pane_id }))?;
        let actual_type = result["type"].as_str().unwrap_or("<missing>");
        if actual_type != "pane_layout" {
            bail!("expected pane_layout result, got {actual_type}");
        }
        let panes = result["layout"]["panes"]
            .as_array()
            .context("pane_layout result did not include panes")?;
        let pane = panes
            .iter()
            .find(|pane| pane["pane_id"].as_str() == Some(pane_id))
            .context("pane_layout result did not include the requested pane")?;
        let width = pane["rect"]["width"]
            .as_u64()
            .context("pane_layout result did not include the pane width")?;
        usize::try_from(width).context("pane width did not fit in usize")
    }

    pub fn copy_mode_jump(&mut self, request: &CopyModeJumpRequest) -> Result<CopyModeJumpResult> {
        let result = self.call(
            "pane.copy_mode_jump",
            json!({
                "pane_id": request.pane_id,
                "viewport_row": request.viewport_row,
                "viewport_col": request.viewport_col,
                "revision": request.revision,
                "offset_from_bottom": request.offset_from_bottom,
            }),
        )?;
        let actual_type = result["type"].as_str().unwrap_or("<missing>");
        if actual_type != "pane_copy_mode_jump" {
            bail!("expected pane_copy_mode_jump result, got {actual_type}");
        }
        Ok(CopyModeJumpResult {
            pane_id: result["pane_id"].as_str().unwrap_or_default().to_string(),
            viewport_row: result["viewport_row"]
                .as_u64()
                .and_then(|v| u16::try_from(v).ok())
                .context("pane_copy_mode_jump missing viewport_row")?,
            viewport_col: result["viewport_col"]
                .as_u64()
                .and_then(|v| u16::try_from(v).ok())
                .context("pane_copy_mode_jump missing viewport_col")?,
            revision: result["revision"]
                .as_u64()
                .context("pane_copy_mode_jump missing revision")?,
            offset_from_bottom: result["offset_from_bottom"]
                .as_u64()
                .context("pane_copy_mode_jump missing offset_from_bottom")?,
        })
    }

    pub fn show_notification(&mut self, title: &str) -> Result<NotificationResult> {
        let result = self.call(
            "notification.show",
            json!({
                "title": title
            }),
        )?;
        let actual_type = result["type"].as_str().unwrap_or("<missing>");
        if actual_type != "notification_show" {
            bail!("expected notification_show result, got {actual_type}");
        }
        Ok(NotificationResult {
            shown: result["shown"].as_bool().unwrap_or(false),
            reason: result["reason"].as_str().unwrap_or("unknown").to_string(),
        })
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.to_string();
        self.next_id += 1;

        let mut reader = BufReader::new(UnixStream::connect(&self.socket_path)?);
        let request = json!({"id": id, "method": method, "params": params}).to_string();
        let stream = reader.get_mut();
        stream.write_all(request.as_bytes())?;
        stream.write_all(b"\n")?;

        let mut response = String::new();
        if reader.read_line(&mut response)? == 0 {
            bail!("Herdr closed the API connection before answering {method}");
        }

        let envelope: Value = serde_json::from_str(&response)
            .with_context(|| format!("Herdr returned invalid JSON for {method}"))?;
        if let Some(error) = envelope.get("error") {
            let code = error["code"].as_str().unwrap_or("unknown_error");
            let message = error["message"].as_str().unwrap_or("no message");
            bail!("Herdr API error {code}: {message}");
        }
        if envelope["id"].as_str() != Some(&id) {
            bail!("Herdr response id did not match request id {id}");
        }
        envelope
            .get("result")
            .cloned()
            .context("Herdr response has neither result nor error")
    }
}

pub fn context_focused_pane_id() -> Option<String> {
    let context = std::env::var("HERDR_PLUGIN_CONTEXT_JSON").ok()?;
    let context: Value = serde_json::from_str(&context).ok()?;
    context
        .get("focused_pane_id")?
        .as_str()
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_sock() -> (PathBuf, UnixListener) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let socket_path = PathBuf::from(format!("/tmp/htf-{unique}.sock"));
        let _ = std::fs::remove_file(&socket_path);
        let listener = UnixListener::bind(&socket_path).unwrap();
        (socket_path, listener)
    }

    #[test]
    fn show_notification_sends_notification_show_request() {
        let (socket_path, listener) = temp_sock();

        let handle = std::thread::spawn(move || {
            let (_probe_stream, _) = listener.accept().unwrap();
            let (mut stream, _) = listener.accept().unwrap();

            let mut request = String::new();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            reader.read_line(&mut request).unwrap();
            let json: Value = serde_json::from_str(&request).unwrap();

            assert_eq!(json["id"], "1");
            assert_eq!(json["method"], "notification.show");
            assert_eq!(json["params"]["title"], "Copied: README.md");

            stream
                .write_all(
                    br#"{"id":"1","result":{"type":"notification_show","shown":true,"reason":"shown"}}"#,
                )
                .unwrap();
            stream.write_all(b"\n").unwrap();
        });

        let mut client = SocketClient::connect(&socket_path).unwrap();
        let result = client.show_notification("Copied: README.md").unwrap();
        assert_eq!(
            result,
            NotificationResult {
                shown: true,
                reason: "shown".to_string()
            }
        );
        handle.join().unwrap();
        let _ = std::fs::remove_file(socket_path);
    }

    #[test]
    fn reads_visible_text_revision_and_uses_its_pane_layout_width() {
        let (socket_path, listener) = temp_sock();

        let handle = std::thread::spawn(move || {
            let (_probe_stream, _) = listener.accept().unwrap();

            let (mut read_stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            let mut reader = BufReader::new(read_stream.try_clone().unwrap());
            reader.read_line(&mut request).unwrap();
            let json: Value = serde_json::from_str(&request).unwrap();
            assert_eq!(json["method"], "pane.read");
            assert_eq!(json["params"]["source"], "visible");
            read_stream
                .write_all(
                    br#"{"id":"1","result":{"type":"pane_read","read":{"text":"/tmp/project/\nmain.py","revision":7}}}"#,
                )
                .unwrap();
            read_stream.write_all(b"\n").unwrap();

            let (mut layout_stream, _) = listener.accept().unwrap();
            request.clear();
            let mut reader = BufReader::new(layout_stream.try_clone().unwrap());
            reader.read_line(&mut request).unwrap();
            let json: Value = serde_json::from_str(&request).unwrap();
            assert_eq!(json["method"], "pane.layout");
            assert_eq!(json["params"]["pane_id"], "pane-1");
            layout_stream
                .write_all(
                    br#"{"id":"2","result":{"type":"pane_layout","layout":{"panes":[{"pane_id":"pane-1","rect":{"width":80}}]}}}"#,
                )
                .unwrap();
            layout_stream.write_all(b"\n").unwrap();
        });

        let mut client = SocketClient::connect(&socket_path).unwrap();
        let snap = client.read_visible_pane("pane-1").unwrap();
        assert_eq!(snap.text, "/tmp/project/\nmain.py");
        assert_eq!(snap.revision, 7);
        assert_eq!(client.visible_pane_width("pane-1").unwrap(), 80);

        handle.join().unwrap();
        let _ = std::fs::remove_file(socket_path);
    }

    #[test]
    fn copy_mode_jump_sends_visible_cell_and_viewport_identity() {
        let (socket_path, listener) = temp_sock();

        let handle = std::thread::spawn(move || {
            let (_probe_stream, _) = listener.accept().unwrap();
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            reader.read_line(&mut request).unwrap();
            let json: Value = serde_json::from_str(&request).unwrap();
            assert_eq!(json["method"], "pane.copy_mode_jump");
            assert_eq!(json["params"]["pane_id"], "w1:p1");
            assert_eq!(json["params"]["viewport_row"], 7);
            assert_eq!(json["params"]["viewport_col"], 12);
            assert_eq!(json["params"]["revision"], 42);
            assert_eq!(json["params"]["offset_from_bottom"], 3);
            stream
                .write_all(
                    br#"{"id":"1","result":{"type":"pane_copy_mode_jump","pane_id":"w1:p1","viewport_row":7,"viewport_col":12,"revision":42,"offset_from_bottom":3}}"#,
                )
                .unwrap();
            stream.write_all(b"\n").unwrap();
        });

        let mut client = SocketClient::connect(&socket_path).unwrap();
        let result = client
            .copy_mode_jump(&CopyModeJumpRequest {
                pane_id: "w1:p1".into(),
                viewport_row: 7,
                viewport_col: 12,
                revision: 42,
                offset_from_bottom: 3,
            })
            .unwrap();
        assert_eq!(
            result,
            CopyModeJumpResult {
                pane_id: "w1:p1".into(),
                viewport_row: 7,
                viewport_col: 12,
                revision: 42,
                offset_from_bottom: 3,
            }
        );
        handle.join().unwrap();
        let _ = std::fs::remove_file(socket_path);
    }

    #[test]
    fn pane_scroll_reads_offset_and_revision() {
        let (socket_path, listener) = temp_sock();

        let handle = std::thread::spawn(move || {
            let (_probe_stream, _) = listener.accept().unwrap();
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            reader.read_line(&mut request).unwrap();
            let json: Value = serde_json::from_str(&request).unwrap();
            assert_eq!(json["method"], "pane.get");
            stream
                .write_all(
                    br#"{"id":"1","result":{"type":"pane_info","pane":{"pane_id":"w1:p1","revision":9,"scroll":{"offset_from_bottom":0,"max_offset_from_bottom":12,"viewport_rows":24}}}}"#,
                )
                .unwrap();
            stream.write_all(b"\n").unwrap();
        });

        let mut client = SocketClient::connect(&socket_path).unwrap();
        let scroll = client.pane_scroll("w1:p1").unwrap();
        assert_eq!(scroll.offset_from_bottom, 0);
        assert_eq!(scroll.viewport_rows, 24);
        assert_eq!(scroll.revision, 9);
        handle.join().unwrap();
        let _ = std::fs::remove_file(socket_path);
    }
}
