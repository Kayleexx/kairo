#![allow(clippy::expect_used)]

use std::{
    fs,
    path::PathBuf,
    process::{self, Command},
};

fn kairo() -> Command {
    Command::new(env!("CARGO_BIN_EXE_kairo"))
}

fn fixture(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("kairo-product-{name}-{}", process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("fixture directory should be created");
    path
}

fn value_component() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../components/runtime/value-echo/component.wasm")
}

#[test]
fn add_vendors_and_describes_a_real_component() {
    let directory = fixture("add");
    let import = directory.join("source.wasm");
    fs::copy(value_component(), &import).expect("source Component should copy");

    let add = kairo()
        .current_dir(&directory)
        .arg("add")
        .arg(&import)
        .args([
            "--name",
            "echo",
            "--version",
            "1.2.3",
            "--description",
            "Echo a text value",
        ])
        .output()
        .expect("kairo add should run");
    assert!(add.status.success(), "{add:?}");
    let repeated = kairo()
        .current_dir(&directory)
        .arg("add")
        .arg(&import)
        .args(["--name", "echo", "--version", "1.2.3"])
        .output()
        .expect("re-registering identical content should run");
    assert!(repeated.status.success(), "{repeated:?}");
    fs::remove_file(import).expect("original source should be removable");

    let component = directory.join("components/echo/component.wasm");
    let manifest = directory.join("components/echo/kairo.toml");
    assert!(component.is_file());
    let metadata = fs::read_to_string(manifest).expect("catalog manifest should read");
    assert!(metadata.contains("name = \"echo\""), "{metadata}");
    assert!(metadata.contains("version = \"1.2.3\""), "{metadata}");
    assert!(metadata.contains("Echo a text value"), "{metadata}");
    assert!(metadata.contains("sha256:"), "{metadata}");

    let list = kairo()
        .current_dir(&directory)
        .arg("components")
        .output()
        .expect("component list should run");
    assert!(list.status.success(), "{list:?}");
    let listed = String::from_utf8_lossy(&list.stdout);
    assert!(listed.contains("echo 1.2.3"), "{listed}");
    assert!(listed.contains("value → value"), "{listed}");
    assert!(listed.contains("Echo a text value"), "{listed}");

    let show = kairo()
        .current_dir(&directory)
        .args(["component", "show", "echo"])
        .output()
        .expect("component show should run");
    assert!(show.status.success(), "{show:?}");
    let shown = String::from_utf8_lossy(&show.stdout);
    assert!(shown.contains("value → value"), "{shown}");
    assert!(shown.contains("Exports    run"), "{shown}");
    assert!(shown.contains("Does       Echo a text value"), "{shown}");

    let check = kairo()
        .current_dir(&directory)
        .args(["component", "check", "components/echo/component.wasm"])
        .output()
        .expect("component check should run");
    assert!(check.status.success(), "{check:?}");
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn recipe_compiles_to_the_normal_workflow_and_runs() {
    let directory = fixture("recipe");
    for name in ["prepare", "finish"] {
        let add = kairo()
            .current_dir(&directory)
            .arg("add")
            .arg(value_component())
            .args(["--name", name])
            .output()
            .expect("kairo add should run");
        assert!(add.status.success(), "{add:?}");
    }
    fs::create_dir(directory.join("recipes")).expect("recipes directory should be created");
    fs::write(
        directory.join("recipes/text.yaml"),
        "schema: 1\nname: text\ntitle: Process text\ndescription: Process a text value\naccepts: [text]\nproduces: [text]\ncomponents:\n  - name: prepare\n  - name: finish\ndurability: auto\n",
    )
    .expect("recipe should write");

    let recipes = kairo()
        .current_dir(&directory)
        .arg("recipes")
        .output()
        .expect("recipe list should run");
    assert!(recipes.status.success(), "{recipes:?}");
    let listed = String::from_utf8_lossy(&recipes.stdout);
    assert!(listed.contains("text · Process text"), "{listed}");
    assert!(listed.contains("requires · prepare, finish"), "{listed}");

    let create = kairo()
        .current_dir(&directory)
        .args(["new", "business-flow", "--recipe", "text"])
        .output()
        .expect("recipe creation should run");
    assert!(create.status.success(), "{create:?}");
    let source =
        fs::read_to_string(directory.join("business-flow.yaml")).expect("workflow should read");
    assert!(source.contains("sha256:"), "{source}");
    assert!(source.contains("durability: auto"), "{source}");

    let check = kairo()
        .current_dir(&directory)
        .args(["check", "business-flow"])
        .output()
        .expect("workflow check should run");
    assert!(check.status.success(), "{check:?}");
    let checked = String::from_utf8_lossy(&check.stdout);
    assert!(checked.contains("Process a text value"), "{checked}");
    assert!(checked.contains("input · value"), "{checked}");
    assert!(
        checked.contains("next · kairo run business-flow --value <value>"),
        "{checked}"
    );

    let run = kairo()
        .current_dir(&directory)
        .args(["run", "business-flow", "--value", "hello"])
        .output()
        .expect("workflow should run");
    assert!(run.status.success(), "{run:?}");
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "jgnnq");
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn recipe_reports_every_missing_component_with_real_source_hints() {
    let directory = fixture("recipe-missing");
    fs::create_dir(directory.join("recipes")).expect("recipes directory should be created");
    fs::write(
        directory.join("recipes/imports.yaml"),
        "schema: 1\nname: imports\ntitle: Import workflow\ndescription: Uses published Components\ncomponents:\n  - name: decode\n    source: ghcr.io/acme/decode:v1\n  - name: analyze\n    source: ghcr.io/acme/analyze:v2\n",
    )
    .expect("recipe should write");

    let created = kairo()
        .current_dir(&directory)
        .args(["new", "imports", "--recipe", "imports"])
        .output()
        .expect("recipe creation should run");
    assert!(!created.status.success(), "{created:?}");
    let stderr = String::from_utf8_lossy(&created.stderr);
    assert!(stderr.contains("decode · kairo add ghcr.io/acme/decode:v1 --name decode"));
    assert!(stderr.contains("analyze · kairo add ghcr.io/acme/analyze:v2 --name analyze"));
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn init_installs_curated_recipe_templates_without_overwriting_project_files() {
    let directory = fixture("templates");
    let init = kairo()
        .current_dir(&directory)
        .args(["init", "--local"])
        .output()
        .expect("init should run");
    assert!(init.status.success(), "{init:?}");
    let recipes = kairo()
        .current_dir(&directory)
        .arg("recipes")
        .output()
        .expect("recipes should run");
    assert!(recipes.status.success(), "{recipes:?}");
    let listed = String::from_utf8_lossy(&recipes.stdout);
    assert!(
        listed.contains("video-analysis · Video analysis"),
        "{listed}"
    );
    let custom = directory.join("recipes/video-analysis.yaml");
    fs::write(&custom, "project recipe\n").expect("project recipe should write");
    let repeated = kairo()
        .current_dir(&directory)
        .args(["init", "--local"])
        .output()
        .expect("repeated init should run");
    assert!(repeated.status.success(), "{repeated:?}");
    assert_eq!(
        fs::read_to_string(custom).expect("recipe should read"),
        "project recipe\n"
    );
    let _ = fs::remove_dir_all(directory);
}
