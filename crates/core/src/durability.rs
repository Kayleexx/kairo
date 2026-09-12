#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Durability {
    #[default]
    Ephemeral,
    Required,
    Auto,
}

impl Durability {
    pub fn is_unresolved(self) -> bool {
        self == Self::Auto
    }
}

impl crate::Workflow {
    pub fn requires_durable_artifacts(&self) -> bool {
        self.edges
            .iter()
            .any(|edge| edge.durability == Durability::Required)
    }
}
