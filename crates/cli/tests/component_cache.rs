#![allow(clippy::expect_used)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "kairo-component-cache-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("fixture directory should be created");
        Self(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn component(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(name)
}

fn run(fixture: &Fixture, component: &Path) {
    let output = Command::new(env!("CARGO_BIN_EXE_kairo"))
        .current_dir(&fixture.0)
        .env(
            "KAIRO_COMPONENT_CACHE_DIR",
            fixture.0.join("component-cache"),
        )
        .arg("run-component")
        .arg(component)
        .args(["--input", "1"])
        .output()
        .expect("kairo should start");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn compiled_entries(path: &Path) -> usize {
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .map(|path| {
            if path.is_dir() {
                compiled_entries(&path)
            } else {
                usize::from(path.extension().is_none())
            }
        })
        .sum()
}

#[test]
fn reuses_cached_components_and_invalidates_changed_content() {
    let fixture = Fixture::new();
    let cache = fixture.0.join("component-cache/modules");

    run(&fixture, &component("components/runtime/compute.wat"));
    let first = compiled_entries(&cache);
    assert!(first > 0, "the first run should populate the cache");

    run(&fixture, &component("components/runtime/compute.wat"));
    assert_eq!(compiled_entries(&cache), first);

    run(&fixture, &component("components/probe/component.wat"));
    assert!(compiled_entries(&cache) > first);
}
