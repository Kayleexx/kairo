#![allow(clippy::expect_used)]

use std::path::PathBuf;

use kairo_core::{Config, WorkflowMode};
use kairo_runtime::{ComponentRole, detect_contract};

fn fixture(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

#[test]
fn detects_a_scalar_component() {
    let contract = detect_contract(
        &fixture("demos/basic/multiply-by-nine.wat"),
        Config::default(),
    )
    .expect("scalar component should be detected");
    assert_eq!(contract.role, ComponentRole::ScalarStage);
    assert_eq!(contract.mode(), WorkflowMode::Scalar);
}

#[test]
fn detects_a_value_component() {
    let contract = detect_contract(
        &fixture("components/runtime/value-echo/component.wasm"),
        Config::default(),
    )
    .expect("value component should be detected");
    assert_eq!(contract.role, ComponentRole::ValueStage);
    assert_eq!(contract.mode(), WorkflowMode::Value);
}

#[test]
fn detects_a_stream_consume_metrics_component_standalone() {
    let contract = detect_contract(
        &fixture("components/reference/doc/component.wasm"),
        Config::default(),
    )
    .expect("stream consume-metrics component should be detected on its own");
    assert_eq!(contract.role, ComponentRole::StreamConsumeMetrics);
    assert_eq!(contract.mode(), WorkflowMode::Stream);
    assert_eq!(contract.shape(), "byte stream \u{2192} value");
}

#[test]
fn detects_a_stream_output_component() {
    let contract = detect_contract(
        &fixture("components/reference/doc-redact/component.wasm"),
        Config::default(),
    )
    .expect("stream output component should be detected");
    assert_eq!(contract.role, ComponentRole::StreamOutput);
    assert_eq!(contract.shape(), "byte stream \u{2192} artifact");
}

#[test]
fn detects_a_stream_transform_component() {
    let contract = detect_contract(&fixture("demos/stream/transform.wat"), Config::default())
        .expect("stream transform component should be detected");
    assert_eq!(contract.role, ComponentRole::StreamTransform);
}

#[test]
fn reports_none_for_a_nonexistent_component() {
    assert!(
        detect_contract(
            &fixture("components/does-not-exist.wasm"),
            Config::default()
        )
        .is_none()
    );
}

#[test]
fn a_transform_may_precede_anything_stream_but_a_terminal_may_precede_nothing() {
    assert!(ComponentRole::StreamOutput.can_follow(Some(ComponentRole::StreamTransform)));
    assert!(ComponentRole::StreamTransform.can_follow(Some(ComponentRole::StreamTransform)));
    assert!(!ComponentRole::StreamTransform.can_follow(Some(ComponentRole::StreamOutput)));
    assert!(!ComponentRole::StreamConsume.can_follow(Some(ComponentRole::StreamConsumeMetrics)));
    assert!(ComponentRole::ValueStage.can_follow(None));
    assert!(!ComponentRole::ValueStage.can_follow(Some(ComponentRole::ScalarStage)));
}
