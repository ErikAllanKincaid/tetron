//! Tor hidden service utilities for iroh.
//!
//! This crate provides utilities for creating Tor hidden services that can be used
//! as a custom transport for iroh networking.

use std::{collections::HashMap, future::Future, io, num::NonZeroUsize, pin::Pin, sync::Arc};

use bytes::Bytes;
use iroh::{
    EndpointId, SecretKey, TransportAddr,
    address_lookup::{self, AddressLookup, EndpointData, EndpointInfo, Item},
    endpoint::{
        Builder,
        presets::{Minimal, Preset},
        transports::{CustomEndpoint, CustomSender, CustomTransport, RecvInfo, Transmit},
    },
};
use iroh_base::CustomAddr;
use n0_error::{e, stack_error};
use n0_future::{boxed::BoxFuture, stream};
use n0_watcher::Watchable;
use sha2::{Digest, Sha512};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
    sync::Mutex,
};
use tokio_socks::tcp::Socks5Stream;
use torut::{
    control::{AsyncEvent, AuthenticatedConn, ConnError, UnauthenticatedConn},
    onion::{OnionAddressV3, TorPublicKeyV3, TorSecretKeyV3},
};

/// Errors that can occur when building a Tor transport.
#[stack_error(derive, add_meta)]
#[non_exhaustive]
pub enum BuildError {
    /// Failed to bind the local TCP listener.
    #[error("Failed to bind local listener")]
    BindListener {
        #[error(std_err)]
        source: io::Error,
    },
    /// Failed to connect to the Tor control port.
    #[error("Failed to connect to Tor control port")]
    ControlConnect {
        #[error(std_err)]
        source: io::Error,
    },
    /// Failed to load Tor protocol info.
    #[error("Failed to load Tor protocol info")]
    ProtocolInfo {
        #[error(std_err)]
        source: ConnError,
    },
    /// Failed to determine Tor auth method.
    #[error("Failed to determine Tor auth method")]
    AuthMethod {
        #[error(std_err)]
        source: io::Error,
    },
    /// Failed to authenticate with Tor.
    #[error("Failed to authenticate with Tor")]
    Auth {
        #[error(std_err)]
        source: ConnError,
    },
    /// Failed to create hidden service.
    #[error("Failed to create hidden service")]
    CreateOnion {
        #[error(std_err)]
        source: ConnError,
    },
}

/// Convert an iroh SecretKey to a Tor v3 secret key.
fn iroh_to_tor_secret_key(key: &SecretKey) -> TorSecretKeyV3 {
    let seed = key.to_bytes();
    let hash = Sha512::digest(seed);
    let mut expanded_bytes: [u8; 64] = hash.into();
    expanded_bytes[0] &= 248;
    expanded_bytes[31] &= 63;
    expanded_bytes[31] |= 64;
    TorSecretKeyV3::from(expanded_bytes)
}

/// Get the onion address for an iroh `EndpointId` (public key only).
///
/// Returns `None` if the public key bytes are invalid for Tor (should not happen
/// with valid iroh keys since they use the same curve).
pub(crate) fn onion_address_from_endpoint(endpoint: EndpointId) -> Option<OnionAddressV3> {
    let bytes = endpoint.as_bytes();
    let tor_public = TorPublicKeyV3::from_bytes(bytes).ok()?;
    Some(tor_public.get_onion_address())
}

/// A packet carried over the Tor stream transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TorPacket {
    /// Source endpoint id (32 bytes).
    pub from: EndpointId,
    /// Raw packet payload.
    pub data: Bytes,
    /// Segment size to split up data (optional).
    pub segment_size: Option<u16>,
}

const FLAG_SEGMENT_SIZE: u8 = 0x01;
/// Transport id for the Tor user transport.
const TOR_USER_TRANSPORT_ID: u64 = 0x544f52;

/// Build a user transport address for the Tor transport.
fn tor_user_addr(endpoint: EndpointId) -> CustomAddr {
    CustomAddr::from_parts(TOR_USER_TRANSPORT_ID, endpoint.as_bytes())
}

/// Discovery service that maps any `EndpointId` to its Tor user transport address.
#[derive(Debug, Clone)]
struct TorAddressLookup;

