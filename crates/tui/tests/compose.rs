use std::path::PathBuf;

use kairo_core::{ComponentHash, Durability, WorkflowMode, catalog::ComponentEntry};
use kairo_runtime::{ComponentContract, ComponentRole};
use kairo_tui::compose::{CatalogComponent, compatible, describe, finishable, render};

fn fake_hash(seed: u8) -> ComponentHash {
    ComponentHash::sha256([seed; 32])
}

fn component(name: &str, description: Option<&str>, role: ComponentRole) -> CatalogComponent {
    CatalogComponent {
        entry: ComponentEntry {
            path: PathBuf::from(format!("components/{name}/component.wasm")),
            name: name.to_owned(),
            description: description.map(str::to_owned),
        },
        contract: ComponentContract {
            role,
            hash: fake_hash(role as u8),
        },
    }
}

#[test]
fn compatible_filters_by_real_contract_never_by_name() {
    let components = vec![
        component(
            "resize",
            Some("Resize frames"),
            ComponentRole::StreamTransform,
        ),
        component("totalize", Some("Sum values"), ComponentRole::ValueStage),
        component(
            "redact",
            Some("Redact patterns"),
            ComponentRole::StreamOutput,
        ),
    ];

    let after_transform = compatible(&components, Some(ComponentRole::StreamTransform));
    let names: Vec<_> = after_transform
        .iter()
        .map(|component| component.entry.name.as_str())
        .collect();
    assert_eq!(names, vec!["resize", "redact"]);

    let after_value = compatible(&components, Some(ComponentRole::ValueStage));
    assert_eq!(after_value.len(), 1);
    assert_eq!(after_value[0].entry.name, "totalize");

    assert_eq!(compatible(&components, None).len(), components.len());
}

#[test]
fn describe_shows_real_description_and_shape_never_inferring_from_name() {
    let described = component(
        "resize",
        Some("Resize image/video frames"),
        ComponentRole::StreamTransform,
    );
    assert_eq!(
        describe(&described),
        "resize · Resize image/video frames · byte stream → byte stream"
    );

    let undescribed = component("mystery", None, ComponentRole::ValueStage);
    assert_eq!(describe(&undescribed), "mystery · value → value");
}

#[test]
fn finishable_requires_a_terminal_role_and_enough_steps_for_a_stream_chain() {
    assert!(finishable(ComponentRole::ValueStage, 1));
    assert!(!finishable(ComponentRole::StreamTransform, 3));
    assert!(!finishable(ComponentRole::StreamConsume, 1));
    assert!(finishable(ComponentRole::StreamConsume, 2));
    assert!(finishable(ComponentRole::StreamOutput, 1));
}

#[test]
fn render_produces_the_same_workflow_shape_for_every_ui() {
    let paths = vec![
        PathBuf::from("components/a/component.wasm"),
        PathBuf::from("components/b/component.wasm"),
    ];
    let step_names = vec!["a".to_owned(), "b".to_owned()];
    let durabilities = vec![Durability::Auto];
    let hashes = vec![Some(fake_hash(1)), None];

    let source = render(
        "pipeline",
        WorkflowMode::Value,
        0,
        &paths,
        &hashes,
        &step_names,
        &durabilities,
        None,
        None,
        None,
    );

    assert!(source.contains("workflow: \"pipeline\""));
    assert!(source.contains("mode: value"));
    assert!(source.contains("- name: \"a\"\n    component: \"components/a/component.wasm\""));
    assert!(source.contains(&format!("hash: \"{}\"", fake_hash(1))));
    assert!(source.contains("- from: \"a\"\n    to: \"b\"\n    durability: auto"));
}
