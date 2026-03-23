use anyhow::{anyhow, Result};
use polybot::db::{make_engine, make_session_factory, BotRepository};
use polybot::kpi_gate::{
    default_output_dir, run_kpi_gate, KpiProfile, KpiRunRequest, PostgresKpiGateSink,
};
use std::path::PathBuf;

fn main() -> Result<()> {
    let _ = dotenvy::dotenv();

    let mut args = std::env::args().skip(1);
    let mut bot_id: Option<String> = None;
    let mut profile: Option<KpiProfile> = None;
    let mut start: Option<String> = None;
    let mut end: Option<String> = None;
    let mut output_dir = default_output_dir();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--bot-id" => {
                bot_id = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("missing value for --bot-id"))?,
                );
            }
            "--profile" => {
                let raw = args
                    .next()
                    .ok_or_else(|| anyhow!("missing value for --profile"))?;
                profile = KpiProfile::from_arg(raw.as_str());
                if profile.is_none() {
                    return Err(anyhow!("--profile must be paper or shadow"));
                }
            }
            "--start" => {
                start = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("missing value for --start"))?,
                );
            }
            "--end" => {
                end = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("missing value for --end"))?,
                );
            }
            "--output-dir" => {
                output_dir = PathBuf::from(
                    args.next()
                        .ok_or_else(|| anyhow!("missing value for --output-dir"))?,
                );
            }
            other => return Err(anyhow!("unrecognized argument {}", other)),
        }
    }

    let request = KpiRunRequest {
        bot_id: bot_id.ok_or_else(|| {
            anyhow!(
                "usage: cargo run --bin kpi_gate -- --bot-id <id> --profile <paper|shadow> --start <iso> --end <iso> [--output-dir <path>]"
            )
        })?,
        profile: profile.ok_or_else(|| anyhow!("--profile is required"))?,
        window_start: start.ok_or_else(|| anyhow!("--start is required"))?,
        window_end: end.ok_or_else(|| anyhow!("--end is required"))?,
    };

    let db_url = std::env::var("DB_URL")
        .map_err(|_| anyhow!("DB_URL is required for kpi_gate persistence"))?;
    let engine = make_engine(&db_url);
    BotRepository::init_schema(&engine)?;
    let repo = make_session_factory(engine).repository();
    let mut sink = PostgresKpiGateSink::new(repo.clone());
    let (report, summary_path) = run_kpi_gate(&repo, &mut sink, &request, &output_dir)?;

    println!("summary_path={}", summary_path.display());
    println!("overall_status={}", report.overall_status);
    println!(
        "distinct_trading_days={}",
        report.sample_coverage.distinct_trading_days
    );
    println!("settled_pairs={}", report.sample_coverage.settled_pairs);
    Ok(())
}