impl AddressLookup for TorAddressLookup {
    fn resolve(
        &self,
        endpoint_id: EndpointId,
    ) -> Option<n0_future::boxed::BoxStream<Result<Item, address_lookup::Error>>> {
        let info = EndpointInfo {
            endpoint_id,
            data: EndpointData::new(vec![TransportAddr::Custom(tor_user_addr(endpoint_id))]),
        };
        Some(Box::pin(stream::once(Ok(Item::new(
            info,
            "tor-user-addr",
            None,
        )))))
    }
}

fn parse_user_addr(addr: &CustomAddr) -> io::Result<EndpointId> {
    if addr.id() != TOR_USER_TRANSPORT_ID {
        return Err(io::Error::other("unexpected transport id"));
    }
    let data = addr.data();
    if data.len() != 32 {
        return Err(io::Error::other("unexpected endpoint id length"));
    }
    let bytes: [u8; 32] = data
        .try_into()
        .map_err(|_| io::Error::other("endpoint id bytes"))?;
    EndpointId::from_bytes(&bytes).map_err(io::Error::other)
}

/// Read a single packet from a stream. Returns `Ok(None)` on clean EOF.
pub(crate) async fn read_tor_packet<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> io::Result<Option<TorPacket>> {
    let mut flags = [0u8; 1];
    let mut read = 0usize;
    while read < flags.len() {
        let n = reader.read(&mut flags[read..]).await?;
        if n == 0 {
            return Ok(None);
        }
        read += n;
    }

    let mut from_bytes = [0u8; 32];
    reader.read_exact(&mut from_bytes).await?;
    let from = EndpointId::from_bytes(&from_bytes)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let segment_size = if flags[0] & FLAG_SEGMENT_SIZE != 0 {
        let mut size_bytes = [0u8; 2];
        reader.read_exact(&mut size_bytes).await?;
        Some(u16::from_be_bytes(size_bytes))
    } else {
        None
    };

    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut len_bytes).await?;
    let len = u32::from_be_bytes(len_bytes) as usize;

    let mut data = vec![0u8; len];
    reader.read_exact(&mut data).await?;

    Ok(Some(TorPacket {
        from,
        data: Bytes::from(data),
        segment_size,
    }))
}

/// Write a single packet to a stream.
pub(crate) async fn write_tor_packet<W: AsyncWrite + Unpin>(
    writer: &mut W,
    packet: &TorPacket,
) -> io::Result<()> {
    let mut flags = 0u8;
    if packet.segment_size.is_some() {
        flags |= FLAG_SEGMENT_SIZE;
    }
    writer.write_all(&[flags]).await?;
    writer.write_all(packet.from.as_bytes()).await?;
    if let Some(segment_size) = packet.segment_size {
        writer.write_all(&segment_size.to_be_bytes()).await?;
    }
    let len = u32::try_from(packet.data.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "packet too large"))?;
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(&packet.data).await?;
    writer.flush().await?;
    Ok(())
}

/// A service that reads framed packets from a stream and dispatches them to a channel.
#[derive(Clone)]
pub(crate) struct TorPacketService {
    sender: tokio::sync::mpsc::Sender<TorPacket>,
}

impl TorPacketService {
    /// Create a new service with the given handler.
    pub(crate) fn new(sender: tokio::sync::mpsc::Sender<TorPacket>) -> Self {
        Self { sender }
    }

    /// Handle packets on a single stream until EOF.
    pub(crate) async fn handle_stream(&self, mut stream: TcpStream) -> io::Result<()> {
        while let Some(packet) = read_tor_packet(&mut stream).await? {
            let _ = self.sender.send(packet).await;
        }
        Ok(())
    }
}

/// IO for Tor-backed streams.
pub(crate) struct TorStreamIo {
    accept: Box<dyn Fn() -> BoxFuture<io::Result<TcpStream>> + Send + Sync>,
    connect: Box<dyn Fn(EndpointId) -> BoxFuture<io::Result<TcpStream>> + Send + Sync>,
}

impl TorStreamIo {
    /// Create a new IO wrapper from accept/connect functions.
    pub(crate) fn new<Accept, Connect, AcceptFut, ConnectFut>(
        accept: Accept,
        connect: Connect,
    ) -> Self
    where
        Accept: Fn() -> AcceptFut + Send + Sync + 'static,
        AcceptFut: Future<Output = io::Result<TcpStream>> + Send + 'static,
        Connect: Fn(EndpointId) -> ConnectFut + Send + Sync + 'static,
        ConnectFut: Future<Output = io::Result<TcpStream>> + Send + 'static,
    {
        Self {
            accept: Box::new(move || Box::pin(accept())),
            connect: Box::new(move |endpoint| Box::pin(connect(endpoint))),
        }
    }

