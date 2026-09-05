#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;

use kairo_core::Config;
use kairo_runtime::Runtime;

#[test]
fn loads_probe_component() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../components/probe/component.wat");
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");

    runtime
        .load_component(path)
        .expect("probe component should load");
}

#[test]
fn parses_probe_world() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../wit");

    wit_parser::Resolve::new()
        .push_dir(path)
        .expect("probe WIT world should parse");
}

#[tokio::test]
async fn invokes_probe_component() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../components/probe/component.wat");
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let component = runtime
        .load_component(path)
        .expect("probe component should load");

    let result = runtime
        .run_component(&component)
        .await
        .expect("probe component should run");

    assert_eq!(result.output, 42);
    assert_eq!(result.component_hash, component.hash());
}
