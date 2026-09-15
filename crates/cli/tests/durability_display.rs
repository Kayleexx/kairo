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

fn repository_path(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

#[test]
fn labels_an_auto_edge_as_unprofiled_in_the_workflow_graph() {
    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
    let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "kairo-durability-display-{}-{sequence}",
        process::id()
    ));
    fs::create_dir(&directory).expect("fixture directory should be created");
    let workflow = directory.join("workflow.yaml");
    fs::write(
        &workflow,
        format!(
            "workflow: auto-durability-test\ninput: 20\nsteps:\n  - name: multiply\n    component: {}\n  - name: divide\n    component: {}\nedges:\n  - from: multiply\n    to: divide\n    durability: auto\n",
            repository_path("demos/basic/multiply-by-nine.wat").display(),
            repository_path("demos/basic/divide-by-five.wat").display(),
        ),
    )
    .expect("workflow should be written");

    let output = kairo()
        .arg("workflows")
        .arg(&workflow)
        .output()
        .expect("kairo should start");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("output should be UTF-8");
    assert!(
        stdout
            .contains("auto (no profile yet · run `kairo workflow profile auto-durability-test`)"),
        "expected an unprofiled-auto label naming the real user command, got: {stdout}"
    );

    let _ = fs::remove_dir_all(directory);
}
