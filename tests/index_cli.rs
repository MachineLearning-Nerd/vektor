use std::{
    ffi::OsString,
    fs,
    path::Path,
    process::{Command, Output},
};

#[test]
fn dump_chunks_file_prints_chunk_metadata_and_does_not_create_state() {
    let fixture = tempfile::tempdir().expect("create fixture");
    let fake_home = tempfile::tempdir().expect("create fake home");
    let source = fixture.path().join("src/lib.rs");
    write_file(&source, "fn add(x: i32) -> i32 { x + 1 }\n");

    let output = run_vektor(
        &fake_home,
        vec![
            str_arg("index"),
            str_arg("--dump-chunks"),
            path_arg(&source),
        ],
    );

    assert_success(&output);
    let stdout = stdout(&output);
    assert!(stdout.contains("path:"));
    assert!(stdout.contains("path: lib.rs"));
    assert!(stdout.contains("lines: 1-1"));
    assert!(stdout.contains("language: rust"));
    assert!(stdout.contains("symbol: add"));
    assert!(stdout.contains("symbol_type: function_item"));
    assert!(stdout.contains("content_hash:"));
    assert!(stdout.contains("fn add"));
    assert!(!fake_home.path().join(".vektor").exists());
}

#[test]
fn dump_chunks_directory_discovers_files_deterministically() {
    let fixture = tempfile::tempdir().expect("create fixture");
    let fake_home = tempfile::tempdir().expect("create fake home");
    write_file(fixture.path().join("b.rs"), "fn b() {}\n");
    write_file(fixture.path().join("a.rs"), "fn a() {}\n");

    let output = run_vektor(
        &fake_home,
        vec![
            str_arg("index"),
            str_arg("--dump-chunks"),
            path_arg(fixture.path()),
        ],
    );

    assert_success(&output);
    let stdout = stdout(&output);
    let a_pos = stdout.find("path: a.rs").expect("a.rs chunk appears");
    let b_pos = stdout.find("path: b.rs").expect("b.rs chunk appears");
    assert!(a_pos < b_pos, "{stdout}");
}

#[test]
fn dump_chunks_skips_secret_bearing_chunks_before_stdout() {
    let fixture = tempfile::tempdir().expect("create fixture");
    let fake_home = tempfile::tempdir().expect("create fake home");
    let secret = "AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE";
    write_file(
        fixture.path().join("notes.txt"),
        &format!("safe filename\n{secret}\n"),
    );

    let output = run_vektor(
        &fake_home,
        vec![
            str_arg("index"),
            str_arg("--dump-chunks"),
            path_arg(fixture.path()),
        ],
    );

    assert_success(&output);
    let stdout = stdout(&output);
    assert!(!stdout.contains(secret), "{stdout}");
    assert!(
        !fake_home.path().join(".vektor").exists(),
        "--dump-chunks must stay read-only"
    );
}

// Real `vektor index` (no `--dump-chunks`) now builds the embedder + LanceDB
// store via the shared Phase 3 index core, which requires a downloaded ONNX
// model. These end-to-end tests are #[ignore]d so CI needs no model download;
// see release-notes-v0.3.0.md for the canonical downloaded-model manual CLI
// commands. The equivalent
// changed/unchanged/force, content-hash reuse, and non-UTF-8 behaviour is
// covered without a model by the fake-embedder seam tests in `src/cli.rs`.
#[test]
#[ignore = "requires downloaded ONNX model; see release-notes manual CLI commands"]
fn index_cli_tracks_changed_unchanged_and_force_with_isolated_state() {
    let fixture = tempfile::tempdir().expect("create fixture");
    let fake_home = tempfile::tempdir().expect("create fake home");
    write_file(fixture.path().join("src/lib.rs"), "fn add() -> i32 { 1 }\n");

    let first = run_vektor(&fake_home, vec![str_arg("index"), path_arg(fixture.path())]);
    assert_success(&first);
    let first_stdout = stdout(&first);
    assert!(first_stdout.contains("changed: 1"), "{first_stdout}");
    assert!(first_stdout.contains("unchanged: 0"), "{first_stdout}");
    assert!(first_stdout.contains("chunks: 1"), "{first_stdout}");

    let second = run_vektor(&fake_home, vec![str_arg("index"), path_arg(fixture.path())]);
    assert_success(&second);
    let second_stdout = stdout(&second);
    assert!(second_stdout.contains("changed: 0"), "{second_stdout}");
    assert!(second_stdout.contains("unchanged: 1"), "{second_stdout}");

    let forced = run_vektor(
        &fake_home,
        vec![
            str_arg("index"),
            str_arg("--force"),
            path_arg(fixture.path()),
        ],
    );
    assert_success(&forced);
    let forced_stdout = stdout(&forced);
    assert!(forced_stdout.contains("changed: 1"), "{forced_stdout}");
    assert!(forced_stdout.contains("unchanged: 0"), "{forced_stdout}");
    assert!(contains_file_named(
        fake_home.path().join(".vektor"),
        "state.db"
    ));
}

