#![allow(clippy::expect_used)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Input(PathBuf);

impl Input {
    fn new(extension: &str, bytes: &[u8]) -> Self {
        let path = std::env::temp_dir().join(format!(
            "kairo-video-{}-{}.{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            extension
        ));
        fs::write(&path, bytes).expect("input should be written");
        Self(path)
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[test]
fn decodes_real_h264_mp4_content() {
    let output = run(&reference_mp4());

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "24 frames-analyzed · 106 average-luma · 160 width · 90 height\n"
    );
}

#[test]
fn watches_a_real_h264_mp4_through_the_normal_run_command() {
    let output = Command::new(env!("CARGO_BIN_EXE_kairo"))
        .current_dir(root())
        .args(["run", "video"])
        .arg(reference_mp4())
        .arg("--watch")
        .output()
        .expect("watched video workflow should run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("24 frames-analyzed"));
}

#[test]
fn validates_mp4_content_instead_of_its_extension() {
    let renamed = Input::new("mp4", b"this is not an MP4 container");
    assert_fails(&renamed.0, "neither Y4M nor MP4 content");

    let bytes = fs::read(reference_mp4()).expect("reference MP4 should be readable");
    let truncated = Input::new("mp4", &bytes[..bytes.len() / 2]);
    assert_fails(&truncated.0, "MP4 container is malformed");
}

#[test]
fn rejects_unsupported_codec_and_chroma() {
    assert_fails(
        &fixture("video-unsupported-codec.mp4"),
        "unsupported MP4 codec",
    );
    assert_fails(&fixture("video-unsupported-chroma.mp4"), "non-4:2:0 chroma");
}

#[test]
fn ignores_audio_while_decoding_the_video_track() {
    let output = run(&fixture("video-with-audio.mp4"));

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "24 frames-analyzed · 112 average-luma · 160 width · 90 height\n"
    );
}

#[test]
fn bounds_mp4_dimensions_and_frame_analysis() {
    assert_fails(&fixture("video-too-wide.mp4"), "1280x720 limit");
    let output = run(&fixture("video-48-frames.mp4"));
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("24 frames-analyzed"));

    let output = run(&fixture("video-medium.mp4"));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.starts_with("24 frames-analyzed"));
    assert!(stdout.contains("854 width · 480 height"));
}

#[test]
fn rejects_mp4_input_over_six_mibibytes() {
    let mut bytes = Vec::with_capacity(6 * 1024 * 1024 + 1);
    bytes.extend_from_slice(b"\0\0\0\x18ftypisom");
    bytes.resize(6 * 1024 * 1024 + 1, 0);
    let oversized = Input::new("mp4", &bytes);

    assert_fails(&oversized.0, "6 MiB limit");
}

fn run(input: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_kairo"))
        .current_dir(root())
        .args(["run", "video"])
        .arg(input)
        .output()
        .expect("video workflow should run")
}

fn assert_fails(input: &Path, expected: &str) {
    let output = run(input);
    assert!(!output.status.success(), "invalid video was accepted");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(expected),
        "unexpected error: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn reference_mp4() -> PathBuf {
    root().join("demos/reference/video-processing/sample.mp4")
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
