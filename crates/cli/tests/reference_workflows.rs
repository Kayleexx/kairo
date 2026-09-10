#![allow(clippy::expect_used)]

use std::{fs, path::PathBuf, process::Command};

#[test]
fn runs_reference_stream_workflows_by_name() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (name, expected) in [
        (
            "video-processing",
            b"1 frames \xc2\xb7 66 average-luma\n".as_slice(),
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
        .args(["run", "document-processing", "--input-file"])
        .arg(&input)
        .current_dir(root)
        .output()
        .expect("reference workflow should validate input");
    let _ = fs::remove_file(input);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("summarize-records"));
}
