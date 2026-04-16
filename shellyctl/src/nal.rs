//! Std-backed implementations of `embedded-nal-async` traits using tokio.

use core::net::{IpAddr, SocketAddr};
use std::io;

use embedded_nal_async::{AddrType, Dns, TcpConnect};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// A std/tokio-backed network stack that implements [`TcpConnect`] and [`Dns`].
#[derive(Clone, Copy, Default)]
pub struct StdStack;

// ── Error wrapper ──────────────────────────────────────────────────────

/// Wraps [`std::io::Error`] to satisfy [`embedded_io::Error`].
#[derive(Debug)]
pub struct IoError(io::Error);

impl core::fmt::Display for IoError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for IoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

impl embedded_io::Error for IoError {
    fn kind(&self) -> embedded_io::ErrorKind {
        self.0.kind().into()
    }
}

// ── Connection wrapper ─────────────────────────────────────────────────

/// A TCP connection wrapping a tokio [`TcpStream`].
pub struct StdConnection(TcpStream);

impl embedded_io::ErrorType for StdConnection {
    type Error = IoError;
}

impl embedded_io_async::Read for StdConnection {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        self.0.read(buf).await.map_err(IoError)
    }
}

impl embedded_io_async::Write for StdConnection {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, IoError> {
        self.0.write(buf).await.map_err(IoError)
    }

    async fn flush(&mut self) -> Result<(), IoError> {
        self.0.flush().await.map_err(IoError)
    }
}

// ── TcpConnect ─────────────────────────────────────────────────────────

impl TcpConnect for StdStack {
    type Error = IoError;
    type Connection<'a> = StdConnection;

    async fn connect(&self, remote: SocketAddr) -> Result<StdConnection, IoError> {
        let stream = TcpStream::connect(remote).await.map_err(IoError)?;
        Ok(StdConnection(stream))
    }
}

// ── Dns ────────────────────────────────────────────────────────────────

/// DNS errors.
#[derive(Debug)]
pub struct DnsError(String);

impl core::fmt::Display for DnsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Dns for StdStack {
    type Error = DnsError;

    async fn get_host_by_name(&self, host: &str, addr_type: AddrType) -> Result<IpAddr, DnsError> {
        // tokio::net::lookup_host needs "host:port" format
        let lookup = format!("{host}:0");
        let mut addrs = tokio::net::lookup_host(&lookup)
            .await
            .map_err(|e| DnsError(format!("dns lookup failed for {host}: {e}")))?;
        addrs
            .find(|a| match addr_type {
                AddrType::IPv4 => a.is_ipv4(),
                AddrType::IPv6 => a.is_ipv6(),
                AddrType::Either => true,
            })
            .map(|a| a.ip())
            .ok_or_else(|| DnsError(format!("no address found for {host}")))
    }

    async fn get_host_by_address(
        &self,
        _addr: IpAddr,
        _result: &mut [u8],
    ) -> Result<usize, DnsError> {
        Err(DnsError("reverse DNS not implemented".into()))
    }
}
