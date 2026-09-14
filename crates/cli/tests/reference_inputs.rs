#![allow(clippy::expect_used)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Input(PathBuf);

impl Input {
    fn new(extension: &str, bytes: &[u8]) -> Self {
        let path = std::env::temp_dir().join(format!(
            "kairo-reference-{}-{}.{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            extension
        ));
        fs::write(&path, bytes).expect("input should be written");
        Self(path)
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[test]
fn parses_multiple_y4m_frames_and_chroma_modes() {
    let mono = Input::new(
        "y4m",
        b"YUV4MPEG2 W2 H1 F1:1 Ip A0:0 Cmono\nFRAME\n\x0a\x14FRAME\n\x1e\x28",
    );
    assert_run(
        "video",
        mono.0.as_path(),
        "2 frames-analyzed · 25 average-luma · 20 average-luma-change · 2 width · 1 height\n",
    );

    let color = Input::new(
        "y4m",
        b"YUV4MPEG2 W2 H2 F1:1 Ip A0:0 C420jpeg\nFRAME\n\x0a\x14\x1e\x28\xff\xff",
    );
    assert_run(
        "video",
        color.0.as_path(),
        "1 frames-analyzed · 25 average-luma · 0 average-luma-change · 2 width · 2 height\n",
    );
}

#[test]
fn validates_y4m_content_instead_of_the_extension() {
    for bytes in [
        b"not a y4m file".as_slice(),
        b"YUV4MPEG2 W2 H2 C420jpeg\nFRAME\n\x01\x02".as_slice(),
        b"YUV4MPEG2 W99999 H2 Cmono\nFRAME\n".as_slice(),
    ] {
        let input = Input::new("y4m", bytes);
        assert_run_fails("video", input.0.as_path());
    }
}

#[test]
fn parses_jsonl_and_csv_invoice_records() {
    let jsonl = Input::new(
        "jsonl",
        br#" { "status": "paid", "amount_cents": 1200, "id": "a\u0031" }
{"id":"b","amount_cents":50,"status":"open"}
"#,
    );
    assert_run(
        "invoice",
        jsonl.0.as_path(),
        "2 records · 1250 total-cents\n",
    );

    let csv = Input::new(
        "csv",
        b"id,amount_cents,status\r\n\"invoice,1\",\"1200\",paid\r\ninvoice-2,50,\"open\"\r\n",
    );
    assert_run("invoice", csv.0.as_path(), "2 records · 1250 total-cents\n");
}

#[test]
fn rejects_malformed_or_oversized_invoice_records() {
    for bytes in [
        b"{\"id\":\"a\",\"amount_cents\":1}\n".to_vec(),
        b"id,amount_cents,status\na,1\n".to_vec(),
        b"id,amount_cents,status\na,4294967296,paid\n".to_vec(),
        format!("id,amount_cents,status\n{},1,paid\n", "a".repeat(65_536)).into_bytes(),
    ] {
        let input = Input::new("data", &bytes);
        assert_run_fails("invoice", input.0.as_path());
    }
}

#[test]
fn analyzes_utf8_text_documents() {
    let text = Input::new("txt", b"hello world\n\nsecond line\n");
    assert_run(
        "doc",
        text.0.as_path(),
        "3 lines · 4 words · 25 characters · 2 paragraphs · 11 longest-line\n",
    );
}

#[test]
fn extracts_text_from_a_real_docx_container() {
    assert_run(
        "doc",
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/document.docx"),
        "2 lines · 7 words · 54 characters · 1 paragraphs · 27 longest-line\n",
    );
}

#[test]
fn detects_docx_content_without_trusting_the_extension() {
    let bytes =
        fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/document.docx"))
            .expect("DOCX fixture should be readable");
    let renamed = Input::new("txt", &bytes);

    assert_run(
        "doc",
        &renamed.0,
        "2 lines · 7 words · 54 characters · 1 paragraphs · 27 longest-line\n",
    );
}

#[test]
fn rejects_malformed_and_expansive_docx_containers() {
    let malformed = Input::new("docx", b"PK\x03\x04not-a-docx");
    assert_run_error("doc", &malformed.0, "DOCX ZIP container is invalid");
    assert_run_error(
        "doc",
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/document-bomb.docx"),
        "expanded-size limit",
    );
}

#[test]
fn preserves_utf8_characters_split_across_stream_batches() {
    let mut bytes = vec![b'a'; 65_535];
    bytes.extend_from_slice("😀".as_bytes());
    let text = Input::new("txt", &bytes);
    assert_run(
        "doc",
        text.0.as_path(),
        "1 lines · 1 words · 65536 characters · 1 paragraphs · 65536 longest-line\n",
    );
}

#[test]
fn rejects_binary_documents_with_a_useful_component_error() {
    for (bytes, expected) in [
        (b"%PDF-1.7\n".as_slice(), "PDF extraction is unavailable"),
        (
            b"PK\x03\x04not-a-docx".as_slice(),
            "DOCX ZIP container is invalid",
        ),
        (b"text\0binary".as_slice(), "document is binary data"),
    ] {
        let input = Input::new("txt", bytes);
        assert_run_error("doc", input.0.as_path(), expected);
    }
}

#[test]
fn bounds_text_document_input() {
    let input = Input::new("txt", &vec![b'a'; 96 * 1024 + 1]);
    assert_run_error("doc", input.0.as_path(), "96 KiB limit");
}

#[test]
fn positional_input_and_legacy_flag_are_exclusive() {
    let input = Input::new("csv", b"id,amount_cents,status\na,1,paid\n");
    let output = command()
        .args(["run", "doc"])
        .arg(&input.0)
        .arg("--input-file")
        .arg(&input.0)
        .output()
        .expect("argument validation should run");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be used with"));
}

#[test]
fn does_not_claim_worker_recovery_for_local_stream_input() {
    let input = Input::new("y4m", b"YUV4MPEG2 W1 H1 Cmono\nFRAME\n\x20");
    let output = command()
        .args(["run", "video"])
        .arg(&input.0)
        .args(["--watch", "--workers", "2"])
        .output()
        .expect("unsupported worker request should be rejected");

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("`--workers` is not used by local stream workflows")
    );
}

#[test]
fn reports_missing_and_non_file_inputs() {
    let missing = std::env::temp_dir().join("kairo-definitely-missing-input.y4m");
    let output = command()
        .args(["run", "video"])
        .arg(&missing)
        .output()
        .expect("missing input should be rejected");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("was not found"));

