use std::{process::Command, thread, time::Duration};

use crate::setup::SetupError;

const CONTAINER: &str = "kairo-minio";
const MINIO_IMAGE: &str =
    "minio/minio@sha256:14cea493d9a34af32f524e538b8346cf79f3321eff8e708c1e2960462bd8936e";
const MC_IMAGE: &str =
    "minio/mc@sha256:a7fe349ef4bd8521fb8497f55c6042871b2ae640607cf99d9bede5e9bdf11727";

pub(super) fn ensure_local_storage(access_key: &str, secret_key: &str) -> Result<(), SetupError> {
    match docker(
        &[
            "container",
            "inspect",
            CONTAINER,
            "--format",
            "{{.State.Running}}",
        ],
        "inspect local MinIO",
    ) {
        Ok(running) if running.trim() == "true" => create_bucket(access_key, secret_key),
        Ok(_) => {
            docker(&["start", CONTAINER], "start local MinIO")?;
            create_bucket(access_key, secret_key)
        }
        Err(SetupError::Docker { .. }) => {
            let root_user = format!("MINIO_ROOT_USER={access_key}");
            let root_password = format!("MINIO_ROOT_PASSWORD={secret_key}");
            docker(
                &[
                    "run",
                    "-d",
                    "--name",
                    CONTAINER,
                    "-p",
                    "9000:9000",
                    "-e",
                    &root_user,
                    "-e",
                    &root_password,
                    MINIO_IMAGE,
                    "server",
                    "/data",
                ],
                "start local MinIO",
            )?;
            create_bucket(access_key, secret_key)
        }
        Err(error) => Err(error),
    }
}

fn create_bucket(access_key: &str, secret_key: &str) -> Result<(), SetupError> {
    let mut last_error = String::new();
    let command = format!(
        "mc alias set local http://127.0.0.1:9000 {access_key} {secret_key} && mc mb --ignore-existing local/kairo"
    );
    for _ in 0..30 {
        match docker(
            &[
                "run",
                "--rm",
                "--network",
                "container:kairo-minio",
                "--entrypoint",
                "/bin/sh",
                MC_IMAGE,
                "-c",
                &command,
            ],
            "create the local artifact bucket",
        ) {
            Ok(_) => return Ok(()),
            Err(SetupError::Docker { message, .. }) => last_error = message,
            Err(error) => return Err(error),
        }
        thread::sleep(Duration::from_secs(1));
    }
    Err(SetupError::Docker {
        operation: "create the local artifact bucket",
        message: last_error,
    })
}

fn docker(arguments: &[&str], operation: &'static str) -> Result<String, SetupError> {
    let output = Command::new("docker")
        .args(arguments)
        .output()
        .map_err(|source| SetupError::DockerStart { operation, source })?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned());
    }
    Err(SetupError::Docker {
        operation,
        message: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    })
}
