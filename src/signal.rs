use crate::logging::LogLike;
use anyhow::{anyhow, Result};
use csv::WriterBuilder;
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashSet, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tungstenite::Message;

#[path = "copy_trading.rs"]
mod copy_trading;

fn now_ts() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SignalTrade {
    pub provider: String,
    pub key: String,
    pub market_slug: String,
    pub direction: String,
    pub confidence: f64,
    pub entry_price: f64,
    pub event_timestamp: String,
    pub raw: Option<Value>,
    pub received_ts: f64,
}

impl SignalTrade {
    pub fn to_dict(&self) -> Value {
        json!({
            "provider": self.provider,
            "key": self.key,
            "market_slug": self.market_slug,
            "direction": self.direction,
            "confidence": self.confidence,
            "entry_price": self.entry_price,
            "event_timestamp": self.event_timestamp,
            "received_ts": self.received_ts,
        })
    }
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
        let line = match serde_json::to_string(obj) {
            Ok(v) => v,
            Err(_) => json!({"_non_json": format!("{obj:?}")}).to_string(),
        };
        let _guard = self.lock.lock().ok();
        if let Ok(mut f) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = writeln!(f, "{line}");
        }
    }
}

#[derive(Debug)]
pub struct CsvFileService {
    path: PathBuf,
    enabled: bool,
    fieldnames: Vec<String>,
    lock: Mutex<()>,
    header_written: Mutex<bool>,
}

