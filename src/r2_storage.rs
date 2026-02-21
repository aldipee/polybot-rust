use crate::env_utils::env_bool;
use crate::logging::LogLike;
use anyhow::{anyhow, Context, Result};
use aws_config::BehaviorVersion;
use aws_config::meta::region::RegionProviderChain;
use aws_credential_types::provider::SharedCredentialsProvider;
use aws_credential_types::Credentials;
use aws_sdk_s3::error::DisplayErrorContext;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::{config::Region, Client};
use chrono::Utc;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::runtime::Builder as TokioRuntimeBuilder;

#[derive(Debug, Clone)]
struct R2UploadConfig {
    endpoint: String,
    bucket: String,
    access_key_id: String,
    secret_access_key: String,
    region: String,
    prefix: String,
    include_exec_latency: bool,
    log_dir: String,
    exec_latency_log_dir: String,
}

#[derive(Debug, Clone)]
struct UploadItem {
    local_path: PathBuf,
    key_suffix: String,
}

#[derive(Debug, Default)]
struct UploadStats {
    uploaded_files: usize,
    uploaded_bytes: u64,
    skipped_files: usize,
    failed_files: usize,
    failed_reasons: Vec<String>,
}

impl R2UploadConfig {
    fn from_env() -> Result<Self> {
        let endpoint = required_env("R2_ENDPOINT")?;
        let bucket = required_env("R2_BUCKET")?;
        let access_key_id = required_env("R2_ACCESS_KEY_ID")?;
        let secret_access_key = required_env("R2_SECRET_ACCESS_KEY")?;
        let region = env::var("R2_REGION")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "auto".to_string());
        let prefix = env::var("R2_UPLOAD_PREFIX")
            .ok()
            .map(|v| v.trim().trim_matches('/').to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "polybot-logs".to_string());
        let include_exec_latency = env_bool("R2_UPLOAD_INCLUDE_EXEC_LATENCY", true);
        let log_dir = env::var("LOG_DIR")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "output".to_string());
        let exec_latency_log_dir = env::var("EXEC_LATENCY_LOG_DIR")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "logs".to_string());

        Ok(Self {
            endpoint: endpoint.trim_end_matches('/').to_string(),
            bucket,
            access_key_id,
            secret_access_key,
            region,
            prefix,
            include_exec_latency,
            log_dir,
            exec_latency_log_dir,
        })
    }
}

fn required_env(name: &str) -> Result<String> {
    let value = env::var(name).unwrap_or_default();
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("Missing required env var {name}"));
    }
    Ok(trimmed.to_string())
}

