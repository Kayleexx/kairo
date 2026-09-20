#![allow(clippy::expect_used)]

use std::path::{Path, PathBuf};

use kairo_core::{ComponentRole, DraftStep, Durability, Workflow, WorkflowDraft, WorkflowMode};

#[test]
fn canonical_draft_round_trips_through_the_runtime_workflow_model() {
    let source = WorkflowDraft {
        name: "business-flow".to_owned(),
        description: Some("Process a customer's document".to_owned()),
        accepts: vec!["document".to_owned()],
        produces: vec!["analysis".to_owned()],
        mode: WorkflowMode::Value,
        scalar_input: 0,
        steps: vec![
            DraftStep {
                name: "prepare".to_owned(),
                component: PathBuf::from("components/prepare/component.wasm"),
                hash: None,
            },
            DraftStep {
                name: "analyze".to_owned(),
                component: PathBuf::from("components/analyze/component.wasm"),
                hash: None,
            },
        ],
        durabilities: vec![Durability::Auto],
        output: None,
        wait: None,
        effect: None,
    }
    .to_yaml()
    .expect("draft should serialize");

    let workflow =
        Workflow::parse(&source, Path::new("."), 8).expect("serialized workflow should parse");
    assert_eq!(workflow.name(), "business-flow");
    assert_eq!(
        workflow.description(),
        Some("Process a customer's document")
    );
    assert_eq!(workflow.accepts(), ["document"]);
    assert_eq!(workflow.produces(), ["analysis"]);
    assert_eq!(workflow.durability_after_step(0), Durability::Auto);
}

#[test]
fn terminal_stream_roles_finish_but_cannot_accept_another_component() {
    assert!(ComponentRole::StreamConsume.finishable(2));
    assert!(!ComponentRole::StreamConsume.can_continue());
    assert!(ComponentRole::StreamTransform.can_continue());
}
