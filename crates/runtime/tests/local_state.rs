#![allow(clippy::expect_used)]

use std::{fs, process};

use kairo_runtime::discover_cells_in;

#[test]
fn skips_the_legacy_effect_provider_database() {
    let directory = std::env::temp_dir().join(format!("kairo-local-state-{}", process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir(&directory).expect("fixture directory should be created");
    fs::write(directory.join("effects.db"), []).expect("effect database should be created");
    fs::write(directory.join("workflow.db"), []).expect("run database should be created");

    let cells = discover_cells_in(&directory).expect("cells should be discovered");
    assert_eq!(cells.len(), 1);
    assert_eq!(cells[0].name, "workflow");
    let _ = fs::remove_dir_all(directory);
}
