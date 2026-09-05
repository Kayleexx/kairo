#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    error::Error as _,
    fs,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
};

use kairo_core::Config;
use kairo_runtime::{Runtime, RuntimeError};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str, contents: &str) -> Self {
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("kairo-{name}-{}-{sequence}.wat", process::id()));
        fs::write(&path, contents).expect("fixture should be written");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn probe_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../components/probe/component.wat")
}

#[test]
fn produces_stable_content_hashes() {
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let first = runtime
        .load_component(probe_path())
        .expect("probe component should load");
    let second = runtime
        .load_component(probe_path())
        .expect("probe component should load again");

    assert_eq!(first.hash(), second.hash());
    assert!(first.hash().to_string().starts_with("sha256:"));
    assert_eq!(first.hash().to_string().len(), 71);
}

#[test]
fn rejects_oversized_components() {
    let fixture = Fixture::new("oversized", "(component)");
    let runtime = Runtime::new(Config {
        max_component_bytes: 4,
        ..Config::default()
    })
    .expect("runtime should initialize");

    let error = runtime
        .load_component(fixture.path())
        .err()
        .expect("oversized component should fail");

    assert!(matches!(
        error,
        RuntimeError::ComponentTooLarge { max_bytes: 4, .. }
    ));
}

#[test]
fn preserves_component_io_sources() {
    let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("kairo-missing-{}-{sequence}", process::id()));
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");

    let error = runtime
        .load_component(path)
        .err()
        .expect("missing component should fail");

    assert!(matches!(&error, RuntimeError::OpenComponent { .. }));
    assert!(error.source().is_some());
}

#[test]
fn rejects_malformed_components() {
    let fixture = Fixture::new("malformed", "not a component");
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");

    let error = runtime
        .load_component(fixture.path())
        .err()
        .expect("malformed component should fail");

    assert!(matches!(error, RuntimeError::InvalidComponent { .. }));
}

#[tokio::test]
async fn rejects_ungranted_imports() {
    let fixture = Fixture::new(
        "ungranted-import",
        r#"(component
            (type $host-type (func))
            (import "host-action" (func $host-action (type $host-type)))
            (core module $probe
                (func (export "ping") (result i32) i32.const 42))
            (core instance $probe-instance (instantiate $probe))
            (type $ping-type (func async (result u32)))
            (func $ping (type $ping-type)
                (canon lift (core func $probe-instance "ping")))
            (export "ping" (func $ping)))"#,
    );
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let component = runtime
        .load_component(fixture.path())
        .expect("component should load");

    let error = runtime
        .run_component(&component)
        .await
        .expect_err("ungranted import should fail");

    assert!(matches!(&error, RuntimeError::Instantiate { .. }));
    if let RuntimeError::Instantiate { source } = error {
        assert!(source.to_string().contains("host-action"));
    }
}

#[tokio::test]
async fn rejects_the_wrong_interface() {
    let fixture = Fixture::new(
        "wrong-interface",
        r#"(component
            (core module $probe
                (func (export "pong") (result i32) i32.const 42))
            (core instance $probe-instance (instantiate $probe))
            (type $pong-type (func async (result u32)))
            (func $pong (type $pong-type)
                (canon lift (core func $probe-instance "pong")))
            (export "pong" (func $pong)))"#,
    );
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let component = runtime
        .load_component(fixture.path())
        .expect("component should load");

    assert!(matches!(
        runtime.run_component(&component).await,
        Err(RuntimeError::Instantiate { .. })
    ));
}

#[tokio::test]
async fn rejects_components_over_the_memory_limit() {
    let fixture = Fixture::new(
        "memory-limit",
        r#"(component
            (core module $probe
                (memory 2)
                (func (export "ping") (result i32) i32.const 42))
            (core instance $probe-instance (instantiate $probe))
            (type $ping-type (func async (result u32)))
            (func $ping (type $ping-type)
                (canon lift (core func $probe-instance "ping")))
            (export "ping" (func $ping)))"#,
    );
    let runtime = Runtime::new(Config {
        max_memory_bytes: 64 * 1024,
        ..Config::default()
    })
    .expect("runtime should initialize");
    let component = runtime
        .load_component(fixture.path())
        .expect("component should load");

    assert!(matches!(
        runtime.run_component(&component).await,
        Err(RuntimeError::MemoryLimitExceeded {
            max_memory_bytes,
            ..
        }) if max_memory_bytes == 64 * 1024
    ));
}

#[tokio::test]
async fn stops_components_that_exhaust_fuel() {
    let fixture = Fixture::new(
        "fuel",
        r#"(component
            (core module $probe
                (func (export "ping") (result i32)
                    (loop $spin (br $spin))
                    i32.const 42))
            (core instance $probe-instance (instantiate $probe))
            (type $ping-type (func async (result u32)))
            (func $ping (type $ping-type)
                (canon lift (core func $probe-instance "ping")))
            (export "ping" (func $ping)))"#,
    );
    let runtime = Runtime::new(Config {
        execution_fuel: 100,
        ..Config::default()
    })
    .expect("runtime should initialize");
    let component = runtime
        .load_component(fixture.path())
        .expect("component should load");

    let error = runtime
        .run_component(&component)
        .await
        .expect_err("runaway component should fail");

    assert!(matches!(
        error,
        RuntimeError::FuelExhausted { fuel: 100, .. }
    ));
}
