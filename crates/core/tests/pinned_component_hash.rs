#![allow(clippy::expect_used)]

use std::path::Path;

use kairo_core::{ComponentHash, Workflow, WorkflowError};

const VALID_HASH: &str = "sha256:abababababababababababababababababababababababababababababababab";

#[test]
fn parses_a_pinned_hash_into_the_step() {
    let source = format!(
        "workflow: records\ninput: 1\nsteps:\n  - name: step\n    component: step.wat\n    hash: {VALID_HASH}\nedges: []\n"
    );
    let workflow = Workflow::parse(&source, Path::new("."), 8).expect("workflow should parse");

    assert_eq!(
        workflow.steps()[0].pinned_hash,
        Some(VALID_HASH.parse::<ComponentHash>().expect("valid hash"))
    );
}

#[test]
fn omitted_hash_leaves_the_step_unpinned() {
    let source =
        "workflow: records\ninput: 1\nsteps:\n  - name: step\n    component: step.wat\nedges: []\n";
    let workflow = Workflow::parse(source, Path::new("."), 8).expect("workflow should parse");

    assert_eq!(workflow.steps()[0].pinned_hash, None);
}

#[test]
fn rejects_a_malformed_pinned_hash() {
    for hash in ["not-a-hash", "sha256:short", "md5:0123456789abcdef"] {
        let source = format!(
            "workflow: records\ninput: 1\nsteps:\n  - name: step\n    component: step.wat\n    hash: \"{hash}\"\nedges: []\n"
        );
        assert!(matches!(
            Workflow::parse(&source, Path::new("."), 8),
            Err(WorkflowError::InvalidPinnedHash { step }) if step == "step"
        ));
    }
}
