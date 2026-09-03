#![forbid(unsafe_code)]

pub mod app;
pub mod cli;
pub mod client;
pub mod config;
pub mod crypto;
pub mod journal;
pub mod market;
pub mod orders;
pub mod realtime;
pub mod repl;
pub mod session;
pub mod topic;
pub mod ui;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
