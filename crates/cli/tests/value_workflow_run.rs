#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    fs,
    path::PathBuf,
    process::{self, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};

fn kairo() -> Command {
    Command::new(env!("CARGO_BIN_EXE_kairo"))
}

fn repository_path(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
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

fn value_workflow(directory: &Directory) -> PathBuf {
    let workflow = directory.0.join("echo-flow.yaml");
    fs::write(
        &workflow,
        format!(
            "workflow: echo-flow\ndescription: echoes a value back unchanged\nmode: value\nio:\n  input: value\naccepts: [text]\nsteps:\n  - name: echo\n    component: {}\nedges: []\n",
            repository_path("components/runtime/value-echo/component.wasm").display(),
        ),
    )
    .expect("workflow should write");
    workflow
}

#[test]
fn runs_a_value_workflow_with_an_explicit_input_file() {
    let directory = Directory::new("value-run-input-file");
    let workflow = value_workflow(&directory);
    let input = directory.0.join("input.txt");
    fs::write(&input, "hello").expect("input file should write");

    let output = kairo()
        .current_dir(&directory.0)
        .arg("run")
        .arg(&workflow)
        .arg("--input-file")
        .arg(&input)
        .output()
        .expect("kairo run should run");
    assert!(output.status.success(), "{output:?}");
    // value-echo increments every byte by one: h->i, e->f, l->m, l->m, o->p
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "ifmmp");
}

#[test]
fn exports_a_value_workflow_result_to_a_file() {
    let directory = Directory::new("value-run-export");
    let workflow = value_workflow(&directory);
    let input = directory.0.join("input.txt");
    fs::write(&input, "hi").expect("input file should write");
    let exported = directory.0.join("result.bin");

    let output = kairo()
        .current_dir(&directory.0)
        .arg("run")
        .arg(&workflow)
        .arg("--input-file")
        .arg(&input)
        .arg("--output")
        .arg(&exported)
        .output()
        .expect("kairo run should run");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(fs::read(&exported).expect("output should exist"), b"ij");
}

/// a non-technical user's plain `kairo run <workflow>` with no flags at all must never crash or
/// silently misbehave when it can't prompt (no terminal attached, e.g. a script/CI pipe) -- it
/// must fail with one clear, actionable message.
#[test]
fn fails_clearly_instead_of_hanging_when_input_is_needed_and_stdin_is_not_a_terminal() {
    let directory = Directory::new("value-run-no-tty");
    let workflow = value_workflow(&directory);

    let output = kairo()
        .current_dir(&directory.0)
        .arg("run")
        .arg(&workflow)
        .stdin(Stdio::piped())
        .output()
        .expect("kairo run should run");
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("needs a value input"), "{stderr}");
}
