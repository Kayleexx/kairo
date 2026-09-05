#![allow(clippy::expect_used, clippy::unwrap_used)]

use kairo_core::ComponentId;

#[test]
fn rejects_empty_component_id() {
    assert!(ComponentId::new("  ").is_err());
}
