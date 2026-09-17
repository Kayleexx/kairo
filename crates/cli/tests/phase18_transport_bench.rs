#![cfg(unix)]
#![allow(clippy::expect_used)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::{self, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use kairo_control::LiveEdgeState;

const SIZES: [usize; 3] = [64 * 1024, 4 * 1024 * 1024, 16 * 1024 * 1024];
const WARMUPS: usize = 1;
const REPETITIONS: usize = 5;

#[derive(Clone, Copy, Debug)]
enum Transport {
    LocalDirect,
    RemoteLive,
    DurableArtifact,
}

impl Transport {
    fn label(self) -> &'static str {
        match self {
            Self::LocalDirect => "local-direct",
            Self::RemoteLive => "remote-live",
            Self::DurableArtifact => "durable-artifact",
        }
    }
}

#[derive(Debug)]
struct Sample {
    transport: Transport,
    bytes: u64,
    wall_us: u64,
    bytes_sent: u64,
    bytes_received: u64,
    durable_bytes: u64,
    peak_buffered_bytes: Option<u64>,
}

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
            "kairo-phase18-bench-{}-{}",
            process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("benchmark directory");
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
#[ignore = "real Phase 18 transport benchmark"]
fn compares_real_stream_transports() {
    let directory = Directory::new();
    let mut service = kairo()
        .current_dir(&directory.0)
        .args(["serve", "--workers", "2"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("service");
    wait_for_workers(&directory.0);
    let mut samples = Vec::new();
    for size in SIZES {
        for attempt in 0..WARMUPS + REPETITIONS {
            let byte = (attempt as u8).wrapping_add(1);
            let input = directory.0.join(format!("input-{size}-{attempt}.bin"));
            fs::write(&input, vec![byte; size]).expect("benchmark input");
            let mut transports = [
                Transport::LocalDirect,
                Transport::RemoteLive,
                Transport::DurableArtifact,
            ];
            transports.rotate_left(attempt % 3);
            for transport in transports {
                let sample = run_sample(&directory.0, input.as_path(), byte, transport, attempt);
                if attempt >= WARMUPS {
                    samples.push(sample);
                }
            }
        }
    }
    print_report(&samples);
    let _ = kairo().current_dir(&directory.0).arg("down").output();
    let _ = service.wait();
}

fn run_sample(
    directory: &Path,
    input: &Path,
    byte: u8,
    transport: Transport,
    attempt: usize,
) -> Sample {
    let size = fs::metadata(input).expect("input metadata").len();
    let id = format!("bench-{}-{size}-{attempt}", transport.label());
    let workflow = directory.join(format!("{id}.yaml"));
    write_workflow(&workflow, input, transport);
    let artifact_root = directory.join(".kairo/artifacts");
    let durable_before = directory_bytes(&artifact_root);
    let started = Instant::now();
    let mut command = kairo();
    command
        .current_dir(directory)
        .arg("run")
        .arg(&workflow)
        .args(["--run", &id]);
    if !matches!(transport, Transport::LocalDirect) {
        command.arg("--watch");
    }
    let output = command.output().expect("benchmark run");
    let wall_us = started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
    assert!(output.status.success(), "{output:?}");
    let state = directory.join(".kairo").join(format!("{id}.db"));
    let inspection = kairo_runtime::inspect_stream_run(&state)
        .expect("stream inspection")
        .expect("stream run");
    let checksum = (byte as u32).wrapping_mul(size as u32);
    assert_eq!(inspection.high, Some(size));
    assert_eq!(inspection.low, Some(checksum));

    let (bytes_sent, bytes_received, peak_buffered_bytes) = match transport {
        Transport::RemoteLive => remote_metrics(directory, &id, size),
        Transport::LocalDirect => {
            let endpoint = kairo_control::load_endpoint(&directory.join(".kairo"))
                .expect("benchmark endpoint");
            let snapshot = kairo_control::snapshot(&endpoint).expect("benchmark snapshot");
            assert!(snapshot.runs.iter().all(|run| run.id != id));
            (
                0,
                0,
                inspection
                    .metrics
                    .as_ref()
                    .map(|metrics| metrics.largest_batch_bytes as u64),
            )
        }
        Transport::DurableArtifact => {
            let endpoint = kairo_control::load_endpoint(&directory.join(".kairo"))
                .expect("benchmark endpoint");
            let snapshot = kairo_control::snapshot(&endpoint).expect("benchmark snapshot");
            assert!(snapshot.live_edges.iter().all(|edge| edge.run_id != id));
            (0, 0, None)
        }
    };
    let durable_bytes = directory_bytes(&artifact_root).saturating_sub(durable_before);
    assert_eq!(
        durable_bytes > 0,
        matches!(transport, Transport::DurableArtifact)
    );
    Sample {
        transport,
        bytes: size,
        wall_us,
        bytes_sent,
        bytes_received,
        durable_bytes,
        peak_buffered_bytes,
    }
}

fn remote_metrics(directory: &Path, id: &str, bytes: u64) -> (u64, u64, Option<u64>) {
    let endpoint =
        kairo_control::load_endpoint(&directory.join(".kairo")).expect("benchmark endpoint");
    let snapshot = kairo_control::snapshot(&endpoint).expect("benchmark snapshot");
    let edge = snapshot
        .live_edges
        .iter()
        .find(|edge| edge.run_id == id && matches!(edge.state, LiveEdgeState::Completed))
        .expect("completed remote-live edge");
    assert_ne!(edge.producer_worker, edge.consumer_worker);
    let observed = edge.observation.as_ref().expect("observed live edge");
    let sent = observed.producer.as_ref().expect("producer metrics");
    let received = observed.consumer.as_ref().expect("consumer metrics");
    assert_eq!(sent.bytes, bytes);
    assert_eq!(received.bytes, bytes);
    (
        sent.bytes,
        received.bytes,
        Some(sent.peak_buffered_bytes.max(received.peak_buffered_bytes)),
    )
}

fn write_workflow(path: &Path, input: &Path, transport: Transport) {
    let durability = matches!(transport, Transport::DurableArtifact)
        .then_some("    durability: required\n")
        .unwrap_or_default();
    fs::write(
        path,
        format!(
            "workflow: phase18-benchmark\nmode: stream\nresources:\n  fuel: 500000000\n  memory_bytes: 33554432\ninput: {}\nsteps:\n  - name: transform\n    component: {}\n  - name: consume\n    component: {}\nedges:\n  - from: transform\n    to: consume\n{durability}",
            input.display(),
            repository_path("demos/stream/transform.wat").display(),
            repository_path("demos/stream/consume.wat").display(),
        ),
    )
    .expect("benchmark workflow");
}

fn wait_for_workers(directory: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(endpoint) = kairo_control::load_endpoint(&directory.join(".kairo"))
            && kairo_control::snapshot(&endpoint).is_ok_and(|snapshot| {
                snapshot
                    .workers
                    .iter()
                    .filter(|worker| worker.healthy)
                    .count()
                    == 2
            })
        {
            return;
        }
        assert!(Instant::now() < deadline, "workers did not register");
        std::thread::yield_now();
    }
}

fn directory_bytes(path: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries.filter_map(Result::ok).fold(0, |total, entry| {
        let path = entry.path();
        total.saturating_add(if path.is_dir() {
            directory_bytes(&path)
        } else {
            entry.metadata().map_or(0, |metadata| metadata.len())
        })
    })
}

fn print_report(samples: &[Sample]) {
    println!(
        "transport,payload_bytes,p50_latency_ms,throughput_mib_s,bytes_sent,bytes_received,durable_bytes,peak_buffered_bytes,ttfb"
    );
    for size in SIZES.map(|size| size as u64) {
        for transport in [
            Transport::LocalDirect,
            Transport::RemoteLive,
            Transport::DurableArtifact,
        ] {
            let selected: Vec<&Sample> = samples
                .iter()
                .filter(|sample| {
                    sample.bytes == size && sample.transport.label() == transport.label()
                })
                .collect();
            let wall_us = median(selected.iter().map(|sample| sample.wall_us));
            let sent = median(selected.iter().map(|sample| sample.bytes_sent));
            let received = median(selected.iter().map(|sample| sample.bytes_received));
            let durable = median(selected.iter().map(|sample| sample.durable_bytes));
            let peak = selected
                .iter()
                .filter_map(|sample| sample.peak_buffered_bytes)
                .max()
                .map_or_else(|| "n/a".to_owned(), |value| value.to_string());
            let throughput = size as f64 / (wall_us as f64 / 1_000_000.0) / (1024.0 * 1024.0);
            println!(
                "{},{size},{:.3},{throughput:.3},{sent},{received},{durable},{peak},n/a",
                transport.label(),
                wall_us as f64 / 1000.0
            );
        }
    }
}

fn median(values: impl Iterator<Item = u64>) -> u64 {
    let mut values: Vec<u64> = values.collect();
    values.sort_unstable();
    values[values.len() / 2]
}
