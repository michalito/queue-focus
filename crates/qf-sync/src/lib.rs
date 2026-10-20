//! Blocking transport lives on an owned worker, never inside a host's engine
//! lock or UI thread. Setup uses the same transport while the apps are closed.
pub mod setup;
mod transport;
mod worker;

pub use transport::{Config, Failure, Transport, CONFIG_FILE};
pub use worker::{Worker, WorkerUpdate};

#[cfg(test)]
mod tests;
