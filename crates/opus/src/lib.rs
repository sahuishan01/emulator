//! Original Opus decoder (RFC 6716 with the RFC 8251 updates).
//!
//! Written for the emulator's `hwopus` service. The structure follows the
//! RFC and takes performance ideas from the reference implementation
//! (libopus): a 32-bit range decoder, an FFT-based inverse MDCT, table-driven
//! PVQ enumeration and the fixed-point SILK synthesis. No code is linked
//! from or shared with any other decoder.

#![allow(clippy::needless_range_loop, clippy::too_many_arguments)]

pub mod celt;
mod decoder;
mod multistream;
mod silk;

pub use decoder::Decoder;
pub use multistream::MultiStreamDecoder;
pub mod packet;
pub mod range;

/// Decoder errors. Values mirror the libopus error codes in meaning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    BadArgument,
    BufferTooSmall,
    InternalError,
    InvalidPacket,
}
