use std::collections::BTreeMap;

use crate::{Workflow, WorkflowMode};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GroupSpan {
    pub start_index: usize,
    pub end_index: usize,
}

impl Workflow {
    /// Whether this workflow's steps may ever be split into more than one ExecutionGroup.
    /// Stream workflows never declare wait/effect (`WorkflowError::StreamControl`), so the check
    /// below is never false for them -- kept explicit rather than assumed.
    pub fn is_groupable(&self) -> bool {
        matches!(self.mode(), WorkflowMode::Scalar | WorkflowMode::Stream)
            && self.effect().is_none()
            && self.wait().is_none()
    }
}

/// Partitions a workflow's steps into contiguous ExecutionGroups. `resolved_durability` maps a
/// step index to whether its outgoing edge is durably required (either declared `required` or an
/// `auto` edge already resolved to required) -- a missing entry is treated as not-required, never
/// as an assumption either way. A group boundary sits only on a durably-required edge, so this
/// never turns an ephemeral edge into a cross-group cut.
pub fn plan_groups(
    workflow: &Workflow,
    resolved_durability: &BTreeMap<usize, bool>,
) -> Vec<GroupSpan> {
    let step_count = workflow.steps().len();
    let mut groups = Vec::new();
    let mut start = 0;
    for index in 0..step_count {
        let is_last_step = index + 1 == step_count;
        let is_required_boundary = resolved_durability.get(&index).copied().unwrap_or(false);
        if is_last_step || is_required_boundary {
            groups.push(GroupSpan {
                start_index: start,
                end_index: index,
            });
            start = index + 1;
        }
    }
    groups
}
