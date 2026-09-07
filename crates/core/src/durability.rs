#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Durability {
    #[default]
    Ephemeral,
    Required,
}
