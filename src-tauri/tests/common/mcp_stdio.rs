//! The real `ranch-mcp` binary, driven over stdio as an agent client would,
//! against a throwaway database (`RANCH_MCP_DATA_DIR`, debug builds only).

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{json, Value};

/// Where the app keeps `ranch.db` under a given home directory.
pub fn app_data_dir(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/dev.ranch.app")
    } else {
        home.join("dev.ranch.app")
    }
}

pub struct Mcp {
    child: Child,
    stdin: ChildStdin,
    lines: std::io::Lines<BufReader<ChildStdout>>,
    next_id: u64,
}

impl Mcp {
    /// Start `ranch-mcp` on the database under `home` and initialise the session.
    pub fn start(home: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ranch-mcp"))
            .env("RANCH_MCP_DATA_DIR", app_data_dir(home))
            .env("RUST_LOG", "error")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("ranch-mcp starts");
        let stdin = child.stdin.take().unwrap();
        let lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut mcp = Self {
            child,
            stdin,
            lines,
            next_id: 1,
        };
        let init = mcp.request(
            "initialize",
            json!({
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": { "name": "ranch-tests", "version": "0" }
            }),
        );
        assert!(init.get("result").is_some(), "{init}");
        mcp.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        mcp
    }

    fn send(&mut self, message: Value) {
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
    }

    /// One JSON-RPC request, its reply as it came.
    pub fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        for line in self.lines.by_ref() {
            let Ok(value) = serde_json::from_str::<Value>(&line.unwrap()) else {
                continue;
            };
            if value.get("id") == Some(&json!(id)) {
                return value;
            }
        }
        panic!("ranch-mcp closed stdout before answering {method}");
    }

    /// The text of a tool's answer; panics on an error.
    pub fn call(&mut self, tool: &str, arguments: Value) -> String {
        let reply = self.request(
            "tools/call",
            json!({ "name": tool, "arguments": arguments }),
        );
        assert!(reply.get("error").is_none(), "{tool}: {reply}");
        reply
            .pointer("/result/content/0/text")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{tool}: no text in {reply}"))
            .to_string()
    }

    /// The machine-readable `kind` of a tool's error; panics on a success.
    pub fn error_kind(&mut self, tool: &str, arguments: Value) -> String {
        let reply = self.request(
            "tools/call",
            json!({ "name": tool, "arguments": arguments }),
        );
        reply
            .pointer("/error/data/kind")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{tool} should have been refused: {reply}"))
            .to_string()
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
