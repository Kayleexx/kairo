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

#[test]
fn lists_and_shows_a_saved_report() {
    let directory = Directory::new("bench-reports");
    assert!(
        kairo()
            .current_dir(&directory.0)
            .args(["bench", "run"])
            .arg(repository_path("demos/checkout/workflow.yaml"))
            .args(["--warmups", "0", "--repetitions", "2"])
            .output()
            .expect("bench should run")
            .status
            .success()
    );

    let list = kairo()
        .current_dir(&directory.0)
        .args(["bench", "list"])
        .output()
        .expect("list should run");
    assert!(list.status.success(), "{list:?}");
    let list_stdout = String::from_utf8_lossy(&list.stdout);
    assert!(list_stdout.contains("reports · 1"), "{list_stdout}");
    assert!(list_stdout.contains("2 successes"), "{list_stdout}");

    let name = fs::read_dir(directory.0.join(".kairo/benchmarks"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .file_name();
    let show = kairo()
        .current_dir(&directory.0)
        .args(["bench", "show"])
        .arg(&name)
        .output()
        .expect("show should run");
    assert!(show.status.success(), "{show:?}");
    let show_stdout = String::from_utf8_lossy(&show.stdout);
    assert!(show_stdout.contains("checkout-settlement") || show_stdout.contains("workflow.yaml"));
    assert!(show_stdout.contains("2 successes"), "{show_stdout}");
}

#[test]
fn reports_when_there_is_nothing_to_list() {
    let directory = Directory::new("bench-reports-empty");
    let list = kairo()
        .current_dir(&directory.0)
        .args(["bench", "list"])
        .output()
        .expect("list should run");
    assert!(list.status.success(), "{list:?}");
    assert!(
        String::from_utf8_lossy(&list.stdout).contains("no benchmark reports"),
        "{list:?}"
    );
}