    let output = command()
        .args(["run", "video"])
        .arg(std::env::temp_dir())
        .output()
        .expect("directory input should be rejected");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not a regular file"));
}

#[test]
fn explains_when_a_workflow_has_no_default_input() {
    let transform = root().join("demos/stream/transform.wat");
    let consume = root().join("demos/stream/consume.wat");
    let workflow = Input::new(
        "yaml",
        format!(
            "workflow: custom-stream\nmode: stream\nsteps:\n  - {{ name: pass, component: {} }}\n  - {{ name: consume, component: {} }}\nedges:\n  - {{ from: pass, to: consume }}\n",
            transform.display(),
            consume.display()
        )
        .as_bytes(),
    );
    let output = command()
        .arg("run")
        .arg(&workflow.0)
        .output()
        .expect("missing default should be rejected");
    let error = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(error.contains("workflow `custom-stream` needs a file input"));
    assert!(error.contains("kairo run custom-stream <file>"));
}

#[test]
fn records_general_input_provenance_for_inspect() {
    let directory = std::env::temp_dir().join(format!(
        "kairo-provenance-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&directory).expect("run directory should be created");
    let input = Input::new("txt", b"one two\n");
    let workflow = root().join("demos/reference/doc/workflow.yaml");
    let run = Command::new(env!("CARGO_BIN_EXE_kairo"))
        .current_dir(&directory)
        .arg("run")
        .arg(workflow)
        .arg(&input.0)
        .args(["--run", "input-provenance"])
        .output()
        .expect("stream workflow should run");
    assert!(run.status.success(), "run failed: {run:?}");

    let inspection = Command::new(env!("CARGO_BIN_EXE_kairo"))
        .current_dir(&directory)
        .args(["inspect", "input-provenance"])
        .output()
        .expect("stream run should be inspectable");
    let stdout = String::from_utf8_lossy(&inspection.stdout);
    assert!(
        inspection.status.success(),
        "inspect failed: {inspection:?}"
    );
    assert!(stdout.contains("input · kairo-reference-"));
    assert!(stdout.contains(" · user"));
    assert!(stdout.contains("accepts · txt"));
    assert!(stdout.contains("identity · sha256:"));
    assert!(stdout.contains("locality · local · rerun requires this input"));
    assert!(
        stdout
            .contains("output · 1 lines · 2 words · 8 characters · 1 paragraphs · 7 longest-line")
    );
    assert!(!stdout.contains(&input.0.display().to_string()));
    let _ = fs::remove_dir_all(directory);
}

fn assert_run(workflow: &str, input: &Path, expected: &str) {
    let output = command()
        .args(["run", workflow])
        .arg(input)
        .output()
        .expect("reference workflow should run");
    assert!(output.status.success(), "{workflow} failed: {output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
}

fn assert_run_fails(workflow: &str, input: &Path) {
    assert_run_error(workflow, input, "stream step");
}

fn assert_run_error(workflow: &str, input: &Path, expected: &str) {
    let output = command()
        .args(["run", workflow])
        .arg(input)
        .output()
        .expect("invalid input should be rejected");
    assert!(!output.status.success(), "invalid input was accepted");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(expected),
        "unexpected error: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kairo"));
    command.current_dir(root());
    command
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
