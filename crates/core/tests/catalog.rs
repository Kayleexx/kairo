#![allow(clippy::expect_used)]

use std::{
    fs,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
};

use kairo_core::catalog;

fn temp_dir(name: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("kairo-catalog-{name}-{}-{sequence}", process::id()));
    fs::create_dir_all(&path).expect("fixture directory should be created");
    path
}

#[test]
fn lists_built_components_with_their_cargo_description() {
    let root = temp_dir("described");
    let component = root.join("count-primes");
    fs::create_dir_all(&component).expect("component directory should be created");
    fs::write(
        component.join("Cargo.toml"),
        "[package]\nname = \"count-primes\"\ndescription = \"Count primes up to a number\"\n",
    )
    .expect("manifest should write");
    fs::write(
        component.join("component.wasm"),
        b"not real wasm, just a marker file",
    )
    .expect("component.wasm should write");

    let entries = catalog::list(&[&root]);

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "count-primes");
    assert_eq!(entries[0].path, component.join("component.wasm"));
    assert_eq!(
        entries[0].description.as_deref(),
        Some("Count primes up to a number")
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn lists_a_component_with_no_description_as_none_never_inferring_one_from_its_name() {
    let root = temp_dir("undescribed");
    let component = root.join("mystery-step");
    fs::create_dir_all(&component).expect("component directory should be created");
    fs::write(
        component.join("Cargo.toml"),
        "[package]\nname = \"mystery-step\"\n",
    )
    .expect("manifest should write");
    fs::write(component.join("component.wasm"), b"marker").expect("component.wasm should write");

    let entries = catalog::list(&[&root]);

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "mystery-step");
    assert_eq!(entries[0].description, None);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn skips_a_missing_root_instead_of_erroring() {
    let entries = catalog::list(&[Path::new("/does/not/exist/kairo-catalog-fixture")]);
    assert!(entries.is_empty());
}

#[test]
fn catalog_manifest_supplies_identity_and_provenance_without_hiding_the_vendored_path() {
    let root = temp_dir("manifest");
    let component = root.join("decode");
    fs::create_dir_all(&component).expect("component directory should be created");
    fs::write(component.join("component.wasm"), b"marker").expect("component should write");
    fs::write(
        component.join("kairo.toml"),
        "schema = 1\nname = \"video-decode\"\nversion = \"1.0.0\"\nhash = \"sha256:0000000000000000000000000000000000000000000000000000000000000000\"\n[source]\nkind = \"local\"\nreference = \"./decode.wasm\"\n",
    )
    .expect("manifest should write");

    let entries = catalog::list(&[&root]);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "video-decode");
    assert_eq!(entries[0].version.as_deref(), Some("1.0.0"));
    assert_eq!(entries[0].path, component.join("component.wasm"));
    assert_eq!(
        entries[0]
            .source
            .as_ref()
            .map(|source| source.reference.as_str()),
        Some("./decode.wasm")
    );
    let _ = fs::remove_dir_all(root);
}
