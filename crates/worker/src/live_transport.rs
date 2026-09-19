//! `remote-live` physical Edge transport (Phase 18): a QUIC relay for handing a durable-boundary's
//! bytes directly to the next worker instead of a durable-artifact round trip.
//!
//! Trust: this only runs between workers already trusted by the control plane, so the client
//! accepts the server's self-signed cert without a PKI check; fencing (stale-epoch rejection) is
//! enforced at the application layer instead.

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};

use kairo_runtime::{RelaySink, RelaySource};
use quinn::{ClientConfig, Endpoint as QuinnEndpoint, ServerConfig};
use thiserror::Error;

mod wire;

use wire::{
    Frame, read_frame, read_ok_or_rejection, read_request, write_eof, write_frame, write_ok,
    write_rejection, write_request,
};

pub(crate) const MAX_FRAME_BYTES: usize = 256 * 1024;
const ALPN: &[u8] = b"kairo-live-edge/1";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
type StreamingAcknowledgement = (
    std::sync::mpsc::SyncSender<()>,
    std::sync::mpsc::Receiver<Result<(), String>>,
);

#[derive(Debug, Error)]
pub enum LiveTransportError {
    #[error("failed to configure the live transport")]
    Configure(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("failed to bind the live transport listener: {0}")]
    Bind(#[source] std::io::Error),
    #[error("failed to connect to `{addr}`")]
    Connect {
        addr: SocketAddr,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("live transport handshake timed out while trying to {phase}")]
    HandshakeTimeout { phase: &'static str },
    #[error("live source rejected this fetch: {reason}")]
    Rejected { reason: String },
    #[error("live transport I/O failed: {0}")]
    Io(#[source] std::io::Error),
    #[error("live source produced a frame larger than the {MAX_FRAME_BYTES}-byte limit")]
    FrameTooLarge,
}

pub(super) fn other(source: impl std::error::Error + Send + Sync + 'static) -> LiveTransportError {
    LiveTransportError::Io(std::io::Error::other(source))
}

/// identifies one logical Edge invocation for fencing -- the same triple the control plane uses
/// to fence a stale worker's writes.
#[derive(Clone, Copy, Debug)]
pub struct EdgeIdentity<'a> {
    pub run_id: &'a str,
    pub edge_id: &'a str,
    pub epoch: u64,
}

pub struct LiveMetrics {
    pub bytes: u64,
    pub duration: Duration,
    pub first_byte: Option<Duration>,
    // keeps the producer connection alive until the consumer result is reported.
    _connection: Option<quinn::Connection>,
}

/// one worker's live-transport endpoint -- both a server (offers bytes it just produced) and a
/// client (fetches bytes another worker is offering).
pub struct LiveEndpoint {
    quinn: QuinnEndpoint,
}

impl LiveEndpoint {
    pub fn bind() -> Result<Self, LiveTransportError> {
        // idempotent: rustls only allows one process-wide provider, and multiple workers/tests
        // in the same process each call `bind()` independently.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (cert, key) = self_signed_cert()?;
        let mut server_crypto = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert], key)
            .map_err(|source| LiveTransportError::Configure(Box::new(source)))?;
        server_crypto.alpn_protocols = vec![ALPN.to_vec()];
        let quic_crypto = quinn::crypto::rustls::QuicServerConfig::try_from(server_crypto)
            .map_err(|source| LiveTransportError::Configure(Box::new(source)))?;
        let server_config = ServerConfig::with_crypto(Arc::new(quic_crypto));
        let client_config = trusting_client_config()?;
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
        let mut quinn =
            QuinnEndpoint::server(server_config, addr).map_err(LiveTransportError::Bind)?;
        quinn.set_default_client_config(client_config);
        Ok(Self { quinn })
    }

    /// the address other workers should dial to fetch bytes this worker is offering -- shared via
    /// the control plane's own `Request::Yield`, never by a normal user.
    pub fn local_addr(&self) -> Result<SocketAddr, LiveTransportError> {
        self.quinn.local_addr().map_err(LiveTransportError::Io)
    }

