use std::path::Path;

use kairo_runtime::{DurableWait, WorkflowWaitError, inspect_workflow_wait};

pub(super) fn print(path: &Path) -> Result<(), WorkflowWaitError> {
    let Some(state) = inspect_workflow_wait(path)? else {
        return Ok(());
    };
    println!("\nwait");
    match state.wait {
        DurableWait::Signal { name } => println!("  signal · {name}"),
        DurableWait::Timer { .. } => println!("  timer · durable"),
    }
    println!(
        "  state · {}",
        if state.completed {
            "resumed"
        } else {
            "waiting"
        }
    );
    Ok(())
}
