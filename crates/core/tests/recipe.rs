#![allow(clippy::expect_used)]

use std::{fs, process};

use kairo_core::recipe::{Recipe, RecipeError};

#[test]
fn rejects_an_empty_component_source_hint() {
    let path = std::env::temp_dir().join(format!("kairo-recipe-{}.yaml", process::id()));
    fs::write(
        &path,
        "schema: 1\nname: example\ntitle: Example\ndescription: Example recipe\ncomponents:\n  - name: decode\n    source: ''\n",
    )
    .expect("recipe should write");
    let error = Recipe::load(&path, 4096).expect_err("empty source should be rejected");
    assert!(matches!(
        error,
        RecipeError::InvalidComponentSource { component } if component == "decode"
    ));
    let _ = fs::remove_file(path);
}