    /// Relays one bounded component stream to one caller. This is intentionally not a `Vec<u8>`
    /// handoff: `source.next()` waits for downstream QUIC capacity, which wakes the component
    /// stream consumer only after the relay has room again.
    pub async fn serve_relay(
        &self,
        expected: EdgeIdentity<'_>,
        source: RelaySource,
    ) -> Result<LiveMetrics, LiveTransportError> {
        let started_at = Instant::now();
        let incoming = self
            .quinn
            .accept()
            .await
            .ok_or_else(|| LiveTransportError::Io(std::io::Error::other("endpoint closed")))?;
        let connection = incoming
            .await
            .map_err(|source| LiveTransportError::Connect {
                addr: self
                    .quinn
                    .local_addr()
                    .unwrap_or(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)),
                source: Box::new(source),
            })?;
        let (mut send, mut recv) = tokio::time::timeout(CONNECT_TIMEOUT, connection.accept_bi())
            .await
            .map_err(|_| handshake_timeout("accept the bidirectional stream"))?
            .map_err(other)?;
        let request = tokio::time::timeout(CONNECT_TIMEOUT, read_request(&mut recv))
            .await
            .map_err(|_| handshake_timeout("read the edge request"))??;
        if request.run_id != expected.run_id
            || request.edge_id != expected.edge_id
            || request.epoch != expected.epoch
        {
            write_rejection(
                &mut send,
                "edge identity does not match what this worker is offering",
            )
            .await?;
            return Err(LiveTransportError::Rejected {
                reason: "caller requested a different edge than the one being offered".to_owned(),
            });
        }
        write_ok(&mut send).await?;
        let mut bytes = 0_u64;
        let mut sequence = 0_u64;
        let mut first_byte = None;
        while let Some(chunk) = source.next().await.map_err(relay_error)? {
            if chunk.len() > MAX_FRAME_BYTES {
                return Err(LiveTransportError::FrameTooLarge);
            }
            bytes = bytes.saturating_add(chunk.len() as u64);
            first_byte.get_or_insert_with(|| started_at.elapsed());
            write_frame(&mut send, sequence, &chunk).await?;
            sequence = sequence.saturating_add(1);
        }
        write_eof(&mut send, sequence).await?;
        send.finish().map_err(other)?;
        Ok(LiveMetrics {
            bytes,
            duration: started_at.elapsed(),
            first_byte,
            _connection: Some(connection),
        })
    }

    /// Connects a live source to a bounded relay feeding a consumer Component. It pauses QUIC
    /// reads while the relay is full, so QUIC flow control reaches the source rather than
    /// accumulating an unbounded receiver-side buffer.
    pub async fn fetch_relay(
        &self,
        addr: SocketAddr,
        identity: EdgeIdentity<'_>,
        sink: RelaySink,
    ) -> Result<LiveMetrics, LiveTransportError> {
        self.fetch_relay_started(addr, identity, sink, None).await
    }

    pub async fn fetch_relay_started(
        &self,
        addr: SocketAddr,
        identity: EdgeIdentity<'_>,
        sink: RelaySink,
        started: Option<StreamingAcknowledgement>,
    ) -> Result<LiveMetrics, LiveTransportError> {
        let started_at = Instant::now();
        let connecting = self.quinn.connect(addr, "kairo-worker").map_err(|source| {
            LiveTransportError::Connect {
                addr,
                source: Box::new(source),
            }
        })?;
        let connection = tokio::time::timeout(CONNECT_TIMEOUT, connecting)
            .await
            .map_err(|_| LiveTransportError::Connect {
                addr,
                source: Box::new(std::io::Error::from(std::io::ErrorKind::TimedOut)),
            })?
            .map_err(|source| LiveTransportError::Connect {
                addr,
                source: Box::new(source),
            })?;
        let (mut send, mut recv) = tokio::time::timeout(CONNECT_TIMEOUT, connection.open_bi())
            .await
            .map_err(|_| handshake_timeout("open the bidirectional stream"))?
            .map_err(other)?;
        tokio::time::timeout(CONNECT_TIMEOUT, write_request(&mut send, identity))
            .await
            .map_err(|_| handshake_timeout("write the edge request"))??;
        send.finish().map_err(other)?;
        tokio::time::timeout(CONNECT_TIMEOUT, read_ok_or_rejection(&mut recv))
            .await
            .map_err(|_| handshake_timeout("read the edge response"))??;
        if let Some((started, acknowledged)) = started {
            let _ = started.send(());
            acknowledged
                .recv()
                .map_err(|_| {
                    LiveTransportError::Io(std::io::Error::other(
                        "live observation acknowledgement was dropped",
                    ))
                })?
                .map_err(|message| LiveTransportError::Io(std::io::Error::other(message)))?;
        }
        let mut bytes = 0_u64;
        let mut expected_sequence = 0_u64;
        let mut first_byte = None;
        loop {
            let frame = read_frame(&mut recv).await?;
            match frame {
                Frame::Data {
                    sequence,
                    bytes: frame_bytes,
                } => {
                    if sequence != expected_sequence {
                        return Err(LiveTransportError::Io(std::io::Error::other(
                            "frames arrived out of order",
                        )));
                    }
                    expected_sequence += 1;
                    first_byte.get_or_insert_with(|| started_at.elapsed());
                    let length = frame_bytes.len() as u64;
                    sink.send(frame_bytes).await.map_err(relay_error)?;
                    bytes = bytes.saturating_add(length);
                }
                Frame::Eof { final_sequence } if final_sequence == expected_sequence => break,
                Frame::Eof { .. } => {
                    return Err(LiveTransportError::Io(std::io::Error::other(
                        "stream ended after the wrong number of frames",
                    )));
                }
            }
        }
        sink.close();
        Ok(LiveMetrics {
            bytes,
            duration: started_at.elapsed(),
            first_byte,
            _connection: None,
        })
    }
}

fn relay_error(message: String) -> LiveTransportError {
    LiveTransportError::Io(std::io::Error::other(message))
}

fn handshake_timeout(phase: &'static str) -> LiveTransportError {
    LiveTransportError::HandshakeTimeout { phase }
}

fn self_signed_cert() -> Result<
    (
        rustls::pki_types::CertificateDer<'static>,
        rustls::pki_types::PrivateKeyDer<'static>,
    ),
    LiveTransportError,
> {
    let generated = rcgen::generate_simple_self_signed(vec!["kairo-worker".to_owned()])
        .map_err(|source| LiveTransportError::Configure(Box::new(source)))?;
    let key = rustls::pki_types::PrivateKeyDer::Pkcs8(generated.key_pair.serialize_der().into());
    Ok((generated.cert.der().clone(), key))
}

#[derive(Debug)]
struct NoServerVerification;

impl rustls::client::danger::ServerCertVerifier for NoServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn trusting_client_config() -> Result<ClientConfig, LiveTransportError> {
    let mut crypto = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoServerVerification))
        .with_no_client_auth();
    crypto.alpn_protocols = vec![ALPN.to_vec()];
    let quic_crypto = quinn::crypto::rustls::QuicClientConfig::try_from(crypto)
        .map_err(|source| LiveTransportError::Configure(Box::new(source)))?;
    Ok(ClientConfig::new(Arc::new(quic_crypto)))
}