    /// Connect to the remote endpoint's stream transport.
    fn connect(&self, endpoint: EndpointId) -> BoxFuture<io::Result<TcpStream>> {
        (self.connect)(endpoint)
    }

    /// Accept the next incoming stream.
    fn accept(&self) -> BoxFuture<io::Result<TcpStream>> {
        (self.accept)()
    }
}

/// tetron-local patch (PATCH.md, TOR-DIAL-001 follow-up): bounds a stuck
/// SOCKS5 connect to a peer's onion service -- see `TorPacketSender::
/// get_or_connect`'s own comment for why this was previously unbounded.
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// tetron-local patch (PATCH.md, Patch 2, TOR-DIAL-001 follow-up): how long
/// `TorCustomTransportBuilder::build` waits for `HS_DESC_UPLOAD_QUORUM`
/// `HS_DESC UPLOADED` confirmations before giving up and returning anyway.
/// Chosen generously relative to the ~30s-120s range this project has
/// observed v3 onion descriptor publication take in practice.
const HS_DESC_PUBLISH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);

/// How often the publish-wait loop pumps `AuthenticatedConn`'s response
/// read loop (via a cheap `noop()` round trip) to notice a buffered
/// `HS_DESC` event promptly rather than only on some later, unrelated
/// command.
const HS_DESC_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

/// tetron-local patch (PATCH.md, Patch 3, TOR-DIAL-001 follow-up): how many
/// distinct `HS_DESC UPLOADED` confirmations `build` waits for before
/// considering the descriptor published. Tor's v3 onion service spec
/// (`rend-spec-v3`) uploads each descriptor to a deterministic set of
/// `hsdir_n_replicas` (2) x `hsdir_spread_store` (4) = 8 distinct HSDirs; a
/// dialing client computes and queries that same set independently, with
/// no fallback beyond it. Waiting for only one (the pre-Patch-3 behavior)
/// let dialing start before most of the responsible directories had the
/// descriptor, producing `Host unreachable` / Tor's own "No more HSDir
/// available to query" from any client whose independently-computed query
/// set included one of the other 7 -- see `tetron/spec/core.py`'s
/// `TorDialPathWiring`, Fix 6, for the full investigation.
const HS_DESC_UPLOAD_QUORUM: usize = 8;

/// Pure decision for `build`'s HS_DESC publish-wait loop (Patch 3): whether
/// to stop waiting given the current confirmation count, the quorum
/// required, and whether the deadline has passed. Extracted so the
/// threshold/timeout logic is directly unit-testable without a live Tor
/// control connection, mirroring tetron core's own `path_flap_decision`
/// pattern (`src/forward.rs`). The deadline is always a fallback, never a
/// requirement -- a pathological network that never reaches quorum still
/// proceeds after `HS_DESC_PUBLISH_TIMEOUT`, exactly as the pre-Patch-3
/// single-confirmation wait did, so this is strictly an improvement to the
/// common case with no regression to the worst case.
fn hs_desc_wait_should_stop(confirmations: usize, quorum: usize, deadline_passed: bool) -> bool {
    confirmations >= quorum || deadline_passed
}

/// Packet writer that reuses per-endpoint streams.
///
/// tetron-local patch (PATCH.md, Patch 4, TOR-DIAL-001 follow-up, Fix 8):
/// each peer maps to a per-peer connect *slot* (`Mutex<Option<...>>`), not
/// directly to a stream. This is what lets concurrent `send()` calls for
/// the same peer de-duplicate onto one connect attempt instead of racing
/// independent ones -- see `get_or_connect`'s own doc comment for why that
/// matters.
pub(crate) struct TorPacketSender {
    io: Arc<TorStreamIo>,
    streams: Mutex<HashMap<EndpointId, Arc<Mutex<Option<Arc<Mutex<TcpStream>>>>>>>,
}

impl TorPacketSender {
    /// Create a new sender with the provided connector.
    pub(crate) fn new(io: Arc<TorStreamIo>) -> Self {
        Self {
            io,
            streams: Mutex::new(HashMap::new()),
        }
    }

    /// Send a packet to the given endpoint, reusing an existing stream when available.
    pub(crate) async fn send(&self, to: EndpointId, packet: &TorPacket) -> io::Result<()> {
        let stream = self.get_or_connect(to).await?;
        let mut guard = stream.lock().await;
        match write_tor_packet(&mut *guard, packet).await {
            Ok(()) => Ok(()),
            Err(err) => {
                drop(guard);
                self.streams.lock().await.remove(&to);
                Err(err)
            }
        }
    }

