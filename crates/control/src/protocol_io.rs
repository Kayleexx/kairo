use std::{
    io::{BufRead, BufReader, Read},
    net::TcpStream,
};

use crate::ControlError;

const MAX_MESSAGE_BYTES: u64 = 1024 * 1024;

pub(crate) fn read<T: for<'a> serde::Deserialize<'a>>(
    stream: &mut TcpStream,
) -> Result<T, ControlError> {
    let mut bytes = Vec::new();
    BufReader::new(stream)
        .take(MAX_MESSAGE_BYTES + 1)
        .read_until(b'\n', &mut bytes)
        .map_err(|source| ControlError::Io { source })?;
    if bytes.len() as u64 > MAX_MESSAGE_BYTES {
        return Err(ControlError::TooLarge);
    }
    serde_json::from_slice(&bytes).map_err(|source| ControlError::Protocol { source })
}
