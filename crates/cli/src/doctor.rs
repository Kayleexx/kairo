use std::path::Path;

use crate::{CliError, Result, lifecycle, setup};

pub(crate) async fn run(json: bool, fix: bool) -> Result<()> {
    let mut failed = false;
    let project = Path::new(".kairo").is_dir();
    if project {
        human(json, "✓ project state");
    } else {
        human(json, "! project state · run `kairo init`");
        failed = true;
    }
    failed |= !check_storage(json, fix).await;
    failed |= !check_local_service(json, fix);
    if json {
        println!("{{\"project\":{project},\"healthy\":{}}}", !failed);
    }
    if failed {
        return Err(CliError::Doctor);
    }
    Ok(())
}

async fn check_storage(json: bool, fix: bool) -> bool {
    if fix && let Err(error) = setup::ensure_storage() {
        human(json, &format!("! storage · {error}\n  try: kairo init"));
        return false;
    }
    match setup::check_storage().await {
        Ok(check) => {
            human(json, &format!("✓ storage · {}", check.backend));
            true
        }
        Err(error) => {
            human(json, &format!("! storage · {error}\n  try: kairo init"));
            false
        }
    }
}

fn check_local_service(json: bool, fix: bool) -> bool {
    match kairo_control::load_endpoint(Path::new(".kairo")) {
        Ok(endpoint) => match kairo_control::snapshot(&endpoint) {
            Ok(snapshot) => {
                let healthy = snapshot
                    .workers
                    .iter()
                    .filter(|worker| worker.healthy)
                    .count();
                if healthy > 0 {
                    human(json, &format!("✓ local service · {healthy} workers"));
                    return true;
                }
                fix && repair_local_service(json) || report_local_service_broken(json)
            }
            Err(_) => fix && repair_local_service(json) || report_local_service_broken(json),
        },
        Err(_) => {
            human(json, "○ local service · not running · optional: kairo up");
            true
        }
    }
}

fn report_local_service_broken(json: bool) -> bool {
    human(
        json,
        "! local service · unavailable · try: kairo down && kairo up",
    );
    false
}

fn repair_local_service(json: bool) -> bool {
    let workers = setup::project_workers().ok().flatten().unwrap_or(2);
    if lifecycle::stop().is_err() || lifecycle::start(workers, false).is_err() {
        return false;
    }
    human(
        json,
        &format!("✓ local service · repaired · restarted with {workers} workers"),
    );
    true
}

fn human(json: bool, message: &str) {
    if !json {
        println!("{message}");
    }
}