impl CsvFileService {
    pub fn new(path: impl Into<String>, fieldnames: Vec<String>, enabled: bool) -> Self {
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
            Ok(f) => f,
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
            let ts = format!("{:.0}", now_ts());
            let backup = self.path.with_file_name(format!("{stem}.old_{ts}.{ext}"));
            let _ = fs::rename(&self.path, &backup);
            if let Ok(mut hdr) = self.header_written.lock() {
                *hdr = false;
            }
        }
    }

    pub fn append_row(&self, row: &Value) {
        if !self.enabled {
            return;
        }
        let _guard = self.lock.lock().ok();
        let mut as_map = serde_json::Map::<String, Value>::new();
        if let Value::Object(obj) = row {
            for (k, v) in obj {
                as_map.insert(k.clone(), v.clone());
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
            Some(w) => w,
            None => return,
        };

        if let Ok(mut hw) = self.header_written.lock() {
            if !*hw {
                let _ = writer.write_record(self.fieldnames.iter());
                *hw = true;
            }
        }

        let record: Vec<String> = self
            .fieldnames
            .iter()
            .map(|k| {
                let v = as_map
                    .get(k)
                    .cloned()
                    .unwrap_or(Value::String(String::new()));
                match v {
                    Value::Null => String::new(),
                    Value::Bool(b) => b.to_string(),
                    Value::Number(n) => n.to_string(),
                    Value::String(s) => s,
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
            "signal_key",
            "signal_direction",
            "signal_provider",
            "signal_market_slug",
            "signal_received_ts",
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
            "signal_to_decision_us",
            "signal_to_decision_ms",
            "signal_to_post_start_us",
            "signal_to_post_start_ms",
            "signal_to_post_end_us",
            "signal_to_post_end_ms",
            "signal_to_submit_us",
            "signal_to_submit_ms",
            "signal_to_fill_ms",
            "post_start_to_fill_ms",
            "decision_to_fill_ms",
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

#[derive(Debug)]
pub struct SignalInbox {
    dq: Mutex<VecDeque<SignalTrade>>,
    cv: Condvar,
    stop_event: Option<Arc<AtomicBool>>,
    maxlen: usize,
}

impl SignalInbox {
    pub fn new(stop_event: Option<Arc<AtomicBool>>, maxlen: usize) -> Self {
        Self {
            dq: Mutex::new(VecDeque::new()),
            cv: Condvar::new(),
            stop_event,
            maxlen: if maxlen == 0 { 5000 } else { maxlen },
        }
    }

    pub fn put(&self, sig: SignalTrade) {
        if let Ok(mut dq) = self.dq.lock() {
            while dq.len() >= self.maxlen {
                dq.pop_front();
            }
            dq.push_back(sig);
            self.cv.notify_all();
        }
    }

    pub fn peek(&self, timeout: Option<f64>) -> Option<SignalTrade> {
        let deadline = timeout.map(|t| Instant::now() + Duration::from_secs_f64(t.max(0.0)));
        let mut guard = self.dq.lock().ok()?;
        loop {
            if let Some(v) = guard.front() {
                return Some(v.clone());
            }
            if self
                .stop_event
                .as_ref()
                .map(|e| e.load(Ordering::SeqCst))
                .unwrap_or(false)
            {
                return None;
            }
            if let Some(dl) = deadline {
                if Instant::now() >= dl {
                    return None;
                }
                let rem = dl.saturating_duration_since(Instant::now());
                let wait = rem.min(Duration::from_millis(500));
                let (g, _) = self.cv.wait_timeout(guard, wait).ok()?;
                guard = g;
            } else {
                guard = self
                    .cv
                    .wait_timeout(guard, Duration::from_millis(500))
                    .ok()?
                    .0;
            }
        }
    }

    pub fn get(&self, timeout: Option<f64>) -> Option<SignalTrade> {
        let deadline = timeout.map(|t| Instant::now() + Duration::from_secs_f64(t.max(0.0)));
        let mut guard = self.dq.lock().ok()?;
        loop {
            if let Some(v) = guard.pop_front() {
                return Some(v);
            }
            if self
                .stop_event
                .as_ref()
                .map(|e| e.load(Ordering::SeqCst))
                .unwrap_or(false)
            {
                return None;
            }
            if let Some(dl) = deadline {
                if Instant::now() >= dl {
                    return None;
                }
                let rem = dl.saturating_duration_since(Instant::now());
                let wait = rem.min(Duration::from_millis(500));
                let (g, _) = self.cv.wait_timeout(guard, wait).ok()?;
                guard = g;
            } else {
                guard = self
                    .cv
                    .wait_timeout(guard, Duration::from_millis(500))
                    .ok()?
                    .0;
            }
        }
    }

    pub fn get_for_slug(&self, market_slug: &str, timeout: Option<f64>) -> Option<SignalTrade> {
        let target = market_slug.trim().to_string();
        if target.is_empty() {
            return self.get(timeout);
        }
        let deadline = timeout.map(|t| Instant::now() + Duration::from_secs_f64(t.max(0.0)));
        let mut guard = self.dq.lock().ok()?;
        loop {
            if let Some(pos) = guard
                .iter()
                .position(|s| s.market_slug.trim() == target.as_str())
            {
                return guard.remove(pos);
            }
            if self
                .stop_event
                .as_ref()
                .map(|e| e.load(Ordering::SeqCst))
                .unwrap_or(false)
            {
                return None;
            }
            if let Some(dl) = deadline {
                if Instant::now() >= dl {
                    return None;
                }
                let rem = dl.saturating_duration_since(Instant::now());
                let wait = rem.min(Duration::from_millis(500));
                let (g, _) = self.cv.wait_timeout(guard, wait).ok()?;
                guard = g;
            } else {
                guard = self
                    .cv
                    .wait_timeout(guard, Duration::from_millis(500))
                    .ok()?
                    .0;
            }
        }
    }

    pub fn len(&self) -> usize {
        self.dq.lock().map(|dq| dq.len()).unwrap_or(0)
    }
}

#[derive(Default)]
struct DedupState {
    seen: HashSet<String>,
    seen_order: VecDeque<String>,
}

pub struct SignalHub {
    pub ws_url: String,
    pub inbox: Arc<SignalInbox>,
    pub stop_event: Arc<AtomicBool>,
    pub file_service: Option<Arc<JsonlFileService>>,
    pub logger: Option<Arc<dyn LogLike>>,
    pub reconnect_min: f64,
    pub reconnect_max: f64,
    pub ping_interval: f64,
    pub ping_timeout: f64,
    pub tls_min: f64,
    pub insecure: bool,
    pub ws_debug: bool,
    pub log_raw: bool,
    connected: Arc<AtomicBool>,
    last_msg_ts: Arc<Mutex<f64>>,
    last_conn_ts: Arc<Mutex<f64>>,
    dedup: Arc<Mutex<DedupState>>,
    seen_max: usize,
    copy_logic: copy_trading::CopyTradingLogic,
    subscribe_payload: Option<String>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl SignalHub {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ws_url: impl Into<String>,
        inbox: Arc<SignalInbox>,
        stop_event: Arc<AtomicBool>,
        file_service: Option<Arc<JsonlFileService>>,
        logger: Option<Arc<dyn LogLike>>,
        reconnect_min: f64,
        reconnect_max: f64,
        ping_interval: f64,
        ping_timeout: f64,
        tls_min: f64,
        insecure: bool,
        ws_debug: bool,
        log_raw: bool,
    ) -> Self {
        let ws_url = ws_url.into().trim().to_string();
        let copy_logic = copy_trading::CopyTradingLogic::new(&ws_url);
        let subscribe_payload = copy_logic.subscription_payload();
        Self {
            ws_url,
            inbox,
            stop_event,
            file_service,
            logger,
            reconnect_min,
            reconnect_max,
            ping_interval,
            ping_timeout,
            tls_min,
            insecure,
            ws_debug,
            log_raw,
            connected: Arc::new(AtomicBool::new(false)),
            last_msg_ts: Arc::new(Mutex::new(0.0)),
            last_conn_ts: Arc::new(Mutex::new(0.0)),
            dedup: Arc::new(Mutex::new(DedupState::default())),
            seen_max: 10000,
            copy_logic,
            subscribe_payload,
            thread: Mutex::new(None),
        }
    }

    pub fn start(self: &Arc<Self>) {
        if let Ok(mut slot) = self.thread.lock() {
            if slot.as_ref().map(|h| !h.is_finished()).unwrap_or(false) {
                return;
            }
            let this = Arc::clone(self);
            *slot = Some(thread::spawn(move || this._run_loop()));
        }
    }

    pub fn close(&self) {
        self.stop_event.store(true, Ordering::SeqCst);
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    pub fn last_message_age_s(&self) -> f64 {
        let ts = self.last_msg_ts.lock().map(|v| *v).unwrap_or(0.0);
        if ts <= 0.0 {
            f64::INFINITY
        } else {
            (now_ts() - ts).max(0.0)
        }
    }

    fn _log(&self, msg: &str) {
        if let Some(logger) = &self.logger {
            logger.info(msg);
        } else {
            println!("{msg}");
        }
    }

    fn _log_warn(&self, msg: &str) {
        if let Some(logger) = &self.logger {
            logger.warning(msg);
        } else {
            eprintln!("{msg}");
        }
    }

    fn _log_err(&self, msg: &str) {
        if let Some(logger) = &self.logger {
            logger.error(msg);
        } else {
            eprintln!("{msg}");
        }
    }

    fn _dedup_ok(&self, key: &str) -> bool {
        let k = key.trim();
        if k.is_empty() {
            return false;
        }
        let mut st = match self.dedup.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        if st.seen.contains(k) {
            return false;
        }
        st.seen.insert(k.to_string());
        st.seen_order.push_back(k.to_string());
        while st.seen_order.len() > self.seen_max {
            if let Some(old) = st.seen_order.pop_front() {
                st.seen.remove(&old);
            }
        }
        true
    }

    fn _extract_signal(&self, msg: &Value) -> Option<SignalTrade> {
        let sig = self.copy_logic.extract_signal(msg, now_ts())?;
        let key = sig.key.trim().to_string();
        if !self._dedup_ok(&key) {
            return None;
        }
        Some(sig)
    }

    // Compatibility placeholders with Python method names.
    fn _sslopt(&self) -> Option<Value> {
        let _ = (self.tls_min, self.insecure);
        None
    }

    fn _on_open(&self) {
        self._log("[SIGNAL_WS] open");
    }

    fn _on_close(&self, code: Option<u16>, msg: Option<&str>) {
        self._log_warn(&format!(
            "[SIGNAL_WS] close code={} msg={}",
            code.map(|v| v.to_string())
                .unwrap_or_else(|| "-".to_string()),
            msg.unwrap_or("")
        ));
    }

    fn _on_error(&self, err: &str) {
        self._log_err(&format!("[SIGNAL_WS] error: {err}"));
    }

    fn _on_message(&self, message: &str) {
        if let Err(e) = self.ingest_raw_message(message) {
            self._log_warn(&format!("[SIGNAL_WS] parse error: {e}"));
        }
    }

    fn _mk_ws(&self) -> Result<(), String> {
        if self.ws_url.is_empty() {
            return Err("missing SIGNAL_WS_URL".to_string());
        }
        Ok(())
    }

    pub fn ingest_raw_message(&self, raw: &str) -> Result<Option<SignalTrade>> {
        let text = raw.trim();
        if text.is_empty() || text.eq_ignore_ascii_case("ping") || text.eq_ignore_ascii_case("pong")
        {
            return Ok(None);
        }

        let msg: Value = serde_json::from_str(text).map_err(|e| anyhow!(e))?;
        if self.log_raw {
            if let Some(fs) = &self.file_service {
                fs.append(&json!({"ts": now_ts(), "raw": msg}));
            }
        }
        if let Some(sig) = self._extract_signal(&msg) {
            if let Some(fs) = &self.file_service {
                fs.append(&json!({"ts": now_ts(), "signal": sig.to_dict()}));
            }
            self.inbox.put(sig.clone());
            return Ok(Some(sig));
        }
        Ok(None)
    }

    fn _run_loop(self: Arc<Self>) {
        if self.ws_url.is_empty() {
            self._log_err("[SIGNAL_WS] missing SIGNAL_WS_URL");
            return;
        }
        let mut backoff = self.reconnect_min.max(0.1);
        while !self.stop_event.load(Ordering::SeqCst) {
            self.connected.store(false, Ordering::SeqCst);
            if let Err(e) = self._mk_ws() {
                self._log_err(&format!("[SIGNAL_WS] mk_ws error: {e}"));
                thread::sleep(Duration::from_secs_f64(self.reconnect_min.max(0.1)));
                continue;
            }

            let conn = tungstenite::connect(&self.ws_url);
            let (mut ws, _) = match conn {
                Ok(v) => v,
                Err(e) => {
                    self._log_err(&format!("[SIGNAL_WS] connect error: {e}"));
                    let sleep_for = (backoff.min(self.reconnect_max))
                        * (0.7 + rand::thread_rng().gen_range(0.0..0.6));
                    self._log_warn(&format!("[SIGNAL_WS] reconnecting in {sleep_for:.1}s ..."));
                    thread::sleep(Duration::from_secs_f64(sleep_for.max(0.1)));
                    backoff = (backoff * 2.0).min(self.reconnect_max);
                    continue;
                }
            };

            backoff = self.reconnect_min.max(0.1);
            self.connected.store(true, Ordering::SeqCst);
            self._on_open();
            if let Ok(mut ts) = self.last_conn_ts.lock() {
                *ts = now_ts();
            }
            self._log("[SIGNAL_WS] connected");
            if let Some(subscribe_payload) = &self.subscribe_payload {
                if self.ws_debug {
                    self._log(&format!("[SIGNAL_WS] send subscribe: {subscribe_payload}"));
                }
                if let Err(e) = ws.send(Message::Text(subscribe_payload.clone().into())) {
                    self._log_err(&format!("[SIGNAL_WS] subscribe send error: {e}"));
                    let _ = ws.close(None);
                    self.connected.store(false, Ordering::SeqCst);
                    let sleep_for = self.reconnect_min.max(0.1);
                    thread::sleep(Duration::from_secs_f64(sleep_for));
                    continue;
                }
            }

            let mut last_ping = Instant::now();
            while !self.stop_event.load(Ordering::SeqCst) {
                if self.ping_interval > 0.0
                    && last_ping.elapsed() >= Duration::from_secs_f64(self.ping_interval)
                {
                    let _ = ws.send(Message::Ping(Vec::new().into()));
                    last_ping = Instant::now();
                }

                let msg = match ws.read() {
                    Ok(m) => m,
                    Err(e) => {
                        self._on_error(&e.to_string());
                        break;
                    }
                };
                if let Ok(mut ts) = self.last_msg_ts.lock() {
                    *ts = now_ts();
                }

                match msg {
                    Message::Text(text) => {
                        let text_ref: &str = text.as_ref();
                        if text_ref.eq_ignore_ascii_case("ping") {
                            let _ = ws.send(Message::Text("pong".into()));
                            continue;
                        }
                        if text_ref.eq_ignore_ascii_case("pong") {
                            continue;
                        }
                        if self.ws_debug {
                            self._log(&format!("[SIGNAL_WS] recv: {text_ref}"));
                        }
                        self._on_message(text_ref);
                    }
                    Message::Binary(bin) => {
                        if let Ok(text) = String::from_utf8(bin.to_vec()) {
                            self._on_message(&text);
                        }
                    }
                    Message::Close(frame) => {
                        let code = frame.as_ref().map(|f| u16::from(f.code));
                        let reason = frame.as_ref().map(|f| f.reason.to_string());
                        self._on_close(code, reason.as_deref());
                        break;
                    }
                    _ => {}
                }
            }

            self.connected.store(false, Ordering::SeqCst);
            if self.stop_event.load(Ordering::SeqCst) {
                break;
            }
            let sleep_for =
                (backoff.min(self.reconnect_max)) * (0.7 + rand::thread_rng().gen_range(0.0..0.6));
            self._log_warn(&format!("[SIGNAL_WS] reconnecting in {sleep_for:.1}s ..."));
            thread::sleep(Duration::from_secs_f64(sleep_for.max(0.1)));
            backoff = (backoff * 2.0).min(self.reconnect_max);
        }
        self.connected.store(false, Ordering::SeqCst);
    }
}
