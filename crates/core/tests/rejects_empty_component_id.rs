use kairo_core::ComponentId;

#[test]
fn rejects_empty_component_id() {
    assert!(ComponentId::new("  ").is_err());
}
