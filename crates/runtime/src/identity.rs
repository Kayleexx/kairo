use kairo_core::ComponentHash;
use sha2::{Digest, Sha256};

pub(crate) struct StepIdentity {
    pub(crate) name: String,
    pub(crate) hash: ComponentHash,
    pub(crate) durable_after: bool,
}

pub(crate) fn workflow_fingerprint(name: &str, input: u32, steps: &[StepIdentity]) -> String {
    let mut digest = Sha256::new();
    hash_part(&mut digest, name.as_bytes());
    hash_part(&mut digest, &input.to_le_bytes());
    for step in steps {
        hash_part(&mut digest, step.name.as_bytes());
        hash_part(&mut digest, step.hash.to_string().as_bytes());
        hash_part(&mut digest, &[u8::from(step.durable_after)]);
    }
    format!("{:x}", digest.finalize())
}

// unlike `workflow_fingerprint`, excludes `input` and `durable_after` -- stable across inputs
// and across a `durability: auto` edge's resolved value, for use as a profile key.
pub(crate) fn workflow_shape(name: &str, steps: &[(String, ComponentHash)]) -> String {
    let mut digest = Sha256::new();
    hash_part(&mut digest, name.as_bytes());
    for (step_name, hash) in steps {
        hash_part(&mut digest, step_name.as_bytes());
        hash_part(&mut digest, hash.to_string().as_bytes());
    }
    format!("{:x}", digest.finalize())
}

fn hash_part(digest: &mut Sha256, value: &[u8]) {
    digest.update(value.len().to_le_bytes());
    digest.update(value);
}
