use chrono::Local;
use serde_json::json;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub trait LogLike: Send + Sync {
    fn info(&self, msg: &str);
    fn warning(&self, msg: &str);
    fn error(&self, msg: &str);
}

#[derive(Debug)]
pub struct ItemLogger {
    item_id: String,
    item_dir: PathBuf,
    write_lock: Mutex<()>,
}

impl ItemLogger {
    pub fn new(item_id: impl Into<String>) -> Self {
        let item_id = item_id.into();
        let log_dir = env::var("LOG_DIR").unwrap_or_else(|_| "output".to_string());
        let item_dir = PathBuf::from(log_dir).join(&item_id);
        let _ = fs::create_dir_all(&item_dir);
        Self {
            item_id,
            item_dir,
            write_lock: Mutex::new(()),
        }
    }

    fn write_text_log(&self, level: &str, msg: &str) {
        let ts = Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string();
        let line = format!("{ts}|{level}| {msg}\n");
        let path = self.item_dir.join("app.log");
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = f.write_all(line.as_bytes());
        }
    }

    fn write_json_log(&self, level: &str, msg: &str) {
        let rec = json!({
            "time": {
                "repr": Local::now().to_rfc3339(),
                "timestamp": Local::now().timestamp_millis() as f64 / 1000.0
            },
            "level": {"name": level},
            "message": msg,
            "extra": {
                "item_id": self.item_id,
                "item_dir": self.item_dir.to_string_lossy().to_string()
            }
        });
        let line = format!(
            "{}\n",
            serde_json::to_string(&rec).unwrap_or_else(|_| "{}".to_string())
        );
        let path = self.item_dir.join("app.json");
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = f.write_all(line.as_bytes());
        }
    }

    fn emit(&self, level: &str, msg: &str) {
        let stderr_line = format!(
            "{}|{}| {}",
            Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
            level,
            msg
        );
        eprintln!("{stderr_line}");
        if self.write_lock.lock().is_ok() {
            self.write_text_log(level, msg);
            self.write_json_log(level, msg);
        }
    }
}

impl LogLike for ItemLogger {
    fn info(&self, msg: &str) {
        self.emit("INFO", msg);
    }

    fn warning(&self, msg: &str) {
        self.emit("WARNING", msg);
    }

    fn error(&self, msg: &str) {
        self.emit("ERROR", msg);
    }
}

pub fn setup_item_logger(name: &str) -> Arc<dyn LogLike> {
    Arc::new(ItemLogger::new(name.to_string()))
}
