use std::path::Path;

pub(super) fn status(
    name: &str,
) -> Result<Option<kairo_control::RunStatus>, kairo_control::ControlError> {
    let endpoint = match kairo_control::load_endpoint(Path::new(".kairo")) {
        Ok(endpoint) => endpoint,
        Err(kairo_control::ControlError::Unavailable) => return Ok(None),
        Err(error) => return Err(error),
    };
    kairo_control::status(&endpoint, name.to_owned())
}

pub(super) fn print(name: &str, status: kairo_control::RunStatus) {
    println!("{name}");
    match status {
        kairo_control::RunStatus::Queued => println!("  state · queued"),
        kairo_control::RunStatus::Running { worker, .. } => {
            println!("  state · running");
            println!("  worker · {worker}");
        }
        kairo_control::RunStatus::Waiting { reason } => {
            if let Some(signal) = reason.strip_prefix("signal:") {
                println!("  state · waiting for {signal}");
                println!("  worker · released");
                println!("\nnext · kairo signal {name}");
            } else {
                println!("  state · waiting for its timer");
                println!("  worker · released");
            }
        }
        kairo_control::RunStatus::Completed { output, worker } => {
            println!("  state · completed");
            if !worker.is_empty() {
                println!("  worker · {worker}");
            }
            println!("  output · {output}");
        }
        kairo_control::RunStatus::Failed { message } => {
            println!("  state · failed");
            println!("  error · {message}");
        }
        kairo_control::RunStatus::CancelRequested => {
            println!("  state · cancel requested");
        }
        kairo_control::RunStatus::Canceled => {
            println!("  state · canceled");
        }
    }
}

pub(super) fn print_state(name: &str, status: &kairo_control::RunStatus) -> bool {
    match status {
        kairo_control::RunStatus::Waiting { reason } => {
            if let Some(signal) = reason.strip_prefix("signal:") {
                println!("  state · waiting for {signal}");
                println!("  worker · released");
                println!("  next · kairo signal {name}");
            } else {
                println!("  state · waiting for its timer");
                println!("  worker · released");
            }
            true
        }
        kairo_control::RunStatus::Queued => {
            println!("  state · queued");
            true
        }
        kairo_control::RunStatus::Running { worker, .. } => {
            println!("  state · running on {worker}");
            true
        }
        kairo_control::RunStatus::Failed { message } => {
            println!("  state · failed");
            println!("  error · {message}");
            true
        }
        kairo_control::RunStatus::Completed { .. } => false,
        kairo_control::RunStatus::CancelRequested => {
            println!("  state · cancel requested");
            true
        }
        kairo_control::RunStatus::Canceled => {
            println!("  state · canceled");
            true
        }
    }
}
