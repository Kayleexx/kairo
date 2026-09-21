#![allow(clippy::expect_used)]

use std::{
    fs,
    io::{Read as _, Write as _},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{self, Command},
    sync::{Arc, Mutex, mpsc},
    thread,
};

use sha2::{Digest as _, Sha256};

fn kairo() -> Command {
    Command::new(env!("CARGO_BIN_EXE_kairo"))
}

fn fixture(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("kairo-oci-{name}-{}", process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("fixture directory should be created");
    path
}

fn component() -> Vec<u8> {
    fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../components/runtime/value-echo/component.wasm"),
    )
    .expect("Component should read")
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

struct Registry {
    address: String,
    requests: Arc<Mutex<usize>>,
    stop: mpsc::Sender<()>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Registry {
    fn start(component: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("registry should bind");
        listener
            .set_nonblocking(true)
            .expect("registry should be nonblocking");
        let address = listener.local_addr().expect("registry address").to_string();
        let config = br#"{}"#.to_vec();
        let component_digest = digest(&component);
        let config_digest = digest(&config);
        let manifest = format!(
            r#"{{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","config":{{"mediaType":"application/vnd.wasm.config.v0+json","digest":"{config_digest}","size":{}}},"layers":[{{"mediaType":"application/vnd.wasm.content.layer.v1+wasm","digest":"{component_digest}","size":{}}}]}}"#,
            config.len(),
            component.len(),
        )
        .into_bytes();
        let manifest_digest = digest(&manifest);
        let requests = Arc::new(Mutex::new(0));
        let request_count = Arc::clone(&requests);
        let (stop, stopped) = mpsc::channel();
        let thread = thread::spawn(move || {
            loop {
                if stopped.try_recv().is_ok() {
                    break;
                }
                match listener.accept() {
                    Ok((stream, _)) => {
                        *request_count.lock().expect("request count should lock") += 1;
                        respond(
                            stream,
                            &manifest,
                            &manifest_digest,
                            &config,
                            &config_digest,
                            &component,
                            &component_digest,
                        );
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::yield_now()
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            address,
            requests,
            stop,
            thread: Some(thread),
        }
    }

    fn request_count(&self) -> usize {
        *self.requests.lock().expect("request count should lock")
    }
}

impl Drop for Registry {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            thread.join().expect("registry thread should stop");
        }
    }
}

fn respond(
    mut stream: TcpStream,
    manifest: &[u8],
    manifest_digest: &str,
    config: &[u8],
    config_digest: &str,
    component: &[u8],
    component_digest: &str,
) {
    let mut request = [0_u8; 4096];
    let Ok(count) = stream.read(&mut request) else {
        return;
    };
    let request = String::from_utf8_lossy(&request[..count]);
    let path = request.split_whitespace().nth(1).unwrap_or_default();
    let (status, media_type, digest_header, body) = if path.contains("/manifests/") {
        (
            "200 OK",
            "application/vnd.oci.image.manifest.v1+json",
            manifest_digest,
            manifest,
        )
    } else if path.ends_with(config_digest) {
        (
            "200 OK",
            "application/vnd.wasm.config.v0+json",
            config_digest,
            config,
        )
    } else if path.ends_with(component_digest) {
        (
            "200 OK",
            "application/vnd.wasm.content.layer.v1+wasm",
            component_digest,
            component,
        )
    } else {
        ("404 Not Found", "text/plain", "", b"missing" as &[u8])
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {media_type}\r\nContent-Length: {}\r\nDocker-Content-Digest: {digest_header}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(body);
}

#[test]
fn oci_import_pins_a_digest_uses_cache_and_refetches_corruption() {
    let directory = fixture("import");
    let registry = Registry::start(component());
    let reference = format!("{}/acme/echo:v1", registry.address);
    let add = kairo()
        .current_dir(&directory)
        .args(["add", &reference, "--name", "remote-echo"])
        .output()
        .expect("OCI add should run");
    assert!(add.status.success(), "{add:?}");
    let stdout = String::from_utf8_lossy(&add.stdout);
    assert!(stdout.contains("added remote-echo"), "{stdout}");
    assert!(stdout.contains("digest: sha256:"), "{stdout}");
    assert!(registry.request_count() >= 4);

    let manifest = fs::read_to_string(directory.join("components/remote-echo/kairo.toml"))
        .expect("catalog metadata should read");
    let parsed: toml::Value = toml::from_str(&manifest).expect("metadata should parse");
    let digest = parsed["source"]["resolved_digest"]
        .as_str()
        .expect("digest should be pinned");
    let pinned = format!("{}/acme/echo@{digest}", registry.address);
    let before_cache_hit = registry.request_count();
    let cached = kairo()
        .current_dir(&directory)
        .args(["add", &pinned, "--name", "cached-echo"])
        .output()
        .expect("pinned cache add should run");
    assert!(cached.status.success(), "{cached:?}");
    assert_eq!(registry.request_count(), before_cache_hit);
    fs::create_dir(directory.join("recipes")).expect("recipe directory should create");
    fs::write(
        directory.join("recipes/oci.yaml"),
        "schema: 1\nname: oci\ntitle: OCI pipeline\ndescription: Use imported Components\ncomponents:\n  - name: remote-echo\n  - name: cached-echo\ndurability: auto\n",
    )
    .expect("recipe should write");
    let workflow = kairo()
        .current_dir(&directory)
        .args(["new", "oci-flow", "--recipe", "oci"])
        .output()
        .expect("OCI recipe should build a workflow");
    assert!(workflow.status.success(), "{workflow:?}");
    let run = kairo()
        .current_dir(&directory)
        .args(["run", "oci-flow", "--value", "hello"])
        .output()
        .expect("OCI workflow should run");
    assert!(run.status.success(), "{run:?}");

    let cache = directory.join(format!(
        ".kairo/cache/oci/sha256/{}/component.wasm",
        &digest[7..]
    ));
    fs::write(&cache, b"corrupt").expect("cache should corrupt");
    let refetched = kairo()
        .current_dir(&directory)
        .args(["add", &pinned, "--name", "refetched-echo"])
        .output()
        .expect("corrupted cache should refetch");
    assert!(refetched.status.success(), "{refetched:?}");
    assert!(registry.request_count() > before_cache_hit);
    let info = kairo()
        .current_dir(&directory)
        .args(["component", "info", "remote-echo"])
        .output()
        .expect("component info should run");
    assert!(info.status.success(), "{info:?}");
    assert!(String::from_utf8_lossy(&info.stdout).contains("Digest     sha256:"));
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn invalid_oci_reference_has_an_actionable_error() {
    let directory = fixture("invalid");
    let output = kairo()
        .current_dir(&directory)
        .args(["add", "ghcr.io/Bad/Component:v1"])
        .output()
        .expect("invalid OCI add should run");
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("valid OCI Component reference"), "{error}");
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn missing_artifact_has_an_actionable_error() {
    let directory = fixture("missing");
    let listener = TcpListener::bind("127.0.0.1:0").expect("registry should bind");
    let address = listener.local_addr().expect("registry address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("request should arrive");
        let mut request = [0_u8; 1024];
        let _ = stream.read(&mut request);
        let _ = stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    });
    let output = kairo()
        .current_dir(&directory)
        .args(["add", &format!("{address}/acme/missing:v1")])
        .output()
        .expect("missing OCI add should run");
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("check the reference"), "{error}");
    server.join().expect("registry thread should stop");
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn private_oci_error_suggests_standard_registry_authentication() {
    let directory = fixture("private");
    let listener = TcpListener::bind("127.0.0.1:0").expect("registry should bind");
    let address = listener.local_addr().expect("registry address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("request should arrive");
        let mut request = [0_u8; 1024];
        let _ = stream.read(&mut request);
        let _ = stream.write_all(
            b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
    });
    let output = kairo()
        .current_dir(&directory)
        .args(["add", &format!("{address}/acme/private:v1")])
        .output()
        .expect("private OCI add should run");
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("docker login"), "{error}");
    server.join().expect("registry thread should stop");
    let _ = fs::remove_dir_all(directory);
}
