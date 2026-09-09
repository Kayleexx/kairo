use std::path::Path;

use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum ReceiptError {
    #[error("failed to inspect effect receipts")]
    Read(#[from] kairo_runtime::JournalError),
}

pub(crate) fn print(path: &Path) -> Result<(), ReceiptError> {
    let rows = kairo_runtime::inspect_receipts(path)?;
    if rows.is_empty() {
        return Ok(());
    }
    println!("\neffects");
    for row in rows {
        println!(
            "  {} · {}\n    id · {}",
            row.operation, row.status, row.effect_id
        );
        if let Some(result) = row.result_ref {
            println!("    result · {result}");
        }
    }
    Ok(())
}
