use anyhow::{anyhow, Result};
use polybot::replay;
use std::path::Path;

fn install_rustls_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

fn main() -> Result<()> {
    install_rustls_crypto_provider();
    let mut args = std::env::args().skip(1);
    let Some(root_dir) = args.next() else {
        return Err(anyhow!("usage: cargo run --bin replay -- <scenario_dir>"));
    };
    if args.next().is_some() {
        return Err(anyhow!("usage: cargo run --bin replay -- <scenario_dir>"));
    }
    replay::run_replay_scenario(Path::new(root_dir.as_str()))
}
