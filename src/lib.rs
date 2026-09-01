#![forbid(unsafe_code)]

pub mod cli;
pub mod client;
pub mod config;
pub mod session;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
