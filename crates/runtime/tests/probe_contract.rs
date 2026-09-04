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
