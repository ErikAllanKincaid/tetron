/// Error type of `tokio-socks`
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// Failure caused by an IO error.
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// Failure when parsing a `String`.
    #[error("{0}")]
    ParseError(#[from] std::string::ParseError),
    /// Failure due to invalid target address. It contains the detailed error
    /// message.
    #[error("Target address is invalid: {0}")]
    InvalidTargetAddress(&'static str),
    /// Proxy server unreachable.
    #[error("Proxy server unreachable")]
    ProxyServerUnreachable,
    /// Proxy server returns an invalid version number.
    #[error("Invalid response version")]
    InvalidResponseVersion,
    /// No acceptable auth methods
    #[error("No acceptable auth methods")]
    NoAcceptableAuthMethods,
    /// Unknown auth method
    #[error("Unknown auth method")]
    UnknownAuthMethod,
    /// General SOCKS server failure
    #[error("General SOCKS server failure")]
    GeneralSocksServerFailure,
    /// Connection not allowed by ruleset
    #[error("Connection not allowed by ruleset")]
    ConnectionNotAllowedByRuleset,
    /// Network unreachable
    #[error("Network unreachable")]
    NetworkUnreachable,
    /// Host unreachable
    #[error("Host unreachable")]
    HostUnreachable,
    /// Connection refused
    #[error("Connection refused")]
    ConnectionRefused,
    /// TTL expired
    #[error("TTL expired")]
    TtlExpired,
    /// Command not supported
    #[error("Command not supported")]
    CommandNotSupported,
    /// Address type not supported
    #[error("Address type not supported")]
    AddressTypeNotSupported,
    /// Unknown error
    #[error("Unknown error")]
    UnknownError,
    /// Invalid reserved byte
    #[error("Invalid reserved byte")]
    InvalidReservedByte,
    /// Unknown address type
    #[error("Unknown address type")]
    UnknownAddressType,
    /// Invalid authentication values. It contains the detailed error message.
    #[error("Invalid auth values: {0}")]
    InvalidAuthValues(&'static str),
    /// Password auth failure
    #[error("Password auth failure, code: {0}")]
    PasswordAuthFailure(u8),

    #[error("Authorization required")]
    AuthorizationRequired,

    #[error("Request rejected because SOCKS server cannot connect to identd on the client")]
    IdentdAuthFailure,

    #[error("Request rejected because the client program and identd report different user-ids")]
    InvalidUserIdAuthFailure,

    /// tetron-local patch (PATCH.md, TOR-DIAL-001 follow-up): a SOCKS5
    /// CONNECT reply byte outside the standard RFC 1928 0x00-0x08 range.
    /// Tor's own SOCKS implementation uses 0xF0-0xF7 (see
    /// <https://spec.torproject.org/socks-extensions.html>) to report
    /// hidden-service-specific failure reasons (descriptor not found,
    /// introduction failed, rendezvous failed, etc.) when a client
    /// requests `ExtendedErrors` on its `SocksPort` -- distinct, specific
    /// information that the upstream `_ => Err(Error::UnknownAuthMethod)`
    /// catch-all this replaces silently discarded (and mislabeled, since
    /// that variant's own name refers to auth negotiation, not a CONNECT
    /// reply). The raw byte is preserved so a caller can tell these apart
    /// without needing every possible extended code named here.
    #[error("Extended (non-standard) SOCKS5 reply code: {0:#04x}")]
    ExtendedError(u8),
}

///// Result type of `tokio-socks`
// pub type Result<T> = std::result::Result<T, Error>;
