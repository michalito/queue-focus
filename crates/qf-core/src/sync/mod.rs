//! Shared queue protocol. Reduction is pure: identity and time are inputs.
//! Transport, receipts and persistence live outside the reducer.

mod client;
mod clock;
mod persistence;
mod protocol;
mod reducer;

pub use client::*;
pub use clock::ClockSample;
pub use persistence::{
    read_private, read_private_bytes, save_private, save_private_bytes, sync_parent, DataLock,
};
pub use protocol::*;
pub use reducer::ServerState;
pub use uuid::Uuid as Id;

#[cfg(test)]
mod tests;
