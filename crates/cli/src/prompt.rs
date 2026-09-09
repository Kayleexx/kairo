use std::io::{self, Write};

use super::new::NewError;

pub(super) fn required(label: &str) -> Result<String, NewError> {
    let value = ask(label, "")?;
    if value.is_empty() {
        Err(NewError::MissingValue {
            field: label.to_owned(),
        })
    } else {
        Ok(value)
    }
}

pub(super) fn ask(label: &str, default: &str) -> Result<String, NewError> {
    print!(
        "{label}{}: ",
        if default.is_empty() {
            String::new()
        } else {
            format!(" [{default}]")
        }
    );
    io::stdout().flush().map_err(|source| NewError::Write {
        path: "<prompt>".into(),
        source,
    })?;
    let mut value = String::new();
    io::stdin()
        .read_line(&mut value)
        .map_err(|source| NewError::Write {
            path: "<prompt>".into(),
            source,
        })?;
    let value = value.trim();
    Ok(if value.is_empty() {
        default.to_owned()
    } else {
        value.to_owned()
    })
}

pub(super) fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}
