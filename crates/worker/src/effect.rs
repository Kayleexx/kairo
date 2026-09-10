use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    time::Duration,
};

use kairo_control::RunRequest;
use rusqlite::{Connection, params};

const MAX_RESPONSE: u64 = 16 * 1024;

pub(crate) fn apply(run: &RunRequest, operation: &str, value: u32) -> Result<(), String> {
    let connection = Connection::open(&run.state).map_err(error)?;
    connection.execute_batch("CREATE TABLE IF NOT EXISTS effect_receipts(effect_id TEXT PRIMARY KEY, operation TEXT NOT NULL, input_value INTEGER NOT NULL, status TEXT NOT NULL, result_ref TEXT, reused INTEGER NOT NULL DEFAULT 0)").map_err(error)?;
    ensure_reused_column(&connection)?;
    let effect_id = format!("{}:{operation}", run.id);
    connection.execute("INSERT OR IGNORE INTO effect_receipts(effect_id, operation, input_value, status) VALUES (?1, ?2, ?3, 'prepared')", params![effect_id, operation, value]).map_err(error)?;
    let receipt: (String, u32, String, Option<String>) = connection.query_row("SELECT operation, input_value, status, result_ref FROM effect_receipts WHERE effect_id = ?1", [&effect_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))).map_err(error)?;
    if receipt.0 != operation || receipt.1 != value {
        return Err("effect receipt does not match this workflow execution".to_owned());
    }
    if receipt.2 == "committed" && receipt.3.is_some() {
        connection
            .execute(
                "UPDATE effect_receipts SET reused = 1 WHERE effect_id = ?1",
                [&effect_id],
            )
            .map_err(error)?;
        return Ok(());
    }
    let address = endpoint()?;
    let (result, reused) = post(address, operation, &effect_id, value)?;
    connection.execute("UPDATE effect_receipts SET status = 'committed', result_ref = ?2, reused = ?3 WHERE effect_id = ?1 AND status = 'prepared'", params![effect_id, result, reused]).map_err(error)?;
    Ok(())
}

fn ensure_reused_column(connection: &Connection) -> Result<(), String> {
    let mut statement = connection
        .prepare("PRAGMA table_info(effect_receipts)")
        .map_err(error)?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(error)?;
    if !columns.iter().any(|column| column == "reused") {
        connection
            .execute(
                "ALTER TABLE effect_receipts ADD COLUMN reused INTEGER NOT NULL DEFAULT 0",
                [],
            )
            .map_err(error)?;
    }
    Ok(())
}

fn endpoint() -> Result<SocketAddr, String> {
    let source = fs::read_to_string(".kairo/effects.addr")
        .map_err(|_| "effect service is unavailable; run `kairo effects serve`".to_owned())?;
    source
        .trim()
        .parse()
        .map_err(|_| "effect service address is invalid".to_owned())
}

fn post(
    address: SocketAddr,
    operation: &str,
    key: &str,
    value: u32,
) -> Result<(String, bool), String> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))
        .map_err(|_| "effect service is unavailable; run `kairo effects serve`".to_owned())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(error)?;
    let body = value.to_string();
    write!(stream, "POST /{operation} HTTP/1.1\r\nHost: {address}\r\nIdempotency-Key: {key}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).map_err(error)?;
    let mut reader = BufReader::new(stream);
    let mut status = String::new();
    reader.read_line(&mut status).map_err(error)?;
    if !status.starts_with("HTTP/1.1 200 ") {
        return Err(format!(
            "effect service rejected the request: {}",
            status.trim()
        ));
    }
    let mut reused = false;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).map_err(error)?;
        if line == "\r\n" || line.is_empty() {
            break;
        }
        if line.eq_ignore_ascii_case("Kairo-Effect-Reused: true\r\n") {
            reused = true;
        }
    }
    let mut result = String::new();
    reader
        .take(MAX_RESPONSE)
        .read_to_string(&mut result)
        .map_err(error)?;
    let result = result.trim();
    if result.is_empty() {
        return Err("effect service returned an empty result".to_owned());
    }
    Ok((result.to_owned(), reused))
}

fn error(error: impl std::fmt::Display) -> String {
    error.to_string()
}
