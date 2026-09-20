#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    fs,
    path::PathBuf,
    process::{self, Command},
    sync::atomic::{AtomicU64, Ordering},
};

fn kairo() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kairo"));
    if let Some(directory) = std::env::var_os("KAIRO_TEST_COMPONENT_TARGET_DIR") {
        command.env("CARGO_TARGET_DIR", directory);
    }
    command
}

struct Directory(PathBuf);

impl Directory {
    fn new(name: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("kairo-{name}-{}-{sequence}", process::id()));
        fs::create_dir(&path).expect("fixture directory should be created");
        Self(path)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn scaffolds_a_component_project_that_refuses_to_overwrite_itself() {
    let directory = Directory::new("component-new");

    let output = kairo()
        .current_dir(&directory.0)
        .args(["component", "new", "order-validate"])
        .output()
        .expect("kairo component new should run");
    assert!(output.status.success(), "{output:?}");

    let project = directory.0.join("components/order-validate");
    assert!(project.join("Cargo.toml").is_file());
    assert!(project.join("wit/workflow.wit").is_file());
    assert!(project.join("src/lib.rs").is_file());

    let manifest = fs::read_to_string(project.join("Cargo.toml")).expect("Cargo.toml should read");
    assert!(manifest.contains("name = \"order-validate\""));
    assert!(manifest.contains("wit-bindgen"));

    // scaffolding again must not silently clobber whatever the user has started writing.
    let repeat = kairo()
        .current_dir(&directory.0)
        .args(["component", "new", "order-validate"])
        .output()
        .expect("kairo component new should run");
    assert!(!repeat.status.success());
}

#[test]
fn rejects_an_invalid_component_name() {
    let directory = Directory::new("component-new-invalid");
    let output = kairo()
        .current_dir(&directory.0)
        .args(["component", "new", "Not Valid!"])
        .output()
        .expect("kairo component new should run");
    assert!(!output.status.success());
    assert!(!directory.0.join("components").exists());
}

/// exercises the real `cargo build` -> `wasm-tools component new` -> `wasm-tools validate`
/// pipeline end to end -- no mocking of the toolchain.
#[test]
fn builds_a_scaffolded_component_into_a_valid_wasm_component() {
    let directory = Directory::new("component-build");
    let scaffold = kairo()
        .current_dir(&directory.0)
        .args(["component", "new", "echo"])
        .output()
        .expect("kairo component new should run");
    assert!(scaffold.status.success(), "{scaffold:?}");

    let build = kairo()
        .current_dir(&directory.0)
        .args(["component", "build", "components/echo"])
        .output()
        .expect("kairo component build should run");
    assert!(build.status.success(), "{build:?}");

    let component_path = directory.0.join("components/echo/component.wasm");
    assert!(component_path.is_file());

    let check = kairo()
        .current_dir(&directory.0)
        .args(["component", "check"])
        .arg(&component_path)
        .output()
        .expect("kairo component check should run");
    assert!(check.status.success(), "{check:?}");
}

/// the full authoring path a new user actually follows -- `component new` scaffolds a
/// `value-stage` component by default, so `workflow create` must recognize that interface and
/// generate a `mode: value` workflow, not assume the legacy scalar `u32` shape.
#[test]
fn creates_and_runs_a_value_workflow_from_a_freshly_scaffolded_component() {
    let directory = Directory::new("component-to-value-workflow");
    let scaffold = kairo()
        .current_dir(&directory.0)
        .args(["component", "new", "shout"])
        .output()
        .expect("kairo component new should run");
    assert!(scaffold.status.success(), "{scaffold:?}");

    let lib_rs = directory.0.join("components/shout/src/lib.rs");
    let source = fs::read_to_string(&lib_rs).expect("scaffolded source should read");
    fs::write(
        &lib_rs,
        source.replace("Ok(input)", "Ok(input.to_ascii_uppercase())"),
    )
    .expect("scaffolded source should update");

    let build = kairo()
        .current_dir(&directory.0)
        .args(["component", "build", "components/shout"])
        .output()
        .expect("kairo component build should run");
    assert!(build.status.success(), "{build:?}");

    let create = kairo()
        .current_dir(&directory.0)
        .args([
            "workflow",
            "create",
            "--name",
            "shout-demo",
            "--component",
            "components/shout/component.wasm",
        ])
        .output()
        .expect("kairo workflow create should run");
    assert!(create.status.success(), "{create:?}");

    let workflow = fs::read_to_string(directory.0.join("shout-demo.yaml"))
        .expect("generated workflow should read");
    assert!(workflow.contains("mode: value"), "{workflow}");

    let input = directory.0.join("input.txt");
    fs::write(&input, "hello kairo").expect("input file should write");
    let run = kairo()
        .current_dir(&directory.0)
        .args(["run", "shout-demo.yaml", "--input-file"])
        .arg(&input)
        .output()
        .expect("kairo run should run");
    assert!(run.status.success(), "{run:?}");
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "HELLO KAIRO");
}