    /// Get this peer's existing stream, or connect one -- de-duplicating
    /// concurrent callers for the *same* peer onto a single connect
    /// attempt (Fix 8).
    ///
    /// Before this patch, a peer was only recorded in `streams` *after* a
    /// connect succeeded, so every caller that arrived while a connect for
    /// that same peer was still in flight independently started its own.
    /// `poll_send` spawns a fresh, independent, fire-and-forget task per
    /// outbound packet/chunk with no coordination between them -- and QUIC's
    /// own PATH_CHALLENGE retransmission (plus this project's own
    /// backup-path probing) fires well inside the many seconds a real Tor
    /// hidden-service rendezvous can take, so this was not a rare edge case
    /// but the *common* case under any real load: confirmed live via Tor's
    /// own log showing 146 distinct SOCKS connections in one ~5-minute
    /// window (roughly one every 2 seconds) and repeatedly having to
    /// invalidate and re-fetch a descriptor a competing attempt had just
    /// interrupted -- a self-perpetuating stampede that could never let a
    /// single attempt run long enough to complete. See
    /// `tetron/spec/core.py`'s `TorDialPathWiring`, Fix 8, for the full
    /// investigation.
    ///
    /// Now each peer maps to a connect *slot* up front; the first caller to
    /// lock an empty slot performs the real connect and fills it, and every
    /// other concurrent caller -- once it acquires the same slot's lock --
    /// finds it already filled and reuses it immediately, with zero
    /// additional connects.
    async fn get_or_connect(&self, to: EndpointId) -> io::Result<Arc<Mutex<TcpStream>>> {
        let slot = self
            .streams
            .lock()
            .await
            .entry(to)
            .or_insert_with(|| Arc::new(Mutex::new(None)))
            .clone();

        let mut guard = slot.lock().await;
        if let Some(existing) = guard.as_ref() {
            return Ok(existing.clone());
        }

        // tetron-local patch (PATCH.md, TOR-DIAL-001 follow-up): the
        // underlying SOCKS5 connect to a peer's onion service had no
        // timeout at all -- a stuck rendezvous/circuit build hangs this
        // future forever, and since `poll_send` (below) spawns `send`
        // fire-and-forget, that hang was completely invisible: no error
        // ever surfaced, QUIC's own PATH_CHALLENGE just never got sent and
        // the path silently timed out at the full idle-timeout ceiling
        // (300s, tetron's own `CUSTOM_TRANSPORT_PATH_MAX_IDLE_TIMEOUT`)
        // with nothing to explain why. Bounding it here lets a genuinely
        // stuck connect fail within a fraction of that window instead,
        // giving the retry-on-abandon mechanism (VEILID-016, widened for
        // `UnusableAfterNetworkChange`) more attempts inside the same
        // overall budget.
        let stream = tokio::time::timeout(CONNECT_TIMEOUT, self.io.connect(to))
            .await
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("Tor connect to {to} timed out after {CONNECT_TIMEOUT:?}"),
                )
            })??;
        let stream = Arc::new(Mutex::new(stream));
        *guard = Some(stream.clone());
        Ok(stream)
    }

    /// Close and remove a cached stream for the given endpoint.
    #[allow(dead_code)]
    pub(crate) async fn close(&self, to: EndpointId) -> io::Result<()> {
        let slot = self.streams.lock().await.remove(&to);
        if let Some(slot) = slot
            && let Some(stream) = slot.lock().await.take()
        {
            let mut guard = stream.lock().await;
            guard.shutdown().await?;
        }
        Ok(())
    }

    /// Close and remove all cached streams.
    #[allow(dead_code)]
    pub(crate) async fn close_all(&self) -> io::Result<()> {
        let slots: Vec<_> = self.streams.lock().await.drain().map(|(_, v)| v).collect();
        for slot in slots {
            if let Some(stream) = slot.lock().await.take() {
                let mut guard = stream.lock().await;
                guard.shutdown().await?;
            }
        }
        Ok(())
    }
}

const DEFAULT_RECV_CAPACITY: usize = 64 * 1024;
const DEFAULT_SOCKS_PORT: u16 = 9050;
const DEFAULT_CONTROL_PORT: u16 = 9051;
const DEFAULT_ONION_PORT: u16 = 9999;

