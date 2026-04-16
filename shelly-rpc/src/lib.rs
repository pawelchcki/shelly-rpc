//! `shelly-rpc` — a `no_std`-first async client library for Shelly Gen2+
//! smart devices.
//!
//! The crate depends on [`reqwless`] for HTTP and [`embedded_nal_async`] for
//! the networking abstraction. Users bring their own `TcpConnect + Dns`
//! implementation — Embassy on embedded, or a thin tokio/async-std wrapper
//! on the host side.
//!
//! # Quick start (host)
//!
//! ```ignore
//! let stack = /* your TcpConnect + Dns impl */;
//! let mut device = Device::new(&stack, &stack, "http://192.168.1.50")?;
//! let mut buf = [0u8; 4096];
//! let info = device.device_info(&mut buf).await?;
//! println!("{}: {}", info.id, info.app);
//! ```

#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(rust_2018_idioms)]

pub mod cloud;
pub mod device;
pub mod error;
pub mod rpc;

pub(crate) mod util;

pub use crate::device::Device;
pub use crate::error::Error;
