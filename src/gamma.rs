use crate::logging::LogLike;
use anyhow::{anyhow, Result};
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

pub const GAMMA: &str = "https://gamma-api.polymarket.com";

pub fn fetch_market_by_slug(
    slug: &str,
    logger: Option<&Arc<dyn LogLike>>,
) -> Result<Option<Value>> {
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| anyhow!(e))?;

    let resp = client
        .get(format!("{GAMMA}/markets"))
        .query(&[("slug", slug)])
        .send();

    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            if let Some(l) = logger {
                l.error(&format!("Gamma request failed for slug={slug}: {e}"));
            } else {
                eprintln!("Gamma request failed for slug={slug}: {e}");
            }
            return Ok(None);
        }
    };

    let data = match resp.json::<Value>() {
        Ok(v) => v,
        Err(e) => {
            if let Some(l) = logger {
                l.error(&format!("Gamma JSON parse failed for slug={slug}: {e}"));
            } else {
                eprintln!("Gamma JSON parse failed for slug={slug}: {e}");
            }
            return Ok(None);
        }
    };

    let arr = match data {
        Value::Array(a) => a,
        _ => Vec::new(),
    };
    if arr.is_empty() {
        if let Some(l) = logger {
            l.warning(&format!("No market yet for slug={slug}"));
        } else {
            eprintln!("No market yet for slug={slug}");
        }
        return Ok(None);
    }
    Ok(arr.first().cloned())
}

pub fn maybe_json_list(x: &Value) -> Value {
    match x {
        Value::Array(_) => x.clone(),
        Value::String(s) => serde_json::from_str::<Value>(s).unwrap_or_else(|_| x.clone()),
        _ => x.clone(),
    }
}

pub fn _maybe_json_list(x: &Value) -> Value {
    maybe_json_list(x)
}

fn norm(s: &str) -> String {
    s.trim().to_ascii_lowercase()
}

pub fn parse_tokens_and_condition(m: &Value) -> Result<(String, String, String)> {
    let condition_id = m
        .get("conditionId")
        .or_else(|| m.get("condition_id"))
        .or_else(|| m.get("conditionID"))
        .or_else(|| m.get("condition"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| anyhow!("Gamma market missing conditionId"))?;

    let clob_ids_raw = m
        .get("clobTokenIds")
        .or_else(|| m.get("clob_token_ids"))
        .or_else(|| m.get("clobTokenIDs"))
        .ok_or_else(|| anyhow!("Gamma market missing clobTokenIds"))?;
    let clob_ids_val = maybe_json_list(clob_ids_raw);
    let clob_ids = clob_ids_val
        .as_array()
        .ok_or_else(|| anyhow!("Unexpected clobTokenIds: {clob_ids_val}"))?;
    if clob_ids.len() < 2 {
        return Err(anyhow!("Unexpected clobTokenIds: {clob_ids_val}"));
    }

    let outcomes_raw = m.get("outcomes");
    let outcomes = match outcomes_raw {
        Some(Value::Array(v)) => Some(v.clone()),
        Some(Value::String(s)) => serde_json::from_str::<Value>(s)
            .ok()
            .and_then(|v| v.as_array().cloned()),
        _ => None,
    };

    let mut yes_i: Option<usize> = None;
    let mut no_i: Option<usize> = None;
    if let Some(outcomes) = outcomes {
        if outcomes.len() == clob_ids.len() {
            for (i, o) in outcomes.iter().enumerate() {
                if let Some(name) = o.as_str() {
                    let n = norm(name);
                    if n == "yes" || n == "up" {
                        yes_i = Some(i);
                    }
                    if n == "no" || n == "down" {
                        no_i = Some(i);
                    }
                }
            }
        }
    }

    let yi = yes_i.unwrap_or(0);
    let ni = no_i.unwrap_or(1);
    let yes_asset = clob_ids
        .get(yi)
        .map(|v| v.to_string().trim_matches('"').to_string())
        .ok_or_else(|| anyhow!("missing YES index"))?;
    let no_asset = clob_ids
        .get(ni)
        .map(|v| v.to_string().trim_matches('"').to_string())
        .ok_or_else(|| anyhow!("missing NO index"))?;

    Ok((yes_asset, no_asset, condition_id))
}
