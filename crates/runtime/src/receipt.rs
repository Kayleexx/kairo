use std::path::Path;

use rusqlite::{Connection, OptionalExtension};

use crate::JournalError;

#[derive(Clone, Debug)]
pub struct EffectReceipt {
    pub effect_id: String,
    pub operation: String,
    pub status: String,
    pub result_ref: Option<String>,
}

pub fn inspect_receipts(path: &Path) -> Result<Vec<EffectReceipt>, JournalError> {
    let connection = Connection::open(path).map_err(|source| JournalError::Open { source })?;
    let exists: Option<String> = connection
        .query_row(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='effect_receipts'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|source| JournalError::Read { source })?;
    if exists.is_none() {
        return Ok(Vec::new());
    }
    let mut statement = connection.prepare("SELECT effect_id, operation, status, result_ref FROM effect_receipts ORDER BY effect_id").map_err(|source| JournalError::Read { source })?;
    let rows = statement
        .query_map([], |row| {
            Ok(EffectReceipt {
                effect_id: row.get(0)?,
                operation: row.get(1)?,
                status: row.get(2)?,
                result_ref: row.get(3)?,
            })
        })
        .map_err(|source| JournalError::Read { source })?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|source| JournalError::Read { source })
}
