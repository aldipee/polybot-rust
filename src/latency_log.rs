use csv::WriterBuilder;
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

fn now_ts() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[derive(Debug)]
pub struct JsonlFileService {
    path: PathBuf,
    enabled: bool,
    lock: Mutex<()>,
}

impl JsonlFileService {
    pub fn new(path: impl Into<String>, enabled: bool) -> Self {
        let raw = path.into();
        let path = PathBuf::from(raw.trim());
        let enabled = enabled && !path.as_os_str().is_empty();
        if enabled {
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
        }
        Self {
            path,
            enabled,
            lock: Mutex::new(()),
        }
    }

    pub fn append(&self, obj: &Value) {
        if !self.enabled {
            return;
        }
        let line = serde_json::to_string(obj)
            .unwrap_or_else(|_| serde_json::json!({"_non_json": format!("{obj:?}")}).to_string());
        let _guard = self.lock.lock().ok();
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = writeln!(file, "{line}");
        }
    }
}

#[derive(Debug)]
struct CsvFileService {
    path: PathBuf,
    enabled: bool,
    fieldnames: Vec<String>,
    lock: Mutex<()>,
    header_written: Mutex<bool>,
}

impl CsvFileService {
    fn new(path: impl Into<String>, fieldnames: Vec<String>, enabled: bool) -> Self {
        let raw = path.into();
        let path = PathBuf::from(raw.trim());
        let enabled = enabled && !path.as_os_str().is_empty();
        if enabled {
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
        }
        let header_written = if enabled && path.exists() {
            fs::metadata(&path).map(|m| m.len() > 0).unwrap_or(false)
        } else {
            false
        };
        let mut svc = Self {
            path,
            enabled,
            fieldnames,
            lock: Mutex::new(()),
            header_written: Mutex::new(header_written),
        };
        svc.ensure_header_schema_match();
        svc
    }

    fn ensure_header_schema_match(&mut self) {
        if !self.enabled || !self.path.exists() {
            return;
        }
        let file = match fs::File::open(&self.path) {
            Ok(file) => file,
            Err(_) => return,
        };
        let mut reader = BufReader::new(file);
        let mut first = String::new();
        if reader.read_line(&mut first).is_err() {
            return;
        }
        let existing: Vec<String> = first
            .trim_end()
            .split(',')
            .map(|s| s.trim().to_string())
            .collect();
        if !existing.is_empty() && existing != self.fieldnames {
            let stem = self
                .path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("latency")
                .to_string();
            let ext = self
                .path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("csv")
                .to_string();
            let backup = self
                .path
                .with_file_name(format!("{stem}.old_{:.0}.{ext}", now_ts()));
            let _ = fs::rename(&self.path, &backup);
            if let Ok(mut header_written) = self.header_written.lock() {
                *header_written = false;
            }
        }
    }

    fn append_row(&self, row: &Value) {
        if !self.enabled {
            return;
        }
        let _guard = self.lock.lock().ok();
        let mut as_map = serde_json::Map::<String, Value>::new();
        if let Value::Object(obj) = row {
            for (key, value) in obj {
                as_map.insert(key.clone(), value.clone());
            }
        } else {
            as_map.insert("value".to_string(), Value::String(format!("{row:?}")));
        }
        let mut writer = match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .ok()
            .map(|f| WriterBuilder::new().has_headers(false).from_writer(f))
        {
            Some(writer) => writer,
            None => return,
        };
        if let Ok(mut header_written) = self.header_written.lock() {
            if !*header_written {
                let _ = writer.write_record(self.fieldnames.iter());
                *header_written = true;
            }
        }
        let record: Vec<String> = self
            .fieldnames
            .iter()
            .map(|key| {
                let value = as_map
                    .get(key)
                    .cloned()
                    .unwrap_or(Value::String(String::new()));
                match value {
                    Value::Null => String::new(),
                    Value::Bool(v) => v.to_string(),
                    Value::Number(v) => v.to_string(),
                    Value::String(v) => v,
                    other => serde_json::to_string(&other).unwrap_or_default(),
                }
            })
            .collect();
        let _ = writer.write_record(record);
        let _ = writer.flush();
    }
}

#[derive(Debug)]
pub struct LatencyLogService {
    enabled: bool,
    jsonl: Option<Arc<JsonlFileService>>,
    csv: Option<Arc<CsvFileService>>,
}

impl LatencyLogService {
    pub fn default_csv_fields() -> Vec<String> {
        vec![
            "ts_utc",
            "event",
            "exec_mode",
            "market_slug",
            "order_id",
            "asset_id",
            "side",
            "origin",
            "source",
            "price",
            "qty",
            "decision_ts",
            "post_start_ts",
            "post_end_ts",
            "order_submit_ts",
            "fill_ts",
            "prep_us",
            "prep_ms",
            "sign_us",
            "sign_ms",
            "sign_total_us",
            "sign_total_ms",
            "decision_to_post_start_us",
            "decision_to_post_start_ms",
            "post_start_to_post_end_us",
            "post_start_to_post_end_ms",
            "decision_to_post_end_us",
            "decision_to_post_end_ms",
            "submit_to_fill_ms",
            "meta_json",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    }

    pub fn new(
        jsonl_path: impl Into<String>,
        csv_path: impl Into<String>,
        enabled: bool,
        jsonl_enabled: bool,
        csv_enabled: bool,
        csv_fields: Option<Vec<String>>,
    ) -> Self {
        let enabled_flag = enabled;
        let jsonl = if enabled_flag && jsonl_enabled {
            Some(Arc::new(JsonlFileService::new(jsonl_path, true)))
        } else {
            None
        };
        let csv = if enabled_flag && csv_enabled {
            Some(Arc::new(CsvFileService::new(
                csv_path,
                csv_fields.unwrap_or_else(Self::default_csv_fields),
                true,
            )))
        } else {
            None
        };
        Self {
            enabled: enabled_flag,
            jsonl,
            csv,
        }
    }

    pub fn append(&self, obj: &Value) {
        if !self.enabled {
            return;
        }
        if let Some(jsonl) = &self.jsonl {
            jsonl.append(obj);
        }
        if let Some(csv) = &self.csv {
            let mut row = obj.clone();
            if let Value::Object(ref mut map) = row {
                if !map.contains_key("meta_json") {
                    map.insert(
                        "meta_json".to_string(),
                        Value::String(serde_json::to_string(obj).unwrap_or_default()),
                    );
                }
            }
            csv.append_row(&row);
        }
    }
}
