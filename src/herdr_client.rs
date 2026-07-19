use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use crate::smart_nav::{Direction, ForegroundProcess};

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

    pub fn read_visible_pane(&mut self, pane_id: &str) -> Result<String> {
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
        Ok(result["read"]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string())
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
                    br#"{"id":"1","result":{"type":"pane_read","read":{"text":"/tmp/project/\nmain.py"}}}"#,
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
        assert_eq!(
            client.read_visible_pane("pane-1").unwrap(),
            "/tmp/project/\nmain.py"
        );
        assert_eq!(client.visible_pane_width("pane-1").unwrap(), 80);

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
