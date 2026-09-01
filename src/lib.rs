#![forbid(unsafe_code)]

pub mod cli;
pub mod client;
pub mod config;
pub mod crypto;
pub mod journal;
pub mod market;
pub mod orders;
pub mod realtime;
pub mod session;
pub mod topic;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
