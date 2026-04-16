//! Error type shared across the crate.

use core::fmt;

/// All errors surfaced by `shelly-rpc`.
///
/// The variants are intentionally coarse-grained so the type stays usable on
/// `no_std` targets where attaching a boxed source would require `alloc`.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The underlying network transport failed.
    Transport,
    /// The device returned a response that could not be parsed.
    Parse,
    /// A stack-allocated buffer was too small.
    BufferTooSmall,
    /// The device returned a non-2xx HTTP status code.
    Http(u16),
    /// The device replied with a JSON-RPC error envelope. The wrapped value
    /// is the Gen2+ RPC error code (e.g. `-32601` for `Method not found`).
    Rpc(i32),
    /// The Shelly Cloud API returned `"isok": false`.
    CloudApi,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Transport => f.write_str("transport error"),
            Error::Parse => f.write_str("failed to parse device response"),
            Error::BufferTooSmall => f.write_str("buffer too small"),
            Error::Http(code) => write!(f, "http error: status {code}"),
            Error::Rpc(code) => write!(f, "rpc error: code {code}"),
            Error::CloudApi => f.write_str("cloud API returned isok: false"),
        }
    }
}