/// `kairo workflow new <name> <component-names...>` should resolve each name against
/// `components/<name>/component.wasm` and need no path or flag at all.
#[test]
fn creates_a_workflow_from_bare_component_names() {
    let directory = Directory::new("component-name-workflow");
    let scaffold = kairo()
        .current_dir(&directory.0)
        .args(["component", "new", "step-one"])
        .output()
        .expect("kairo component new should run");
    assert!(scaffold.status.success(), "{scaffold:?}");

    let build = kairo()
        .current_dir(&directory.0)
        .args(["component", "build", "components/step-one"])
        .output()
        .expect("kairo component build should run");
    assert!(build.status.success(), "{build:?}");

    let create = kairo()
        .current_dir(&directory.0)
        .args(["workflow", "new", "named-flow", "step-one"])
        .output()
        .expect("kairo workflow new should run");
    assert!(create.status.success(), "{create:?}");
    let source = fs::read_to_string(directory.0.join("named-flow.yaml"))
        .expect("generated workflow should read");
    assert!(
        source.contains("components/step-one/component.wasm"),
        "{source}"
    );
}

#[test]
fn workflow_new_rejects_unknown_components_without_scaffolding_them() {
    let directory = Directory::new("unknown-component-workflow");
    assert!(!directory.0.join("components").exists());

    let create = kairo()
        .current_dir(&directory.0)
        .args(["workflow", "new", "locality", "not-registered"])
        .output()
        .expect("kairo workflow new should run");
    assert!(!create.status.success(), "{create:?}");
    assert!(String::from_utf8_lossy(&create.stderr).contains("kairo add"));
    assert!(!directory.0.join("components").exists());
}

/// a normal user should never have to run `kairo workflow profile` themselves before the first
/// `kairo run` on a fresh `durability: auto` edge -- the first run measures it, quietly, and
/// caches the result so the second run reuses it instead of measuring again.
#[test]
fn first_run_profiles_automatically_and_the_profile_keeps_improving() {
    let directory = Directory::new("auto-profile-on-first-run");
    let component = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../components/runtime/value-echo/component.wasm");
    for name in ["warm-up", "finish"] {
        let add = kairo()
            .current_dir(&directory.0)
            .arg("add")
            .arg(&component)
            .args(["--name", name])
            .output()
            .expect("kairo add should run");
        assert!(add.status.success(), "{add:?}");
    }
    let create = kairo()
        .current_dir(&directory.0)
        .args(["workflow", "new", "greet", "warm-up", "finish"])
        .output()
        .expect("kairo workflow new should run");
    assert!(create.status.success(), "{create:?}");

    // no `kairo workflow profile` here on purpose -- this is the behavior under test.
    let first_run = kairo()
        .current_dir(&directory.0)
        .args(["run", "greet.yaml", "--value", "hi"])
        .output()
        .expect("kairo run should run");
    assert!(first_run.status.success(), "{first_run:?}");
    assert_eq!(String::from_utf8_lossy(&first_run.stdout).trim(), "jk");
    let profiles = fs::read_dir(directory.0.join(".kairo/profiles"))
        .expect("profiling should have created a profile directory")
        .count();
    assert_eq!(
        profiles, 1,
        "the first run should have measured and cached a profile on its own"
    );

    let inspect = kairo()
        .current_dir(&directory.0)
        .arg("inspect")
        .output()
        .expect("kairo inspect should run");
    assert!(inspect.status.success(), "{inspect:?}");
    let rendered = String::from_utf8_lossy(&inspect.stdout);
    assert!(
        rendered.contains("recompute") || rendered.contains("checkpoint"),
        "inspect should show a real resolved decision from the auto-measured profile: {rendered}"
    );
    let profile_json = fs::read_to_string(
        fs::read_dir(directory.0.join(".kairo/profiles"))
            .expect("profiles directory should exist")
            .next()
            .expect("a profile file should exist")
            .expect("profile entry should read")
            .path(),
    )
    .expect("profile file should read");

    let second_run = kairo()
        .current_dir(&directory.0)
        .args(["run", "greet.yaml", "--value", "hi"])
        .output()
        .expect("kairo run should run");
    assert!(second_run.status.success(), "{second_run:?}");

    let profiles: Vec<_> = fs::read_dir(directory.0.join(".kairo/profiles"))
        .expect("profiles directory should still exist")
        .collect();
    assert_eq!(
        profiles.len(),
        1,
        "the second run must reuse the cached profile, not write a second one"
    );
    let profile_json_after =
        fs::read_to_string(profiles[0].as_ref().unwrap().path()).expect("profile file should read");

    // the second run must never reset accumulated history -- it either tops up a still-thin
    // profile (this one, with only 2 real samples so far) or, once trusted, just adds its own
    // ordinary observation. Either way the real sample count only ever grows.
    let samples_before = warm_up_samples(&profile_json);
    let samples_after = warm_up_samples(&profile_json_after);
    assert!(
        samples_after > samples_before,
        "profile samples should accumulate across runs, not reset: {samples_before} -> {samples_after}"
    );
}

fn warm_up_samples(profile_json: &str) -> u64 {
    let profile: serde_json::Value =
        serde_json::from_str(profile_json).expect("profile should be valid JSON");
    profile["edges"]["warm-up"]["samples"]
        .as_u64()
        .expect("warm-up edge should have a real sample count")
}
