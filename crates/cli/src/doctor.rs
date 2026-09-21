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
    check_binary_freshness(json);
    if json {
        println!("{{\"project\":{project},\"healthy\":{}}}", !failed);
    }
    if failed {
        return Err(CliError::Doctor);
    }
    Ok(())
}

const BUILD_GIT_SHA: &str = env!("KAIRO_BUILD_GIT_SHA");

/// only meaningful inside a Kairo source checkout -- informational only, since a normal project
/// directory's `Cargo.toml` (if any) never carries this repository's own workspace metadata.
fn check_binary_freshness(json: bool) {
    if BUILD_GIT_SHA == "unknown" {
        return;
    }
    let Some(checkout) = find_checkout_root(&std::env::current_dir().unwrap_or_default()) else {
        return;
    };
    let Some(source_sha) = checkout_head(&checkout) else {
        return;
    };
    if source_sha == BUILD_GIT_SHA {
        return;
    }
    human(
        json,
        &format!(
            "○ Kairo binary does not match this checkout.\n\n  binary: {}\n  source: {}\n\n  rebuild/install:\n    cargo build --release",
            short(BUILD_GIT_SHA),
            short(&source_sha)
        ),
    );
}

fn short(sha: &str) -> &str {
    &sha[..12.min(sha.len())]
}

const CHECKOUT_MARKER: &str = "https://github.com/Kayleexx/kairo";

fn find_checkout_root(start: &Path) -> Option<std::path::PathBuf> {
    let mut directory = start;
    loop {
        let manifest = directory.join("Cargo.toml");
        if std::fs::read_to_string(&manifest).is_ok_and(|source| source.contains(CHECKOUT_MARKER)) {
            return Some(directory.to_path_buf());
        }
        directory = directory.parent()?;
    }
}

fn checkout_head(checkout: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(checkout)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8(output.stdout).ok())
        .flatten()
        .map(|sha| sha.trim().to_owned())
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
