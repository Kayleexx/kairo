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

/// `kairo workflows` must never hide a valid project-local workflow just because it has no
/// `description` -- that curation bar only applies to the bundled `demos/reference` catalog.
#[test]
fn a_project_local_workflow_without_a_description_still_appears() {
    let directory = Directory::new("workflow-discovery");
    fs::create_dir(directory.0.join("workflows")).expect("workflows directory should be created");
    fs::write(
        directory.0.join("workflows/no-description.yaml"),
        format!(
            "workflow: no-description\ninput: 3\nsteps:\n  - name: multiply\n    component: {}\nedges: []\n",
            repository_path("demos/basic/multiply-by-nine.wat").display(),
        ),
    )
    .expect("workflow file should be written");

    let output = kairo()
        .current_dir(&directory.0)
        .arg("workflows")
        .output()
        .expect("kairo workflows should run");
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("no-description"),
        "workflow without a description should still be listed: {stdout}"
    );
}
