#![allow(clippy::expect_used)]

use std::path::Path;

use kairo_core::{Workflow, WorkflowError};

#[test]
fn accepts_a_safe_stream_output_declaration() {
    let workflow = Workflow::parse(
        "workflow: output\nmode: stream\noutput:\n  filename: redacted.txt\n  content_type: text/plain\nsteps:\n  - { name: redact, component: redact.wasm }\nedges: []\n",
        Path::new("."),
        8,
    )
    .expect("workflow should parse");

    assert_eq!(
        workflow.output().expect("output is present").filename,
        "redacted.txt"
    );
}

#[test]
fn rejects_unsafe_output_filenames() {
    for filename in ["", "../result.txt", "nested/result.txt", "/result.txt"] {
        let source = format!(
            "workflow: output\nmode: stream\noutput:\n  filename: {filename:?}\n  content_type: text/plain\nsteps:\n  - {{ name: redact, component: redact.wasm }}\nedges: []\n"
        );
        assert!(matches!(
            Workflow::parse(&source, Path::new("."), 8),
            Err(WorkflowError::InvalidOutputFilename)
        ));
    }
}
