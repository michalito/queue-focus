//! Shared queue protocol. Reduction is pure: identity and time are inputs.
//! Transport, receipts and persistence live outside the reducer.

mod protocol;
mod reducer;

pub use protocol::*;
pub use reducer::ServerState;
pub use uuid::Uuid as Id;

#[cfg(test)]
mod tests;
