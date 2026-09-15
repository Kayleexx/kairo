#![allow(clippy::expect_used)]

use std::path::Path;

use kairo_core::{IoInput, IoOutput, Workflow, WorkflowError};

const SCALAR: &str = "input: 1\nsteps:\n  - name: step\n    component: step.wat\nedges: []\n";

#[test]
fn parses_io_and_produces_metadata() {
    let source = format!(
        "workflow: records\nproduces: [json]\nio:\n  input: value\n  output: artifact\n  filename: result.bin\n{SCALAR}"
    );
    let workflow = Workflow::parse(&source, Path::new("."), 8).expect("io metadata should parse");

    assert_eq!(workflow.produces(), ["json"]);
    assert_eq!(workflow.io().input, IoInput::Value);
    assert_eq!(workflow.io().output, IoOutput::Artifact);
    assert_eq!(workflow.io().filename.as_deref(), Some("result.bin"));
}

#[test]
fn defaults_io_and_produces_when_omitted() {
    let source = format!("workflow: records\n{SCALAR}");
    let workflow = Workflow::parse(&source, Path::new("."), 8).expect("workflow should parse");

    assert!(workflow.produces().is_empty());
    assert_eq!(workflow.io().input, IoInput::None);
    assert_eq!(workflow.io().output, IoOutput::None);
    assert_eq!(workflow.io().filename, None);
}

#[test]
fn rejects_ambiguous_produces_metadata() {
    for metadata in ["produces: [json, json]\n", "produces: ['not a format']\n"] {
        let source = format!("workflow: records\n{metadata}{SCALAR}");
        assert!(matches!(
            Workflow::parse(&source, Path::new("."), 8),
            Err(WorkflowError::InvalidProduces)
        ));
    }
}

#[test]
fn rejects_an_unsafe_io_filename() {
    let source = format!("workflow: records\nio:\n  filename: ../escape\n{SCALAR}");
    assert!(matches!(
        Workflow::parse(&source, Path::new("."), 8),
        Err(WorkflowError::InvalidIoFilename)
    ));
}

#[test]
fn rejects_an_unknown_io_input_kind() {
    let source = format!("workflow: records\nio:\n  input: bogus\n{SCALAR}");
    assert!(matches!(
        Workflow::parse(&source, Path::new("."), 8),
        Err(WorkflowError::InvalidYaml { .. })
    ));
}
