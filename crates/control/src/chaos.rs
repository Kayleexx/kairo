use crate::Response;

pub(crate) fn kill(worker: &str, pid: u32) -> Response {
    #[cfg(unix)]
    {
        match std::process::Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .status()
        {
            Ok(status) if status.success() => Response::Ok,
            Ok(_) => Response::Error {
                message: format!("could not kill worker `{worker}`"),
            },
            Err(error) => Response::Error {
                message: format!("could not kill worker `{worker}`: {error}"),
            },
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (worker, pid);
        Response::Error {
            message: "worker chaos kill is currently supported on unix".into(),
        }
    }
}
