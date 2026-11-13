//! Original AES-128 and storage-mode primitives used by the NCA loader.
//!
//! This prototype prioritizes portable correctness tests. The table-based AES
//! implementation is not constant-time and must not be treated as hardened
//! key-handling code.

pub mod aes128;
pub mod keys;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CryptoError {
    XtsInputMustContainWholeBlocks,
    CounterWouldWrap,
}