#[test]
#[ignore = "requires downloaded ONNX model; see release-notes manual CLI commands"]
fn index_cli_uses_parent_relative_state_keys_for_file_inputs() {
    let fixture = tempfile::tempdir().expect("create fixture");
    let fake_home = tempfile::tempdir().expect("create fake home");
    let source = fixture.path().join("src/lib.rs");
    write_file(&source, "fn add() -> i32 { 1 }\n");

    let file_run = run_vektor(&fake_home, vec![str_arg("index"), path_arg(&source)]);
    assert_success(&file_run);
    let file_stdout = stdout(&file_run);
    assert!(file_stdout.contains("changed: 1"), "{file_stdout}");

    let parent_run = run_vektor(
        &fake_home,
        vec![
            str_arg("index"),
            path_arg(source.parent().expect("source has parent")),
        ],
    );
    assert_success(&parent_run);
    let parent_stdout = stdout(&parent_run);
    assert!(parent_stdout.contains("changed: 0"), "{parent_stdout}");
    assert!(parent_stdout.contains("unchanged: 1"), "{parent_stdout}");
}

#[test]
#[ignore = "requires downloaded ONNX model; see release-notes manual CLI commands"]
fn index_cli_chunks_readable_non_utf8_files_without_marking_failed() {
    let fixture = tempfile::tempdir().expect("create fixture");
    let fake_home = tempfile::tempdir().expect("create fake home");
    let binaryish = fixture.path().join("data.bin");
    fs::write(&binaryish, [0xff, b'\n']).expect("write non-utf8 file");

    let output = run_vektor(&fake_home, vec![str_arg("index"), path_arg(fixture.path())]);

    assert_success(&output);
    let stdout = stdout(&output);
    assert!(stdout.contains("changed: 1"), "{stdout}");
    assert!(stdout.contains("failed: 0"), "{stdout}");
    assert!(stdout.contains("chunks: 1"), "{stdout}");
}

fn run_vektor(fake_home: &tempfile::TempDir, args: Vec<OsString>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_vektor"));
    command
        .args(args)
        .env("HOME", fake_home.path())
        .env("USERPROFILE", fake_home.path());

    for key in VEKTOR_ENV_KEYS {
        command.env_remove(key);
    }

    command.output().expect("run vektor")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "status: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        stdout(output),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn write_file(path: impl AsRef<Path>, content: &str) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent dir");
    }
    fs::write(path, content).expect("write file");
}

fn str_arg(value: &str) -> OsString {
    OsString::from(value)
}

fn path_arg(path: impl AsRef<Path>) -> OsString {
    path.as_ref().as_os_str().to_os_string()
}

fn contains_file_named(path: impl AsRef<Path>, file_name: &str) -> bool {
    let Ok(entries) = fs::read_dir(path) else {
        return false;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.file_name().and_then(|name| name.to_str()) == Some(file_name) {
            return true;
        }
        if path.is_dir() && contains_file_named(path, file_name) {
            return true;
        }
    }

    false
}

const VEKTOR_ENV_KEYS: &[&str] = &[
    "VEKTOR__EMBEDDING__BACKEND",
    "VEKTOR__EMBEDDING__OPENAI_API_KEY",
    "VEKTOR__EMBEDDING__OPENAI_BASE_URL",
    "VEKTOR__EMBEDDING__OPENAI_MODEL",
    "VEKTOR__EMBEDDING__OLLAMA_URL",
    "VEKTOR__EMBEDDING__OLLAMA_MODEL",
    "VEKTOR__EMBEDDING__ONNX_MODEL",
    "VEKTOR__EMBEDDING__FALLBACK_TO_ONNX",
    "VEKTOR__EMBEDDING__MAX_REQUESTS_PER_MINUTE",
    "VEKTOR__INDEX__DATA_DIR",
    "VEKTOR__INDEX__MAX_FILE_SIZE_KB",
    "VEKTOR__INDEX__CHUNK_MAX_LINES",
    "VEKTOR__INDEX__CHUNK_OVERLAP_PCT",
    "VEKTOR__INDEX__DOC_CHUNK_MAX_LINES",
    "VEKTOR__WATCHER__DEBOUNCE_MS",
    "VEKTOR__WATCHER__ENABLED",
    "VEKTOR__SERVER__MODE",
];
