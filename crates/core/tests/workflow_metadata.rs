#![allow(clippy::expect_used)]

use std::path::Path;

use kairo_core::{Workflow, WorkflowError};

const STREAM: &str = "mode: stream\ninput: input.bin\nsteps:\n  - name: transform\n    component: transform.wat\n  - name: consume\n    component: consume.wat\nedges:\n  - from: transform\n    to: consume\n";

#[test]
fn parses_bounded_description_and_result_labels() {
    let source = format!(
        "workflow: records\ndescription: aggregate invoice records\naliases: [invoices]\naccepts: [jsonl, csv]\nresult:\n  high: records\n  low: total-cents\n{STREAM}"
    );
    let workflow = Workflow::parse(&source, Path::new("."), 8).expect("metadata should parse");

    assert_eq!(workflow.description(), Some("aggregate invoice records"));
    assert!(workflow.matches_name("records"));
    assert!(workflow.matches_name("invoices"));
    assert!(!workflow.matches_name("orders"));
    assert_eq!(workflow.accepts(), ["jsonl", "csv"]);
    let labels = workflow
        .stream_result_labels()
        .expect("result labels should exist");
    assert_eq!(labels.high, "records");
    assert_eq!(labels.low, "total-cents");
}

#[test]
fn permits_a_stream_input_to_be_supplied_at_run_time() {
    let source = format!(
        "workflow: records\n{}",
        STREAM.replace("input: input.bin\n", "")
    );
    let workflow = Workflow::parse(&source, Path::new("."), 8)
        .expect("stream input may be supplied by the caller");

    assert_eq!(workflow.stream_input(), None);
}

#[test]
fn rejects_ambiguous_discovery_metadata() {
    for metadata in [
        "aliases: [records]\n",
        "aliases: [invoices, invoices]\n",
        "accepts: [jsonl, jsonl]\n",
        "accepts: ['not a format']\n",
    ] {
        let source = format!("workflow: records\n{metadata}{STREAM}");
        assert!(Workflow::parse(&source, Path::new("."), 8).is_err());
    }
}

#[test]
fn rejects_invalid_result_metadata() {
    let empty = format!("workflow: records\nresult:\n  high: ''\n  low: total\n{STREAM}");
    assert!(matches!(
        Workflow::parse(&empty, Path::new("."), 8),
        Err(WorkflowError::InvalidStreamResult)
    ));

    let scalar = "workflow: scalar\ninput: 1\nresult:\n  high: one\n  low: two\nsteps:\n  - name: step\n    component: step.wat\nedges: []\n";
    assert!(matches!(
        Workflow::parse(scalar, Path::new("."), 8),
        Err(WorkflowError::ScalarStreamResult)
    ));
}

#[test]
fn rejects_terminal_control_characters_in_descriptions() {
    let source = format!("workflow: records\ndescription: \"unsafe\\e[2J\"\n{STREAM}");
    assert!(matches!(
        Workflow::parse(&source, Path::new("."), 8),
        Err(WorkflowError::InvalidDescription)
    ));
}
