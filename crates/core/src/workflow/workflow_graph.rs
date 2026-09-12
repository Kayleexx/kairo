use std::collections::{HashMap, HashSet, VecDeque};

use crate::{ComponentId, Durability, WorkflowEdge, WorkflowEffect, WorkflowStep};

use super::{DurabilityDocument, EdgeDocument, WorkflowError};

pub(crate) fn validate_edges(
    documents: Vec<EdgeDocument>,
    steps: &[WorkflowStep],
    indices: &HashMap<ComponentId, usize>,
) -> Result<(Vec<WorkflowEdge>, Vec<usize>), WorkflowError> {
    let mut adjacency = vec![Vec::new(); steps.len()];
    let mut indegree = vec![0usize; steps.len()];
    let mut seen = HashSet::with_capacity(documents.len());
    let mut edges = Vec::with_capacity(documents.len());
    for (index, edge) in documents.into_iter().enumerate() {
        let from = ComponentId::new(edge.from).map_err(|_| WorkflowError::EmptyEdgeStep {
            index,
            endpoint: "from",
        })?;
        let to = ComponentId::new(edge.to).map_err(|_| WorkflowError::EmptyEdgeStep {
            index,
            endpoint: "to",
        })?;
        let from_index = *indices
            .get(&from)
            .ok_or_else(|| WorkflowError::UnknownStep {
                step: from.to_string(),
            })?;
        let to_index = *indices.get(&to).ok_or_else(|| WorkflowError::UnknownStep {
            step: to.to_string(),
        })?;
        if !seen.insert((from_index, to_index)) {
            return Err(WorkflowError::DuplicateEdge {
                from: from.to_string(),
                to: to.to_string(),
            });
        }
        adjacency[from_index].push(to_index);
        indegree[to_index] += 1;
        edges.push(WorkflowEdge {
            from,
            to,
            durability: match edge.durability {
                DurabilityDocument::Ephemeral => Durability::Ephemeral,
                DurabilityDocument::Required => Durability::Required,
                DurabilityDocument::Auto => Durability::Auto,
            },
        });
    }
    for (index, outputs) in adjacency.iter().enumerate() {
        if outputs.len() > 1 {
            return Err(WorkflowError::MultipleOutputs {
                step: steps[index].id.to_string(),
            });
        }
        if indegree[index] > 1 {
            return Err(WorkflowError::MultipleInputs {
                step: steps[index].id.to_string(),
            });
        }
    }
    let mut ready: VecDeque<_> = indegree
        .iter()
        .enumerate()
        .filter_map(|(index, degree)| (*degree == 0).then_some(index))
        .collect();
    let mut order = Vec::with_capacity(steps.len());
    while let Some(index) = ready.pop_front() {
        order.push(index);
        for &next in &adjacency[index] {
            indegree[next] -= 1;
            if indegree[next] == 0 {
                ready.push_back(next);
            }
        }
    }
    if order.len() != steps.len() {
        return Err(WorkflowError::Cycle);
    }
    if edges.len().saturating_add(1) != steps.len() {
        return Err(WorkflowError::Disconnected);
    }
    Ok((edges, order))
}

pub(crate) fn validate_boundaries(
    steps: &[WorkflowStep],
    edges: &[WorkflowEdge],
    wait_after: Option<&ComponentId>,
    effect: Option<&WorkflowEffect>,
) -> Result<(), WorkflowError> {
    validate_boundary("wait", wait_after, steps, edges)?;
    let effect_after = effect.and_then(WorkflowEffect::after);
    validate_boundary("effect", effect_after, steps, edges)?;
    if wait_after.is_some() && wait_after == effect_after {
        return Err(WorkflowError::ConflictingBoundaries);
    }
    Ok(())
}

fn validate_boundary(
    kind: &'static str,
    after: Option<&ComponentId>,
    steps: &[WorkflowStep],
    edges: &[WorkflowEdge],
) -> Result<(), WorkflowError> {
    let Some(after) = after else {
        return Ok(());
    };
    if !steps.iter().any(|step| &step.id == after) {
        return Err(WorkflowError::UnknownBoundaryStep {
            kind,
            step: after.to_string(),
        });
    }
    if !edges
        .iter()
        .any(|edge| &edge.from == after && edge.durability == Durability::Required)
    {
        return Err(WorkflowError::InvalidBoundary {
            kind,
            step: after.to_string(),
        });
    }
    Ok(())
}
