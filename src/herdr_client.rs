use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use crate::smart_nav::{Direction, ForegroundProcess};

const RPC_IO_TIMEOUT: Duration = Duration::from_secs(2);

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisiblePaneSnapshot {
    pub text: String,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneScrollSnapshot {
    pub offset_from_bottom: u64,
    pub max_offset_from_bottom: u64,
    pub viewport_rows: u64,
    pub revision: u64,
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneProcessInfo {
    pub pane_id: String,
    pub foreground_processes: Vec<ForegroundProcess>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusDirectionResult {
    pub changed: bool,
    pub reason: Option<String>,
    pub source_pane_id: String,
    pub focused_pane_id: Option<String>,
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
        Ok(VisiblePaneSnapshot {
            text: read["text"].as_str().unwrap_or_default().to_string(),
            revision: read["revision"]
                .as_u64()
                .context("pane_read result did not include revision")?,
        })
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
                .and_then(|value| u16::try_from(value).ok())
                .context("pane_copy_mode_jump missing viewport_row")?,
            viewport_col: result["viewport_col"]
                .as_u64()
                .and_then(|value| u16::try_from(value).ok())
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

    pub fn process_info(&mut self, pane_id: &str) -> Result<PaneProcessInfo> {
        let result = self.call(
            "pane.process_info",
            json!({
                "pane_id": pane_id
            }),
        )?;
        let actual_type = result["type"].as_str().unwrap_or("<missing>");
        if actual_type != "pane_process_info" {
            bail!("expected pane_process_info result, got {actual_type}");
        }
        let info = &result["process_info"];
        let returned_pane = info["pane_id"].as_str().unwrap_or(pane_id).to_string();
        let processes = info["foreground_processes"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|p| ForegroundProcess {
                        name: p["name"].as_str().unwrap_or("").to_string(),
                        argv0: p["argv0"].as_str().map(ToString::to_string),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(PaneProcessInfo {
            pane_id: returned_pane,
            foreground_processes: processes,
        })
    }

    pub fn send_keys(&mut self, pane_id: &str, keys: &[&str]) -> Result<()> {
        let result = self.call(
            "pane.send_keys",
            json!({
                "pane_id": pane_id,
                "keys": keys
            }),
        )?;
        // pane.send_keys returns ResponseResult::Ok { type: "ok" }.
        let actual_type = result["type"].as_str().unwrap_or("<missing>");
        if actual_type != "ok" {
            bail!("expected ok result from pane.send_keys, got {actual_type}");
        }
        Ok(())
    }

    pub fn focus_direction(
        &mut self,
        pane_id: &str,
        direction: Direction,
    ) -> Result<FocusDirectionResult> {
        let result = self.call(
            "pane.focus_direction",
            json!({
                "pane_id": pane_id,
                "direction": direction.as_str()
            }),
        )?;
        let actual_type = result["type"].as_str().unwrap_or("<missing>");
        if actual_type != "pane_focus_direction" {
            bail!("expected pane_focus_direction result, got {actual_type}");
        }
        let focus = &result["focus"];
        Ok(FocusDirectionResult {
            changed: focus["changed"].as_bool().unwrap_or(false),
            reason: focus["reason"].as_str().map(ToString::to_string),
            source_pane_id: focus["source_pane_id"]
                .as_str()
                .unwrap_or(pane_id)
                .to_string(),
            focused_pane_id: focus["focused_pane_id"].as_str().map(ToString::to_string),
        })
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.to_string();
        self.next_id += 1;

        let mut stream = UnixStream::connect(&self.socket_path)
            .with_context(|| format!("cannot connect to Herdr API while calling {method}"))?;
        stream
            .set_read_timeout(Some(RPC_IO_TIMEOUT))
            .with_context(|| format!("cannot set Herdr API read deadline for {method}"))?;
        stream
            .set_write_timeout(Some(RPC_IO_TIMEOUT))
            .with_context(|| format!("cannot set Herdr API write deadline for {method}"))?;

        let mut request = json!({"id": id, "method": method, "params": params}).to_string();
        request.push('\n');
        stream
            .write_all(request.as_bytes())
            .map_err(|error| rpc_io_error(method, "writing request", error))?;

        let mut reader = BufReader::new(stream);
        let mut response = String::new();
        let bytes_read = reader
            .read_line(&mut response)
            .map_err(|error| rpc_io_error(method, "reading response", error))?;
        if bytes_read == 0 {
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

fn rpc_io_error(method: &str, operation: &str, error: io::Error) -> anyhow::Error {
    let context = if matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    ) {
        format!(
            "Herdr API timed out while {operation} for {method} after {} ms",
            RPC_IO_TIMEOUT.as_millis()
        )
    } else {
        format!("Herdr API failed while {operation} for {method}")
    };
    anyhow::Error::new(error).context(context)
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
    use std::sync::mpsc;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    #[test]
    fn show_notification_sends_notification_show_request() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let socket_path = std::path::PathBuf::from(format!("/tmp/htf-{unique}.sock"));
        let _ = std::fs::remove_file(&socket_path);
        let listener = UnixListener::bind(&socket_path).unwrap();

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
    fn reads_visible_text_and_uses_its_pane_layout_width() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let socket_path = std::path::PathBuf::from(format!("/tmp/htf-{unique}.sock"));
        let _ = std::fs::remove_file(&socket_path);
        let listener = UnixListener::bind(&socket_path).unwrap();

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
        let snapshot = client.read_visible_pane("pane-1").unwrap();
        assert_eq!(snapshot.text, "/tmp/project/\nmain.py");
        assert_eq!(snapshot.revision, 7);
        assert_eq!(client.visible_pane_width("pane-1").unwrap(), 80);

        handle.join().unwrap();
        let _ = std::fs::remove_file(socket_path);
    }

    #[test]
    fn copy_mode_jump_keeps_viewport_revision_and_scroll_identity() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let socket_path = PathBuf::from(format!("/tmp/htf-jump-{unique}.sock"));
        let _ = std::fs::remove_file(&socket_path);
        let listener = UnixListener::bind(&socket_path).unwrap();

        let handle = std::thread::spawn(move || {
            let (_probe_stream, _) = listener.accept().unwrap();
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            reader.read_line(&mut request).unwrap();
            let json: Value = serde_json::from_str(&request).unwrap();
            assert_eq!(json["method"], "pane.copy_mode_jump");
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
        assert_eq!(result.revision, 42);
        assert_eq!(result.offset_from_bottom, 3);
        handle.join().unwrap();
        let _ = std::fs::remove_file(socket_path);
    }

    #[test]
    fn process_info_parses_foreground_names_and_argv0() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let socket_path = std::path::PathBuf::from(format!("/tmp/htf-pi-{unique}.sock"));
        let _ = std::fs::remove_file(&socket_path);
        let listener = UnixListener::bind(&socket_path).unwrap();

        let handle = std::thread::spawn(move || {
            let (_probe_stream, _) = listener.accept().unwrap();
            let (mut stream, _) = listener.accept().unwrap();

            let mut request = String::new();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            reader.read_line(&mut request).unwrap();
            let json: Value = serde_json::from_str(&request).unwrap();
            assert_eq!(json["method"], "pane.process_info");
            assert_eq!(json["params"]["pane_id"], "w1:p2");

            stream
                .write_all(
                    br#"{"id":"1","result":{"type":"pane_process_info","process_info":{"pane_id":"w1:p2","foreground_processes":[{"pid":1,"name":"nvim","argv0":"/usr/bin/nvim"}]}}}"#,
                )
                .unwrap();
            stream.write_all(b"\n").unwrap();
        });

        let mut client = SocketClient::connect(&socket_path).unwrap();
        let info = client.process_info("w1:p2").unwrap();
        assert_eq!(info.pane_id, "w1:p2");
        assert_eq!(info.foreground_processes.len(), 1);
        assert_eq!(info.foreground_processes[0].name, "nvim");
        assert_eq!(
            info.foreground_processes[0].argv0.as_deref(),
            Some("/usr/bin/nvim")
        );
        handle.join().unwrap();
        let _ = std::fs::remove_file(socket_path);
    }

    #[test]
    fn send_keys_and_focus_direction_use_supported_socket_methods() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let socket_path = std::path::PathBuf::from(format!("/tmp/htf-nav-{unique}.sock"));
        let _ = std::fs::remove_file(&socket_path);
        let listener = UnixListener::bind(&socket_path).unwrap();

        let handle = std::thread::spawn(move || {
            let (_probe_stream, _) = listener.accept().unwrap();

            // send_keys
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            reader.read_line(&mut request).unwrap();
            let json: Value = serde_json::from_str(&request).unwrap();
            assert_eq!(json["method"], "pane.send_keys");
            assert_eq!(json["params"]["pane_id"], "w1:p2");
            assert_eq!(json["params"]["keys"], json!(["ctrl+h"]));
            stream
                .write_all(br#"{"id":"1","result":{"type":"ok"}}"#)
                .unwrap();
            stream.write_all(b"\n").unwrap();

            // focus_direction
            let (mut stream, _) = listener.accept().unwrap();
            request.clear();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            reader.read_line(&mut request).unwrap();
            let json: Value = serde_json::from_str(&request).unwrap();
            assert_eq!(json["method"], "pane.focus_direction");
            assert_eq!(json["params"]["pane_id"], "w1:p1");
            assert_eq!(json["params"]["direction"], "right");
            stream
                .write_all(
                    br#"{"id":"2","result":{"type":"pane_focus_direction","focus":{"changed":true,"source_pane_id":"w1:p1","focused_pane_id":"w1:p2"}}}"#,
                )
                .unwrap();
            stream.write_all(b"\n").unwrap();

            // focus_direction no neighbor
            let (mut stream, _) = listener.accept().unwrap();
            request.clear();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            reader.read_line(&mut request).unwrap();
            let json: Value = serde_json::from_str(&request).unwrap();
            assert_eq!(json["method"], "pane.focus_direction");
            stream
                .write_all(
                    br#"{"id":"3","result":{"type":"pane_focus_direction","focus":{"changed":false,"reason":"no_neighbor","source_pane_id":"w1:p1","focused_pane_id":"w1:p1"}}}"#,
                )
                .unwrap();
            stream.write_all(b"\n").unwrap();
        });

        let mut client = SocketClient::connect(&socket_path).unwrap();
        client.send_keys("w1:p2", &["ctrl+h"]).unwrap();

        let moved = client.focus_direction("w1:p1", Direction::Right).unwrap();
        assert!(moved.changed);
        assert_eq!(moved.focused_pane_id.as_deref(), Some("w1:p2"));
        assert_eq!(moved.reason, None);

        let edge = client.focus_direction("w1:p1", Direction::Left).unwrap();
        assert!(!edge.changed);
        assert_eq!(edge.reason.as_deref(), Some("no_neighbor"));

        handle.join().unwrap();
        let _ = std::fs::remove_file(socket_path);
    }

    #[test]
    fn call_times_out_when_peer_never_responds() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let socket_path = std::path::PathBuf::from(format!("/tmp/htf-timeout-{unique}.sock"));
        let _ = std::fs::remove_file(&socket_path);
        let listener = UnixListener::bind(&socket_path).unwrap();
        let (release_tx, release_rx) = mpsc::channel();

        let handle = std::thread::spawn(move || {
            let (_probe_stream, _) = listener.accept().unwrap();
            let (stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            let mut reader = BufReader::new(stream);
            reader.read_line(&mut request).unwrap();
            let json: Value = serde_json::from_str(&request).unwrap();
            assert_eq!(json["method"], "pane.process_info");
            let _ = release_rx.recv_timeout(RPC_IO_TIMEOUT * 3);
        });

        let mut client = SocketClient::connect(&socket_path).unwrap();
        let started = Instant::now();
        let result = client.process_info("w1:p2");
        let elapsed = started.elapsed();
        let _ = release_tx.send(());
        handle.join().unwrap();
        let _ = std::fs::remove_file(socket_path);

        let err = result.unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("timed out while reading response for pane.process_info"),
            "expected actionable timeout, got {msg}"
        );
        assert!(
            elapsed < RPC_IO_TIMEOUT * 3,
            "timeout exceeded its bound: {elapsed:?}"
        );
    }

    #[test]
    fn process_info_surfaces_pane_not_found_without_hanging() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let socket_path = std::path::PathBuf::from(format!("/tmp/htf-missing-{unique}.sock"));
        let _ = std::fs::remove_file(&socket_path);
        let listener = UnixListener::bind(&socket_path).unwrap();

        let handle = std::thread::spawn(move || {
            let (_probe_stream, _) = listener.accept().unwrap();
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            reader.read_line(&mut request).unwrap();
            stream
                .write_all(
                    br#"{"id":"1","error":{"code":"pane_not_found","message":"pane not found"}}"#,
                )
                .unwrap();
            stream.write_all(b"\n").unwrap();
        });

        let mut client = SocketClient::connect(&socket_path).unwrap();
        let err = client.process_info("w1:p999").unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("pane_not_found"),
            "expected pane_not_found, got {msg}"
        );
        handle.join().unwrap();
        let _ = std::fs::remove_file(socket_path);
    }
}
