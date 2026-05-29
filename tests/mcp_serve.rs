use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

#[test]
fn mcp_stdio_initialize_list_and_call() {
    let fake_home = tempfile::tempdir().expect("create fake home");
    let mut child = Command::new(env!("CARGO_BIN_EXE_vektor"))
        .arg("serve")
        .args(["--transport", "stdio"])
        .env("HOME", fake_home.path())
        .env("USERPROFILE", fake_home.path())
        .env("VEKTOR_LOG", "trace")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn vektor serve");

    let stdout = child.stdout.take().expect("child stdout");
    let (line_tx, line_rx) = mpsc::channel::<String>();
    let reader_handle = std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            match line {
                Ok(line) => {
                    if line_tx.send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let mut stdin = child.stdin.take().expect("child stdin");

    send(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "vektor-integration-test", "version": "0.1.0" }
            }
        }),
    );
    let init_response = read_response(&line_rx);
    assert_eq!(init_response["id"], 1);
    assert!(
        init_response["result"]["capabilities"]["tools"].is_object(),
        "initialize response must advertise tools capability: {init_response:#}"
    );

    send(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        }),
    );

    send(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list"
        }),
    );
    let list_response = read_response(&line_rx);
    let tools = list_response["result"]["tools"]
        .as_array()
        .expect("tools array");
    let tool_names = tools
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect::<Vec<_>>();
    assert_eq!(
        tool_names,
        ["index_codebase", "search_code", "get_context_for_prompt"]
    );
    assert_tool_schema_property(tools, "index_codebase", "force_full");
    assert_tool_schema_property(tools, "search_code", "top_k");
    assert_tool_schema_property(tools, "search_code", "mode");
    assert_tool_schema_property(tools, "get_context_for_prompt", "max_files");

    send(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "index_codebase",
                "arguments": { "path": "/tmp/dummy" }
            }
        }),
    );
    let call_response = read_response(&line_rx);
    assert_eq!(call_response["id"], 3);
    assert_eq!(call_response["result"]["isError"], false);
    assert_eq!(
        call_response["result"]["structuredContent"]["status"],
        "not implemented yet"
    );
    assert!(call_response["result"]["structuredContent"]["phase"].is_string());
    assert!(
        call_response["result"]["content"][0]["text"]
            .as_str()
            .expect("tool text")
            .contains("not implemented yet")
    );

    send(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "unknown_tool",
                "arguments": {}
            }
        }),
    );
    let error_response = read_response(&line_rx);
    assert_eq!(error_response["id"], 4);
    assert!(error_response["error"].is_object());
    assert_eq!(error_response["error"]["code"], -32602); // invalid_params

    drop(stdin);
    wait_or_kill(&mut child);
    reader_handle.join().expect("stdout reader join");
}

fn send(stdin: &mut std::process::ChildStdin, request: Value) {
    writeln!(stdin, "{request}").expect("write request");
    stdin.flush().expect("flush request");
}

fn read_response(line_rx: &mpsc::Receiver<String>) -> Value {
    let line = line_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("read JSON-RPC response line");

    serde_json::from_str(&line).expect("parse JSON-RPC response")
}

fn assert_tool_schema_property(tools: &[Value], tool_name: &str, property_name: &str) {
    let tool = tools
        .iter()
        .find(|tool| tool["name"] == tool_name)
        .expect("tool exists");
    assert!(
        tool["inputSchema"]["properties"][property_name].is_object(),
        "{tool_name} inputSchema must advertise {property_name}: {tool:#}"
    );
}

fn wait_or_kill(child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        match child.try_wait().expect("poll child") {
            Some(_status) => return,
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }

    child.kill().expect("kill hung child");
    child.wait().expect("wait child after kill");
    panic!("vektor serve did not shut down after stdin EOF");
}
