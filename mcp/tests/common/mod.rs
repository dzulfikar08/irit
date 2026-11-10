//! A raw JSON-RPC client for irit-mcp over stdio, shared by the integration tests.
#![allow(dead_code)]

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

pub struct Client {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next: u64,
}

impl Client {
    pub fn start(cwd: &Path, bin: &Path) -> Client {
        Client::spawn(cwd, &["--bin-dir", bin.to_str().unwrap()], &[])
    }

    /// irit-mcp in `cwd` with run id `test-run`, extra arguments and environment.
    pub fn spawn(cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> Client {
        let mut child = Command::new(env!("CARGO_BIN_EXE_irit-mcp"))
            .args(args)
            .args(["--run-id", "test-run"])
            .envs(env.iter().copied())
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut c = Client { child, stdin, stdout, next: 1 };
        let init = c.request(
            "initialize",
            json!({"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}}),
        );
        assert_eq!(init["result"]["serverInfo"]["name"], "irit-mcp", "{init}");
        c.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        c
    }

    pub fn send(&mut self, msg: Value) {
        let w = self.stdin.as_mut().unwrap();
        writeln!(w, "{msg}").unwrap();
        w.flush().unwrap();
    }

    pub fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let mut line = String::new();
            assert!(self.stdout.read_line(&mut line).unwrap() > 0, "server closed stdout");
            let msg: Value = serde_json::from_str(&line).unwrap();
            if msg["id"] == json!(id) {
                return msg;
            }
        }
    }

    pub fn call(&mut self, tool: &str, args: Value) -> Value {
        let r = self.request("tools/call", json!({"name": tool, "arguments": args}));
        r.get("result").cloned().unwrap_or(r)
    }

    pub fn close(mut self) {
        drop(self.stdin.take());
        assert!(self.child.wait().unwrap().success());
    }
}

pub fn sha256(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(std::fs::read(path).unwrap()).iter().map(|b| format!("{b:02x}")).collect()
}
