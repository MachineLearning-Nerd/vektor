use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::Duration,
};

use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, Request, Respond, ResponseTemplate,
    matchers::{method, path},
};

const EMBEDDING_DIM: usize = 1536;

#[tokio::test]
async fn stage5_context_package_over_mcp_has_full_wire_shape() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/embeddings"))
        .respond_with(EmbeddingResponder)
        .mount(&mock_server)
        .await;

    let tempdir = tempfile::tempdir().expect("tempdir");
    let fake_home = tempdir.path().join("home");
    let data_dir = tempdir.path().join("data");
    let repo = tempdir.path().join("repo");
    std::fs::create_dir_all(repo.join("src/auth")).expect("mkdir src");
    std::fs::create_dir_all(repo.join("tests")).expect("mkdir tests");
    std::fs::write(
        repo.join("src/auth/jwt.py"),
        "def validate_stage5_token(token):\n    return 'stage5jwt' in token\n",
    )
    .expect("write jwt");
    std::fs::write(
        repo.join("tests/test_auth.py"),
        "def test_validate_stage5_token():\n    assert validate_stage5_token('stage5jwt')\n",
    )
    .expect("write test");
    std::fs::create_dir_all(&fake_home).expect("mkdir home");

    let mut child = Command::new(env!("CARGO_BIN_EXE_vektor"))
        .arg("serve")
        .args(["--transport", "stdio"])
        .env("HOME", &fake_home)
        .env("USERPROFILE", &fake_home)
        .env("VEKTOR__EMBEDDING__BACKEND", "openai")
        .env("VEKTOR__EMBEDDING__OPENAI_API_KEY", "sk-test")
        .env("VEKTOR__EMBEDDING__OPENAI_BASE_URL", mock_server.uri())
        .env("VEKTOR__EMBEDDING__FALLBACK_TO_ONNX", "false")
        .env("VEKTOR__EMBEDDING__MAX_REQUESTS_PER_MINUTE", "0")
        .env("VEKTOR__INDEX__DATA_DIR", &data_dir)
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
    let stderr = child.stderr.take().expect("child stderr");
    let stderr_handle = std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            if line.is_err() {
                break;
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
                "clientInfo": { "name": "stage5-context-test", "version": "0.1.0" }
            }
        }),
    );
    assert_eq!(read_response(&line_rx, "initialize")["id"], 1);
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
            "method": "tools/call",
            "params": {
                "name": "index_codebase",
                "arguments": { "path": repo.to_string_lossy() }
            }
        }),
    );
    let index_response = read_response(&line_rx, "index_codebase");
    assert_eq!(
        index_response["result"]["structuredContent"]["status"], "indexed",
        "indexing must succeed before context assembly: {index_response:#}"
    );

    send(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "get_context_for_prompt",
                "arguments": {
                    "path": repo.to_string_lossy(),
                    "query": "validate stage5jwt token",
                    "token_budget": 8000,
                    "max_files": 10,
                    "include_related": true,
                    "min_relevance": 0.0
                }
            }
        }),
    );
    let context_response = read_response(&line_rx, "get_context_for_prompt");
    let content = &context_response["result"]["structuredContent"];
    let context = content["context"].as_array().expect("context array");
    let metadata = &content["metadata"];

    assert!(
        !context.is_empty(),
        "context must include chunks: {content:#}"
    );
    assert!(
        context
            .iter()
            .any(|chunk| chunk["file"] == "src/auth/jwt.py"),
        "context should include the queried source file: {content:#}"
    );
    assert!(metadata["files_included"].is_number());
    assert!(metadata["total_tokens"].is_number());
    assert!(metadata["budget_used_pct"].is_number());
    assert!(metadata["missing_context_warnings"].is_array());
    assert!(metadata["result_confidence"].is_string());
    assert!(metadata["budget_gap_reason"].is_string() || metadata["budget_gap_reason"].is_null());
    assert!(metadata["clusters"].is_array());
    assert_eq!(metadata["index_status"], "full");
    assert!(
        metadata["total_tokens"].as_u64().expect("total_tokens") <= 8000,
        "two-pass token verification must not exceed the requested budget: {content:#}"
    );

    drop(stdin);
    wait_or_kill(&mut child);
    reader_handle.join().expect("stdout reader join");
    stderr_handle.join().expect("stderr reader join");
}

struct EmbeddingResponder;

impl Respond for EmbeddingResponder {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).expect("embedding request json");
        let inputs = body["input"].as_array().expect("input array");
        let data = inputs
            .iter()
            .enumerate()
            .map(|(index, input)| {
                let text = input.as_str().unwrap_or_default();
                let value = deterministic_value(text);
                json!({
                    "index": index,
                    "embedding": vec![value; EMBEDDING_DIM],
                })
            })
            .collect::<Vec<_>>();

        ResponseTemplate::new(200).set_body_json(json!({ "data": data }))
    }
}

fn deterministic_value(text: &str) -> f32 {
    let sum = text
        .bytes()
        .fold(0u32, |acc, byte| acc.wrapping_add(u32::from(byte)));
    (sum % 97) as f32 / 97.0
}

fn send(stdin: &mut std::process::ChildStdin, request: Value) {
    writeln!(stdin, "{request}").expect("write request");
    stdin.flush().expect("flush request");
}

fn read_response(line_rx: &mpsc::Receiver<String>, label: &str) -> Value {
    let line = line_rx
        .recv_timeout(Duration::from_secs(60))
        .unwrap_or_else(|_| panic!("read JSON-RPC response line for {label}"));

    serde_json::from_str(&line).expect("parse JSON-RPC response")
}

fn wait_or_kill(child: &mut Child) {
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) if started.elapsed() > Duration::from_secs(5) => {
                child.kill().expect("kill child");
                let _ = child.wait();
                return;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(error) => panic!("wait for child: {error}"),
        }
    }
}
