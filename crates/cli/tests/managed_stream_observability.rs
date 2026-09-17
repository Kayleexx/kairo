#![allow(clippy::expect_used)]

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
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "kairo-managed-stream-observation-{}-{}",
            process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("fixture directory should be created");
        Self(path)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = kairo().current_dir(&self.0).arg("down").output();
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_stream_workflow_runs_through_workers_and_keeps_inspectable_metadata() {
    let directory = Directory::new();
    let run = kairo()
        .current_dir(&directory.0)
        .args(["run"])
        .arg(repository_path(
            "demos/reference/video-processing/workflow.yaml",
        ))
        .args(["--watch", "--workers", "2", "--run", "managed-stream"])
        .output()
        .expect("managed stream should run");
    assert!(run.status.success(), "{run:?}");
    assert!(
        String::from_utf8_lossy(&run.stdout).contains("frames-analyzed"),
        "{run:?}"
    );

    let inspect = kairo()
        .current_dir(&directory.0)
        .args(["inspect", "managed-stream"])
        .output()
        .expect("managed stream should be inspectable");
    assert!(inspect.status.success(), "{inspect:?}");
    let rendered = String::from_utf8_lossy(&inspect.stdout);
    assert!(rendered.contains("state · completed"), "{rendered}");
    assert!(
        rendered.contains("input · sample.y4m · bundled"),
        "{rendered}"
    );
    assert!(
        rendered.contains("observed transport · remote-live"),
        "{rendered}"
    );
    assert!(
        rendered.contains("workers · worker-") && rendered.contains(" → worker-"),
        "{rendered}"
    );

    let explain = kairo()
        .current_dir(&directory.0)
        .args(["explain", "managed-stream"])
        .output()
        .expect("managed stream should be explainable");
    assert!(explain.status.success(), "{explain:?}");
    let explained = String::from_utf8_lossy(&explain.stdout);
    assert!(
        explained.contains("observed transport  remote-live"),
        "{explained}"
    );
    assert!(
        explained.contains("workers             worker-") && explained.contains(" → worker-"),
        "{explained}"
    );
}

#[test]
fn a_durable_stream_boundary_reuses_its_artifact_through_workers() {
    let directory = Directory::new();
    let input = directory.0.join("input.bin");
    fs::write(&input, vec![7_u8; 512 * 1024]).expect("large input");
    let transform = repository_path("demos/stream/transform.wat");
    let consume = repository_path("demos/stream/consume.wat");
    fs::write(
        directory.0.join("workflow.yaml"),
        format!(
            "workflow: durable-stream\nmode: stream\ninput: {}\nsteps:\n  - name: copy\n    component: {}\n  - name: count\n    component: {}\nedges:\n  - from: copy\n    to: count\n    durability: required\n",
            input.display(),
            transform.display(),
            consume.display(),
        ),
    )
    .expect("workflow should be written");
    let run = kairo()
        .current_dir(&directory.0)
        .args([
            "run",
            "workflow.yaml",
            "--watch",
            "--workers",
            "2",
            "--run",
            "durable-stream-run",
        ])
        .output()
        .expect("durable managed stream should run");
    assert!(run.status.success(), "{run:?}");
    assert!(directory.0.join(".kairo/durable-stream-run.db").exists());
    assert!(String::from_utf8_lossy(&run.stdout).contains("bytes"));
}
