#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Durability {
    #[default]
    Ephemeral,
    Required,
}

impl crate::Workflow {
    pub fn requires_durable_artifacts(&self) -> bool {
        self.edges
            .iter()
            .any(|edge| edge.durability == Durability::Required)
    }
}
