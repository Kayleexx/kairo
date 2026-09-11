#![allow(clippy::expect_used)]

use std::{fs, path::PathBuf, process::Command};

#[test]
fn runs_reference_stream_workflows_by_name() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (name, expected) in [
        (
            "video",
            b"1 frames \xc2\xb7 66 average-luma \xc2\xb7 17 width \xc2\xb7 1 height\n".as_slice(),
        ),
        (
            "video-processing",
            b"1 frames \xc2\xb7 66 average-luma \xc2\xb7 17 width \xc2\xb7 1 height\n".as_slice(),
        ),
        (
            "doc",
            b"3 lines \xc2\xb7 10 words \xc2\xb7 79 characters \xc2\xb7 2 paragraphs\n".as_slice(),
        ),
        (
            "invoice",
            b"3 records \xc2\xb7 6350 total-cents\n".as_slice(),
        ),
        (
            "document-processing",
            b"3 records \xc2\xb7 6350 total-cents\n".as_slice(),
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_kairo"))
            .args(["run", name])
            .current_dir(&root)
            .output()
            .expect("reference workflow should run");

        assert!(output.status.success(), "{name} failed: {output:?}");
        assert_eq!(output.stdout, expected);
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn rejects_malformed_reference_document_input() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let input = std::env::temp_dir().join(format!("kairo-invalid-{}.jsonl", std::process::id()));
    fs::write(&input, b"not-json\n").expect("invalid input should be written");
    let output = Command::new(env!("CARGO_BIN_EXE_kairo"))
        .args(["run", "invoice", "--input-file"])
        .arg(&input)
        .current_dir(root)
        .output()
        .expect("reference workflow should validate input");
    let _ = fs::remove_file(input);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("summarize-records"));
}

#[test]
fn lists_concise_reference_input_contracts() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new(env!("CARGO_BIN_EXE_kairo"))
        .arg("workflows")
        .current_dir(root)
        .output()
        .expect("workflow discovery should run");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("  video\n    analyze real Y4M or bounded H.264/AVC MP4 video"));
    assert!(stdout.contains("    accepts \u{b7} y4m, mp4-h264"));
    assert!(stdout.contains("  doc\n    extract and summarize text from TXT or DOCX documents"));
    assert!(stdout.contains("    accepts \u{b7} txt, docx"));
    assert!(stdout.contains("  invoice\n    validate and aggregate structured invoice records"));
    assert!(stdout.contains("    accepts \u{b7} jsonl, csv"));
    assert!(!stdout.contains("\n  video-processing\n"));
}