/// Type alias for the async event handler function.
type EventHandler = Box<
    dyn Fn(
            torut::control::AsyncEvent<'static>,
        ) -> Pin<Box<dyn Future<Output = Result<(), ConnError>> + Send>>
        + Send
        + Sync,
>;

/// Builder for [`TorCustomTransport`].
///
/// # Defaults
///
/// - SOCKS5 proxy port: 9050
/// - Control port: 9051
/// - Onion service port: 9999
#[derive(Clone, Default)]
pub struct TorCustomTransportBuilder {
    socks_port: u16,
    control_port: u16,
    onion_port: u16,
    #[cfg(test)]
    io: Option<Arc<TorStreamIo>>,
}

impl TorCustomTransportBuilder {
    /// Set the SOCKS5 proxy port (default: 9050).
    pub fn socks_port(mut self, port: u16) -> Self {
        self.socks_port = port;
        self
    }

    /// Set the Tor control port (default: 9051).
    pub fn control_port(mut self, port: u16) -> Self {
        self.control_port = port;
        self
    }

    /// Set the onion service port (default: 9999).
    pub fn onion_port(mut self, port: u16) -> Self {
        self.onion_port = port;
        self
    }

    /// Override with custom IO (for testing with local TCP instead of Tor).
    #[cfg(test)]
    pub(crate) fn io(mut self, io: Arc<TorStreamIo>) -> Self {
        self.io = Some(io);
        self
    }

    /// Build the transport.
    ///
    /// This connects to the Tor control port, creates a hidden service,
    /// and sets up the transport IO.
    ///
    /// # Arguments
    ///
    /// * `secret_key` - The iroh secret key for this endpoint
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Cannot bind the local listener
    /// - Cannot connect to the Tor control port
    /// - Cannot authenticate with Tor
    /// - Cannot create the hidden service
    pub async fn build(self, secret_key: SecretKey) -> Result<Arc<TorCustomTransport>, BuildError> {
        let local_id = secret_key.public();

        #[cfg(test)]
        if let Some(io) = self.io {
            return Ok(Arc::new(TorCustomTransport {
                local_id,
                io,
                control_conn: None,
            }));
        }

        // Bind local listener
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|err| e!(BuildError::BindListener, err))?;
        let local_addr = listener
            .local_addr()
            .map_err(|err| e!(BuildError::BindListener, err))?;
        let listener = Arc::new(listener);

        // Connect to Tor control port and create hidden service
        let control_addr = format!("127.0.0.1:{}", self.control_port);
        let stream = TcpStream::connect(&control_addr)
            .await
            .map_err(|err| e!(BuildError::ControlConnect, err))?;
        let mut conn = UnauthenticatedConn::new(stream);
        let auth_data = conn
            .load_protocol_info()
            .await
            .map_err(|err| e!(BuildError::ProtocolInfo, err))?;
        let auth_method = auth_data
            .make_auth_data()
            .map_err(|err| e!(BuildError::AuthMethod, err))?;
        if let Some(auth) = auth_method {
            conn.authenticate(&auth)
                .await
                .map_err(|err| e!(BuildError::Auth, err))?;
        }
        let mut conn: AuthenticatedConn<TcpStream, EventHandler> = conn.into_authenticated().await;

        // tetron-local patch (PATCH.md, Patch 2, TOR-DIAL-001 follow-up):
        // register for HS_DESC events *before* creating the onion service,
        // so a descriptor-uploaded confirmation that arrives immediately
        // after ADD_ONION can't be missed. See `wait_for_hs_desc_uploaded`'s
        // own doc comment for why this exists at all. `set_events` failing
        // is treated as non-fatal (older Tor, or a control API this event
        // isn't available on) -- degrade to the pre-patch behavior (build()
        // returns immediately, no publish confirmation) rather than
        // blocking hidden-service creation on it.
        let upload_confirmations: Arc<std::sync::atomic::AtomicUsize> =
            Arc::new(std::sync::atomic::AtomicUsize::new(0));

