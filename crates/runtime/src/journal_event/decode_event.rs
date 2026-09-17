use crate::journal::JournalError;
use crate::payload;

use super::JournalEvent;
use super::decode::{StoredEvent, decode_payload};
use super::decode_fields::{
    absent, component_index, corrupt, optional_bool, optional_index, optional_u32,
    optional_unsigned, required, unsigned_u64,
};

pub(super) fn event(stored: StoredEvent) -> Result<JournalEvent, JournalError> {
    let StoredEvent {
        sequence,
        kind,
        index,
        fingerprint,
        name,
        hash,
        input,
        input_payload,
        output,
        output_payload,
        artifact_hash,
        workflow_name,
        duration_us,
        durable_after,
        component_count,
        artifact_backend,
        artifact_bytes,
        planner_profile_id,
        planner_reason,
        planner_recompute_us,
        planner_checkpoint_us,
        planner_checkpoint_bytes,
        planner_samples,
        planner_recorded_at_ms,
        start_index,
        start_input,
        start_payload,
    } = stored;
    match kind.as_str() {
        "workflow_started" => {
            absent(sequence, &index, "component index")?;
            absent(sequence, &name, "component name")?;
            absent(sequence, &hash, "component hash")?;
            absent(sequence, &output, "output")?;
            payload::absent_columns(sequence, &output_payload, "output")?;
            absent(sequence, &artifact_hash, "artifact hash")?;
            absent(sequence, &duration_us, "duration")?;
            absent(sequence, &durable_after, "durability")?;
            absent(sequence, &artifact_backend, "artifact backend")?;
            absent(sequence, &artifact_bytes, "artifact bytes")?;
            absent(sequence, &planner_profile_id, "planner profile id")?;
            absent(sequence, &planner_reason, "planner reason")?;
            absent(sequence, &planner_recompute_us, "planner recompute time")?;
            absent(sequence, &planner_checkpoint_us, "planner checkpoint time")?;
            absent(
                sequence,
                &planner_checkpoint_bytes,
                "planner checkpoint bytes",
            )?;
            absent(sequence, &planner_samples, "planner samples")?;
            absent(
                sequence,
                &planner_recorded_at_ms,
                "planner recorded-at time",
            )?;
            let start_input = if start_input.is_none() && start_payload.is_empty() {
                None
            } else {
                Some(decode_payload(
                    sequence,
                    "start input",
                    start_input,
                    start_payload,
                )?)
            };
            Ok(JournalEvent::WorkflowStarted {
                name: workflow_name,
                fingerprint: required(sequence, fingerprint, "workflow fingerprint")?,
                input: decode_payload(sequence, "input", input, input_payload)?,
                component_count: optional_index(sequence, component_count, "component count")?,
                start_index: optional_index(sequence, start_index, "start index")?,
                start_input,
            })
        }
        "component_started" => {
            absent(sequence, &fingerprint, "workflow fingerprint")?;
            absent(sequence, &output, "output")?;
            payload::absent_columns(sequence, &output_payload, "output")?;
            absent(sequence, &artifact_hash, "artifact hash")?;
            absent(sequence, &workflow_name, "workflow name")?;
            absent(sequence, &duration_us, "duration")?;
            absent(sequence, &component_count, "component count")?;
            absent(sequence, &artifact_backend, "artifact backend")?;
            absent(sequence, &artifact_bytes, "artifact bytes")?;
            absent(sequence, &planner_profile_id, "planner profile id")?;
            absent(sequence, &planner_reason, "planner reason")?;
            absent(sequence, &planner_recompute_us, "planner recompute time")?;
            absent(sequence, &planner_checkpoint_us, "planner checkpoint time")?;
            absent(
                sequence,
                &planner_checkpoint_bytes,
                "planner checkpoint bytes",
            )?;
            absent(sequence, &planner_samples, "planner samples")?;
            absent(
                sequence,
                &planner_recorded_at_ms,
                "planner recorded-at time",
            )?;
            absent(sequence, &start_index, "start index")?;
            absent(sequence, &start_input, "start input")?;
            payload::absent_columns(sequence, &start_payload, "start input")?;
            Ok(JournalEvent::ComponentStarted {
                index: component_index(sequence, index)?,
                name: required(sequence, name, "component name")?,
                hash: required(sequence, hash, "component hash")?,
                input: decode_payload(sequence, "input", input, input_payload)?,
                durable_after: optional_bool(sequence, durable_after, "durability")?,
            })
        }
        "component_completed" => {
            absent(sequence, &fingerprint, "workflow fingerprint")?;
            absent(sequence, &name, "component name")?;
            absent(sequence, &hash, "component hash")?;
            absent(sequence, &input, "input")?;
            payload::absent_columns(sequence, &input_payload, "input")?;
            absent(sequence, &artifact_hash, "artifact hash")?;
            absent(sequence, &workflow_name, "workflow name")?;
            absent(sequence, &durable_after, "durability")?;
            absent(sequence, &component_count, "component count")?;
            absent(sequence, &artifact_backend, "artifact backend")?;
            absent(sequence, &artifact_bytes, "artifact bytes")?;
            absent(sequence, &planner_profile_id, "planner profile id")?;
            absent(sequence, &planner_reason, "planner reason")?;
            absent(sequence, &planner_recompute_us, "planner recompute time")?;
            absent(sequence, &planner_checkpoint_us, "planner checkpoint time")?;
            absent(
                sequence,
                &planner_checkpoint_bytes,
                "planner checkpoint bytes",
            )?;
            absent(sequence, &planner_samples, "planner samples")?;
            absent(
                sequence,
                &planner_recorded_at_ms,
                "planner recorded-at time",
            )?;
            absent(sequence, &start_index, "start index")?;
            absent(sequence, &start_input, "start input")?;
            payload::absent_columns(sequence, &start_payload, "start input")?;
            Ok(JournalEvent::ComponentCompleted {
                index: component_index(sequence, index)?,
                output: decode_payload(sequence, "output", output, output_payload)?,
                duration_us: optional_unsigned(sequence, duration_us, "duration")?,
            })
        }
        "checkpoint_created" => {
            absent(sequence, &fingerprint, "workflow fingerprint")?;
            absent(sequence, &name, "component name")?;
            absent(sequence, &hash, "component hash")?;
            absent(sequence, &input, "input")?;
            payload::absent_columns(sequence, &input_payload, "input")?;
            absent(sequence, &output, "output")?;
            payload::absent_columns(sequence, &output_payload, "output")?;
            absent(sequence, &workflow_name, "workflow name")?;
            absent(sequence, &durable_after, "durability")?;
            absent(sequence, &component_count, "component count")?;
            absent(sequence, &planner_profile_id, "planner profile id")?;
            absent(sequence, &planner_reason, "planner reason")?;
            absent(sequence, &planner_recompute_us, "planner recompute time")?;
            absent(sequence, &planner_checkpoint_us, "planner checkpoint time")?;
            absent(
                sequence,
                &planner_checkpoint_bytes,
                "planner checkpoint bytes",
            )?;
            absent(sequence, &planner_samples, "planner samples")?;
            absent(
                sequence,
                &planner_recorded_at_ms,
                "planner recorded-at time",
            )?;
            absent(sequence, &start_index, "start index")?;
            absent(sequence, &start_input, "start input")?;
            payload::absent_columns(sequence, &start_payload, "start input")?;
            Ok(JournalEvent::CheckpointCreated {
                index: component_index(sequence, index)?,
                hash: required(sequence, artifact_hash, "artifact hash")?,
                backend: artifact_backend,
                bytes: optional_unsigned(sequence, artifact_bytes, "artifact bytes")?,
                duration_us: optional_unsigned(sequence, duration_us, "duration")?,
            })
        }
        "workflow_completed" => {
            absent(sequence, &index, "component index")?;
            absent(sequence, &fingerprint, "workflow fingerprint")?;
            absent(sequence, &name, "component name")?;
            absent(sequence, &hash, "component hash")?;
            absent(sequence, &input, "input")?;
            payload::absent_columns(sequence, &input_payload, "input")?;
            absent(sequence, &artifact_hash, "artifact hash")?;
            absent(sequence, &workflow_name, "workflow name")?;
            absent(sequence, &duration_us, "duration")?;
            absent(sequence, &durable_after, "durability")?;
            absent(sequence, &component_count, "component count")?;
            absent(sequence, &artifact_backend, "artifact backend")?;
            absent(sequence, &artifact_bytes, "artifact bytes")?;
            absent(sequence, &planner_profile_id, "planner profile id")?;
            absent(sequence, &planner_reason, "planner reason")?;
            absent(sequence, &planner_recompute_us, "planner recompute time")?;
            absent(sequence, &planner_checkpoint_us, "planner checkpoint time")?;
            absent(
                sequence,
                &planner_checkpoint_bytes,
                "planner checkpoint bytes",
            )?;
            absent(sequence, &planner_samples, "planner samples")?;
            absent(
                sequence,
                &planner_recorded_at_ms,
                "planner recorded-at time",
            )?;
            absent(sequence, &start_index, "start index")?;
            absent(sequence, &start_input, "start input")?;
            payload::absent_columns(sequence, &start_payload, "start input")?;
            Ok(JournalEvent::WorkflowCompleted {
                output: decode_payload(sequence, "output", output, output_payload)?,
            })
        }
        "recovery_timed" => {
            absent(sequence, &index, "component index")?;
            absent(sequence, &fingerprint, "workflow fingerprint")?;
            absent(sequence, &name, "component name")?;
            absent(sequence, &hash, "component hash")?;
            absent(sequence, &input, "input")?;
            payload::absent_columns(sequence, &input_payload, "input")?;
            absent(sequence, &output, "output")?;
            payload::absent_columns(sequence, &output_payload, "output")?;
            absent(sequence, &artifact_hash, "artifact hash")?;
            absent(sequence, &workflow_name, "workflow name")?;
            absent(sequence, &durable_after, "durability")?;
            absent(sequence, &component_count, "component count")?;
            absent(sequence, &artifact_backend, "artifact backend")?;
            absent(sequence, &artifact_bytes, "artifact bytes")?;
            absent(sequence, &planner_profile_id, "planner profile id")?;
            absent(sequence, &planner_reason, "planner reason")?;
            absent(sequence, &planner_recompute_us, "planner recompute time")?;
            absent(sequence, &planner_checkpoint_us, "planner checkpoint time")?;
            absent(
                sequence,
                &planner_checkpoint_bytes,
                "planner checkpoint bytes",
            )?;
            absent(sequence, &planner_samples, "planner samples")?;
            absent(
                sequence,
                &planner_recorded_at_ms,
                "planner recorded-at time",
            )?;
            absent(sequence, &start_index, "start index")?;
            absent(sequence, &start_input, "start input")?;
            payload::absent_columns(sequence, &start_payload, "start input")?;
            Ok(JournalEvent::RecoveryTimed {
                duration_us: unsigned_u64(sequence, duration_us, "duration")?,
            })
        }
        "durability_planned" => {
            absent(sequence, &fingerprint, "workflow fingerprint")?;
            absent(sequence, &name, "component name")?;
            absent(sequence, &hash, "component hash")?;
            absent(sequence, &input, "input")?;
            payload::absent_columns(sequence, &input_payload, "input")?;
            absent(sequence, &output, "output")?;
            payload::absent_columns(sequence, &output_payload, "output")?;
            absent(sequence, &artifact_hash, "artifact hash")?;
            absent(sequence, &workflow_name, "workflow name")?;
            absent(sequence, &duration_us, "duration")?;
            absent(sequence, &component_count, "component count")?;
            absent(sequence, &artifact_backend, "artifact backend")?;
            absent(sequence, &artifact_bytes, "artifact bytes")?;
            absent(sequence, &start_index, "start index")?;
            absent(sequence, &start_input, "start input")?;
            payload::absent_columns(sequence, &start_payload, "start input")?;
            Ok(JournalEvent::DurabilityPlanned {
                index: component_index(sequence, index)?,
                required: required(
                    sequence,
                    optional_bool(sequence, durable_after, "durability")?,
                    "durability",
                )?,
                profile_id: required(sequence, planner_profile_id, "planner profile id")?,
                reason: required(sequence, planner_reason, "planner reason")?,
                recompute_us: optional_unsigned(
                    sequence,
                    planner_recompute_us,
                    "planner recompute time",
                )?,
                checkpoint_us: optional_unsigned(
                    sequence,
                    planner_checkpoint_us,
                    "planner checkpoint time",
                )?,
                checkpoint_bytes: optional_unsigned(
                    sequence,
                    planner_checkpoint_bytes,
                    "planner checkpoint bytes",
                )?,
                samples: optional_u32(sequence, planner_samples, "planner samples")?,
                recorded_at_ms: optional_unsigned(
                    sequence,
                    planner_recorded_at_ms,
                    "planner recorded-at time",
                )?,
            })
        }
        _ => Err(corrupt(sequence, format!("unknown event kind `{kind}`"))),
    }
}
