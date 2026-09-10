use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
};

use rusqlite::{Connection, OptionalExtension, params};

const MAX_REQUEST: u64 = 16 * 1024;

pub(crate) fn serve(database: &Path, response_delay_ms: u64) -> Result<(), String> {
    if let Some(parent) = database
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(error)?;
    }
    let connection = Connection::open(database).map_err(error)?;
    connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS effects(idempotency_key TEXT PRIMARY KEY, operation TEXT NOT NULL, input_value INTEGER NOT NULL, result_ref TEXT NOT NULL)").map_err(error)?;
    let listener = TcpListener::bind("127.0.0.1:0").map_err(error)?;
    let address = listener.local_addr().map_err(error)?;
    fs::create_dir_all(".kairo").map_err(error)?;
    fs::write(".kairo/effects.addr", address.to_string()).map_err(error)?;
    eprintln!("effect service ready · {address} · {}", database.display());
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let _ = handle(stream, &connection, response_delay_ms);
            }
            Err(source) => return Err(error(source)),
        }
    }
    Ok(())
}

fn handle(
    mut stream: TcpStream,
    connection: &Connection,
    response_delay_ms: u64,
) -> Result<(), String> {
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .map_err(error)?;
    let mut reader = BufReader::new(stream.try_clone().map_err(error)?);
    let mut request = String::new();
    reader.read_line(&mut request).map_err(error)?;
    let operation = request
        .strip_prefix("POST /")
        .and_then(|line| line.strip_suffix(" HTTP/1.1\r\n"))
        .filter(|value| !value.is_empty() && value.len() <= 64)
        .ok_or_else(|| "invalid effect request".to_owned())?;
    let mut key = None;
    let mut length = None;
    let mut read = request.len() as u64;
    loop {
        let mut line = String::new();
        read = read.saturating_add(reader.read_line(&mut line).map_err(error)? as u64);
        if read > MAX_REQUEST {
            return respond(&mut stream, 413, "request too large");
        }
        if line == "\r\n" {
            break;
        }
        if let Some(value) = line.strip_prefix("Idempotency-Key: ") {
            key = Some(value.trim().to_owned());
        }
        if let Some(value) = line.strip_prefix("Content-Length: ") {
            length = value.trim().parse::<usize>().ok();
        }
        if line.is_empty() {
            return Err("truncated effect request".to_owned());
        }
    }
    let length = length
        .filter(|value| *value <= MAX_REQUEST as usize)
        .ok_or_else(|| "invalid effect body length".to_owned())?;
    let mut body = vec![0; length];
    reader.read_exact(&mut body).map_err(error)?;
    let value = std::str::from_utf8(&body)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or_else(|| "invalid effect value".to_owned())?;
    let key = key
        .filter(|value| !value.is_empty() && value.len() <= 256)
        .ok_or_else(|| "invalid idempotency key".to_owned())?;
    let existing = connection
        .query_row(
            "SELECT operation, input_value, result_ref FROM effects WHERE idempotency_key = ?1",
            [&key],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u32>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .map_err(error)?;
    let (result, reused) = match existing {
        Some((stored_operation, stored_value, result))
            if stored_operation == operation && stored_value == value =>
        {
            (result, true)
        }
        Some(_) => return respond(&mut stream, 409, "idempotency key conflict"),
        None => {
            let result = format!(
                "effect-{}",
                connection.last_insert_rowid().saturating_add(1)
            );
            connection.execute("INSERT INTO effects(idempotency_key, operation, input_value, result_ref) VALUES (?1, ?2, ?3, ?4)", params![key, operation, value, result]).map_err(error)?;
            (result, false)
        }
    };
    if response_delay_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(response_delay_ms));
    }
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nKairo-Effect-Reused: {reused}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{result}",
        result.len()
    )
    .map_err(error)
}

fn respond(stream: &mut TcpStream, status: u16, body: &str) -> Result<(), String> {
    let label = if status == 200 { "OK" } else { "Error" };
    write!(
        stream,
        "HTTP/1.1 {status} {label}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .map_err(error)
}

fn error(error: impl std::fmt::Display) -> String {
    error.to_string()
}
