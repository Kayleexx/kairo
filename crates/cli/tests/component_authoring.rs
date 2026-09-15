#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    fs,
    path::PathBuf,
    process::{self, Command},
    sync::atomic::{AtomicU64, Ordering},
};

fn kairo() -> Command {
    Command::new(env!("CARGO_BIN_EXE_kairo"))
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

/// the headline authoring path: from a totally clean workspace (no `components/` directory at
/// all), `kairo workflow new <name> <component-names...>` must scaffold and build every named
/// component automatically -- a normal user should never have to run `component new`/
/// `component build` by hand first. Then profiles and runs the result end to end with a literal
/// `--value`, proving the generated `durability: auto` edges and the value input both really work.
#[test]
fn workflow_new_scaffolds_builds_profiles_and_runs_end_to_end() {
    let directory = Directory::new("auto-scaffold-workflow");
    assert!(!directory.0.join("components").exists());

    let create = kairo()
        .current_dir(&directory.0)
        .args([
            "workflow",
            "new",
            "locality",
            "warm-up",
            "slow-compute",
            "finish",
        ])
        .output()
        .expect("kairo workflow new should run");
    assert!(create.status.success(), "{create:?}");

    for name in ["warm-up", "slow-compute", "finish"] {
        assert!(
            directory
                .0
                .join("components")
                .join(name)
                .join("component.wasm")
                .is_file(),
            "expected {name} to be scaffolded and built automatically"
        );
    }
    let source =
        fs::read_to_string(directory.0.join("locality.yaml")).expect("workflow should read");
    for name in ["warm-up", "slow-compute", "finish"] {
        assert!(
            source.contains(&format!("components/{name}/component.wasm")),
            "{source}"
        );
    }
    assert!(source.contains("durability: auto"), "{source}");

    let profile = kairo()
        .current_dir(&directory.0)
        .args([
            "workflow",
            "profile",
            "locality",
            "--value",
            "hello",
            "--repetitions",
            "2",
        ])
        .output()
        .expect("kairo workflow profile should run");
    assert!(profile.status.success(), "{profile:?}");

    let run = kairo()
        .current_dir(&directory.0)
        .args(["run", "locality.yaml", "--value", "hello"])
        .output()
        .expect("kairo run should run");
    assert!(run.status.success(), "{run:?}");
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "hello");
}