        // Create the hidden service
        let tor_key = iroh_to_tor_secret_key(&secret_key);
        let onion_addr = tor_key.public().get_onion_address();
        let target_hs_address = onion_addr.get_address_without_dot_onion().to_string();
        {
            let upload_confirmations = upload_confirmations.clone();
            let target_hs_address = target_hs_address.clone();
            let handler: EventHandler = Box::new(move |event: AsyncEvent<'static>| {
                let upload_confirmations = upload_confirmations.clone();
                let target_hs_address = target_hs_address.clone();
                Box::pin(async move {
                    if let Some(line) = event.lines.first()
                        && let Some(rest) = line.strip_prefix("HS_DESC UPLOADED ")
                        && rest.split(' ').next() == Some(target_hs_address.as_str())
                    {
                        upload_confirmations.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                    Ok(())
                })
            });
            conn.set_async_event_handler(Some(handler));
        }
        let hs_desc_events_enabled = match conn.set_events(false, &mut ["HS_DESC"].into_iter()).await {
            Ok(()) => true,
            Err(err) => {
                tracing::warn!(%err, "could not subscribe to HS_DESC events -- proceeding without a publish-confirmation wait");
                false
            }
        };

        let listeners = [(self.onion_port, local_addr)];
        match conn
            .add_onion_v3(&tor_key, false, false, false, None, &mut listeners.iter())
            .await
        {
            Ok(()) => {}
            Err(ConnError::InvalidResponseCode(552)) => {
                // Service already exists, that's fine
            }
            Err(err) => return Err(e!(BuildError::CreateOnion, err)),
        }

        tracing::info!(
            "Hidden service created: {}.onion:{}",
            onion_addr.get_address_without_dot_onion(),
            self.onion_port
        );

        // tetron-local patch (PATCH.md, Patch 2, TOR-DIAL-001 follow-up):
        // `ADD_ONION`'s own `250 OK` only confirms the service was created
        // *locally* -- Tor's control-spec documents the descriptor upload
        // to the HSDir network as a genuinely separate, asynchronous step
        // (the `HS_DESC` event's `UPLOADED` action), and callers that start
        // dialing this service immediately have no guarantee it has
        // reached the network at all yet. `AuthenticatedConn` has no
        // dedicated "wait for the next event" call -- `noop()` is used
        // purely to pump its response-read loop (which drains and
        // dispatches any buffered `650` lines, including ours, before
        // returning) at a steady interval instead.
        if hs_desc_events_enabled {
            let deadline = tokio::time::Instant::now() + HS_DESC_PUBLISH_TIMEOUT;
            loop {
                let confirmations =
                    upload_confirmations.load(std::sync::atomic::Ordering::SeqCst);
                let deadline_passed = tokio::time::Instant::now() >= deadline;
                if hs_desc_wait_should_stop(confirmations, HS_DESC_UPLOAD_QUORUM, deadline_passed)
                {
                    if confirmations >= HS_DESC_UPLOAD_QUORUM {
                        tracing::info!(
                            onion = %target_hs_address,
                            confirmations,
                            quorum = HS_DESC_UPLOAD_QUORUM,
                            "hidden service descriptor confirmed uploaded to quorum of HSDirs"
                        );
                    } else {
                        tracing::warn!(
                            onion = %target_hs_address,
                            confirmations,
                            quorum = HS_DESC_UPLOAD_QUORUM,
                            timeout = ?HS_DESC_PUBLISH_TIMEOUT,
                            "HS_DESC UPLOADED quorum not reached within the timeout; proceeding anyway (the service may still not be fully reachable yet)"
                        );
                    }
                    break;
                }
                // Any error here (including a transient one) just means this
                // particular poll tick didn't confirm anything -- the loop
                // retries until the deadline regardless.
                let _ = conn.noop().await;
                tokio::time::sleep(HS_DESC_POLL_INTERVAL).await;
            }
        }

        let socks_addr: std::net::SocketAddr =
            format!("127.0.0.1:{}", self.socks_port).parse().unwrap();
        let onion_port = self.onion_port;
        let io = Arc::new(TorStreamIo::new(
            move || {
                let listener = listener.clone();
                async move {
                    let (stream, _) = listener.accept().await?;
                    Ok(stream)
                }
            },
            move |endpoint| async move {
                let onion = onion_address_from_endpoint(endpoint).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "invalid endpoint id")
                })?;
                let onion_addr = format!("{}.onion", onion.get_address_without_dot_onion());
                let stream = Socks5Stream::connect(socks_addr, (onion_addr.as_str(), onion_port))
                    .await
                    .map_err(io::Error::other)?;
                Ok(stream.into_inner())
            },
        ));

        Ok(Arc::new(TorCustomTransport {
            local_id,
            io,
            control_conn: Some(Arc::new(conn)),
        }))
    }
}

