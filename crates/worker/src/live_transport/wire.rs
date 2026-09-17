use crate::live_transport::{EdgeIdentity, LiveTransportError, MAX_FRAME_BYTES, other};

pub(super) struct Request {
    pub(super) run_id: String,
    pub(super) edge_id: String,
    pub(super) epoch: u64,
}

pub(super) async fn write_request(
    send: &mut quinn::SendStream,
    identity: EdgeIdentity<'_>,
) -> Result<(), LiveTransportError> {
    let mut buffer = Vec::new();
    write_string(&mut buffer, identity.run_id);
    write_string(&mut buffer, identity.edge_id);
    buffer.extend_from_slice(&identity.epoch.to_be_bytes());
    send.write_all(&(buffer.len() as u32).to_be_bytes())
        .await
        .map_err(other)?;
    send.write_all(&buffer).await.map_err(other)
}

pub(super) async fn read_request(
    recv: &mut quinn::RecvStream,
) -> Result<Request, LiveTransportError> {
    let mut length = [0_u8; 4];
    recv.read_exact(&mut length).await.map_err(other)?;
    let length = u32::from_be_bytes(length) as usize;
    if length > MAX_FRAME_BYTES {
        return Err(LiveTransportError::FrameTooLarge);
    }
    let mut buffer = vec![0_u8; length];
    recv.read_exact(&mut buffer).await.map_err(other)?;
    let mut cursor = &buffer[..];
    let run_id = read_string(&mut cursor)?;
    let edge_id = read_string(&mut cursor)?;
    if cursor.len() < 8 {
        return Err(LiveTransportError::Io(std::io::Error::other(
            "truncated request",
        )));
    }
    let epoch = u64::from_be_bytes(cursor[..8].try_into().unwrap_or([0; 8]));
    Ok(Request {
        run_id,
        edge_id,
        epoch,
    })
}

fn write_string(buffer: &mut Vec<u8>, value: &str) {
    let bytes = value.as_bytes();
    buffer.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    buffer.extend_from_slice(bytes);
}

fn read_string(cursor: &mut &[u8]) -> Result<String, LiveTransportError> {
    if cursor.len() < 4 {
        return Err(LiveTransportError::Io(std::io::Error::other(
            "truncated string length",
        )));
    }
    let (length_bytes, rest) = cursor.split_at(4);
    let length = u32::from_be_bytes(length_bytes.try_into().unwrap_or([0; 4])) as usize;
    if rest.len() < length {
        return Err(LiveTransportError::Io(std::io::Error::other(
            "truncated string body",
        )));
    }
    let (value, rest) = rest.split_at(length);
    *cursor = rest;
    String::from_utf8(value.to_vec())
        .map_err(|source| LiveTransportError::Io(std::io::Error::other(source)))
}

const KIND_OK: u8 = 0;
const KIND_REJECTED: u8 = 1;
const KIND_DATA: u8 = 2;
const KIND_EOF: u8 = 3;

pub(super) async fn write_ok(send: &mut quinn::SendStream) -> Result<(), LiveTransportError> {
    send.write_all(&[KIND_OK]).await.map_err(other)
}

pub(super) async fn write_rejection(
    send: &mut quinn::SendStream,
    reason: &str,
) -> Result<(), LiveTransportError> {
    let mut buffer = vec![KIND_REJECTED];
    write_string(&mut buffer, reason);
    send.write_all(&buffer).await.map_err(other)
}

pub(super) async fn read_ok_or_rejection(
    recv: &mut quinn::RecvStream,
) -> Result<(), LiveTransportError> {
    let mut kind = [0_u8; 1];
    recv.read_exact(&mut kind).await.map_err(other)?;
    match kind[0] {
        KIND_OK => Ok(()),
        KIND_REJECTED => {
            let mut length = [0_u8; 4];
            recv.read_exact(&mut length).await.map_err(other)?;
            let length = u32::from_be_bytes(length) as usize;
            let mut buffer = vec![0_u8; length.min(MAX_FRAME_BYTES)];
            recv.read_exact(&mut buffer).await.map_err(other)?;
            Err(LiveTransportError::Rejected {
                reason: String::from_utf8_lossy(&buffer).into_owned(),
            })
        }
        other => Err(LiveTransportError::Io(std::io::Error::other(format!(
            "unexpected response kind {other}"
        )))),
    }
}

pub(super) async fn write_frame(
    send: &mut quinn::SendStream,
    sequence: u64,
    bytes: &[u8],
) -> Result<(), LiveTransportError> {
    let mut header = vec![KIND_DATA];
    header.extend_from_slice(&sequence.to_be_bytes());
    header.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    send.write_all(&header).await.map_err(other)?;
    send.write_all(bytes).await.map_err(other)
}

pub(super) async fn write_eof(
    send: &mut quinn::SendStream,
    final_sequence: u64,
) -> Result<(), LiveTransportError> {
    let mut header = vec![KIND_EOF];
    header.extend_from_slice(&final_sequence.to_be_bytes());
    send.write_all(&header).await.map_err(other)
}

pub(super) enum Frame {
    Data { sequence: u64, bytes: Vec<u8> },
    Eof { final_sequence: u64 },
}

pub(super) async fn read_frame(recv: &mut quinn::RecvStream) -> Result<Frame, LiveTransportError> {
    let mut kind = [0_u8; 1];
    recv.read_exact(&mut kind).await.map_err(other)?;
    let mut sequence_bytes = [0_u8; 8];
    recv.read_exact(&mut sequence_bytes).await.map_err(other)?;
    let sequence = u64::from_be_bytes(sequence_bytes);
    match kind[0] {
        KIND_DATA => {
            let mut length = [0_u8; 4];
            recv.read_exact(&mut length).await.map_err(other)?;
            let length = u32::from_be_bytes(length) as usize;
            if length > MAX_FRAME_BYTES {
                return Err(LiveTransportError::FrameTooLarge);
            }
            let mut bytes = vec![0_u8; length];
            recv.read_exact(&mut bytes).await.map_err(other)?;
            Ok(Frame::Data { sequence, bytes })
        }
        KIND_EOF => Ok(Frame::Eof {
            final_sequence: sequence,
        }),
        other => Err(LiveTransportError::Io(std::io::Error::other(format!(
            "unexpected frame kind {other}"
        )))),
    }
}
