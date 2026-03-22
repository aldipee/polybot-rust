#![recursion_limit = "256"]

extern crate self as polybot;

pub mod analysis_import;
pub mod bot;
pub mod config;
pub mod db;
pub mod env_contract;
pub mod env_utils;
pub mod gamma;
pub mod helpers;
pub mod latency_log;
pub mod logging;
pub mod r2_storage;
pub mod replay;
pub mod rtds;

#[cfg(test)]
pub(crate) fn test_env_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}
