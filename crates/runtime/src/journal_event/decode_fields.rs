use crate::journal::JournalError;

pub(super) fn required<T>(sequence: i64, value: Option<T>, field: &str) -> Result<T, JournalError> {
    value.ok_or_else(|| corrupt(sequence, format!("missing {field}")))
}

pub(super) fn absent<T>(sequence: i64, value: &Option<T>, field: &str) -> Result<(), JournalError> {
    if value.is_some() {
        return Err(corrupt(sequence, format!("unexpected {field}")));
    }
    Ok(())
}

pub(super) fn unsigned_u64(
    sequence: i64,
    value: Option<i64>,
    field: &str,
) -> Result<u64, JournalError> {
    let value = required(sequence, value, field)?;
    u64::try_from(value).map_err(|_| corrupt(sequence, format!("invalid {field}")))
}

pub(super) fn optional_unsigned(
    sequence: i64,
    value: Option<i64>,
    field: &str,
) -> Result<Option<u64>, JournalError> {
    value
        .map(|value| {
            u64::try_from(value).map_err(|_| corrupt(sequence, format!("invalid {field}")))
        })
        .transpose()
}

pub(super) fn optional_u32(
    sequence: i64,
    value: Option<i64>,
    field: &str,
) -> Result<Option<u32>, JournalError> {
    value
        .map(|value| {
            u32::try_from(value).map_err(|_| corrupt(sequence, format!("invalid {field}")))
        })
        .transpose()
}

pub(super) fn optional_bool(
    sequence: i64,
    value: Option<i64>,
    field: &str,
) -> Result<Option<bool>, JournalError> {
    value
        .map(|value| match value {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(corrupt(sequence, format!("invalid {field}"))),
        })
        .transpose()
}

pub(super) fn component_index(sequence: i64, value: Option<i64>) -> Result<usize, JournalError> {
    let value = required(sequence, value, "component index")?;
    usize::try_from(value).map_err(|_| corrupt(sequence, "invalid component index"))
}

pub(super) fn optional_index(
    sequence: i64,
    value: Option<i64>,
    field: &str,
) -> Result<Option<usize>, JournalError> {
    value
        .map(|value| {
            usize::try_from(value).map_err(|_| corrupt(sequence, format!("invalid {field}")))
        })
        .transpose()
}

pub(super) fn corrupt(sequence: i64, message: impl Into<String>) -> JournalError {
    JournalError::Corrupt {
        sequence,
        message: message.into(),
    }
}
