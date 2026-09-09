use std::path::Path;

use crate::{CliError, Result, setup};

pub(crate) async fn run(json: bool) -> Result<()> {
    let mut failed = false;
    let project = Path::new(".kairo").is_dir();
    if project {
        human(json, "✓ project state");
    } else {
        human(json, "! project state · run `kairo init`");
        failed = true;
    }
    match setup::check_storage().await {
        Ok(check) => human(json, &format!("✓ storage · {}", check.backend)),
        Err(error) => {
            human(json, &format!("! storage · {error}\n  try: kairo init"));
            failed = true;
        }
    }
    match kairo_control::load_endpoint(Path::new(".kairo")) {
        Ok(endpoint) => match kairo_control::snapshot(&endpoint) {
            Ok(snapshot) => {
                let healthy = snapshot
                    .workers
                    .iter()
                    .filter(|worker| worker.healthy)
                    .count();
                if healthy == 0 {
                    human(
                        json,
                        "! local service · no workers · try: kairo down && kairo up",
                    );
                    failed = true;
                } else {
                    human(json, &format!("✓ local service · {healthy} workers"));
                }
            }
            Err(_) => human(json, "! local service · unavailable · try: kairo up"),
        },
        Err(_) => human(json, "○ local service · not running · optional: kairo up"),
    }
    if json {
        println!("{{\"project\":{project},\"healthy\":{}}}", !failed);
    }
    if failed {
        return Err(CliError::Doctor);
    }
    Ok(())
}

fn human(json: bool, message: &str) {
    if !json {
        println!("{message}");
    }
}
