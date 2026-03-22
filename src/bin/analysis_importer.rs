use anyhow::{anyhow, Result};
use polybot::analysis_import::{self, PostgresAnalysisImportSink};
use polybot::db::{make_engine, make_session_factory, BotRepository};
use std::path::{Path, PathBuf};

fn main() -> Result<()> {
    let _ = dotenvy::dotenv();

    let mut args = std::env::args().skip(1);
    let Some(dataset_dir) = args.next() else {
        return Err(anyhow!(
            "usage: cargo run --bin analysis_importer -- <dataset_dir> [--output-dir <path>]"
        ));
    };

    let mut output_dir = analysis_import::default_output_dir();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--output-dir" => {
                let Some(path) = args.next() else {
                    return Err(anyhow!("missing value for --output-dir"));
                };
                output_dir = PathBuf::from(path);
            }
            other => {
                return Err(anyhow!("unrecognized argument {}", other));
            }
        }
    }

    let db_url = std::env::var("DB_URL")
        .map_err(|_| anyhow!("DB_URL is required for analysis_importer persistence"))?;
    let engine = make_engine(&db_url);
    BotRepository::init_schema(&engine)?;
    let repo = make_session_factory(engine).repository();
    let mut sink = PostgresAnalysisImportSink::new(repo);
    let (result, summary_path) = analysis_import::run_analysis_import(
        Path::new(dataset_dir.as_str()),
        &output_dir,
        &mut sink,
    )?;
    println!("summary_path={}", summary_path.display());
    println!(
        "filtered_market_count={}",
        result.summary.counts.filtered_market_count
    );
    println!(
        "closed_position_pair_count={}",
        result.summary.counts.closed_position_pair_count
    );
    println!(
        "two_sided_participation_rate={}",
        result.summary.metrics.two_sided_participation_rate
    );
    println!("taker_share={}", result.summary.metrics.taker_share);
    println!(
        "weighted_pair_sum_median={}",
        result.summary.metrics.weighted_pair_sum_median
    );
    Ok(())
}
