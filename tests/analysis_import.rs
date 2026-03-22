use anyhow::{Context, Result};
use native_tls::TlsConnector;
use polybot::analysis_import::{
    build_analysis_import_result, run_analysis_import, NoopAnalysisImportSink,
};
use polybot::db::{make_engine, BotRepository};
use postgres::Client;
use postgres_native_tls::MakeTlsConnector;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

fn dataset_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("dataset")
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("analysis_import")
        .join("expected_summary.json")
}

fn temp_output_dir() -> PathBuf {
    std::env::temp_dir().join(format!("polybot-analysis-import-{}", Uuid::new_v4()))
}

#[test]
fn analysis_import_summary_matches_expected_fixture() -> Result<()> {
    let output_dir = temp_output_dir();
    let mut sink = NoopAnalysisImportSink;
    let (result, summary_path) = run_analysis_import(&dataset_dir(), &output_dir, &mut sink)?;
    let actual = fs::read_to_string(&summary_path)
        .with_context(|| format!("failed reading {}", summary_path.display()))?;
    let expected = fs::read_to_string(fixture_path())
        .context("failed reading expected analysis summary fixture")?;
    assert_eq!(actual.trim(), expected.trim());
    assert_eq!(result.summary.counts.parquet_rows, 29342);
    assert_eq!(result.summary.counts.close_rows, 209);
    assert_eq!(result.summary.counts.filtered_market_count, 105);
    assert_eq!(result.summary.counts.closed_position_pair_count, 105);
    assert_eq!(result.summary.counts.two_sided_pairs, 104);
    fs::remove_dir_all(output_dir).ok();
    Ok(())
}

#[test]
#[ignore]
fn analysis_import_postgres_smoke_persists_rows() -> Result<()> {
    let db_url =
        std::env::var("DB_URL").context("DB_URL is required for ignored postgres smoke test")?;
    let engine = make_engine(&db_url);
    BotRepository::init_schema(&engine)?;
    let result = build_analysis_import_result(&dataset_dir())?;
    let repo = polybot::db::make_session_factory(engine.clone()).repository();
    repo.persist_analysis_import(&result)?;

    let tls = TlsConnector::builder()
        .build()
        .context("failed creating postgres TLS connector")?;
    let tls = MakeTlsConnector::new(tls);
    let mut conn = Client::connect(&db_url, tls).context("failed opening postgres db")?;

    let trade_count: i64 = conn
        .query_one(
            "SELECT COUNT(*) FROM analysis_trade_row WHERE import_run_id = $1",
            &[&result.source.import_run_id],
        )?
        .get(0);
    let close_count: i64 = conn
        .query_one(
            "SELECT COUNT(*) FROM analysis_close_position_row WHERE import_run_id = $1",
            &[&result.source.import_run_id],
        )?
        .get(0);
    let rollup_count: i64 = conn
        .query_one(
            "SELECT COUNT(*) FROM analysis_pair_rollup WHERE import_run_id = $1",
            &[&result.source.import_run_id],
        )?
        .get(0);

    assert_eq!(trade_count, 29342);
    assert_eq!(close_count, 209);
    assert_eq!(rollup_count, 105);
    Ok(())
}
