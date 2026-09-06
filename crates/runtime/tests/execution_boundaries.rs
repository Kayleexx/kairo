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

fn runtime_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../components/runtime")
        .join(name)
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

#[tokio::test]
async fn executes_real_computation() {
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let component = runtime
        .load_component(runtime_fixture("compute.wat"))
        .expect("compute component should load");

    let small = runtime
        .run_component(&component, 100)
        .await
        .expect("compute component should run");
    let large = runtime
        .run_component(&component, 5_000)
        .await
        .expect("compute component should run again");

    assert_eq!(small.output, 25);
    assert_eq!(large.output, 669);
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
async fn denies_console_by_default() {
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let component = runtime
        .load_component(runtime_fixture("console.wat"))
        .expect("component should load");

    let error = runtime
        .run_component(&component, 21)
        .await
        .expect_err("ungranted import should fail");

    assert!(matches!(&error, RuntimeError::Instantiate { .. }));
    if let RuntimeError::Instantiate { source } = error {
        assert!(source.to_string().contains("console"));
    }
}

#[tokio::test]
async fn grants_console_explicitly() {
    let runtime = Runtime::new(Config {
        allow_console: true,
        ..Config::default()
    })
    .expect("runtime should initialize");
    let component = runtime
        .load_component(runtime_fixture("console.wat"))
        .expect("component should load");

    let result = runtime
        .run_component(&component, 21)
        .await
        .expect("granted console import should run");

    assert_eq!(result.output, 42);
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
        runtime.run_component(&component, 0).await,
        Err(RuntimeError::Instantiate { .. })
    ));
}

#[tokio::test]
async fn rejects_components_over_the_memory_limit() {
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let component = runtime
        .load_component(runtime_fixture("memory-limit.wat"))
        .expect("component should load");

    assert!(matches!(
        runtime.run_component(&component, 0).await,
        Err(RuntimeError::MemoryLimitExceeded {
            max_memory_bytes,
            ..
        }) if max_memory_bytes == 64 * 1024 * 1024
    ));
}

#[tokio::test]
async fn stops_components_that_exhaust_fuel() {
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let component = runtime
        .load_component(runtime_fixture("runaway.wat"))
        .expect("component should load");

    let error = runtime
        .run_component(&component, 0)
        .await
        .expect_err("runaway component should fail");

    assert!(matches!(
        error,
        RuntimeError::FuelExhausted {
            fuel: 10_000_000,
            ..
        }
    ));
}
