//! One user's authoritative queue. All domain writes, receipts and history
//! fingerprints commit in the same FULL-synchronous SQLite transaction.
mod database;
mod http;

pub use database::{Database, Enrollment, Error};
pub use http::router;

#[cfg(test)]
mod tests;