/// A Tor-backed user transport factory for iroh.
///
/// This holds the configuration and IO for the Tor transport. The actual
/// transport instance is created when iroh calls `bind()` during endpoint setup.
///
/// Use `TorCustomTransport::builder()` to create and configure.
///
/// # Example
///
/// ```ignore
/// let transport = TorCustomTransport::builder(secret_key).build().await;
///
/// Endpoint::builder(transport.preset())
///     .secret_key(secret_key)
///     .bind()
///     .await?
/// ```
#[derive(Clone)]
pub struct TorCustomTransport {
    local_id: EndpointId,
    io: Arc<TorStreamIo>,
    /// Keep the control connection alive to maintain the ephemeral hidden service.
    /// The hidden service is removed when this connection is dropped.
    /// Wrapped in Arc so it can be shared with TorCustomEndpoint.
    #[allow(dead_code)]
    control_conn: Option<Arc<AuthenticatedConn<TcpStream, EventHandler>>>,
}

impl TorCustomTransport {
    /// Create a builder for configuring a Tor user transport.
    pub fn builder() -> TorCustomTransportBuilder {
        TorCustomTransportBuilder {
            socks_port: DEFAULT_SOCKS_PORT,
            control_port: DEFAULT_CONTROL_PORT,
            onion_port: DEFAULT_ONION_PORT,
            #[cfg(test)]
            io: None,
        }
    }

    /// Returns a discovery service for this transport.
    ///
    /// The discovery service maps any `EndpointId` to its Tor user transport address.
    pub fn discovery(&self) -> impl AddressLookup {
        TorAddressLookup
    }

    /// Returns a preset that configures an endpoint to use this Tor transport.
    ///
    /// The preset adds the Tor user transport factory and discovery service.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let transport = TorCustomTransport::builder(sk.clone()).build().await;
    ///
    /// Endpoint::builder(transport.preset())
    ///     .secret_key(sk)
    ///     .bind()
    ///     .await?
    /// ```
    pub fn preset(self: &Arc<Self>) -> impl Preset {
        TorPreset {
            factory: self.clone(),
        }
    }
}

impl std::fmt::Debug for TorCustomTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TorCustomTransport")
            .field("local_id", &self.local_id)
            .finish()
    }
}

impl CustomTransport for TorCustomTransport {
    fn bind(&self) -> io::Result<Box<dyn CustomEndpoint>> {
        let (tx, rx) = tokio::sync::mpsc::channel(DEFAULT_RECV_CAPACITY);
        let service = TorPacketService::new(tx);
        let sender = Arc::new(TorPacketSender::new(self.io.clone()));
        let watchable = Watchable::new(vec![tor_user_addr(self.local_id)]);

        let io = self.io.clone();
        tokio::spawn(async move {
            loop {
                match io.accept().await {
                    Ok(stream) => {
                        let service = service.clone();
                        tokio::spawn(async move {
                            // tetron-local patch (PATCH.md, TOR-DIAL-001
                            // follow-up): same silent-failure pattern as
                            // poll_send's own fix above -- surface it.
                            if let Err(err) = service.handle_stream(stream).await {
                                tracing::warn!(%err, "Tor incoming stream handler failed");
                            }
                        });
                    }
                    Err(err) => {
                        tracing::warn!("Tor accept loop stopped: {err:#}");
                        break;
                    }
                }
            }
        });

        Ok(Box::new(TorCustomEndpoint {
            local_id: self.local_id,
            watchable,
            receiver: rx,
            sender,
        }))
    }
}

/// Internal preset for configuring an iroh endpoint to use the Tor transport.
struct TorPreset {
    factory: Arc<dyn CustomTransport>,
}

impl Preset for TorPreset {
    fn apply(self, builder: Builder) -> Builder {
        // `Minimal` sets the mandatory crypto provider (ring or aws-lc-rs,
        // depending on the enabled iroh tls feature). The rest is Tor-specific.
        Minimal
            .apply(builder)
            .clear_ip_transports()
            .clear_relay_transports()
            .clear_address_lookup()
            .add_custom_transport(self.factory)
            .address_lookup(TorAddressLookup)
    }
}