fn collect_files_recursive(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir).with_context(|| format!("read_dir {}", dir.display()))? {
        let entry = entry.with_context(|| format!("read_dir entry {}", dir.display()))?;
        let path = entry.path();
        let ft = entry
            .file_type()
            .with_context(|| format!("file_type {}", path.display()))?;
        if ft.is_dir() {
            collect_files_recursive(&path, out)?;
            continue;
        }
        if ft.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

fn rel_path_to_key(rel: &Path) -> String {
    rel.to_string_lossy().replace('\\', "/")
}

fn build_upload_items(cfg: &R2UploadConfig, market_slug: &str) -> Result<Vec<UploadItem>> {
    let mut items = Vec::new();

    let market_dir = PathBuf::from(&cfg.log_dir).join(market_slug);
    let mut market_files = Vec::new();
    collect_files_recursive(&market_dir, &mut market_files)?;
    for local_path in market_files {
        let key_suffix = {
            let rel = local_path
                .strip_prefix(&market_dir)
                .unwrap_or(local_path.as_path());
            format!("market/{}", rel_path_to_key(rel))
        };
        items.push(UploadItem {
            local_path,
            key_suffix,
        });
    }

    if cfg.include_exec_latency {
        let exec_dir = PathBuf::from(&cfg.exec_latency_log_dir);
        let mut exec_files = Vec::new();
        collect_files_recursive(&exec_dir, &mut exec_files)?;
        for local_path in exec_files {
            let key_suffix = {
                let rel = local_path
                    .strip_prefix(&exec_dir)
                    .unwrap_or(local_path.as_path());
                format!("exec_latency/{}", rel_path_to_key(rel))
            };
            items.push(UploadItem {
                local_path,
                key_suffix,
            });
        }
    }

    Ok(items)
}

async fn upload_items_async(
    cfg: R2UploadConfig,
    market_slug: &str,
    bot_id: &str,
    items: Vec<UploadItem>,
) -> Result<UploadStats> {
    let credentials = Credentials::new(
        cfg.access_key_id.clone(),
        cfg.secret_access_key.clone(),
        None,
        None,
        "r2-upload",
    );
    let region_provider = RegionProviderChain::first_try(Region::new(cfg.region.clone()));
    let shared_config = aws_config::defaults(BehaviorVersion::latest())
        .region(region_provider)
        .credentials_provider(SharedCredentialsProvider::new(credentials))
        .load()
        .await;
    let s3_config = aws_sdk_s3::config::Builder::from(&shared_config)
        .endpoint_url(cfg.endpoint.clone())
        .force_path_style(true)
        .build();
    let client = Client::from_conf(s3_config);

    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let mut stats = UploadStats::default();

    for item in items {
        let bytes = match fs::read(&item.local_path) {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                stats.skipped_files += 1;
                continue;
            }
            Err(e) => {
                stats.failed_files += 1;
                stats
                    .failed_reasons
                    .push(format!("read {}: {}", item.local_path.display(), e));
                continue;
            }
        };
        let size = bytes.len() as u64;
        let key = if cfg.prefix.is_empty() {
            format!("{}/{}/{}/{}", bot_id, market_slug, stamp, item.key_suffix)
        } else {
            format!(
                "{}/{}/{}/{}/{}",
                cfg.prefix, bot_id, market_slug, stamp, item.key_suffix
            )
        };

        match client
            .put_object()
            .bucket(&cfg.bucket)
            .key(&key)
            .body(ByteStream::from(bytes))
            .send()
            .await
        {
            Ok(_) => {
                stats.uploaded_files += 1;
                stats.uploaded_bytes += size;
            }
            Err(e) => {
                stats.failed_files += 1;
                let err_ctx = DisplayErrorContext(&e).to_string();
                stats
                    .failed_reasons
                    .push(format!(
                        "upload {} key={} bucket={} endpoint={}: {} | debug={:?}",
                        item.local_path.display(),
                        key,
                        cfg.bucket,
                        cfg.endpoint,
                        err_ctx,
                        e
                    ));
            }
        }
    }

    Ok(stats)
}

fn upload_logs_before_rollover_inner(market_slug: &str, bot_id: &str) -> Result<UploadStats> {
    let cfg = R2UploadConfig::from_env()?;
    let items = build_upload_items(&cfg, market_slug)?;
    if items.is_empty() {
        return Ok(UploadStats::default());
    }

    let rt = TokioRuntimeBuilder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime for R2 upload")?;
    rt.block_on(upload_items_async(cfg, market_slug, bot_id, items))
}

pub fn upload_logs_before_rollover(market_slug: &str, bot_id: &str, logger: &Arc<dyn LogLike>) {
    if !env_bool("R2_UPLOAD_ENABLED", false) {
        return;
    }
    match upload_logs_before_rollover_inner(market_slug, bot_id) {
        Ok(stats) => {
            if stats.uploaded_files == 0 && stats.failed_files == 0 {
                logger.info(&format!(
                    "[R2] no files found for market {market_slug}; skipped upload"
                ));
            } else {
                if stats.uploaded_files > 0 {
                    logger.info(&format!(
                        "[R2] uploaded {} files ({} bytes, {} skipped) for market {}",
                        stats.uploaded_files, stats.uploaded_bytes, stats.skipped_files, market_slug
                    ));
                }
                if stats.failed_files > 0 {
                    let preview = stats
                        .failed_reasons
                        .iter()
                        .take(3)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(" | ");
                    logger.warning(&format!(
                        "[R2] {} file uploads failed for market {}. Sample errors: {}",
                        stats.failed_files, market_slug, preview
                    ));
                }
            }
        }
        Err(e) => {
            logger.warning(&format!(
                "[R2] upload failed for market {}: {:#}. Continuing without blocking rollover.",
                market_slug, e
            ));
        }
    }
}