/// Active Tor user endpoint created by [`TorCustomTransport::bind()`].
///
/// This is the actual endpoint that handles sending and receiving packets.
/// Note: The control connection (and thus the hidden service) is kept alive by
/// the `Arc<TorCustomTransport>` that the user holds. The user must keep it alive
/// for the lifetime of the endpoint.
struct TorCustomEndpoint {
    local_id: EndpointId,
    watchable: Watchable<Vec<CustomAddr>>,
    receiver: tokio::sync::mpsc::Receiver<TorPacket>,
    sender: Arc<TorPacketSender>,
}

impl std::fmt::Debug for TorCustomEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TorCustomEndpoint")
            .field("local_id", &self.local_id)
            .finish()
    }
}

struct TorCustomSender {
    local_id: EndpointId,
    sender: Arc<TorPacketSender>,
}

impl std::fmt::Debug for TorCustomSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TorCustomSender")
            .field("local_id", &self.local_id)
            .finish()
    }
}

impl CustomSender for TorCustomSender {
    fn is_valid_send_addr(&self, addr: &CustomAddr) -> bool {
        addr.id() == TOR_USER_TRANSPORT_ID && addr.data().len() == 32
    }

    fn poll_send(
        &self,
        _cx: &mut std::task::Context,
        dst: &CustomAddr,
        _src: Option<&CustomAddr>,
        transmit: &Transmit<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        let to = parse_user_addr(dst).map_err(io::Error::other)?;
        let segment_size = transmit
            .segment_size
            .map(|size| u16::try_from(size).map_err(|_| io::Error::other("segment size too large")))
            .transpose()?;
        let chunk_size = segment_size
            .map(|s| s as usize)
            .unwrap_or(transmit.contents.len().max(1));

        for chunk in transmit.contents.chunks(chunk_size) {
            let packet = TorPacket {
                from: self.local_id,
                data: Bytes::copy_from_slice(chunk),
                segment_size,
            };
            let sender = self.sender.clone();
            tokio::spawn(async move {
                // tetron-local patch (PATCH.md, TOR-DIAL-001 follow-up):
                // this send is fire-and-forget by design (`poll_send`
                // always reports success immediately, matching the
                // `CustomSender` contract's non-blocking requirement) --
                // but a failure was previously swallowed completely
                // silently, with no way to ever learn a QUIC PATH_CHALLENGE
                // (or any other packet) never actually left this node.
                if let Err(err) = sender.send(to, &packet).await {
                    tracing::warn!(peer = %to.fmt_short(), %err, "Tor packet send failed");
                }
            });
        }

        std::task::Poll::Ready(Ok(()))
    }
}

impl CustomEndpoint for TorCustomEndpoint {
    fn watch_local_addrs(&self) -> n0_watcher::Direct<Vec<CustomAddr>> {
        self.watchable.watch()
    }

    fn create_sender(&self) -> Arc<dyn CustomSender> {
        Arc::new(TorCustomSender {
            local_id: self.local_id,
            sender: self.sender.clone(),
        })
    }

    fn max_transmit_segments(&self) -> NonZeroUsize {
        NonZeroUsize::new(32).unwrap()
    }

    fn poll_recv(
        &mut self,
        cx: &mut std::task::Context,
        bufs: &mut [io::IoSliceMut<'_>],
        metas: &mut [noq_udp::RecvMeta],
        recv_infos: &mut [RecvInfo],
    ) -> std::task::Poll<io::Result<usize>> {
        let n = bufs.len().min(metas.len()).min(recv_infos.len());
        if n == 0 {
            return std::task::Poll::Ready(Ok(0));
        }

        let mut filled = 0usize;
        while filled < n {
            match self.receiver.poll_recv(cx) {
                std::task::Poll::Pending => {
                    if filled == 0 {
                        return std::task::Poll::Pending;
                    }
                    break;
                }
                std::task::Poll::Ready(None) => {
                    return std::task::Poll::Ready(Err(io::Error::other("packet channel closed")));
                }
                std::task::Poll::Ready(Some(packet)) => {
                    if bufs[filled].len() < packet.data.len() {
                        continue;
                    }
                    bufs[filled][..packet.data.len()].copy_from_slice(&packet.data);
                    metas[filled].len = packet.data.len();
                    metas[filled].stride = packet
                        .segment_size
                        .map(|s| s as usize)
                        .unwrap_or(packet.data.len());
                    recv_infos[filled] = RecvInfo::new(tor_user_addr(packet.from), None);
                    filled += 1;
                }
            }
        }

        if filled > 0 {
            std::task::Poll::Ready(Ok(filled))
        } else {
            std::task::Poll::Pending
        }
    }
}

#[cfg(test)]
mod tests;
