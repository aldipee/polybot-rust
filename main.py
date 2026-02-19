import logging
import os
import json
import csv
import re
import time
import math
import threading
import signal
import random
import ssl
from collections import deque
from typing import Deque
from datetime import datetime, timezone, timedelta
from logging import Logger
from zoneinfo import ZoneInfo
from dataclasses import dataclass
from decimal import Decimal, ROUND_DOWN, ROUND_UP
from typing import Dict, Optional, Tuple, Any, List
from loguru import logger

import requests
from dotenv import load_dotenv
from py_clob_client import BalanceAllowanceParams
from websocket import WebSocketApp

from py_clob_client.client import ClobClient
from py_clob_client.clob_types import OrderArgs, OpenOrderParams, OrderType, AssetType
from py_clob_client.order_builder.constants import BUY, SELL

from db.session import make_engine, make_session_factory
from db.repository import BotRepository
from db.utils import now_iso_jakarta, date_jakarta, week_start_date_jakarta, month_start_date_jakarta
from sqlalchemy.exc import OperationalError

from utils.logger import setup_item_logger

load_dotenv()

GAMMA = "https://gamma-api.polymarket.com"


# ============================================================
# Market segment defaults (15M / 1H / 4H / 1D)
# ============================================================
SEGMENT_DEFAULTS = {
    "5M": {"duration": 6 * 60, "step": 5 * 60, "stop_buffer": 60, "warmup": 1},
    "15M": {"duration": 15 * 60, "step": 15 * 60, "stop_buffer": 120, "warmup": 1},
    "1H":  {"duration": 60 * 60, "step": 60 * 60, "stop_buffer": 10 * 60, "warmup": 1},
    "4H":  {"duration": 4 * 60 * 60, "step": 4 * 60 * 60, "stop_buffer": 20 * 60, "warmup": 1},
    "1D":  {"duration": 24 * 60 * 60, "step": 24 * 60 * 60, "stop_buffer": 60 * 60, "warmup": 1},
}

def _segment(name: str) -> str:
    n = (name or "15M").strip().upper()
    if n in ("5", "5MIN", "5M"):
        return "5M"
    if n in ("15", "15MIN", "15M"):
        return "15M"
    if n in ("60", "60MIN", "1H", "H", "1HR"):
        return "1H"
    if n in ("240", "240MIN", "4H", "4HR"):
        return "4H"
    if n in ("1D", "D", "DAY", "DAILY"):
        return "1D"
    return n if n in SEGMENT_DEFAULTS else "15M"

def _iso_to_epoch(s: str) -> Optional[int]:
    if not s:
        return None
    try:
        dt = datetime.fromisoformat(s.replace("Z", "+00:00"))
        if dt.tzinfo is None:
            dt = dt.replace(tzinfo=timezone.utc)
        return int(dt.timestamp())
    except Exception:
        return None

# ============================================================
# Helpers: ET slugs (1H / 1D)
# ============================================================
_ET = ZoneInfo("America/New_York")
_MONTHS = {
    "january": 1, "february": 2, "march": 3, "april": 4, "may": 5, "june": 6,
    "july": 7, "august": 8, "september": 9, "october": 10, "november": 11, "december": 12,
}
_NUM_TO_MONTH = {v: k for k, v in _MONTHS.items()}

_RE_1H = re.compile(
    r"^(?P<prefix>.+-)(?P<month>january|february|march|april|may|june|july|august|september|october|november|december)-(?P<day>\d{1,2})-(?P<hour>\d{1,2})(?P<ampm>am|pm)-et$",
    re.IGNORECASE,
)
_RE_1D = re.compile(
    r"^(?P<prefix>.+-on-)(?P<month>january|february|march|april|may|june|july|august|september|october|november|december)-(?P<day>\d{1,2})$",
    re.IGNORECASE,
)

def _infer_year_et() -> int:
    # Use current date in ET to infer year for human-readable slugs.
    return datetime.now(tz=_ET).year

def _parse_1h_slug_et(slug: str) -> Optional[datetime]:
    m = _RE_1H.match(slug)
    if not m:
        return None
    month = _MONTHS[m.group("month").lower()]
    day = int(m.group("day"))
    hour = int(m.group("hour"))
    ampm = m.group("ampm").lower()
    if hour == 12:
        hour = 0
    if ampm == "pm":
        hour += 12
    year = _infer_year_et()
    # Interpret as local ET time
    return datetime(year, month, day, hour, 0, 0, tzinfo=_ET)

def _format_1h_slug_et(prefix: str, dt_et: datetime) -> str:
    month_name = _NUM_TO_MONTH.get(dt_et.month, "january")
    day = dt_et.day
    hour24 = dt_et.hour
    ampm = "am" if hour24 < 12 else "pm"
    hour12 = hour24 % 12
    if hour12 == 0:
        hour12 = 12
    return f"{prefix}{month_name}-{day}-{hour12}{ampm}-et"

def _parse_1d_slug_et(slug: str) -> Optional[datetime]:
    m = _RE_1D.match(slug)
    if not m:
        return None
    month = _MONTHS[m.group("month").lower()]
    day = int(m.group("day"))
    year = _infer_year_et()
    # Daily markets: treat as ET midnight marker (date only)
    return datetime(year, month, day, 0, 0, 0, tzinfo=_ET)

def _format_1d_slug_et(prefix: str, dt_et: datetime) -> str:
    month_name = _NUM_TO_MONTH.get(dt_et.month, "january")
    day = dt_et.day
    return f"{prefix}{month_name}-{day}"

def _increment_human_slug(slug: str, segment: str) -> Optional[str]:
    seg = _segment(segment)
    if seg == "1H":
        m = _RE_1H.match(slug)
        if not m:
            return None
        dt = _parse_1h_slug_et(slug)
        if not dt:
            return None
        dt2 = dt + timedelta(hours=1)
        return _format_1h_slug_et(m.group("prefix"), dt2)
    if seg == "1D":
        m = _RE_1D.match(slug)
        if not m:
            return None
        dt = _parse_1d_slug_et(slug)
        if not dt:
            return None
        dt2 = dt + timedelta(days=1)
        return _format_1d_slug_et(m.group("prefix"), dt2)
    return None

# ============================================================
# Helpers: state
# ============================================================
def load_state(state_file: str) -> Dict:
    if os.path.exists(state_file):
        with open(state_file, "r") as f:
            s = json.load(f)
            s.setdefault("q_yes", 0.0)
            s.setdefault("q_no", 0.0)
            s.setdefault("c_yes", 0.0)
            s.setdefault("c_no", 0.0)
            s.setdefault("seen_trade_keys", [])
            s.setdefault("seen_signal_keys", [])
            s.setdefault("open_orders", {})  # asset_id -> {"order_id","price","size","ts"}

            # SNIPER (directional) strategy bookkeeping (safe to ignore in other modes)
            s.setdefault("sniper_trade_count", 0)
            s.setdefault("sniper_last_entry_ts", 0.0)
            s.setdefault("sniper_last_exit_ts", 0.0)
            s.setdefault("sniper_last_side", "")

            return s
    return {
        "q_yes": 0.0,
        "q_no": 0.0,
        "c_yes": 0.0,
        "c_no": 0.0,
        "seen_trade_keys": [],
        "seen_signal_keys": [],
        "open_orders": {},

        # SNIPER (directional) strategy bookkeeping
        "sniper_trade_count": 0,
        "sniper_last_entry_ts": 0.0,
        "sniper_last_exit_ts": 0.0,
        "sniper_last_side": "",
    }
def save_state(state_file: str, state: Dict) -> None:
    # keep dedup bounded
    if len(state.get("seen_trade_keys", [])) > 5000:
        state["seen_trade_keys"] = state["seen_trade_keys"][-2000:]
    if len(state.get("seen_signal_keys", [])) > 5000:
        state["seen_signal_keys"] = state["seen_signal_keys"][-2000:]
    with open(state_file, "w") as f:
        json.dump(state, f, indent=2)


def locked_profit(state: Dict) -> float:
    q_pair = min(float(state["q_yes"]), float(state["q_no"]))
    return q_pair - (float(state["c_yes"]) + float(state["c_no"]))


def cost_per_pair(state: Dict) -> float:
    q_pair = min(float(state["q_yes"]), float(state["q_no"]))
    if q_pair <= 0:
        return float("inf")
    return (float(state["c_yes"]) + float(state["c_no"])) / q_pair


def round_down(x: float, tick: float) -> float:
    return math.floor(x / tick + 1e-12) * tick


def round_up(x: float, tick: float) -> float:
    return math.ceil(x / tick - 1e-12) * tick


def clamp(x: float, lo: float, hi: float) -> float:
    return max(lo, min(hi, x))

def _D(x) -> Decimal:
    # NEVER Decimal(x) where x is float. Always str(x) to avoid binary float artifacts.
    return Decimal(str(x))

def q_down(x: float, dp: int) -> float:
    q = Decimal("1." + ("0" * dp))
    return float(_D(x).quantize(q, rounding=ROUND_DOWN))

def q_up(x: float, dp: int) -> float:
    q = Decimal("1." + ("0" * dp))
    return float(_D(x).quantize(q, rounding=ROUND_UP))


# ============================================================
# Helpers: env parsing (shared)
# ============================================================
def env_bool(name: str, default: bool = False) -> bool:
    v = os.getenv(name, None)
    if v is None:
        return bool(default)
    return str(v).strip().lower() in ("1", "true", "yes", "y", "on")

def env_float(name: str, default: float) -> float:
    v = os.getenv(name, None)
    if v is None or str(v).strip() == "":
        return float(default)
    try:
        return float(v)
    except Exception:
        return float(default)

def env_int(name: str, default: int) -> int:
    v = os.getenv(name, None)
    if v is None or str(v).strip() == "":
        return int(default)
    try:
        return int(float(v))
    except Exception:
        return int(default)


# ============================================================
# SIGNAL SERVICE (WebSocket -> inbox + JSONL file)
# ============================================================
@dataclass
class SignalTrade:
    provider: str
    key: str
    market_slug: str
    direction: str
    confidence: float = 0.0
    entry_price: float = 0.0
    event_timestamp: str = ""
    raw: Optional[dict] = None
    received_ts: float = 0.0

    def to_dict(self) -> dict:
        return {
            "provider": self.provider,
            "key": self.key,
            "market_slug": self.market_slug,
            "direction": self.direction,
            "confidence": float(self.confidence or 0.0),
            "entry_price": float(self.entry_price or 0.0),
            "event_timestamp": self.event_timestamp,
            "received_ts": float(self.received_ts or 0.0),
        }


class JsonlFileService:
    """Simple JSONL append-only logger with a lock (safe across threads)."""

    def __init__(self, path: str, enabled: bool = True):
        self.path = str(path or "").strip()
        self.enabled = bool(enabled) and bool(self.path)
        self._lock = threading.Lock()
        if self.enabled:
            d = os.path.dirname(self.path) or "."
            os.makedirs(d, exist_ok=True)

    def append(self, obj: dict):
        if not self.enabled:
            return
        try:
            line = json.dumps(obj, ensure_ascii=False)
        except Exception:
            # As a fallback, coerce to string
            line = json.dumps({"_non_json": str(obj)}, ensure_ascii=False)
        with self._lock:
            with open(self.path, "a", encoding="utf-8") as f:
                f.write(line + "\n")





class CsvFileService:
    """Simple CSV append-only logger with a lock (safe across threads).

    This is intentionally minimal and defensive: failures must never break trading.
    """

    def __init__(self, path: str, fieldnames: List[str], enabled: bool = True):
        self.path = str(path or "").strip()
        self.enabled = bool(enabled) and bool(self.path)
        self.fieldnames = list(fieldnames or [])
        self._lock = threading.Lock()
        self._header_written = False

        if self.enabled:
            d = os.path.dirname(self.path) or "."
            os.makedirs(d, exist_ok=True)
            try:
                if os.path.exists(self.path) and os.path.getsize(self.path) > 0:
                    # If header differs from our expected schema, rotate the existing file so we don't mix columns.
                    try:
                        with open(self.path, "r", encoding="utf-8") as f:
                            first = (f.readline() or "").strip()
                        existing = [c.strip() for c in first.split(",")] if first else []
                        if existing and existing != self.fieldnames:
                            base, ext = os.path.splitext(self.path)
                            ts = datetime.now(timezone.utc).strftime("%Y%m%d_%H%M%S")
                            ext = ext or ".csv"
                            backup = f"{base}.old_{ts}{ext}"
                            try:
                                os.rename(self.path, backup)
                            except Exception:
                                # If rename fails (e.g., locked), fall back to a new versioned file.
                                self.path = f"{base}.v2_{ts}{ext}"
                            self._header_written = False
                        else:
                            self._header_written = True
                    except Exception:
                        # If header check fails, append anyway (best-effort).
                        self._header_written = True
            except Exception:
                # If we cannot stat the file, we'll attempt to write the header on first append.
                self._header_written = False

    def append_row(self, row: dict):
        if not self.enabled:
            return
        if not isinstance(row, dict):
            row = {"value": str(row)}

        # Coerce values to CSV-safe primitives.
        safe = {}
        for k in self.fieldnames:
            v = row.get(k, "")
            if v is None:
                v = ""
            elif isinstance(v, (dict, list)):
                try:
                    v = json.dumps(v, ensure_ascii=False)
                except Exception:
                    v = str(v)
            safe[k] = v

        with self._lock:
            try:
                need_header = not self._header_written
                with open(self.path, "a", encoding="utf-8", newline="") as f:
                    w = csv.DictWriter(f, fieldnames=self.fieldnames, extrasaction="ignore")
                    if need_header:
                        try:
                            w.writeheader()
                        except Exception:
                            # If header write fails, still try to write the row.
                            pass
                        self._header_written = True
                    w.writerow(safe)
            except Exception:
                # Never let telemetry break trading.
                return


class LatencyLogService:
    """Writes execution-latency records to JSONL and/or CSV (append-only)."""

    DEFAULT_CSV_FIELDS = [        "ts_utc",
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
        "sign_ms",
        "decision_to_post_start_ms",
        "post_start_to_post_end_ms",
        "decision_to_post_end_ms",
        "signal_to_decision_ms",
        "signal_to_post_start_ms",
        "signal_to_post_end_ms",
        "signal_to_submit_ms",
        "signal_to_fill_ms",
        "post_start_to_fill_ms",
        "decision_to_fill_ms",
        "submit_to_fill_ms",
        "meta_json",
]

    def __init__(
        self,
        jsonl_path: str,
        csv_path: str,
        enabled: bool = True,
        jsonl_enabled: bool = True,
        csv_enabled: bool = True,
        csv_fields: Optional[List[str]] = None,
    ):
        self.enabled = bool(enabled)
        self.jsonl_enabled = bool(jsonl_enabled) and self.enabled
        self.csv_enabled = bool(csv_enabled) and self.enabled

        self._jsonl = JsonlFileService(jsonl_path, enabled=self.jsonl_enabled) if self.jsonl_enabled else None

        fields = list(csv_fields) if isinstance(csv_fields, list) and csv_fields else list(self.DEFAULT_CSV_FIELDS)
        self._csv = CsvFileService(csv_path, fieldnames=fields, enabled=self.csv_enabled) if self.csv_enabled else None

    def append(self, obj: dict):
        if not self.enabled:
            return
        if not isinstance(obj, dict):
            obj = {"value": str(obj)}

        # JSONL: write the raw object
        try:
            if self._jsonl is not None:
                self._jsonl.append(obj)
        except Exception:
            pass

        # CSV: write a normalized row plus a full JSON copy for schema-free analysis.
        try:
            if self._csv is not None:
                row = dict(obj)
                if "meta_json" not in row:
                    try:
                        row["meta_json"] = json.dumps(obj, ensure_ascii=False)
                    except Exception:
                        row["meta_json"] = json.dumps({"_non_json": str(obj)}, ensure_ascii=False)
                self._csv.append_row(row)
        except Exception:
            pass

class SignalInbox:
    """Thread-safe signal buffer with peek/get semantics.

    We intentionally support *peek* so the bot can switch markets without consuming a signal
    (important for SIGNAL_FOLLOW_SLUG).
    """

    def __init__(self, stop_event: Optional[threading.Event] = None, maxlen: int = 5000):
        self._dq: Deque[SignalTrade] = deque()
        self._cv = threading.Condition()
        self._stop_event = stop_event
        self._maxlen = int(maxlen) if int(maxlen) > 0 else 5000

    def put(self, sig: SignalTrade) -> None:
        with self._cv:
            # keep bounded
            while len(self._dq) >= self._maxlen:
                self._dq.popleft()
            self._dq.append(sig)
            self._cv.notify_all()

    def peek(self, timeout: Optional[float] = None) -> Optional[SignalTrade]:
        """Return next signal without removing it."""
        deadline = None if timeout is None else (time.time() + float(timeout))
        with self._cv:
            while len(self._dq) == 0:
                if self._stop_event is not None and self._stop_event.is_set():
                    return None
                if deadline is not None:
                    rem = deadline - time.time()
                    if rem <= 0:
                        return None
                    self._cv.wait(timeout=min(0.5, rem))
                else:
                    self._cv.wait(timeout=0.5)
            return self._dq[0]

    def get(self, timeout: Optional[float] = None) -> Optional[SignalTrade]:
        """Pop next signal."""
        deadline = None if timeout is None else (time.time() + float(timeout))
        with self._cv:
            while len(self._dq) == 0:
                if self._stop_event is not None and self._stop_event.is_set():
                    return None
                if deadline is not None:
                    rem = deadline - time.time()
                    if rem <= 0:
                        return None
                    self._cv.wait(timeout=min(0.5, rem))
                else:
                    self._cv.wait(timeout=0.5)
            return self._dq.popleft()

    def get_for_slug(self, market_slug: str, timeout: Optional[float] = None) -> Optional[SignalTrade]:
        """Pop the next signal matching market_slug, preserving other queued signals.

        This is used when SIGNAL_FOLLOW_SLUG is disabled and the bot should only consume
        signals for its current market without dropping signals for other markets.
        """
        target = str(market_slug or "").strip()
        if not target:
            return self.get(timeout=timeout)

        deadline = None if timeout is None else (time.time() + float(timeout))
        with self._cv:
            while True:
                # Find the first matching signal in the queue.
                match_idx = None
                for i, s in enumerate(self._dq):
                    if str(getattr(s, "market_slug", "")) == target:
                        match_idx = i
                        break

                if match_idx is not None:
                    # Remove by rebuilding deque (deque has no delete-by-index).
                    newdq: Deque[SignalTrade] = deque()
                    found: Optional[SignalTrade] = None
                    for i, s in enumerate(self._dq):
                        if i == match_idx and found is None:
                            found = s
                            continue
                        newdq.append(s)
                    self._dq = newdq
                    return found

                # No match yet; wait.
                if self._stop_event is not None and self._stop_event.is_set():
                    return None
                if deadline is not None:
                    rem = deadline - time.time()
                    if rem <= 0:
                        return None
                    self._cv.wait(timeout=min(0.5, rem))
                else:
                    self._cv.wait(timeout=0.5)


    def __len__(self) -> int:
        with self._cv:
            return len(self._dq)


class SignalHub:
    """Runs a dedicated WS connection to the external signal provider.

    - Parses messages like:
        {"type":"trade","trade":{...}}
    - Deduplicates by trade.id/discord_message_id
    - Pushes normalized SignalTrade objects into SignalInbox
    - Optionally logs to a JSONL file (append-only)

    Safe-by-default:
      - reconnect w/ exponential backoff + jitter
      - ping/pong keepalive (WebSocket-level) when configured
    """

    def __init__(
        self,
        ws_url: str,
        inbox: SignalInbox,
        stop_event: threading.Event,
        file_service: Optional[JsonlFileService] = None,
        logger: Optional[Any] = None,
        reconnect_min: float = 1.0,
        reconnect_max: float = 30.0,
        ping_interval: float = 10.0,
        ping_timeout: float = 7.0,
        tls_min: float = 1.2,
        insecure: bool = False,
        ws_debug: bool = False,
        log_raw: bool = False,
    ):
        self.ws_url = str(ws_url or "").strip()
        self.inbox = inbox
        self.stop_event = stop_event
        self.file_service = file_service
        self.logger = logger
        self.reconnect_min = float(reconnect_min)
        self.reconnect_max = float(reconnect_max)
        self.ping_interval = float(ping_interval)
        self.ping_timeout = float(ping_timeout)
        self.tls_min = float(tls_min)
        self.insecure = bool(insecure)
        self.ws_debug = bool(ws_debug)
        self.log_raw = bool(log_raw)

        self._ws: Optional[WebSocketApp] = None
        self._thread: Optional[threading.Thread] = None
        self._connected = False
        self._last_msg_ts = 0.0
        self._last_conn_ts = 0.0

        # Dedup buffer
        self._seen = set()
        self._seen_order = deque()
        self._seen_max = 10000

    def start(self) -> None:
        if self._thread and self._thread.is_alive():
            return
        t = threading.Thread(target=self._run_loop, daemon=True)
        self._thread = t
        t.start()

    def close(self) -> None:
        try:
            if self._ws is not None:
                self._ws.close()
        except Exception:
            pass

    def is_connected(self) -> bool:
        return bool(self._connected)

    def last_message_age_s(self) -> float:
        if self._last_msg_ts <= 0:
            return float("inf")
        return max(0.0, time.time() - float(self._last_msg_ts))

    def _log(self, msg: str) -> None:
        try:
            if self.logger:
                self.logger.info(msg)
            else:
                print(msg)
        except Exception:
            pass

    def _log_warn(self, msg: str) -> None:
        try:
            if self.logger:
                self.logger.warning(msg)
            else:
                print(msg)
        except Exception:
            pass

    def _log_err(self, msg: str) -> None:
        try:
            if self.logger:
                self.logger.error(msg)
            else:
                print(msg)
        except Exception:
            pass

    def _sslopt(self) -> Optional[dict]:
        # Only for wss://
        if not self.ws_url.lower().startswith("wss://"):
            return None
        try:
            ctx = ssl.create_default_context()
            # Minimum TLS version
            try:
                if float(self.tls_min) >= 1.3:
                    ctx.minimum_version = ssl.TLSVersion.TLSv1_3
                elif float(self.tls_min) >= 1.2:
                    ctx.minimum_version = ssl.TLSVersion.TLSv1_2
            except Exception:
                pass

            if self.insecure:
                # NOTE: insecure TLS is not recommended for production; use only for local testing / trusted networks.
                ctx.check_hostname = False
                ctx.verify_mode = ssl.CERT_NONE

            sslopt = {"context": ctx}
            if self.insecure:
                sslopt["cert_reqs"] = ssl.CERT_NONE
                sslopt["check_hostname"] = False
            return sslopt
        except Exception:
            return None

    def _dedup_ok(self, key: str) -> bool:
        k = str(key or "").strip()
        if not k:
            return False
        if k in self._seen:
            return False
        self._seen.add(k)
        self._seen_order.append(k)
        while len(self._seen_order) > self._seen_max:
            old = self._seen_order.popleft()
            try:
                self._seen.remove(old)
            except Exception:
                pass
        return True

    def _extract_signal(self, msg: dict) -> Optional[SignalTrade]:
        mtype = str(msg.get("type") or msg.get("event") or "").strip().lower()
        if mtype != "trade":
            return None

        trade = msg.get("trade") or {}
        if not isinstance(trade, dict):
            return None

        # Only accept status SIGNAL (if present)
        status = str(trade.get("status") or "").strip().upper()
        if status and status not in ("SIGNAL", "TRADE", "OPEN"):
            return None

        market_slug = str(trade.get("market_slug") or trade.get("market") or "").strip()
        direction = str(trade.get("direction") or trade.get("side") or "").strip().upper()
        if not market_slug or not direction:
            return None

        # Confidence
        conf = 0.0
        try:
            conf = float(trade.get("confidence") or 0.0)
        except Exception:
            conf = 0.0

        # Entry price (primary)
        entry_price = 0.0
        try:
            entry_price = float(trade.get("entry_price") or 0.0)
        except Exception:
            entry_price = 0.0

        # Fallback: parse payload_json if needed
        if entry_price <= 0 and isinstance(trade.get("payload_json"), str):
            try:
                pj = json.loads(trade.get("payload_json") or "{}")
                if isinstance(pj, dict):
                    for k in ("entry_price", "market_price", "price"):
                        if k in pj and entry_price <= 0:
                            try:
                                entry_price = float(pj.get(k) or 0.0)
                            except Exception:
                                entry_price = 0.0
            except Exception:
                pass

        # Unique key for dedup
        trade_id = trade.get("id")
        discord_id = trade.get("discord_message_id") or trade.get("discordMessageId")
        key = ""
        if trade_id is not None and str(trade_id).strip() != "":
            key = f"trade_id:{trade_id}"
        elif discord_id:
            key = f"discord_id:{discord_id}"
        else:
            # last resort: stable-ish composite
            key = f"slug:{market_slug}|dir:{direction}|ts:{trade.get('event_timestamp') or trade.get('created_at') or ''}"

        if not self._dedup_ok(key):
            return None

        event_ts = str(trade.get("event_timestamp") or trade.get("created_at") or "").strip()

        return SignalTrade(
            provider="WEBSOCKET",
            key=str(key),
            market_slug=market_slug,
            direction=direction,
            confidence=float(conf),
            entry_price=float(entry_price),
            event_timestamp=event_ts,
            raw=trade if self.log_raw else None,
            received_ts=float(time.time()),
        )

    def _on_open(self, ws):
        self._connected = True
        self._last_conn_ts = time.time()
        self._log(f"[SIGNAL_WS] connected {self.ws_url}")

    def _on_close(self, ws, code, msg):
        self._connected = False
        self._log_warn(f"[SIGNAL_WS] closed: {code} {msg}")

    def _on_error(self, ws, err):
        self._connected = False
        self._log_err(f"[SIGNAL_WS] error: {err}")

    def _on_message(self, ws, message):
        self._last_msg_ts = time.time()
        try:
            if isinstance(message, (bytes, bytearray)):
                message = message.decode("utf-8", errors="ignore")
        except Exception:
            pass

        if self.ws_debug:
            try:
                s = str(message)
                self._log(f"[SIGNAL_WS][RAW] {s[:500]}")
            except Exception:
                pass

        obj = None
        try:
            obj = json.loads(message)
        except Exception:
            # ignore non-JSON messages (ping/pong/text)
            return

        if not isinstance(obj, dict):
            return

        sig = self._extract_signal(obj)
        if sig is None:
            return

        # Append normalized signal to JSONL file
        try:
            if self.file_service is not None:
                rec = {
                    "received_at": datetime.now(timezone.utc).isoformat(),
                    "provider": sig.provider,
                    "signal": sig.to_dict(),
                }
                if self.log_raw:
                    rec["raw"] = obj
                self.file_service.append(rec)
        except Exception:
            pass

        # Push to inbox
        try:
            self.inbox.put(sig)
        except Exception:
            pass

        if self.logger:
            try:
                self.logger.info(
                    f"[SIGNAL_WS] signal key={sig.key} slug={sig.market_slug} dir={sig.direction} "
                    f"conf={sig.confidence:.3f} entry={sig.entry_price:.4f}"
                )
            except Exception:
                pass

    def _mk_ws(self) -> WebSocketApp:
        return WebSocketApp(
            self.ws_url,
            on_open=self._on_open,
            on_message=self._on_message,
            on_error=self._on_error,
            on_close=self._on_close,
        )

    def _run_loop(self):
        if not self.ws_url:
            self._log_err("[SIGNAL_WS] missing SIGNAL_WS_URL")
            return

        backoff = max(0.1, float(self.reconnect_min))
        while not self.stop_event.is_set():
            self._connected = False
            self._ws = self._mk_ws()

            sslopt = self._sslopt()
            try:
                # Use websocket-client built-in ping/pong if configured (>0)
                ping_int = max(0.0, float(self.ping_interval))
                ping_to = max(0.0, float(self.ping_timeout))
                run_kwargs = {}
                if sslopt is not None:
                    run_kwargs["sslopt"] = sslopt
                if ping_int > 0:
                    run_kwargs["ping_interval"] = ping_int
                    run_kwargs["ping_timeout"] = ping_to if ping_to > 0 else None

                self._ws.run_forever(**run_kwargs)
            except Exception as e:
                self._log_err(f"[SIGNAL_WS] run_forever exception: {e}")

            if self.stop_event.is_set():
                break

            # Reconnect with jittered exponential backoff
            sleep_for = min(float(backoff), float(self.reconnect_max))
            sleep_for *= (0.7 + random.random() * 0.6)
            self._log_warn(f"[SIGNAL_WS] reconnecting in {sleep_for:.1f}s ...")
            time.sleep(max(0.1, sleep_for))
            backoff = min(float(backoff) * 2.0, float(self.reconnect_max))


# ============================================================
# Helpers: Gamma market discovery
# ============================================================
def fetch_market_by_slug(slug: str, logger: Optional[Any] = None) -> Optional[Dict]:
    try:
        r = requests.get(f"{GAMMA}/markets", params={"slug": slug}, timeout=15)
        r.raise_for_status()
        data = r.json()
    except Exception as e:
        if logger:
            logger.error(f"⚠️ Gamma request failed for slug={slug}: {e}")
        else:
            print(f"⚠️ Gamma request failed for slug={slug}: {e}")
        return None

    if not data:
        if logger:
            logger.warning(f"⚠️ No market yet for slug={slug}")
        else:
            print(f"⚠️ No market yet for slug={slug}")
        return None

    return data[0]


def get_next_slug(current_slug: str) -> str:
    """Return the next slug for the configured MARKET_SEGMENT.

    Supported:
      - Timestamp-based slugs (ending with epoch): increment by MARKET_STEP_SECONDS.
      - 1H human-readable ET slugs: e.g. bitcoin-up-or-down-january-26-9pm-et
      - 1D human-readable slugs: e.g. bitcoin-up-or-down-on-january-27

    Note:
      - For human-readable slugs, we infer the year from the current ET year.
        (Gamma startDate/endDate are used for actual trading times.)
    """
    seg = _segment(os.getenv("MARKET_SEGMENT", "15M"))

    # (A) Timestamp suffix: ...-<epoch>
    step = int(os.getenv("MARKET_STEP_SECONDS", str(SEGMENT_DEFAULTS.get(seg, SEGMENT_DEFAULTS["15M"])["step"])))
    try:
        parts = current_slug.split("-")
        ts = int(parts[-1])
        parts[-1] = str(ts + step)
        return "-".join(parts)
    except Exception:
        pass

    # (B) Human-readable ET slugs
    nxt = _increment_human_slug(current_slug, seg)
    return nxt or current_slug


def _maybe_json_list(x):
    if isinstance(x, list):
        return x
    if isinstance(x, str):
        s = x.strip()
        try:
            y = json.loads(s)
            if isinstance(y, list):
                return y
        except Exception:
            pass
    return x


def parse_tokens_and_condition(m: dict) -> Tuple[str, str, str]:
    condition_id = (
        m.get("conditionId")
        or m.get("condition_id")
        or m.get("conditionID")
        or m.get("condition")
    )
    if not condition_id:
        raise ValueError("Gamma market missing conditionId")

    clob_ids = m.get("clobTokenIds") or m.get("clob_token_ids") or m.get("clobTokenIDs")
    if not clob_ids:
        raise ValueError("Gamma market missing clobTokenIds")

    clob_ids = _maybe_json_list(clob_ids)
    if not isinstance(clob_ids, list) or len(clob_ids) < 2:
        raise ValueError(f"Unexpected clobTokenIds: {clob_ids}")

    outcomes_raw = m.get("outcomes")
    outcomes = None
    if isinstance(outcomes_raw, str):
        try:
            outcomes = json.loads(outcomes_raw)
        except Exception:
            outcomes = None
    elif isinstance(outcomes_raw, list):
        outcomes = outcomes_raw

    def norm(x):
        return str(x).strip().lower()

    yes_i, no_i = None, None
    if isinstance(outcomes, list) and len(outcomes) == len(clob_ids):
        for i, o in enumerate(outcomes):
            o2 = norm(o)
            if o2 in ("yes", "up"):
                yes_i = i
            if o2 in ("no", "down"):
                no_i = i

    if yes_i is None or no_i is None:
        yes_i, no_i = 0, 1

    return str(clob_ids[yes_i]), str(clob_ids[no_i]), str(condition_id)


# ============================================================
# Config
# ============================================================
@dataclass
class BotConfig:
    clob_host: str
    ws_base: str
    chain_id: int
    private_key: str
    signature_type: Optional[int] = None
    funder: Optional[str] = None

    # Market segment
    market_segment: str = "15M"   # 15M / 1H / 4H / 1D
    market_duration_seconds: int = 15 * 60
    market_step_seconds: int = 15 * 60

    # Market microstructure
    tick: float = 0.01
    min_shares: float = 5.0  # Polymarket min
    lock_profit_target: float = 0.5

    # Quoting / sizing
    clip_shares: float = 5.0  # we trade in 5-share clips to respect min size
    improve_bid_ticks: int = 0        # 0 = sit at best bid
    maker_buffer_ticks: int = 1       # ensure we stay maker: bid <= ask - buffer
    replace_if_price_moves_ticks: int = 3
    stale_seconds: int = 20

    # Risk controls (settlement no-loss)
    entry_edge_ticks: int = 2     # require (bid_yes + bid_no) <= 1 - entry_edge
    hedge_buffer_ticks: int = 1   # when calculating hedge cap, subtract 1 tick safety
    max_total_cost: float = 20.0
    reserve_usd: float = 2.0

    # Behavior
    cancel_all_on_start: bool = True
    dry_run: bool = False
    log_every: int = 5

    # Feed safety
    market_data_stale_seconds: int = 8
    ws_reconnect_min: float = 0.5
    ws_reconnect_max: float = 5.0

    # Rollover
    stop_buffer_seconds: int = 120  # stop before expiry


# ============================================================
# Maker+Hedge-Cap Bot (updated: warmup+stability, cross-ask safety, emergency FAK hedge)
# ============================================================
class MakerHedgeCapBot:
    """
    Updated behavior:
      1) ACCUMULATE (balanced): only quote if book is stable AND quotes are "cross-ask safe":
            yes_bid + no_ask <= 1 - edge
            no_bid  + yes_ask <= 1 - edge
         This prevents getting picked off into an unhedgeable position when the other side's ask is high.

      2) HEDGE (imbalanced): try maker-hedge up to hedge cap, but if unhedged for too long,
         force a taker hedge using FAK (fill-and-kill / IOC-like) to flatten quickly.

      3) Taker fills are now counted (previous code only counted maker fills).
    """

    def __init__(self, cfg: BotConfig, market_slug: str, bot_logger: Logger, signal_hub: Optional[SignalHub] = None):
        self.cfg = cfg
        self.logger = bot_logger
        self.market_slug = market_slug
        self.signal_hub = signal_hub
        self._owns_signal_hub = False
        self.state_file = f"maker_hedgecap_state_{market_slug}.json"
        self.state = load_state(self.state_file)
        self.state_lock = threading.Lock()
        self.start_trade_iso = now_iso_jakarta()
        self.exit_reason = "RUNNING"

        # Wallet address used for ownership checks in user trade events.
        # Prefer explicit env vars; fall back to configured funder.
        self.wallet_address = (
            os.getenv("WALLET_ADDRESS", "").strip()
            or os.getenv("POLYMARKET_WALLET_ADDRESS", "").strip()
            or os.getenv("POLYMARKET_FUNDER", "").strip()
            or str(getattr(cfg, "funder", "") or "").strip()
        )
        # Normalize to lowercase for comparisons (trade events use 0x-addresses).
        self.wallet_address = (self.wallet_address or "").strip().lower()

        self.min_maker_notional = float(os.getenv("MIN_MAKER_NOTIONAL", "1.00"))
        self.min_taker_notional = float(os.getenv("MIN_TAKER_NOTIONAL", "1.00"))

        # Balance-reconcile accounting for SELL proceeds.
        # When WS trade events are delayed/missed, we reconcile qYES/qNO from the balance API.
        # If balances decreased, that implies we SOLD (or otherwise reduced inventory). We therefore
        # need to *credit* proceeds back into c_yes/c_no. The previous hard-coded 0.5 haircut could
        # massively overstate losses and trip the circuit breaker in MAKER mode.
        #
        # Keep this configurable so you can stay conservative if desired.
        self.reconcile_sell_credit_mult = float(os.getenv("RECONCILE_SELL_CREDIT_MULT", "1.0"))
        self.reconcile_sell_credit_mult = clamp(self.reconcile_sell_credit_mult, 0.0, 1.0)

        # Optional: use a larger clip for the very first cycle to try to hit profit target quickly.
        # Only active when starting flat (qYES=qNO=0) and before the first complete set is finished.
        self.first_clip_shares = float(os.getenv("FIRST_CLIP_SHARES", str(getattr(self.cfg, "first_clip_shares", 0.0))))
        self._first_cycle_started = False
        self._first_cycle_done = False


        # Optional: if true, the first-cycle hedge will try to hedge the FULL delta (not clipped by clip_shares).
        self.first_hedge_full = os.getenv("FIRST_HEDGE_FULL", "false").lower() in ("1","true","yes","y")

        # Parse market timing (supports 15M/1H/4H/1D and non-timestamp slugs)
        # Primary source of truth: Gamma startDate/endDate when available.
        # Fallback: trailing epoch in slug + cfg.market_duration_seconds.
        self.start_ts = int(time.time())
        self.expiry_ts = int(time.time()) + int(self.cfg.market_duration_seconds)

        # Parse market start ts from slug (15m markets)
        try:
            raw_ts = int(market_slug.split("-")[-1])
            self.start_ts = raw_ts
            self.expiry_ts = raw_ts + int(self.cfg.market_duration_seconds)
        except Exception:
            pass

        # --- Runtime safety knobs (NO DB schema changes; configurable via env vars) ---
        # Warmup: avoid quoting in first seconds of market start (price discovery burst).
        self.warmup_seconds = int(os.getenv("WARMUP_SECONDS", str(SEGMENT_DEFAULTS.get(self.cfg.market_segment, SEGMENT_DEFAULTS['15M'])['warmup'])))

        # Book stability gates for ACCUMULATE mode:
        self.max_spread_ticks = int(os.getenv("MAX_SPREAD_TICKS", "6"))          # e.g. 10 ticks = $0.10
        self.parity_tolerance = float(os.getenv("PARITY_TOLERANCE", "0.025"))      # |(mid_yes+mid_no)-1| <= 0.03

        # Emergency hedge:
        self.unhedged_timeout_seconds = float(os.getenv("UNHEDGED_TIMEOUT_SECONDS", "2"))
        self.hedge_slippage_ticks = int(os.getenv("HEDGE_SLIPPAGE_TICKS", "1"))  # pay a little above ask for speed
        self.hedge_taker_order_type = os.getenv("HEDGE_TAKER_ORDER_TYPE", "FAK").upper()  # FAK recommended
        self.taker_order_ttl_seconds = int(os.getenv("TAKER_ORDER_TTL_SECONDS", "120"))
        # Fallback fill accounting: if WS trade events are missed, use user WS 'order' events (size_matched)
        self.taker_fill_fallback_from_order_events = str(os.getenv("TAKER_FILL_FALLBACK_FROM_ORDER_EVENTS", "true")).lower() in ("1", "true", "yes", "y")
        # Strict inflight gating: avoid sending another taker order of the same side/asset while one is unacknowledged
        self.taker_strict_inflight = str(os.getenv("TAKER_STRICT_INFLIGHT", "true")).lower() in ("1", "true", "yes", "y")

        # Throttle taker hedges (avoid spamming every loop)
        self._last_taker_hedge_ts = 0.0
        self._taker_hedge_min_interval = float(os.getenv("TAKER_HEDGE_MIN_INTERVAL", "1.0"))

        # Taker order failure pause (used by pair-arb + hedges)
        self._taker_fail_pause_until = 0.0

        # ============================
        # Execution mode
        # ============================
        # EXEC_MODE:
        #   - MAKER (default): existing maker+hedge-cap strategy
        #   - TAKER_PAIR: only attempt when (ask_yes + ask_no) already <= 1 - (fees + min_profit),
        #                then submit BOTH legs as taker (FOK/FAK) with retries + hard timeout
        self.exec_mode = os.getenv("EXEC_MODE", "MAKER").upper().strip()

        # Strategy selection
        # EXEC_MODE options:
        #   - MAKER (default): Maker + hedge cap (complete-set accumulation)
        #   - TAKER_PAIR: two-leg taker pair-arb (marketable BUYs on both sides)
        #   - SNIPER: internal high-prob fixed-profit directional strategy (91%-99% implied odds)
        #   - SIGNAL_SNIPPER: external signal-driven directional strategy (via WS/FILE)
        self.signal_sniper_mode = self.exec_mode in ("SIGNAL_SNIPPER", "SIGNAL_SNIPER", "SIGNAL_SNIPE", "SIGNAL")
        self.sniper_mode = self.exec_mode in ("SNIPER", "PROB_SNIPER", "HIGH_PROB", "HIGH_PROB_SNIPER", "FIXED_PROFIT")

        # Loop timing (event-driven on best_bid_ask; these are max sleep bounds)
        self.loop_wait_seconds_maker = float(os.getenv("LOOP_WAIT_SECONDS_MAKER", "1.0"))
        self.loop_wait_seconds_taker = float(os.getenv("LOOP_WAIT_SECONDS_TAKER", "0.2"))
        self.loop_wait_seconds_sniper = float(os.getenv("LOOP_WAIT_SECONDS_SNIPER", "0.05"))

        # --- SNIPER strategy knobs (directional) ---
        # This strategy looks for a near-certain side (ask between ~0.91 and 0.99), enters late in the market window,
        # and exits on a fixed take-profit (1%-9%) or (optionally) before expiry to reduce last-second reversal risk.
        self.sniper_price_min = float(os.getenv("SNIPER_PRICE_MIN", "0.91"))
        self.sniper_price_max = float(os.getenv("SNIPER_PRICE_MAX", "0.99"))

        # Profit/loss targets are on *entry notional* (PnL / cost), e.g. 0.02 = +2%.
        self.sniper_take_profit_pct = float(os.getenv("SNIPER_TAKE_PROFIT_PCT", "0.02"))
        self.sniper_stop_loss_pct = float(os.getenv("SNIPER_STOP_LOSS_PCT", "0.03"))

        # Stop-loss execution mode (configurable):
        #   - MARKET (default): current behaviour. On STOP_LOSS we send marketable taker exits (FOK→FAK) with widening slippage.
        #     Pros: highest chance to exit quickly. Cons: can realize worse-than-stop fills in fast gaps.
        #   - LIMIT: stop-limit behaviour. On STOP_LOSS we will NOT sell below the stop floor
        #     floor = entry_ref_price * (1 - SNIPER_STOP_LOSS_PCT). The bot will place/keep a GTC limit SELL at that floor.
        #     Pros: never sells below your configured stop threshold. Cons: may not fill in a fast crash (you can end up holding to resolution).
        self.sniper_stop_loss_mode = os.getenv("SNIPER_STOP_LOSS_MODE", "MARKET").upper().strip()
        # Only used when SNIPER_STOP_LOSS_MODE=LIMIT (or STOP_LIMIT). Default: GTC.
        self.sniper_stop_limit_order_type = os.getenv("SNIPER_STOP_LIMIT_ORDER_TYPE", "GTC").upper().strip()
        # In stop-limit mode, re-submit the stop-limit order if it hasn't filled after this many seconds (0 disables).
        try:
            self.sniper_stop_limit_resubmit_seconds = float(os.getenv("SNIPER_STOP_LIMIT_RESUBMIT_SECONDS", "5.0"))
        except Exception:
            self.sniper_stop_limit_resubmit_seconds = 5.0

        # For 15m markets, a good default is to trade only in the *last ~3 minutes* (≈180s) to reduce reversal risk.
        # Default uses last 20% of the market duration (override with SNIPER_ENTRY_MAX_SECONDS).
        self.sniper_entry_min_seconds = int(os.getenv("SNIPER_ENTRY_MIN_SECONDS", "30"))
        _default_entry_max = max(60, int(0.20 * float(self.cfg.market_duration_seconds)))
        self.sniper_entry_max_seconds = int(os.getenv("SNIPER_ENTRY_MAX_SECONDS", str(_default_entry_max)))
        # Optional "force entry" override:
        # If the favored side is already very high (e.g. 0.95+) *before* the normal entry window starts,
        # you can opt-in to enter anyway. This is intentionally opt-in: upside is smaller at high prices and
        # you increase time-at-risk (more room for a reversal).
        #   - Set SNIPER_FORCE_ENTRY_MIN_PRICE to enable (0.0 disables)
        #   - Optionally set SNIPER_FORCE_ENTRY_MAX_AGE_SECONDS to restrict this rule to the first N seconds after market start.
        self.sniper_force_entry_min_price = float(os.getenv("SNIPER_FORCE_ENTRY_MIN_PRICE", "0.0"))
        self.sniper_force_entry_max_age_seconds = int(os.getenv("SNIPER_FORCE_ENTRY_MAX_AGE_SECONDS", "0"))
        # Optional: if true, force-entry ignores the take-profit feasibility (ROI) gate.
        # Default false for safety.
        self.sniper_force_entry_ignore_roi_gate = os.getenv("SNIPER_FORCE_ENTRY_IGNORE_ROI_GATE", "false").lower() in ("1","true","yes","y")

        # If still in position this close to expiry, optionally force-exit to avoid settlement spikes.
        self.sniper_force_exit_seconds = int(os.getenv("SNIPER_FORCE_EXIT_SECONDS", "20"))
        self.sniper_exit_before_expiry = os.getenv("SNIPER_EXIT_BEFORE_EXPIRY", "true").lower() in ("1", "true", "yes", "y")

        self.sniper_max_trades_per_market = int(os.getenv("SNIPER_MAX_TRADES_PER_MARKET", "1"))
        self.sniper_max_notional_usd = float(
            os.getenv("SNIPER_MAX_NOTIONAL_USD", str(min(100.0, float(self.cfg.max_total_cost) * 0.25))))
        self.sniper_entry_slippage_ticks = int(os.getenv("SNIPER_ENTRY_SLIPPAGE_TICKS", "1"))
        self.sniper_exit_slippage_ticks = int(os.getenv("SNIPER_EXIT_SLIPPAGE_TICKS", "1"))

        # FOK avoids partial fills (prevents "dust" positions below min_shares).
        self.sniper_entry_order_type = os.getenv("SNIPER_ENTRY_ORDER_TYPE", "FOK").upper().strip()
        self.sniper_exit_order_type = os.getenv("SNIPER_EXIT_ORDER_TYPE", "FOK").upper().strip()

        # Entry order type:
        #   - FOK / FAK: IOC-like taker behaviour (default)
        #   - GTC: resting limit order (can be maker if price < ask)
        #   - LIMIT: alias for GTC (more intuitive name)
        # NOTE: Polymarket CLOB orders are always LIMIT orders; these values control time-in-force.
        #
        # If you set SNIPER_ENTRY_ORDER_TYPE=LIMIT (or GTC), SNIPER will allow non-marketable
        # entries (price below ask) and will place a resting order that can fill later.
        # Optional safety: require post-only so the order never crosses the spread:
        #   SNIPER_ENTRY_POST_ONLY=true
        self.sniper_entry_post_only = env_bool("SNIPER_ENTRY_POST_ONLY", False)

        # Optional: bypass the ROI feasibility gate for *normal* entries (force-entry already has its own switch).
        # This is UNSAFE unless you understand the implications (e.g., holding to resolution).
        self.sniper_entry_ignore_roi_gate = env_bool("SNIPER_ENTRY_IGNORE_ROI_GATE", False)

        # Entry execution controls (optional)
        # - Chunked entries reduce "couldn't be fully filled" failures on FOK when top-of-book depth is thin.
        #   Set SNIPER_ENTRY_CHUNK_SHARES to a positive integer (multiple of min_shares) to enable.
        # - Optional fallback order type for entries (e.g. FAK) if FOK fails.
        self.sniper_entry_order_type_fallback = os.getenv("SNIPER_ENTRY_ORDER_TYPE_FALLBACK", "").upper().strip()
        self.sniper_entry_chunk_shares = int(os.getenv("SNIPER_ENTRY_CHUNK_SHARES", "0"))
        self.sniper_entry_inflight_seconds = float(os.getenv("SNIPER_ENTRY_INFLIGHT_SECONDS", "1.5"))
        self.sniper_entry_max_orders = int(os.getenv("SNIPER_ENTRY_MAX_ORDERS", "3"))

        # Pending taker-order budgeting (prevents notional-cap breaches when multiple entry orders are in-flight).
        # For FOK/FAK orders we expect resolution quickly; this age window is used for strict notional reservation.
        self.sniper_pending_order_max_age_seconds = float(os.getenv("SNIPER_PENDING_ORDER_MAX_AGE_SECONDS", "8.0"))

        # If an entry attempt submits orders but no fills arrive, pause briefly before retrying (reduces spam on thin books).
        self.sniper_entry_retry_pause_seconds = float(os.getenv("SNIPER_ENTRY_RETRY_PAUSE_SECONDS", "2.0"))


        # If primary entry order type is FOK and we get "order couldn't be fully filled" errors,
        # optionally shrink the requested size and retry (still no partial fills).
        #   - SNIPER_ENTRY_SHRINK_FACTOR: multiplicative shrink per retry (default 0.5)
        #   - SNIPER_ENTRY_SHRINK_MIN_CHUNK_SHARES: floor on shrink size (0 => min_shares)
        self.sniper_entry_shrink_factor = float(os.getenv("SNIPER_ENTRY_SHRINK_FACTOR", "0.5"))
        self.sniper_entry_shrink_min_chunk_shares = int(os.getenv("SNIPER_ENTRY_SHRINK_MIN_CHUNK_SHARES", "0"))

        # Microstructure filters (thin spreads + sane YES/NO parity)
        self.sniper_max_spread_ticks = int(os.getenv("SNIPER_MAX_SPREAD_TICKS", "2"))
        self.sniper_parity_tolerance = float(os.getenv("SNIPER_PARITY_TOLERANCE", "0.02"))

        # Optional: account for fees in ROI gating (set if you know your effective taker fee rate)
        self.sniper_fee_rate = float(os.getenv("SNIPER_FEE_RATE", "0.0"))  # fraction, e.g. 0.01 = 1%
        self.sniper_min_edge_over_fees = float(os.getenv("SNIPER_MIN_EDGE_OVER_FEES", "0.0"))

        # Small epsilon to tolerate float / tick rounding (e.g., allow 0.991 when max=0.99 on 0.001-tick markets)
        # Default: min(0.005, tick) i.e. half a cent on $0.01 ticks, or 1 tick on sub-cent markets.
        try:
            _tick = float(self.cfg.tick) if float(self.cfg.tick) > 0 else 0.01
        except Exception:
            _tick = 0.01
        self.sniper_price_max_epsilon = float(os.getenv("SNIPER_PRICE_MAX_EPSILON", str(min(0.005, _tick))))

        # HARD cap on any BUY limit price. Prevents accidental 1.00 bids due to slippage/rounding.
        # If you truly want to pay higher, raise SNIPER_HARD_MAX_PRICE explicitly (not recommended).
        self.sniper_hard_max_price = float(os.getenv("SNIPER_HARD_MAX_PRICE", str(min(0.99, float(self.sniper_price_max)))))

        # Exit execution controls
        self.sniper_exit_order_type_fallback = os.getenv("SNIPER_EXIT_ORDER_TYPE_FALLBACK", "FAK").upper().strip()
        self.sniper_exit_chunk_shares = int(os.getenv("SNIPER_EXIT_CHUNK_SHARES", str(max(1, int(math.ceil(float(self.cfg.min_shares) - 1e-12))))))
        self.sniper_cancel_exit_orders_before_retry = os.getenv("SNIPER_CANCEL_EXIT_ORDERS_BEFORE_RETRY", "true").lower() in ("1", "true", "yes", "y")

        # Stop-loss sanity: require the stop condition to persist for N seconds to avoid thin-book fakeouts.
        self.sniper_min_hold_seconds = float(os.getenv("SNIPER_MIN_HOLD_SECONDS", "1.0"))
        self.sniper_stop_confirm_seconds = float(os.getenv("SNIPER_STOP_CONFIRM_SECONDS", "1.0"))

        # Entry sanity: require the entry *signal* (all entry gates passing) to persist
        # for N seconds before we submit an entry order. This reduces "touch-and-reverse"
        # whipsaws where price briefly enters the window, we buy, then immediately
        # mean-reverts into your stop.
        #
        # 0 disables (default) to preserve legacy behaviour.
        self.sniper_entry_confirm_seconds = float(os.getenv("SNIPER_ENTRY_CONFIRM_SECONDS", "0.0"))

        # ---- Endgame blind-post mode ----
        # In highly competitive short-dated markets (5m/15m), WS feeds can disconnect or go stale
        # right at the end. Normal SNIPER entry logic requires fresh 2-sided BBO data and may skip
        # the exact last-second "queue at 0.99" strategy.
        #
        # If enabled, the bot will attempt to place ONE resting limit BUY ("blind post") in the last
        # N seconds before expiry (and optionally for a short grace period after expiry) even if the
        # market/user websockets are disconnected or the book is one-sided (e.g. losing side bid=0).
        #
        # Side selection:
        #   - SNIPER_ENDGAME_SIDE=AUTO (default): choose the side with the higher last-known ask, and
        #     (by default) require it to be >= SNIPER_PRICE_MIN to avoid accidental bad buys.
        #   - SNIPER_ENDGAME_SIDE=YES|NO: force a specific side (recommended if you have an external feed).
        #
        # Price:
        #   - SNIPER_ENDGAME_BLIND_POST_PRICE=0 -> uses SNIPER_HARD_MAX_PRICE (or SNIPER_PRICE_MAX).
        # Size:
        #   - SNIPER_ENDGAME_BLIND_POST_SIZE_SHARES=0 -> uses normal sniper budget sizing (max_notional / max_total_cost).
        self.sniper_endgame_blind_post = env_bool("SNIPER_ENDGAME_BLIND_POST", False)
        self.sniper_endgame_blind_post_window_seconds = env_float("SNIPER_ENDGAME_BLIND_POST_WINDOW_SECONDS", 2.0)
        self.sniper_endgame_side = os.getenv("SNIPER_ENDGAME_SIDE", "AUTO").upper().strip()
        self.sniper_endgame_require_price_min = env_bool("SNIPER_ENDGAME_REQUIRE_PRICE_MIN", True)
        self.sniper_endgame_blind_post_price = env_float("SNIPER_ENDGAME_BLIND_POST_PRICE", 0.0)
        self.sniper_endgame_blind_post_size_shares = env_int("SNIPER_ENDGAME_BLIND_POST_SIZE_SHARES", 0)
        self.sniper_endgame_blind_post_max_stale_seconds = env_float("SNIPER_ENDGAME_BLIND_POST_MAX_STALE_SECONDS", 60.0)

        # Allow the SNIPER loop to keep running a bit after expiry so endgame orders can be posted/fill.
        # 0 keeps legacy behaviour (stop immediately at expiry).
        self.sniper_expiry_grace_seconds = env_float("SNIPER_EXPIRY_GRACE_SECONDS", 0.0)


        # Internal sniper state (runtime only)
        self._sniper_in_pos = False
        self._sniper_pos_open_ts = 0.0
        self._sniper_stop_breach_since = None

        # Reference entry price for stop-limit behaviour (set when position opens; reset when flat)
        self._sniper_entry_ref_price = 0.0

        # Stop-limit exit order tracking (used only when SNIPER_STOP_LOSS_MODE=LIMIT)
        self._sniper_stop_limit_order_id: Optional[str] = None
        self._sniper_stop_limit_order_ts = 0.0
        self._sniper_stop_limit_order_px = 0.0

        # Entry confirmation (debounce) state
        self._sniper_entry_gate_since: Optional[float] = None
        self._sniper_entry_gate_side: Optional[str] = None

        # Internal throttles
        self.sniper_last_signal_ts = 0.0

        # Endgame blind-post runtime tracking
        self._sniper_endgame_post_last_attempt_ts = 0.0


        # ------------------------------
        # SIGNAL_SNIPPER mode (external signals)
        # ------------------------------
        # This mode follows *external* directional signals (via SIGNAL_PROVIDER).
        # We intentionally reuse the existing SNIPER order/execution plumbing for safety (FOK/FAK, budgeting, TP/SL).
        #
        # Minimal required envs:
        #   EXEC_MODE=SIGNAL_SNIPPER
        #   SIGNAL_PROVIDER=WEBSOCKET
        #   SIGNAL_WS_URL=wss://...
        #
        # Optional slug behavior:
        #   SIGNAL_FOLLOW_SLUG=true  -> if the next signal targets a different market_slug, the bot will switch markets
        #   SIGNAL_REQUIRE_SLUG_MATCH=true -> only trade signals matching the current market_slug (recommended)
        if bool(getattr(self, "signal_sniper_mode", False)):
            self.signal_provider = os.getenv("SIGNAL_PROVIDER", "WEBSOCKET").upper().strip()
            self.signal_follow_slug = env_bool("SIGNAL_FOLLOW_SLUG", False)
            self.signal_require_slug_match = env_bool("SIGNAL_REQUIRE_SLUG_MATCH", True)
            self.signal_use_once = env_bool("SIGNAL_USE_ONCE", True)
            self.signal_ignore_time_window = env_bool("SIGNAL_IGNORE_TIME_WINDOW", True)

            # Signal quality gate
            self.signal_confidence_min = env_float("SIGNAL_CONFIDENCE_MIN", 0.0)

            # Debugging
            self.signal_debug = env_bool("SIGNAL_DEBUG", False)
            self.signal_ws_debug = env_bool("SIGNAL_WS_DEBUG", False)

            # Extra safety: require live ask close to signal entry_price/market_price (0 disables)
            self.signal_price_drift_max_ticks = env_float("SIGNAL_PRICE_DRIFT_MAX_TICKS", 0.0)

            # Map SIGNAL_* execution knobs onto SNIPER variables (so we can reuse _sniper_try_enter/_sniper_try_exit)
            self.sniper_price_min = env_float("SIGNAL_PRICE_MIN", float(self.sniper_price_min))
            self.sniper_price_max = env_float("SIGNAL_PRICE_MAX", float(self.sniper_price_max))
            self.sniper_hard_max_price = env_float("SIGNAL_HARD_MAX_PRICE", float(getattr(self, "sniper_hard_max_price", self.sniper_price_max)))

            self.sniper_take_profit_pct = env_float("SIGNAL_TAKE_PROFIT_PCT", float(self.sniper_take_profit_pct))
            self.sniper_stop_loss_pct = env_float("SIGNAL_STOP_LOSS_PCT", float(self.sniper_stop_loss_pct))

            # Optional: stop-loss execution mode overrides for SIGNAL_SNIPPER (inherits SNIPER_* defaults if unset)
            self.sniper_stop_loss_mode = os.getenv("SIGNAL_STOP_LOSS_MODE", str(self.sniper_stop_loss_mode)).upper().strip()
            self.sniper_stop_limit_order_type = os.getenv("SIGNAL_STOP_LIMIT_ORDER_TYPE", str(self.sniper_stop_limit_order_type)).upper().strip()
            self.sniper_stop_limit_resubmit_seconds = env_float("SIGNAL_STOP_LIMIT_RESUBMIT_SECONDS", float(getattr(self, "sniper_stop_limit_resubmit_seconds", 5.0)))

            self.sniper_max_spread_ticks = env_int("SIGNAL_MAX_SPREAD_TICKS", int(self.sniper_max_spread_ticks))
            self.sniper_parity_tolerance = env_float("SIGNAL_PARITY_TOLERANCE", float(self.sniper_parity_tolerance))

            self.sniper_entry_order_type = os.getenv("SIGNAL_ENTRY_ORDER_TYPE", str(self.sniper_entry_order_type)).upper().strip()
            self.sniper_exit_order_type = os.getenv("SIGNAL_EXIT_ORDER_TYPE", str(self.sniper_exit_order_type)).upper().strip()

            # Optional: limit-entry behaviours for signals (only relevant when *_ENTRY_ORDER_TYPE is GTC/LIMIT)
            self.sniper_entry_post_only = env_bool(
                "SIGNAL_ENTRY_POST_ONLY",
                bool(getattr(self, "sniper_entry_post_only", False)),
            )
            # Optional: bypass ROI feasibility gate for signal entries (rare; default false)
            self.sniper_entry_ignore_roi_gate = env_bool(
                "SIGNAL_ENTRY_IGNORE_ROI_GATE",
                bool(getattr(self, "sniper_entry_ignore_roi_gate", False)),
            )

            # Optional per-signal-mode fallbacks (inherits SNIPER_* if unset)
            self.sniper_entry_order_type_fallback = os.getenv(
                "SIGNAL_ENTRY_ORDER_TYPE_FALLBACK",
                str(getattr(self, "sniper_entry_order_type_fallback", "")),
            ).upper().strip()
            self.sniper_exit_order_type_fallback = os.getenv(
                "SIGNAL_EXIT_ORDER_TYPE_FALLBACK",
                str(getattr(self, "sniper_exit_order_type_fallback", "")),
            ).upper().strip()


            self.sniper_max_notional_usd = env_float("SIGNAL_MAX_NOTIONAL_USD", float(self.sniper_max_notional_usd))
            self.sniper_max_trades_per_market = env_int("SIGNAL_MAX_TRADES_PER_MARKET", int(self.sniper_max_trades_per_market))

            # Optional slippage overrides (ticks)
            self.sniper_entry_slippage_ticks = env_int("SIGNAL_ENTRY_SLIPPAGE_TICKS", int(self.sniper_entry_slippage_ticks))
            self.sniper_exit_slippage_ticks = env_int("SIGNAL_EXIT_SLIPPAGE_TICKS", int(self.sniper_exit_slippage_ticks))

            # Exit timing safety (optional; defaults to SNIPER_* values)
            self.sniper_exit_before_expiry = env_bool("SIGNAL_EXIT_BEFORE_EXPIRY", bool(self.sniper_exit_before_expiry))
            self.sniper_force_exit_seconds = env_int("SIGNAL_FORCE_EXIT_SECONDS", int(self.sniper_force_exit_seconds))

            # WS settings (used only if this bot instance creates its own SignalHub; otherwise main() can provide one)
            self.signal_ws_url = os.getenv("SIGNAL_WS_URL", "").strip()
            self.signal_ws_reconnect_min = env_float("SIGNAL_WS_RECONNECT_MIN", 1.0)
            self.signal_ws_reconnect_max = env_float("SIGNAL_WS_RECONNECT_MAX", 30.0)
            self.signal_ws_ping_interval = env_float("SIGNAL_WS_PING_INTERVAL", 10.0)
            self.signal_ws_ping_timeout = env_float("SIGNAL_WS_PING_TIMEOUT", 7.0)
            self.signal_ws_tls_min = env_float("SIGNAL_WS_TLS_MIN", 1.2)
            self.signal_ws_insecure = env_bool("SIGNAL_WS_INSECURE", False)

            # JSONL file logging ("file service") for external signals
            self.signal_file_dir = os.getenv("SIGNAL_FILE_DIR", "./signals").strip() or "./signals"
            self.signal_file_path = os.getenv("SIGNAL_FILE_PATH", "").strip()
            if not self.signal_file_path:
                safe_slug = re.sub(r"[^a-zA-Z0-9_\-\.]+", "_", str(self.market_slug or "unknown"))
                self.signal_file_path = os.path.join(self.signal_file_dir, f"signal_ws_{safe_slug}.jsonl")


        # ------------------------------
        # Repeating SNIPER mode (optional)
        # ------------------------------
        # Default behaviour (repeat disabled) is the original sniper flow: at most one
        # completed trade per market, then the bot stops and rolls to the next market.
        #
        # If SNIPER_REPEAT_MODE is enabled, the bot will keep cycling trades in the *same*
        # market after a successful TP exit, until:
        #   - trade_count reaches SNIPER_MAX_TRADES_PER_MARKET, OR
        #   - the entry window closes / we get too near expiry, OR
        #   - the market expires.
        #
        # Guardrails (recommended):
        #   - SNIPER_REPEAT_COOLDOWN_SECONDS: minimum idle time after an exit before a new entry.
        #   - SNIPER_REPEAT_STOP_AFTER_STOP_LOSS: stop repeating for this market after a stop-loss exit.
        self.sniper_repeat_mode = os.getenv(
            "SNIPER_REPEAT_MODE",
            os.getenv("SNIPER_REPEAT", "false"),
        ).lower() in ("1", "true", "yes", "y")
        try:
            self.sniper_repeat_cooldown_seconds = max(
                0.0, float(os.getenv("SNIPER_REPEAT_COOLDOWN_SECONDS", "2.0"))
            )
        except Exception:
            self.sniper_repeat_cooldown_seconds = 2.0
        self.sniper_repeat_stop_after_stop_loss = os.getenv(
            "SNIPER_REPEAT_STOP_AFTER_STOP_LOSS",
            os.getenv("SNIPER_STOP_AFTER_STOP_LOSS", "true"),
        ).lower() in ("1", "true", "yes", "y")

        if self.sniper_mode:
            # In sniper mode we usually want to trade closer to expiry than maker mode.
            # If you explicitly set STOP_BUFFER_SECONDS, we respect it; otherwise we clamp to <= 15s.
            if "SNIPER_STOP_BUFFER_SECONDS" in os.environ:
                self.cfg.stop_buffer_seconds = int(os.getenv("SNIPER_STOP_BUFFER_SECONDS", "15"))
            elif "STOP_BUFFER_SECONDS" not in os.environ:
                self.cfg.stop_buffer_seconds = min(int(self.cfg.stop_buffer_seconds), 15)

        # Status logging cadence in seconds (decoupled from loop frequency)
        self.log_every_seconds = float(os.getenv("LOG_EVERY_SECONDS", str(self.cfg.log_every)))
        self._last_status_log_ts = 0.0

                # Debug mode (prints why trades are skipped and key numbers)
        self.debug_mode = (
            os.getenv("DEBUG_MODE", "false").lower() in ("1", "true", "yes", "y")
            or os.getenv("PAIR_ARB_DEBUG", "false").lower() in ("1", "true", "yes", "y")
            or os.getenv("MAKER_DEBUG", "false").lower() in ("1", "true", "yes", "y")
        )
        # Maker debug (inherits DEBUG_MODE)
        self.maker_debug = (
            os.getenv("MAKER_DEBUG", "false").lower() in ("1", "true", "yes", "y")
            or self.debug_mode
        )
        # Pair-arb specific debug (inherits DEBUG_MODE)
        self.pair_arb_debug = (
            os.getenv("PAIR_ARB_DEBUG", "false").lower() in ("1", "true", "yes", "y")
            or self.debug_mode
        )
        self.debug_throttle_seconds = float(os.getenv("DEBUG_THROTTLE_SECONDS", "1.0"))
        self._debug_last_ts: Dict[str, float] = {}
        # ============================
        # Deterministic mini state-machine (FSM)
        # ============================
        # Purpose: make the bot's "balanced vs exposed" behaviour reproducible and avoid action spam.
        # States: BALANCED -> EXPOSED -> COOLDOWN -> BALANCED
        self.fsm_enabled = str(os.getenv("FSM_ENABLED", "true")).lower() in ("1", "true", "yes", "y")
        self.fsm_cooldown_seconds = float(os.getenv("FSM_COOLDOWN_SECONDS", "0.35"))  # pause quoting after re-hedge
        self.fsm_force_cancel_on_exposed_entry = str(os.getenv("FSM_FORCE_CANCEL_ON_EXPOSED_ENTRY", "true")).lower() in ("1", "true", "yes", "y")
        self.fsm_cancel_exchange_on_exposed_entry = str(os.getenv("FSM_CANCEL_EXCHANGE_ON_EXPOSED_ENTRY", "true")).lower() in ("1", "true", "yes", "y")
        self.fsm_dont_add_to_heavy = str(os.getenv("FSM_DONT_ADD_TO_HEAVY", "true")).lower() in ("1", "true", "yes", "y")
        self.fsm_max_exposure_shares = float(os.getenv("FSM_MAX_EXPOSURE_SHARES", "0"))  # 0=disabled
        self.fsm_state = "BALANCED"
        self._fsm_state_enter_ts = time.time()
        self._fsm_cooldown_until = 0.0
        self._fsm_exposed_since: Optional[float] = None



        # Optional runtime overrides for key config fields (lets you tune without DB changes)
        self._apply_cfg_overrides_from_env()

        # ============================
        # Pair-arb taker mode (closest to "atomic" on CLOB)
        # ============================
        self.pair_arb_order_type = os.getenv("PAIR_ARB_ORDER_TYPE", "FOK").upper().strip()
        # Hard timeout (200-500ms typical); we wait on WS fills up to this bound.
        self.pair_arb_timeout_seconds = float(os.getenv("PAIR_ARB_TIMEOUT_SECONDS", "0.35"))
        self.pair_arb_max_retries = int(os.getenv("PAIR_ARB_MAX_RETRIES", "3"))
        self.pair_arb_retry_backoff_ms_min = int(os.getenv("PAIR_ARB_RETRY_BACKOFF_MS_MIN", "150"))
        self.pair_arb_retry_backoff_ms_max = int(os.getenv("PAIR_ARB_RETRY_BACKOFF_MS_MAX", "400"))
        self.pair_arb_cooldown_seconds = float(os.getenv("PAIR_ARB_COOLDOWN_SECONDS", "0.5"))

        # Profit filter on asks: attempt only if (ask_yes + ask_no) <= 1 - (min_profit + est_fees + safety)
        self.pair_arb_min_profit_ticks = int(os.getenv("PAIR_ARB_MIN_PROFIT_TICKS", "2"))
        self.pair_arb_safety_ticks = int(os.getenv("PAIR_ARB_SAFETY_TICKS", "0"))
        self.pair_arb_slippage_ticks = int(os.getenv("PAIR_ARB_SLIPPAGE_TICKS", "0"))
        # If you pay taker fees, set this to total effective fee per complete-set as a decimal fraction of $1.
        # Example: 0.01 means "up to 1¢ per $1 notional" (conservative).
        self.pair_arb_fee_rate = float(os.getenv("PAIR_ARB_FEE_RATE", "0.0"))

        self.pair_arb_use_stability_gate = os.getenv("PAIR_ARB_USE_STABILITY_GATE", "true").lower() == "true"
        self.pair_arb_cancel_before_attempt = os.getenv("PAIR_ARB_CANCEL_BEFORE_ATTEMPT", "true").lower() == "true"
        self.pair_arb_reconcile_after_timeout = os.getenv("PAIR_ARB_RECONCILE_AFTER_TIMEOUT", "true").lower() == "true"
        self.pair_arb_pause_on_error_seconds = float(os.getenv("PAIR_ARB_PAUSE_ON_ERROR_SECONDS", "1.0"))
        self.pair_arb_pause_on_unknown_seconds = float(os.getenv("PAIR_ARB_PAUSE_ON_UNKNOWN_SECONDS", "0.5"))

        # Optional guards against extreme skew/legs (set high to disable)
        self.pair_arb_max_skew_ticks = int(os.getenv("PAIR_ARB_MAX_SKEW_TICKS", "999999"))
        self.pair_arb_max_leg_price = float(os.getenv("PAIR_ARB_MAX_LEG_PRICE", "0.99"))
        self.pair_arb_allow_gtc = os.getenv("PAIR_ARB_ALLOW_GTC", "false").lower() == "true"

        # Pair-arb sizing (integer shares)
        self.pair_arb_max_shares = int(os.getenv("PAIR_ARB_MAX_SHARES", str(int(math.floor(self.cfg.clip_shares + 1e-12)))))

        # Exposure handling when one leg fills but the other doesn't (configurable)
        #   - UNWIND: immediately taker-sell the heavy leg (fastest risk removal; may realize a small loss)
        #   - HEDGE: try to buy missing leg using hedge-cap rules + emergency hedge; if cannot, optionally UNWIND after a grace window
        #   - WAIT: do nothing (NOT recommended)
        self.exposure_policy = os.getenv("EXPOSURE_POLICY", "UNWIND").upper().strip()
        self.exposure_unwind_slippage_ticks = int(os.getenv("EXPOSURE_UNWIND_SLIPPAGE_TICKS", "0"))
        self.exposure_hedge_then_unwind = os.getenv("EXPOSURE_HEDGE_THEN_UNWIND", "true").lower() in ("1", "true", "yes", "y")
        self.exposure_hedge_grace_seconds = float(os.getenv("EXPOSURE_HEDGE_GRACE_SECONDS", "0.3"))

        # Maker-mode exposure policy (applies when we become imbalanced in MAKER mode)
        # Policies:
        #   - HEDGE: (default legacy behavior) try to buy missing side within hedge-cap; may wait if cap-blocked.
        #   - UNWIND: immediately SELL the heavy leg to flatten exposure (fastest risk removal; may realize a small loss).
        #   - HEDGE_THEN_UNWIND: hedge if possible, but if cap-blocked beyond a grace window, unwind heavy.
        self.maker_exposure_policy = os.getenv("MAKER_EXPOSURE_POLICY", str(self.exposure_policy)).upper().strip()
        self.maker_exposure_unwind_slippage_ticks = int(os.getenv("MAKER_EXPOSURE_UNWIND_SLIPPAGE_TICKS", str(self.exposure_unwind_slippage_ticks)))
        self.maker_exposure_unwind_order_type = os.getenv("MAKER_EXPOSURE_UNWIND_ORDER_TYPE", str(self.hedge_taker_order_type)).upper().strip()
        self.maker_exposure_hedge_then_unwind = os.getenv("MAKER_EXPOSURE_HEDGE_THEN_UNWIND", str(self.exposure_hedge_then_unwind)).lower() in ("1","true","yes","y")
        self.maker_exposure_hedge_grace_seconds = float(os.getenv("MAKER_EXPOSURE_HEDGE_GRACE_SECONDS", str(self.exposure_hedge_grace_seconds)))
        # If >0, force unwind after this many seconds regardless (strict safety hard stop for exposure).
        self.maker_exposure_max_seconds = float(os.getenv("MAKER_EXPOSURE_MAX_SECONDS", "0.0"))

        self._pair_arb_last_attempt_ts = 0.0

        # Reprice throttling: avoid constantly cancel/replacing and getting picked off.
        self.reprice_min_seconds = float(os.getenv("REPRICE_MIN_SECONDS", "1.5"))

        # Exchange reconciliation (prevents duplicate live orders per asset when cancels lag / feed flaps).
        self.reconcile_exchange_orders = os.getenv("RECONCILE_EXCHANGE_ORDERS", "true").lower() == "true"
        self.reconcile_interval_seconds = float(os.getenv("RECONCILE_INTERVAL_SECONDS", "5.0"))
        self.cancel_replace_guard_seconds = float(os.getenv("CANCEL_REPLACE_GUARD_SECONDS", "0.75"))
        self._last_reconcile_ts: Dict[str, float] = {}
        self._cancel_pending_until: Dict[str, float] = {}

        # Paired-entry gate: require the opposite ASK to be hedgeable (no-loss) at time of quoting.
        # Example: if we bid YES at y_bid, require no_ask <= 1 - y_bid - buffer.
        self.paired_entry_buffer_ticks = int(os.getenv("PAIRED_ENTRY_BUFFER_TICKS", str(max(2, self.cfg.hedge_buffer_ticks))))
        # Minimum entry edge enforcement (protects against DB/ENV misconfig that allows zero-edge trades).
        self.min_entry_edge_ticks = int(os.getenv("MIN_ENTRY_EDGE_TICKS", "2"))
        # OCO (remove "free option"): if we are quoting BOTH sides and one side gets filled,
        # cancel BOTH quotes immediately to prevent the market from repeatedly filling the "wrong" side.
        self.oco_on_fill = os.getenv("OCO_ON_FILL", "true").lower() == "true"
        self.oco_min_fill_shares = float(
            os.getenv(
                "OCO_MIN_FILL_SHARES",
                str(max(1.0, 0.2 * float(self.cfg.min_shares))),
            )
        )
        self.oco_pause_seconds = float(os.getenv("OCO_PAUSE_SECONDS", "1.0"))
        self._quote_pause_until = 0.0  # accumulate-mode pause only

        # Quote invalidation: if resting quotes become unhedgeable (given the opposite ASK), cancel them BEFORE they get picked off.
        self.quote_invalidation_enabled = os.getenv("QUOTE_INVALIDATION_ENABLED", "true").lower() == "true"
        self.quote_invalidation_buffer_ticks = int(
            os.getenv("QUOTE_INVALIDATION_BUFFER_TICKS", str(self.paired_entry_buffer_ticks))
        )
        self.quote_invalidation_pause_seconds = float(os.getenv("QUOTE_INVALIDATION_PAUSE_SECONDS", "2.0"))

        # Hedge order churn control (keep protective hedge working longer).
        self.hedge_stale_seconds = int(os.getenv("HEDGE_STALE_SECONDS", "20"))

        # Emergency hedge cap-block spam control.
        self.cap_blocked_cooldown_seconds = float(os.getenv("CAP_BLOCKED_COOLDOWN_SECONDS", "3.0"))
        self._cap_blocked_until = 0.0
        self._cap_blocked_asset = None
        self._cap_blocked_cap = None

        # Unhedged timer
        self._unhedged_since: Optional[float] = None

        # Circuit breaker (bounded-loss flattening)
        self.max_loss_enabled = os.getenv("MAX_LOSS_ENABLED", "true").lower() == "true"
        self.max_loss_usd_per_market = float(os.getenv("MAX_LOSS_USD_PER_MARKET", "1"))
        self.max_loss_grace_seconds = float(os.getenv("MAX_LOSS_GRACE_SECONDS", "15"))
        self.max_loss_confirm_seconds = float(os.getenv("MAX_LOSS_CONFIRM_SECONDS", "25"))
        self.max_loss_runaway_gap_ticks = int(os.getenv("MAX_LOSS_RUNAWAY_GAP_TICKS", "25"))
        self._max_loss_breach_since: Optional[float] = None
        self._last_position_change_ts: float = time.time()

        # Remember our taker hedge order IDs so we can attribute user WS trade events to the intended asset.
        # Map: taker_order_id -> {"ts": float, "asset_id": str}
        self._recent_taker_orders: Dict[str, dict] = {}
        self._recent_taker_lock = threading.Lock()

        # ============================
        # Execution latency tracking (signal -> submit -> fill)
        # ============================
        # Goal: measure latency (ms) from SIGNAL received -> order SUBMIT -> FILL (execution)
        # Default ON. Disable with EXEC_LATENCY_LOG_ENABLED=false
        self.exec_latency_log_enabled = env_bool("EXEC_LATENCY_LOG_ENABLED", True)
        # Console logs for submit timing breakdown (decision -> send -> ack)
        # Defaults: ON for non-MAKER modes; OFF for MAKER (to avoid spam).
        self.exec_latency_submit_breakdown_console = env_bool("EXEC_LATENCY_LOG_SUBMIT_BREAKDOWN_CONSOLE", True)
        self.exec_latency_submit_breakdown_console_maker = env_bool("EXEC_LATENCY_LOG_SUBMIT_BREAKDOWN_CONSOLE_MAKER", False)
        # Keep per-order context long enough for late maker fills; prune by TTL + max records.
        self.exec_latency_context_ttl_seconds = float(os.getenv("EXEC_LATENCY_CONTEXT_TTL_SECONDS", "21600"))  # 6 hours
        self.exec_latency_max_context_records = int(os.getenv("EXEC_LATENCY_MAX_CONTEXT_RECORDS", "50000"))

        # Active signal context (set only while processing a signal-driven action)
        self._active_signal_ctx: Optional[dict] = None
        self._active_signal_ctx_lock = threading.Lock()

        # Order execution context (order_id -> metadata), used later when fills arrive.
        self._order_exec_ctx: Dict[str, dict] = {}
        self._order_exec_ctx_order: Deque[Tuple[float, str]] = deque()  # (submit_ts, order_id)
        self._order_exec_ctx_lock = threading.Lock()

        # ============================
        # Execution latency file logs (CSV/JSONL)
        # ============================
        # Default ON (if execution latency tracking is enabled).
        # Customize with:
        #   EXEC_LATENCY_FILE_LOG_ENABLED=true/false
        #   EXEC_LATENCY_LOG_DIR=./logs
        #   EXEC_LATENCY_JSONL_PATH=./logs/exec_latency.jsonl
        #   EXEC_LATENCY_CSV_PATH=./logs/exec_latency.csv
        #   EXEC_LATENCY_JSONL_ENABLED=true/false
        #   EXEC_LATENCY_CSV_ENABLED=true/false
        #   EXEC_LATENCY_FILE_LOG_SUBMIT_SIGNAL_EVENTS=true/false
        #   EXEC_LATENCY_FILE_LOG_SUBMIT_ALL_EVENTS=true/false
        self.exec_latency_file_log_enabled = env_bool("EXEC_LATENCY_FILE_LOG_ENABLED", True)
        self.exec_latency_jsonl_enabled = env_bool("EXEC_LATENCY_JSONL_ENABLED", True)
        self.exec_latency_csv_enabled = env_bool("EXEC_LATENCY_CSV_ENABLED", True)

        # By default we only file-log SUBMIT events for signal-driven orders (to avoid huge logs in MAKER mode).
        self.exec_latency_file_log_submit_signal_events = env_bool("EXEC_LATENCY_FILE_LOG_SUBMIT_SIGNAL_EVENTS", True)
        self.exec_latency_file_log_submit_all_events = env_bool("EXEC_LATENCY_FILE_LOG_SUBMIT_ALL_EVENTS", False)

        self.exec_latency_log_dir = os.getenv("EXEC_LATENCY_LOG_DIR", "./logs").strip() or "./logs"
        self.exec_latency_jsonl_path = os.getenv("EXEC_LATENCY_JSONL_PATH", "").strip()
        self.exec_latency_csv_path = os.getenv("EXEC_LATENCY_CSV_PATH", "").strip()

        if not self.exec_latency_jsonl_path:
            self.exec_latency_jsonl_path = os.path.join(self.exec_latency_log_dir, "exec_latency.jsonl")
        if not self.exec_latency_csv_path:
            self.exec_latency_csv_path = os.path.join(self.exec_latency_log_dir, "exec_latency.csv")

        self._exec_latency_log_service: Optional[LatencyLogService] = None
        try:
            if bool(self.exec_latency_log_enabled) and bool(self.exec_latency_file_log_enabled):
                self._exec_latency_log_service = LatencyLogService(
                    jsonl_path=self.exec_latency_jsonl_path,
                    csv_path=self.exec_latency_csv_path,
                    enabled=True,
                    jsonl_enabled=bool(self.exec_latency_jsonl_enabled),
                    csv_enabled=bool(self.exec_latency_csv_enabled),
                )
        except Exception:
            # Never let telemetry break trading.
            self._exec_latency_log_service = None


        # Balance/allowance cache (helps recover when WS trade/order events are delayed or missed)
        # token_id -> (ts, balance, allowance)
        self._ba_cache: Dict[str, Tuple[float, float, float]] = {}
        self._ba_cache_lock = threading.Lock()

        self.client = self._init_clob_client()

        # Discover tokens
        m = fetch_market_by_slug(market_slug, self.logger)
        if m is None:
            raise RuntimeError("NO_MARKET")  # handled by outer loop

        # Override timing from Gamma if available (works for non-timestamp slugs too)
        s_iso = m.get("startDate") or m.get("start_date")
        e_iso = m.get("endDate") or m.get("end_date")
        s_ep = _iso_to_epoch(s_iso) if isinstance(s_iso, str) else None
        e_ep = _iso_to_epoch(e_iso) if isinstance(e_iso, str) else None
        if s_ep:
            self.start_ts = s_ep
        if e_ep and e_ep > 0:
            self.expiry_ts = e_ep
        else:
            self.expiry_ts = int(self.start_ts) + int(self.cfg.market_duration_seconds)
        self.expiry_ts = self.expiry_ts + 10
        self.yes_asset, self.no_asset, self.condition_id = parse_tokens_and_condition(m)

        # --- Orderbook cache + market microstructure autodetect (tick_size/min_order_size) ---
        self._book_cache: Dict[str, Tuple[float, dict]] = {}
        self._book_cache_lock = threading.Lock()
        self.book_cache_ttl_seconds = float(os.getenv("BOOK_CACHE_TTL_SECONDS", "0.5"))
        self.orderbook_http_timeout = float(os.getenv("ORDERBOOK_HTTP_TIMEOUT", "3.0"))
        self.autodetect_market_params = os.getenv("AUTO_DETECT_MARKET_PARAMS", "true").lower() in ("1", "true", "yes", "y")
        self.market_tick_size = None
        self.market_min_order_size = None

        # Depth gate (liquidity check before quoting / before mismatched unwind)
        self.depth_gate_enabled = os.getenv("DEPTH_GATE_ENABLED", "true").lower() in ("1", "true", "yes", "y")
        self.depth_gate_levels = int(os.getenv("DEPTH_GATE_LEVELS", "50"))
        self.depth_gate_min_mult = float(os.getenv("DEPTH_GATE_MIN_MULT", "1.0"))
        self.depth_gate_max_age_seconds = float(os.getenv("DEPTH_GATE_MAX_AGE_SECONDS", "1.5"))
        self.depth_gate_warn_only = os.getenv("DEPTH_GATE_WARN_ONLY", "false").lower() in ("1", "true", "yes", "y")

        # WS congestion safety (avoid false "mismatch" -> unnecessary UNWIND)
        self.mismatch_reconcile_from_balance = os.getenv("MISMATCH_RECONCILE_FROM_BALANCE", "true").lower() in ("1", "true", "yes", "y")

        # Unwind chunking (reduces slippage)
        try:
            _min_int = max(1, int(math.ceil(float(self.cfg.min_shares) - 1e-12)))
        except Exception:
            _min_int = 1
        self.unwind_chunk_shares = float(os.getenv("UNWIND_CHUNK_SHARES", str(_min_int)))
        self.unwind_max_passes = int(os.getenv("UNWIND_MAX_PASSES", "4"))
        self.unwind_wait_after_order_seconds = float(os.getenv("UNWIND_WAIT_AFTER_ORDER_SECONDS", "0.6"))
        self.unwind_depth_gate_enabled = os.getenv("UNWIND_DEPTH_GATE_ENABLED", "true").lower() in ("1", "true", "yes", "y")

        # Size precision for maker orders (shares can be fractional in some markets)
        self.size_decimals = int(os.getenv("SIZE_DECIMALS", "6"))

        # Apply market params from /book (tick_size / min_order_size) BEFORE we start quoting
        try:
            self._sync_market_params_from_book(force=True)
        except Exception:
            pass


        self.logger.info(f"✅ Market Found: {market_slug}")
        self.logger.info(f"Condition ID: {self.condition_id}")
        self.logger.info(f"YES asset: {self.yes_asset}")
        self.logger.info(f"NO  asset: {self.no_asset}")
        self.logger.info(f"Start ts: {self.start_ts} | Expiry ts: {self.expiry_ts}")
        self.logger.info(
            f"⚙️ Mode={self.exec_mode} tick={float(self.cfg.tick):.6f} min_shares={self.cfg.min_shares} clip={self.cfg.clip_shares} "
            f"entry_edge_ticks={self.cfg.entry_edge_ticks} hedge_buf_ticks={self.cfg.hedge_buffer_ticks} maker_buf_ticks={self.cfg.maker_buffer_ticks}"
        )
        self.logger.info(
            f"⚙️ MakerExposure={self._normalize_exposure_policy(self.maker_exposure_policy)} "
            f"(grace={self.maker_exposure_hedge_grace_seconds}s max={self.maker_exposure_max_seconds}s) "
            f"unhedged_timeout={self.unhedged_timeout_seconds}s"
        )
        if getattr(self, "debug_mode", False):
            self.logger.info(
                f"🐛 DEBUG_MODE enabled maker_debug={getattr(self,'maker_debug',False)} "
                f"pair_arb_debug={getattr(self,'pair_arb_debug',False)} throttle={getattr(self,'debug_throttle_seconds',1.0)}s"
            )


        self.api_creds = self.client.create_or_derive_api_creds()
        self.client.set_api_creds(self.api_creds)

        self.user_api_key = self.api_creds.api_key  # used to identify maker fills

        # best bid/ask cache via market ws
        self.best = {}      # asset_id -> {"bid":..., "ask":...}
        self.best_ts = {}   # asset_id -> time.time()
        self.best_lock = threading.Lock()

        # Event-driven loop: wake on market best_bid_ask updates and on fills
        self.market_update_event = threading.Event()
        self.position_update_event = threading.Event()
        self.wake_event = threading.Event()  # wakes main loop on market *or* fills

        self.stop_event = threading.Event()
        self._ticks = 0
        self._in_feed_pause = False

        # WS
        self.market_connected = False
        self.user_connected = False
        self.market_ws = None
        self.user_ws = None

        if self.cfg.cancel_all_on_start:
            self.cancel_all_orders_exchange(reason="startup cleanup")

    def _init_clob_client(self) -> ClobClient:
        if self.cfg.signature_type is not None and self.cfg.funder:
            return ClobClient(
                self.cfg.clob_host,
                key=self.cfg.private_key,
                chain_id=self.cfg.chain_id,
                signature_type=self.cfg.signature_type,
                funder=self.cfg.funder,
            )
        return ClobClient(self.cfg.clob_host, key=self.cfg.private_key, chain_id=self.cfg.chain_id)

    # ---------------- WS plumbing ----------------
    def _mk_ws(self, channel: str, on_msg):
        url = f"{self.cfg.ws_base}/ws/{channel}"
        return WebSocketApp(
            url,
            on_open=lambda ws: self._on_open(ws, channel),
            on_message=on_msg,
            on_error=lambda ws, err: self._on_error(ws, channel, err),
            on_close=lambda ws, code, msg: self._on_close(ws, channel, code, msg),
        )

    def _on_open(self, ws, channel: str):
        if channel == "market":
            self.market_connected = True
            sub = {
                "assets_ids": [self.yes_asset, self.no_asset],
                "type": "market",
                "custom_feature_enabled": True,  # enables best_bid_ask
            }
            ws.send(json.dumps(sub))

        elif channel == "user":
            self.user_connected = True
            auth = {
                "apiKey": self.api_creds.api_key,
                "secret": self.api_creds.api_secret,
                "passphrase": self.api_creds.api_passphrase,
            }
            sub = {
                "markets": [self.condition_id],
                "type": "user",
                "auth": auth,
            }
            ws.send(json.dumps(sub))
        else:
            raise ValueError("unknown ws channel")

        t = threading.Thread(target=self._ping_loop, args=(ws,), daemon=True)
        t.start()

    def _on_error(self, ws, channel: str, err):
        self.logger.error(f"[{channel}] error: {err}")
        if channel == "market":
            self.market_connected = False
        else:
            self.user_connected = False

    def _on_close(self, ws, channel: str, code, msg):
        self.logger.info(f"[{channel}] closed: {code} {msg}")
        if channel == "market":
            self.market_connected = False
        else:
            self.user_connected = False

    def _ping_loop(self, ws):
        while not self.stop_event.is_set():
            try:
                ws.send("PING")
            except Exception:
                return
            time.sleep(10)

    def _ws_runner(self, channel: str, on_msg):
        backoff = self.cfg.ws_reconnect_min
        while not self.stop_event.is_set():
            ws = self._mk_ws(channel, on_msg)
            if channel == "market":
                self.market_ws = ws
            else:
                self.user_ws = ws

            try:
                ws.run_forever()
            except Exception as e:
                self.logger.error(f"[{channel}] run_forever exception: {e}")

            if self.stop_event.is_set():
                break

            if channel == "market":
                self.market_connected = False
            else:
                self.user_connected = False

            sleep_for = min(backoff, self.cfg.ws_reconnect_max)
            sleep_for *= (0.7 + random.random() * 0.6)
            self.logger.info(f"[{channel}] reconnecting in {sleep_for:.1f}s...")
            time.sleep(sleep_for)
            backoff = min(backoff * 2, self.cfg.ws_reconnect_max)

    # ------------- market events -------------
    def _handle_market_event(self, msg: dict):
        et = msg.get("event_type") or msg.get("type") or ""
        et = str(et).lower().strip()

        # Tick-size changes can happen mid-market. If we see a tick_size update, refresh params and pull quotes.
        if et in ("tick_size_change", "ticksizechange", "tick_size", "ticksize"):
            raw = (
                msg.get("tick_size")
                or msg.get("tickSize")
                or msg.get("new_tick_size")
                or msg.get("newTickSize")
                or msg.get("value")
            )
            try:
                new_tick = float(raw)
            except Exception:
                new_tick = 0.0

            if new_tick > 0 and abs(float(new_tick) - float(getattr(self.cfg, "tick", 0.01))) > 1e-12:
                old_tick = float(getattr(self.cfg, "tick", 0.01))
                try:
                    self.cfg.tick = float(new_tick)
                except Exception:
                    pass

                # Best-effort: refresh from /book (min_order_size can also change)
                try:
                    self._sync_market_params_from_book(force=True)
                except Exception:
                    pass
                try:
                    self._apply_tick_dependent_params()
                except Exception:
                    pass

                try:
                    self.logger.info(f"🧷 tick_size change {old_tick:.6f} -> {float(self.cfg.tick):.6f} | cancel quotes")
                except Exception:
                    pass

                # Cancel quotes so we don't keep invalid-tick orders resting
                try:
                    self.cancel_all_open_orders_local(reason="tick size change")
                    self._cancel_exchange_orders_for_assets([self.yes_asset, self.no_asset], reason="tick size change")
                except Exception:
                    pass

                try:
                    self.wake_event.set()
                except Exception:
                    pass
            return

        if et != "best_bid_ask":
            return

        asset_id = msg.get("asset_id")
        if not asset_id:
            return
        bid = float(msg.get("best_bid") or 0)
        ask = float(msg.get("best_ask") or 0)
        with self.best_lock:
            self.best[asset_id] = {"bid": bid, "ask": ask}
            self.best_ts[asset_id] = time.time()
        try:
            self.market_update_event.set()
        except Exception:
            pass
        try:
            self.wake_event.set()
        except Exception:
            pass


    def on_market_message(self, ws, message: str):
        try:
            payload = json.loads(message)
        except Exception:
            return
        if isinstance(payload, list):
            for item in payload:
                if isinstance(item, dict):
                    self._handle_market_event(item)
        elif isinstance(payload, dict):
            self._handle_market_event(payload)

    def _market_data_fresh(self) -> bool:
        if not self.market_connected or not self.user_connected:
            return False
        now = time.time()
        with self.best_lock:
            for aid in (self.yes_asset, self.no_asset):
                ts = self.best_ts.get(aid, 0.0)
                if ts <= 0:
                    return False
                if (now - ts) > self.cfg.market_data_stale_seconds:
                    return False
        return True

    def _best_bid_ask(self, asset_id: str) -> Optional[Tuple[float, float]]:
        with self.best_lock:
            b = self.best.get(asset_id)
        if not b:
            return None
        bid = float(b.get("bid") or 0.0)
        ask = float(b.get("ask") or 0.0)
        return bid, ask

    def _dbg(self, msg: str, key: str = "dbg", throttle_s: Optional[float] = None) -> None:
        """Debug print with per-key throttling (enabled by DEBUG_MODE or PAIR_ARB_DEBUG)."""
        if not getattr(self, "debug_mode", False):
            return
        now = time.time()
        try:
            t = float(self.debug_throttle_seconds) if throttle_s is None else float(throttle_s)
        except Exception:
            t = 1.0
        try:
            last = float(self._debug_last_ts.get(key, 0.0))
        except Exception:
            last = 0.0
        if (now - last) >= max(0.0, t):
            self.logger.info(msg)
            try:
                self._debug_last_ts[key] = now
            except Exception:
                pass



    def _dbg_maker(self, msg: str, key: str = "maker_dbg", throttle_s: Optional[float] = None) -> None:
        """Maker-mode debug print (requires MAKER_DEBUG or DEBUG_MODE)."""
        if not getattr(self, "maker_debug", False):
            return
        self._dbg(msg, key=key, throttle_s=throttle_s)



    
    # ============================
    # Orderbook (REST) helpers: depth + market params
    # ============================
    def _book_url(self) -> str:
        base = str(getattr(self.cfg, "clob_host", "") or "").rstrip("/")
        if not base:
            base = "https://clob.polymarket.com"
        return f"{base}/book"

    def _extract_float_any(self, obj: dict, keys: Tuple[str, ...]) -> Optional[float]:
        if not isinstance(obj, dict):
            return None
        for k in keys:
            if k in obj and obj.get(k) is not None:
                try:
                    return float(obj.get(k))
                except Exception:
                    try:
                        return float(str(obj.get(k)).strip())
                    except Exception:
                        pass
        return None

    def _fetch_book_summary_http(self, token_id: str) -> Optional[dict]:
        """Fetch /book summary for a token_id via raw HTTP."""
        try:
            url = self._book_url()
            timeout = float(getattr(self, "orderbook_http_timeout", 3.0) or 3.0)
            r = requests.get(url, params={"token_id": str(token_id)}, timeout=timeout)
            r.raise_for_status()
            data = r.json()
        except Exception as e:
            try:
                self.logger.warning(f"[BOOK] fetch failed token={str(token_id)[-6:]} err={e}")
            except Exception:
                pass
            return None

        # Normalize shapes: some gateways may wrap under {"data": {...}}
        if isinstance(data, dict) and isinstance(data.get("data"), dict):
            data = data.get("data")

        if not isinstance(data, dict):
            return None
        return data

    def _get_book_cached(self, token_id: str, max_age_seconds: Optional[float] = None, force: bool = False) -> Optional[dict]:
        """Return cached orderbook summary (bids/asks + tick/min) with TTL."""
        tid = str(token_id)
        now = time.time()
        ttl = float(getattr(self, "book_cache_ttl_seconds", 0.5) or 0.5)
        if max_age_seconds is not None:
            ttl = min(ttl, float(max_age_seconds))
        if ttl < 0:
            ttl = 0.0

        if not force:
            try:
                with self._book_cache_lock:
                    rec = self._book_cache.get(tid)
                if rec:
                    ts, book = rec
                    if (now - float(ts)) <= ttl:
                        return book
            except Exception:
                pass

        book = self._fetch_book_summary_http(tid)
        if not book:
            return None
        try:
            with self._book_cache_lock:
                self._book_cache[tid] = (now, book)
        except Exception:
            pass
        return book

    def _iter_book_levels(self, levels: Any):
        """Yield (price,size) from various orderbook formats."""
        if not levels:
            return

        # Some /book variants return a dict wrapper like {"levels": [...]} or {"data": [...]}.
        if isinstance(levels, dict):
            for k in ("levels", "data", "rows", "orders", "orderbook", "book"):
                v = levels.get(k)
                if isinstance(v, list) and v:
                    levels = v
                    break

        # Some variants return a price->size mapping like {"0.96": "10", "0.97": "5"}.
        if isinstance(levels, dict):
            for k, v in list(levels.items()):
                try:
                    p = float(k)
                    s = float(v)
                    if s > 0:
                        yield p, s
                except Exception:
                    continue
            return

        # Default: list of [price,size] or dict entries
        for lvl in list(levels or []):
            try:
                if isinstance(lvl, (list, tuple)) and len(lvl) >= 2:
                    p = float(lvl[0])
                    s = float(lvl[1])
                    yield p, s
                    continue

                if isinstance(lvl, dict):
                    p = lvl.get("price") if "price" in lvl else lvl.get("p")
                    # Size key varies across gateways
                    s = (
                        lvl.get("size")
                        if "size" in lvl
                        else lvl.get("s")
                        if "s" in lvl
                        else lvl.get("quantity")
                        if "quantity" in lvl
                        else lvl.get("qty")
                        if "qty" in lvl
                        else lvl.get("amount")
                        if "amount" in lvl
                        else lvl.get("shares")
                    )
                    if p is None or s is None:
                        continue
                    yield float(p), float(s)
            except Exception:
                continue

    def _book_side_levels(self, book: Any, side: str) -> Optional[Any]:
        """Best-effort extraction of bids/asks array from /book response variants."""
        if not isinstance(book, dict):
            return None
        side_key = str(side or "").lower().strip()
        if not side_key:
            return None

        # Side-key aliases across different gateways/versions.
        aliases: Dict[str, Tuple[str, ...]] = {
            "asks": ("asks", "ask", "sell", "sells", "offers", "offer", "sell_orders", "sellOrders"),
            "bids": ("bids", "bid", "buy", "buys", "bids_list", "buy_orders", "buyOrders"),
        }

        def _pick(d: dict) -> Optional[Any]:
            if not isinstance(d, dict):
                return None
            keys = aliases.get(side_key, (side_key,))
            for base_k in keys:
                for k in (base_k, base_k.upper(), base_k.capitalize()):
                    if k in d:
                        return d.get(k)
            return None

        # Direct
        lvls = _pick(book)
        if lvls is not None:
            return lvls

        # Nested wrappers seen in some gateways
        for ck in ("book", "orderbook", "orderBook", "data", "result"):
            sub = book.get(ck)
            if isinstance(sub, dict):
                lvls = _pick(sub)
                if lvls is not None:
                    return lvls

        return None

    def _cum_depth(self, token_id: str, side: str, price_limit: float, max_levels: Optional[int] = None, max_age_seconds: Optional[float] = None) -> float:
        """Cumulative size available on one side up to a price limit.

        - asks: sum sizes where price <= price_limit
        - bids: sum sizes where price >= price_limit
        """
        book = self._get_book_cached(token_id, max_age_seconds=max_age_seconds)
        if not book:
            return 0.0

        side_key = str(side or "").lower().strip()
        lvls = self._book_side_levels(book, side_key)
        if lvls is None:
            return 0.0

        lim = float(price_limit)
        ml = int(max_levels) if max_levels is not None else None

        # Normalize and sort (some gateways don't guarantee ordering)
        pairs: List[Tuple[float, float]] = []
        for p, s in self._iter_book_levels(lvls):
            try:
                p = float(p)
                s = float(s)
            except Exception:
                continue
            if s <= 0 or p <= 0:
                continue
            pairs.append((p, s))

        if not pairs:
            return 0.0

        if side_key == "asks":
            pairs.sort(key=lambda x: x[0])
        else:
            pairs.sort(key=lambda x: x[0], reverse=True)

        if ml is not None and ml > 0:
            pairs = pairs[:ml]

        total = 0.0
        eps = 1e-12
        if side_key == "asks":
            for p, s in pairs:
                if p <= (lim + eps):
                    total += s
                else:
                    break
        else:
            for p, s in pairs:
                if p >= (lim - eps):
                    total += s
                else:
                    break

        return float(total)

    def _apply_tick_dependent_params(self) -> None:
        """Recompute a few derived parameters when tick/min_shares change."""
        try:
            tick = float(self.cfg.tick) if float(self.cfg.tick) > 0 else 0.01
        except Exception:
            tick = 0.01

        # Update sniper epsilon if not explicitly configured
        if "SNIPER_PRICE_MAX_EPSILON" not in os.environ:
            try:
                self.sniper_price_max_epsilon = float(min(0.005, tick))
            except Exception:
                pass

        # Ensure clip >= min_shares (avoid invalid size)
        try:
            self.cfg.clip_shares = max(float(self.cfg.clip_shares), float(self.cfg.min_shares))
        except Exception:
            pass

        # Update sniper exit chunk default if not explicitly set
        try:
            if "SNIPER_EXIT_CHUNK_SHARES" not in os.environ:
                min_int = max(1, int(math.ceil(float(self.cfg.min_shares) - 1e-12)))
                self.sniper_exit_chunk_shares = int(min_int)
        except Exception:
            pass

        # OCO min fill threshold depends on min_shares; recompute only if not explicitly set
        if "OCO_MIN_FILL_SHARES" not in os.environ:
            try:
                self.oco_min_fill_shares = float(max(1.0, 0.2 * float(self.cfg.min_shares)))
            except Exception:
                pass

    def _sync_market_params_from_book(self, force: bool = False) -> None:
        """Auto-detect tick_size and min_order_size from /book and apply to cfg."""
        if not getattr(self, "autodetect_market_params", True):
            return

        yb = self._get_book_cached(self.yes_asset, force=force, max_age_seconds=0.0)
        nb = self._get_book_cached(self.no_asset, force=force, max_age_seconds=0.0)
        if not yb or not nb:
            return

        tick_y = self._extract_float_any(yb, ("tick_size", "tickSize", "tick"))
        tick_n = self._extract_float_any(nb, ("tick_size", "tickSize", "tick"))
        min_y = self._extract_float_any(yb, ("min_order_size", "minOrderSize", "min_order", "minOrder"))
        min_n = self._extract_float_any(nb, ("min_order_size", "minOrderSize", "min_order", "minOrder"))

        tick = None
        if tick_y and tick_y > 0:
            tick = float(tick_y)
        if tick_n and tick_n > 0:
            tick = float(tick_n) if tick is None else min(float(tick), float(tick_n))

        min_sz = None
        if min_y and min_y > 0:
            min_sz = float(min_y)
        if min_n and min_n > 0:
            min_sz = float(min_n) if min_sz is None else max(float(min_sz), float(min_n))

        old_tick = float(getattr(self.cfg, "tick", 0.01) or 0.01)
        old_min = float(getattr(self.cfg, "min_shares", 1.0) or 1.0)

        changed = False
        if tick and tick > 0 and abs(float(tick) - float(old_tick)) > 1e-12:
            self.cfg.tick = float(tick)
            self.market_tick_size = float(tick)
            changed = True

        if min_sz and min_sz > 0:
            # Never lower below configured value; always satisfy market minimum
            new_min = max(float(old_min), float(min_sz))
            if abs(new_min - float(old_min)) > 1e-12:
                self.cfg.min_shares = float(new_min)
                changed = True
            self.market_min_order_size = float(min_sz)

        if changed:
            try:
                self.logger.info(
                    f"🧩 Market params from /book: tick {old_tick:.6f}->{float(self.cfg.tick):.6f} | "
                    f"min_order_size {old_min:.6f}->{float(self.cfg.min_shares):.6f}"
                )
            except Exception:
                pass
            self._apply_tick_dependent_params()

    def _depth_gate_accumulate(self, size: float, y_bid: float, n_bid: float, buf: float) -> Tuple[bool, str]:
        """Depth gate for MAKER accumulate: ensure hedge liquidity exists if one side fills."""
        if not getattr(self, "depth_gate_enabled", False):
            return True, "disabled"

        try:
            tick = float(self.cfg.tick) if float(self.cfg.tick) > 0 else 0.01
        except Exception:
            tick = 0.01

        need = float(size) * float(getattr(self, "depth_gate_min_mult", 1.0) or 1.0)
        if need <= 0:
            return True, "ok"

        no_lim = clamp(1.0 - float(y_bid) - float(buf), tick, 0.99)
        no_lim = round_down(no_lim, tick)

        yes_lim = clamp(1.0 - float(n_bid) - float(buf), tick, 0.99)
        yes_lim = round_down(yes_lim, tick)

        levels = int(getattr(self, "depth_gate_levels", 50) or 50)
        age = float(getattr(self, "depth_gate_max_age_seconds", 1.5) or 1.5)

        no_depth = self._cum_depth(self.no_asset, "asks", no_lim, max_levels=levels, max_age_seconds=age)
        if no_depth + 1e-9 < need:
            return False, f"no_ask_depth {no_depth:.2f} < need {need:.2f} @<={no_lim:.4f}"

        yes_depth = self._cum_depth(self.yes_asset, "asks", yes_lim, max_levels=levels, max_age_seconds=age)
        if yes_depth + 1e-9 < need:
            return False, f"yes_ask_depth {yes_depth:.2f} < need {need:.2f} @<={yes_lim:.4f}"

        return True, "ok"

    def _reconcile_state_from_balances(self, reason: str = "") -> bool:
        """Reconcile q_yes/q_no (and conservative costs) from CLOB balance API.

        This is a safety net for WS congestion/missed events. It intentionally uses conservative
        price assumptions when we detect missing BUY fills (cost is bumped using current ask).

        Returns True if state was modified.
        """
        if not getattr(self, "mismatch_reconcile_from_balance", False):
            return False

        try:
            b_yes = self._get_balance_allowance_conditional_cached(self.yes_asset, max_age_seconds=0.0)
            b_no = self._get_balance_allowance_conditional_cached(self.no_asset, max_age_seconds=0.0)
        except Exception:
            return False
        if not b_yes or not b_no:
            return False

        yes_bal = float(b_yes[0] or 0.0)
        no_bal = float(b_no[0] or 0.0)

        with self.state_lock:
            qy = float(self.state.get("q_yes", 0.0))
            qn = float(self.state.get("q_no", 0.0))
            cy = float(self.state.get("c_yes", 0.0))
            cn = float(self.state.get("c_no", 0.0))

        changed = False

        y_ba = self._best_bid_ask(self.yes_asset)
        n_ba = self._best_bid_ask(self.no_asset)
        y_ask = float(y_ba[1]) if y_ba else 0.0
        n_ask = float(n_ba[1]) if n_ba else 0.0
        y_bid = float(y_ba[0]) if y_ba else 0.0
        n_bid = float(n_ba[0]) if n_ba else 0.0

        try:
            tick = float(self.cfg.tick) if float(self.cfg.tick) > 0 else 0.01
        except Exception:
            tick = 0.01

        y_ask = clamp(y_ask if y_ask > 0 else 0.99, tick, 0.99)
        n_ask = clamp(n_ask if n_ask > 0 else 0.99, tick, 0.99)
        y_bid = clamp(y_bid if y_bid > 0 else tick, tick, 0.99)
        n_bid = clamp(n_bid if n_bid > 0 else tick, tick, 0.99)

        # YES
        if yes_bal > (qy + 1e-6):
            dq = yes_bal - qy
            cy += dq * y_ask
            qy = yes_bal
            changed = True
        elif yes_bal < (qy - 1e-6):
            dq = qy - yes_bal
            # Credit SELL proceeds conservatively but not absurdly.
            # 0.5 was too punitive and made MAKER appear to lose ~$4-5 on a $0.10 unwind.
            cy -= dq * y_bid * float(getattr(self, "reconcile_sell_credit_mult", 1.0) or 1.0)
            qy = yes_bal
            changed = True

        # NO
        if no_bal > (qn + 1e-6):
            dq = no_bal - qn
            cn += dq * n_ask
            qn = no_bal
            changed = True
        elif no_bal < (qn - 1e-6):
            dq = qn - no_bal
            cn -= dq * n_bid * float(getattr(self, "reconcile_sell_credit_mult", 1.0) or 1.0)
            qn = no_bal
            changed = True

        if not changed:
            return False

        cy = max(0.0, float(cy))
        cn = max(0.0, float(cn))

        with self.state_lock:
            self.state["q_yes"] = float(qy)
            self.state["q_no"] = float(qn)
            self.state["c_yes"] = float(cy)
            self.state["c_no"] = float(cn)
            save_state(self.state_file, self.state)

        try:
            tag = f" ({reason})" if reason else ""
            self.logger.warning(f"🩹 Reconciled state from balances{tag}: qYES={qy:.6f} qNO={qn:.6f} cost={cy+cn:.4f}")
        except Exception:
            pass

        try:
            self.position_update_event.set()
            self.wake_event.set()
        except Exception:
            pass

        return True

    def _chunked_unwind_heavy_leg(self, delta: float, reason: str) -> None:
        """Unwind heavy leg in smaller chunks (reduces slippage + avoids false unwinds under WS lag)."""
        try:
            tick = float(self.cfg.tick) if float(self.cfg.tick) > 0 else 0.01
        except Exception:
            tick = 0.01

        # Reconcile first (WS congestion safety)
        try:
            self._reconcile_state_from_balances(reason=f"unwind:{reason}")
        except Exception:
            pass

        with self.state_lock:
            qy = float(self.state.get("q_yes", 0.0))
            qn = float(self.state.get("q_no", 0.0))
        d = float(qy - qn)

        if abs(d) < float(self.cfg.min_shares):
            return

        min_int = max(1, int(math.ceil(float(self.cfg.min_shares) - 1e-12)))
        remaining = int(math.floor(abs(d) + 1e-12))
        if remaining < min_int:
            return

        # Chunk sizing (integer)
        try:
            chunk_cfg = float(getattr(self, "unwind_chunk_shares", float(self.cfg.min_shares)))
        except Exception:
            chunk_cfg = float(self.cfg.min_shares)
        chunk = int(math.floor(chunk_cfg + 1e-12))
        if chunk < min_int:
            chunk = min_int

        max_passes = int(getattr(self, "unwind_max_passes", 4) or 4)
        wait_s = float(getattr(self, "unwind_wait_after_order_seconds", 0.6) or 0.6)

        # Free balances: cancel any resting orders on heavy
        try:
            self.cancel_all_open_orders_local(reason=f"chunked unwind ({reason})")
            self._cancel_exchange_orders_for_assets([self.yes_asset, self.no_asset], reason=f"chunked unwind ({reason})")
        except Exception:
            pass

        for i in range(max_passes):
            if self.stop_event.is_set():
                return

            with self.state_lock:
                qy = float(self.state.get("q_yes", 0.0))
                qn = float(self.state.get("q_no", 0.0))
            d2 = float(qy - qn)
            if abs(d2) < float(self.cfg.min_shares):
                return

            heavy_asset = self.yes_asset if d2 > 0 else self.no_asset
            rem = int(math.floor(abs(d2) + 1e-12))
            if rem < min_int:
                return

            ba = self._best_bid_ask(heavy_asset)
            if not ba:
                return
            bid = float(ba[0] or 0.0)
            if bid <= 0:
                return

            slip_ticks = int(getattr(self, "maker_exposure_unwind_slippage_ticks", 0) or 0) + i
            px = clamp(round_down(bid - float(slip_ticks) * tick, tick), tick, 0.99)

            # Liquidity-aware size: sell <= cumulative bid depth down to px
            sell_int = min(rem, chunk)
            if getattr(self, "unwind_depth_gate_enabled", True):
                levels = int(getattr(self, "depth_gate_levels", 50) or 50)
                age = float(getattr(self, "depth_gate_max_age_seconds", 1.5) or 1.5)
                depth = self._cum_depth(heavy_asset, "bids", px, max_levels=levels, max_age_seconds=age)
                depth_int = int(math.floor(depth + 1e-9))
                depth_int = (depth_int // min_int) * min_int if depth_int >= min_int else 0
                if depth_int >= min_int:
                    sell_int = min(sell_int, depth_int)
                else:
                    # Not enough visible depth at this price; try again next pass (slightly lower px)
                    continue

            if sell_int < min_int:
                continue

            ot_name = str(getattr(self, "maker_exposure_unwind_order_type", self.hedge_taker_order_type)).upper().strip()
            self.logger.info(
                f"🔻 CHUNKED UNWIND ({reason}) heavy={str(heavy_asset)[-6:]} rem={rem} sell={sell_int} bid={bid:.3f} px={px:.3f} pass={i+1}/{max_passes} type={ot_name}"
            )

            # Fire taker sell
            self._taker_inflight_until = time.time() + max(0.75, wait_s)
            self._place_taker_ask_fak(heavy_asset, px, float(sell_int), order_type_name=ot_name)

            try:
                self.position_update_event.wait(timeout=wait_s)
                self.position_update_event.clear()
            except Exception:
                time.sleep(wait_s)

        return


# ---------------- Deterministic FSM ----------------
    def _fsm_set_state(self, new_state: str, reason: str = "") -> None:
        """Finite-state machine used to make behaviour deterministic.

        States:
          - BALANCED: may quote both legs (accumulate / pair-arb)
          - EXPOSED:  one leg filled; cancel quotes + hedge/unwind only (never quote both)
          - COOLDOWN: short pause after exposure resolved (prevents immediate re-fill churn)
        """
        new_state = str(new_state or "").upper().strip()
        if new_state not in ("BALANCED", "EXPOSED", "COOLDOWN"):
            new_state = "BALANCED"

        old_state = str(getattr(self, "fsm_state", "BALANCED") or "BALANCED").upper().strip()
        if old_state == new_state:
            return

        now = time.time()
        self.fsm_state = new_state
        self._fsm_state_enter_ts = now

        # Transition log (once per transition, low spam)
        r = f" reason={reason}" if reason else ""
        self.logger.info(f"🧭 FSM {old_state} -> {new_state}{r}")

        if new_state == "EXPOSED":
            self._fsm_exposed_since = now
            # Start unhedged timer if not already running
            if getattr(self, "_unhedged_since", None) is None:
                self._unhedged_since = now

            # Deterministic safety: on entry, yank all resting quotes (remove taker's free option)
            if bool(getattr(self, "fsm_force_cancel_on_exposed_entry", True)):
                self.cancel_all_open_orders_local(reason=f"fsm enter EXPOSED{r}")
                if bool(getattr(self, "fsm_cancel_exchange_on_exposed_entry", True)):
                    self._cancel_exchange_orders_for_assets([self.yes_asset, self.no_asset], reason="fsm enter EXPOSED")

            # Prevent immediate re-quote churn while the book stabilizes
            try:
                cd = float(getattr(self, "fsm_cooldown_seconds", 0.0) or 0.0)
                if cd > 0:
                    self._quote_pause_until = max(float(getattr(self, "_quote_pause_until", 0.0)), now + cd)
            except Exception:
                pass

        elif new_state == "COOLDOWN":
            # Short idle pause after becoming balanced again
            cd = float(getattr(self, "fsm_cooldown_seconds", 0.0) or 0.0)
            self._fsm_cooldown_until = now + max(0.0, cd)
            self._quote_pause_until = max(float(getattr(self, "_quote_pause_until", 0.0)), float(self._fsm_cooldown_until))

            # Best-effort: ensure no stale orders remain
            self.cancel_all_open_orders_local(reason="fsm COOLDOWN")
            if bool(getattr(self, "fsm_cancel_exchange_on_exposed_entry", True)):
                self._cancel_exchange_orders_for_assets([self.yes_asset, self.no_asset], reason="fsm COOLDOWN")

        elif new_state == "BALANCED":
            self._fsm_exposed_since = None
            # When fully balanced, clear unhedged timer (run loop will also clear it).
            self._unhedged_since = None
    def _apply_cfg_overrides_from_env(self) -> None:
        """Override selected BotConfig fields from env vars without touching the DB.

        Supported overrides (env -> cfg field):
          - MIN_SHARES -> cfg.min_shares
          - CLIP_SHARES -> cfg.clip_shares
          - ENTRY_EDGE_TICKS -> cfg.entry_edge_ticks
          - HEDGE_BUFFER_TICKS -> cfg.hedge_buffer_ticks
          - MAKER_BUFFER_TICKS -> cfg.maker_buffer_ticks
          - IMPROVE_BID_TICKS -> cfg.improve_bid_ticks
          - REPLACE_IF_PRICE_MOVES_TICKS -> cfg.replace_if_price_moves_ticks
          - STALE_SECONDS -> cfg.stale_seconds

        These are *runtime-only* overrides.
        """
        def _env_int(name: str, default: int) -> int:
            v = (os.getenv(name) or "").strip()
            if not v:
                return int(default)
            try:
                return int(float(v))
            except Exception:
                return int(default)

        def _env_float(name: str, default: float) -> float:
            v = (os.getenv(name) or "").strip()
            if not v:
                return float(default)
            try:
                return float(v)
            except Exception:
                return float(default)

        before = {
            "min_shares": float(getattr(self.cfg, "min_shares", 0.0)),
            "clip_shares": float(getattr(self.cfg, "clip_shares", 0.0)),
            "entry_edge_ticks": int(getattr(self.cfg, "entry_edge_ticks", 0)),
            "hedge_buffer_ticks": int(getattr(self.cfg, "hedge_buffer_ticks", 0)),
            "maker_buffer_ticks": int(getattr(self.cfg, "maker_buffer_ticks", 0)),
            "improve_bid_ticks": int(getattr(self.cfg, "improve_bid_ticks", 0)),
            "replace_if_price_moves_ticks": int(getattr(self.cfg, "replace_if_price_moves_ticks", 0)),
            "stale_seconds": int(getattr(self.cfg, "stale_seconds", 0)),
        }

        # Apply overrides
        self.cfg.min_shares = _env_float("MIN_SHARES", float(self.cfg.min_shares))
        self.cfg.clip_shares = _env_float("CLIP_SHARES", float(self.cfg.clip_shares))
        self.cfg.entry_edge_ticks = _env_int("ENTRY_EDGE_TICKS", int(self.cfg.entry_edge_ticks))
        self.cfg.hedge_buffer_ticks = _env_int("HEDGE_BUFFER_TICKS", int(self.cfg.hedge_buffer_ticks))
        self.cfg.maker_buffer_ticks = _env_int("MAKER_BUFFER_TICKS", int(self.cfg.maker_buffer_ticks))
        self.cfg.improve_bid_ticks = _env_int("IMPROVE_BID_TICKS", int(self.cfg.improve_bid_ticks))
        self.cfg.replace_if_price_moves_ticks = _env_int(
            "REPLACE_IF_PRICE_MOVES_TICKS",
            int(self.cfg.replace_if_price_moves_ticks),
        )
        self.cfg.stale_seconds = _env_int("STALE_SECONDS", int(self.cfg.stale_seconds))

        # Hard clamps (safety)
        self.cfg.min_shares = max(1.0, float(self.cfg.min_shares))
        self.cfg.clip_shares = max(float(self.cfg.min_shares), float(self.cfg.clip_shares))
        self.cfg.entry_edge_ticks = max(0, int(self.cfg.entry_edge_ticks))
        self.cfg.hedge_buffer_ticks = max(0, int(self.cfg.hedge_buffer_ticks))
        self.cfg.maker_buffer_ticks = max(0, int(self.cfg.maker_buffer_ticks))

        after = {
            "min_shares": float(getattr(self.cfg, "min_shares", 0.0)),
            "clip_shares": float(getattr(self.cfg, "clip_shares", 0.0)),
            "entry_edge_ticks": int(getattr(self.cfg, "entry_edge_ticks", 0)),
            "hedge_buffer_ticks": int(getattr(self.cfg, "hedge_buffer_ticks", 0)),
            "maker_buffer_ticks": int(getattr(self.cfg, "maker_buffer_ticks", 0)),
            "improve_bid_ticks": int(getattr(self.cfg, "improve_bid_ticks", 0)),
            "replace_if_price_moves_ticks": int(getattr(self.cfg, "replace_if_price_moves_ticks", 0)),
            "stale_seconds": int(getattr(self.cfg, "stale_seconds", 0)),
        }

        if before != after:
            try:
                if getattr(self, "debug_mode", False):
                    self.logger.warning(f"[CFG OVERRIDE] applied env overrides: before={before} after={after}")
            except Exception:
                pass
    def _accumulate_allowed(self) -> Tuple[bool, str]:
        """
        Gate ACCUMULATE mode:
          - Warmup seconds at start
          - Reasonable spreads on both legs
          - Midpoint parity sanity (mid_yes + mid_no ~ 1)
        """
        # warmup (only matters for balanced / accumulate)
        now = time.time()
        if now < (self.start_ts + self.warmup_seconds):
            return False, "warmup"

        y = self._best_bid_ask(self.yes_asset)
        n = self._best_bid_ask(self.no_asset)
        if not y or not n:
            return False, "missing_quotes"

        yb, ya = y
        nb, na = n
        if yb <= 0 or ya <= 0 or nb <= 0 or na <= 0:
            return False, "zero_bid_ask"

        # spreads
        spr_y_ticks = (ya - yb) / self.cfg.tick
        spr_n_ticks = (na - nb) / self.cfg.tick
        if spr_y_ticks > self.max_spread_ticks or spr_n_ticks > self.max_spread_ticks:
            return False, f"wide_spread(y={spr_y_ticks:.1f} n={spr_n_ticks:.1f})"

        # parity sanity
        mid_y = 0.5 * (yb + ya)
        mid_n = 0.5 * (nb + na)
        parity = (mid_y + mid_n)
        if abs(parity - 1.0) > self.parity_tolerance:
            return False, f"parity_off({parity:.3f})"

        return True, "ok"

    # ---------------- Paired quote safety (remove "free option") ----------------
    def _paired_quotes_active(self) -> bool:
        """True if we currently have BOTH YES and NO maker bids resting locally."""
        with self.state_lock:
            oo = dict(self.state.get("open_orders") or {})
        return (str(self.yes_asset) in oo) and (str(self.no_asset) in oo)

    def _quotes_invalidated(self) -> Tuple[bool, str]:
        """
        Detect whether any resting quote has become unsafe (unhedgeable) given current opposite-side ASK.
        If unsafe, we cancel quotes to avoid getting picked off (wrong side fills while the other runs away).
        """
        if not self.quote_invalidation_enabled:
            return False, "disabled"

        yq = self._best_bid_ask(self.yes_asset)
        nq = self._best_bid_ask(self.no_asset)
        if not yq or not nq:
            return False, "missing_quotes"

        _, y_ask = yq
        _, n_ask = nq
        if y_ask <= 0 or n_ask <= 0:
            return False, "zero_ask"

        buf = float(self.quote_invalidation_buffer_ticks) * self.cfg.tick

        with self.state_lock:
            oo = dict(self.state.get("open_orders") or {})
        y_o = oo.get(str(self.yes_asset))
        n_o = oo.get(str(self.no_asset))

        reasons = []

        # If our YES bid fills, we must be able to hedge by buying NO at NO_ask without locking a loss.
        if y_o:
            y_p = float((y_o or {}).get("price") or 0.0)
            if y_p > 0 and n_ask > (1.0 - y_p - buf):
                reasons.append(f"YES bid {y_p:.2f} + NO ask {n_ask:.2f} > {1.0 - buf:.2f}")

        # If our NO bid fills, we must be able to hedge by buying YES at YES_ask without locking a loss.
        if n_o:
            n_p = float((n_o or {}).get("price") or 0.0)
            if n_p > 0 and y_ask > (1.0 - n_p - buf):
                reasons.append(f"NO bid {n_p:.2f} + YES ask {y_ask:.2f} > {1.0 - buf:.2f}")

        # Optional: if BOTH resting quotes no longer meet the entry edge, pull them (avoid adverse selection).
        if y_o and n_o:
            y_p = float((y_o or {}).get("price") or 0.0)
            n_p = float((n_o or {}).get("price") or 0.0)
            effective_edge_ticks = max(int(self.cfg.entry_edge_ticks), int(self.min_entry_edge_ticks))
            entry_edge = effective_edge_ticks * self.cfg.tick
            if (y_p + n_p) > (1.0 - entry_edge):
                reasons.append(f"edge_lost(sum={y_p + n_p:.2f} > {1.0 - entry_edge:.2f})")

        if reasons:
            return True, "; ".join(reasons)

        return False, "ok"

    def _oco_after_maker_fill(self, filled_qty_total: float) -> bool:
        """
        OCO behavior: if we were quoting BOTH sides and we get a meaningful fill on either side,
        cancel BOTH quotes immediately to cap inventory growth and remove the taker's free option.
        """
        if not self.oco_on_fill:
            return False
        if float(filled_qty_total) < float(self.oco_min_fill_shares):
            return False

        if not self._paired_quotes_active():
            return False

        # Cancel both quotes (local) to prevent further fills while we transition into hedge mode.
        self.logger.info(f"🧹 OCO: cancel both quotes after fill (filled={filled_qty_total:.2f})")
        self.cancel_all_open_orders_local(reason="OCO after fill")
        # Also cancel any stray exchange orders we may have lost track of (prevents duplicate live orders).
        self._cancel_exchange_orders_for_assets([self.yes_asset, self.no_asset], reason="OCO after fill")
        self._quote_pause_until = time.time() + float(self.oco_pause_seconds)
        return True

    # ------------- user events (fills + order updates) -------------
    def _apply_fill(self, asset_id: str, price: float, filled: float, trade_key: str, side: str = "BUY") -> bool:
        """Apply a fill to our persistent state.

        This bot is primarily a BUY-maker bot, but we also use taker SELL in circuit-breaker flattening.
        We therefore track *net cash outflow* per asset:
          - BUY  : q += filled, c += price*filled
          - SELL : q -= filled, c -= price*filled

        This keeps locked_profit() meaningful as: min(q_yes,q_no) - (c_yes+c_no).
        """
        side_u = (side or "BUY").upper().strip()
        if side_u not in {"BUY", "SELL"}:
            return False

        try:
            filled = float(filled)
            price = float(price)
        except Exception:
            return False

        if filled <= 0 or price <= 0:
            return False

        sign = 1.0 if side_u == "BUY" else -1.0
        qty = sign * filled

        with self.state_lock:
            seen = set(self.state.get("seen_trade_keys", []))
            if trade_key in seen:
                return False

            # Front-load mode bookkeeping: if we start the market flat and get our first fill,
            # mark the first cycle as started. We'll mark it done once we return flat with >= min_shares paired.
            try:
                if not self._first_cycle_started:
                    qy0 = float(self.state.get("q_yes", 0.0))
                    qn0 = float(self.state.get("q_no", 0.0))
                    if qy0 == 0.0 and qn0 == 0.0:
                        self._first_cycle_started = True
            except Exception:
                pass

            if asset_id == self.yes_asset:
                self.state["q_yes"] = float(self.state.get("q_yes", 0.0)) + qty
                self.state["c_yes"] = float(self.state.get("c_yes", 0.0)) + (price * qty)
                # Clamp numerical drift
                if self.state["q_yes"] < 0:
                    self.state["q_yes"] = 0.0
                    # if we oversold due to rounding, also clamp cost to not explode
                    self.state["c_yes"] = float(self.state.get("c_yes", 0.0))
            elif asset_id == self.no_asset:
                self.state["q_no"] = float(self.state.get("q_no", 0.0)) + qty
                self.state["c_no"] = float(self.state.get("c_no", 0.0)) + (price * qty)
                if self.state["q_no"] < 0:
                    self.state["q_no"] = 0.0
                    self.state["c_no"] = float(self.state.get("c_no", 0.0))
            else:
                return False

            self.state.setdefault("seen_trade_keys", []).append(trade_key)
            save_state(self.state_file, self.state)

        # If first cycle started and we are now back to flat with a completed set, mark first cycle done.
        try:
            if self._first_cycle_started and not self._first_cycle_done:
                with self.state_lock:
                    qy = float(self.state.get("q_yes", 0.0))
                    qn = float(self.state.get("q_no", 0.0))
                if min(qy, qn) >= float(self.cfg.min_shares) and abs(qy - qn) < 0.25:
                    self._first_cycle_done = True
        except Exception:
            pass

        # Track last position change (used by circuit breaker debouncing)
        try:
            self._last_position_change_ts = time.time()
        except Exception:
            pass

        try:
            self.position_update_event.set()
        except Exception:
            pass
        try:
            self.wake_event.set()
        except Exception:
            pass

        return True

    # ---------------- Execution latency tracking (signal -> submit -> fill) ----------------
    def _lat_ms(self, t1: float, t0: float) -> Optional[int]:
        """Return milliseconds between t0->t1 (rounded)."""
        try:
            return int(round((float(t1) - float(t0)) * 1000.0))
        except Exception:
            return None

    def _set_active_signal_context(self, sig: SignalTrade, purpose: str = "SIGNAL") -> None:
        """Set a short-lived context so any orders submitted in this call chain can be linked to this signal."""
        if not bool(getattr(self, "exec_latency_log_enabled", False)):
            return
        try:
            ctx = {
                "signal_key": str(getattr(sig, "key", "") or ""),
                "signal_received_ts": float(getattr(sig, "received_ts", 0.0) or 0.0),
                "signal_provider": str(getattr(sig, "provider", "") or ""),
                "signal_market_slug": str(getattr(sig, "market_slug", "") or ""),
                "signal_direction": str(getattr(sig, "direction", "") or ""),
                "signal_confidence": float(getattr(sig, "confidence", 0.0) or 0.0),
                "signal_entry_price": float(getattr(sig, "entry_price", 0.0) or 0.0),
                "signal_event_timestamp": str(getattr(sig, "event_timestamp", "") or ""),
                "purpose": str(purpose or "SIGNAL"),
                "ctx_set_ts": float(time.time()),
            }
            if float(ctx.get("signal_received_ts") or 0.0) <= 0:
                ctx["signal_received_ts"] = float(time.time())
            with self._active_signal_ctx_lock:
                self._active_signal_ctx = ctx
        except Exception:
            # Never let telemetry break trading.
            pass

    def _clear_active_signal_context(self) -> None:
        try:
            with self._active_signal_ctx_lock:
                self._active_signal_ctx = None
        except Exception:
            pass

    def _get_active_signal_context(self) -> Optional[dict]:
        try:
            with self._active_signal_ctx_lock:
                return dict(self._active_signal_ctx) if isinstance(self._active_signal_ctx, dict) else None
        except Exception:
            return None


    def _utc_iso(self, ts: float) -> str:
        """UTC ISO timestamp helper (never raises)."""
        try:
            return datetime.fromtimestamp(float(ts), tz=timezone.utc).isoformat()
        except Exception:
            return ""

    def _should_file_log_submit_event(self, sig_ts: float) -> bool:
        """Whether to file-log a SUBMIT event.

        Default behavior:
          - log SUBMIT events for signal-driven orders
          - do NOT log all maker quote submits unless explicitly enabled
        """
        try:
            if not bool(getattr(self, "exec_latency_file_log_enabled", False)):
                return False
            if bool(getattr(self, "exec_latency_file_log_submit_all_events", False)):
                return True
            if float(sig_ts or 0.0) > 0 and bool(getattr(self, "exec_latency_file_log_submit_signal_events", True)):
                return True
            return False
        except Exception:
            return False

    def _latency_file_append(self, rec: dict) -> None:
        """Append a latency record to CSV/JSONL (best-effort)."""
        try:
            svc = getattr(self, "_exec_latency_log_service", None)
            if svc is None:
                return
            svc.append(rec if isinstance(rec, dict) else {"value": str(rec)})
        except Exception:
            # Never let telemetry break trading.
            return
    def _prune_order_exec_context_locked(self, now_ts: float) -> None:
        """Prune old per-order contexts. Caller must hold self._order_exec_ctx_lock."""
        try:
            ttl = float(getattr(self, "exec_latency_context_ttl_seconds", 0.0) or 0.0)
            maxn = int(getattr(self, "exec_latency_max_context_records", 0) or 0)
        except Exception:
            ttl = 0.0
            maxn = 0

        cutoff = (float(now_ts) - ttl) if ttl > 0 else None

        # TTL prune using insertion-order deque
        if cutoff is not None:
            while self._order_exec_ctx_order:
                ts, oid = self._order_exec_ctx_order[0]
                if float(ts) >= float(cutoff):
                    break
                self._order_exec_ctx_order.popleft()
                rec = self._order_exec_ctx.get(oid)
                # Delete only if the stored submit_ts matches the deque's ts (avoid clobbering newer rec)
                try:
                    if rec and abs(float(rec.get("order_submit_ts", 0.0)) - float(ts)) < 1e-6:
                        self._order_exec_ctx.pop(oid, None)
                except Exception:
                    self._order_exec_ctx.pop(oid, None)

        # Max-size prune
        if maxn and maxn > 0:
            while len(self._order_exec_ctx_order) > maxn:
                _ts, oid = self._order_exec_ctx_order.popleft()
                self._order_exec_ctx.pop(oid, None)

    def _track_order_execution_context(
        self,
        order_id: str,
        asset_id: Optional[str] = None,
        side: Optional[str] = None,
        px_limit: Optional[float] = None,
        size: Optional[float] = None,
        origin: str = "",
        # Optional timing hooks (used to measure decision->send->ack precisely)
        decision_ts: Optional[float] = None,
        post_start_ts: Optional[float] = None,
        post_end_ts: Optional[float] = None,
        decision_ns: Optional[int] = None,
        sign_start_ns: Optional[int] = None,
        sign_end_ns: Optional[int] = None,
        post_start_ns: Optional[int] = None,
        post_end_ns: Optional[int] = None,
    ) -> None:
        """Remember metadata about an order so we can compute latency when fills arrive.

        Important:
          - This is called *after* we receive an order_id (post_order ack). To measure true submit latency,
            callers should pass post_start_ts/post_end_ts captured around the HTTP submit.
        """
        if not bool(getattr(self, "exec_latency_log_enabled", False)):
            return

        oid = str(order_id or "").strip()
        if not oid:
            return

        def _diff_ms_ns(t0_ns: Optional[int], t1_ns: Optional[int]) -> Optional[int]:
            try:
                if t0_ns is None or t1_ns is None:
                    return None
                a = int(t0_ns)
                b = int(t1_ns)
                if b < a:
                    return None
                return int(round((b - a) / 1_000_000.0))
            except Exception:
                return None

        # Prefer caller-provided precise timestamps
        now_ts = float(time.time())
        submit_ts = float(post_end_ts) if (post_end_ts is not None and float(post_end_ts or 0.0) > 0.0) else now_ts
        send_ts = float(post_start_ts) if (post_start_ts is not None and float(post_start_ts or 0.0) > 0.0) else submit_ts
        decide_ts = float(decision_ts) if (decision_ts is not None and float(decision_ts or 0.0) > 0.0) else send_ts

        # Derived submit breakdown (ms)
        sign_ms = _diff_ms_ns(sign_start_ns, sign_end_ns)
        decide_to_send_ms = _diff_ms_ns(decision_ns, post_start_ns)
        send_to_ack_ms = _diff_ms_ns(post_start_ns, post_end_ns)
        decide_to_ack_ms = _diff_ms_ns(decision_ns, post_end_ns)

        if decide_to_send_ms is None:
            decide_to_send_ms = self._lat_ms(send_ts, decide_ts) if (send_ts > 0 and decide_ts > 0) else None
        if send_to_ack_ms is None:
            send_to_ack_ms = self._lat_ms(submit_ts, send_ts) if (submit_ts > 0 and send_ts > 0) else None
        if decide_to_ack_ms is None:
            decide_to_ack_ms = self._lat_ms(submit_ts, decide_ts) if (submit_ts > 0 and decide_ts > 0) else None

        sig_ctx = self._get_active_signal_context()

        rec: dict = {
            "order_id": oid,
            # Submit/ack time (when post_order returned with an order_id)
            "order_submit_ts": submit_ts,
            # Extra breakdown timestamps
            "decision_ts": decide_ts,
            "post_start_ts": send_ts,
            "post_end_ts": submit_ts,
            # Derived breakdown metrics
            "sign_ms": sign_ms,
            "decision_to_post_start_ms": decide_to_send_ms,
            "post_start_to_post_end_ms": send_to_ack_ms,
            "decision_to_post_end_ms": decide_to_ack_ms,
            # Order metadata
            "asset_id": str(asset_id or ""),
            "side": str(side or "").upper().strip(),
            "px_limit": float(px_limit) if px_limit is not None else None,
            "size": float(size) if size is not None else None,
            "origin": str(origin or ""),
            # Raw perf timestamps (for deeper offline analysis)
            "decision_ns": int(decision_ns) if decision_ns is not None else None,
            "sign_start_ns": int(sign_start_ns) if sign_start_ns is not None else None,
            "sign_end_ns": int(sign_end_ns) if sign_end_ns is not None else None,
            "post_start_ns": int(post_start_ns) if post_start_ns is not None else None,
            "post_end_ns": int(post_end_ns) if post_end_ns is not None else None,
        }

        sig_ts = 0.0
        sig_to_submit_ms = None
        sig_to_decide_ms = None
        sig_to_send_ms = None
        sig_to_ack_ms = None

        if isinstance(sig_ctx, dict) and float(sig_ctx.get("signal_received_ts", 0.0) or 0.0) > 0:
            sig_ts = float(sig_ctx.get("signal_received_ts") or 0.0)
            rec.update(
                {
                    "signal_key": str(sig_ctx.get("signal_key") or ""),
                    "signal_received_ts": sig_ts,
                    "signal_provider": str(sig_ctx.get("signal_provider") or ""),
                    "signal_market_slug": str(sig_ctx.get("signal_market_slug") or ""),
                    "signal_direction": str(sig_ctx.get("signal_direction") or ""),
                    "signal_confidence": float(sig_ctx.get("signal_confidence") or 0.0),
                    "signal_entry_price": float(sig_ctx.get("signal_entry_price") or 0.0),
                    "signal_event_timestamp": str(sig_ctx.get("signal_event_timestamp") or ""),
                    "signal_purpose": str(sig_ctx.get("purpose") or ""),
                }
            )

            # Signal->X metrics
            try:
                sig_to_decide_ms = self._lat_ms(decide_ts, sig_ts)
            except Exception:
                sig_to_decide_ms = None
            try:
                sig_to_send_ms = self._lat_ms(send_ts, sig_ts)
            except Exception:
                sig_to_send_ms = None
            try:
                sig_to_ack_ms = self._lat_ms(submit_ts, sig_ts)
            except Exception:
                sig_to_ack_ms = None
            sig_to_submit_ms = sig_to_ack_ms

            rec.update(
                {
                    "signal_to_decision_ms": sig_to_decide_ms,
                    "signal_to_post_start_ms": sig_to_send_ms,
                    "signal_to_post_end_ms": sig_to_ack_ms,
                }
            )

        try:
            with self._order_exec_ctx_lock:
                self._order_exec_ctx[oid] = rec
                self._order_exec_ctx_order.append((submit_ts, oid))
                self._prune_order_exec_context_locked(submit_ts)
        except Exception:
            return

        # Console: submit breakdown (decision->send->ack). Default ON for non-MAKER.
        try:
            if bool(getattr(self, "exec_latency_submit_breakdown_console", False)):
                em = str(getattr(self, "exec_mode", "") or "").upper().strip()
                allow_maker = bool(getattr(self, "exec_latency_submit_breakdown_console_maker", False))
                allow = True
                if em == "MAKER" and not allow_maker:
                    # In MAKER mode, avoid spam unless explicitly enabled, but still allow taker/hedge submits.
                    if not str(origin or "").upper().startswith("TAKER"):
                        allow = False
                if allow:
                    aid_tail = str(asset_id)[-6:] if asset_id else ""
                    self.logger.info(
                        f"[LATENCY][SUBMIT] decide->send={decide_to_send_ms}ms send->ack={send_to_ack_ms}ms decide->ack={decide_to_ack_ms}ms "
                        f"sign={sign_ms}ms oid={oid[:10]}.. asset={aid_tail} side={str(side or '').upper()} origin={origin}"
                    )
        except Exception:
            pass

        # Console: legacy signal->submit line (keep)
        try:
            if sig_ts > 0:
                if sig_to_submit_ms is None:
                    sig_to_submit_ms = self._lat_ms(submit_ts, sig_ts)
                if sig_to_submit_ms is not None:
                    aid_tail = str(asset_id)[-6:] if asset_id else ""
                    self.logger.info(
                        f"[LATENCY][SIGNAL->SUBMIT] {sig_to_submit_ms}ms key={rec.get('signal_key','')} dir={rec.get('signal_direction','')} "
                        f"oid={oid[:10]}.. asset={aid_tail} side={str(side or '').upper()} origin={origin}"
                    )
        except Exception:
            pass

        # File log (CSV/JSONL): SUBMIT event
        try:
            if self._should_file_log_submit_event(sig_ts=sig_ts):
                self._latency_file_append(
                    {
                        "event": "SUBMIT",
                        "ts": float(submit_ts),
                        "ts_utc": self._utc_iso(submit_ts),
                        "market_slug": str(getattr(self, "market_slug", "") or ""),
                        "exec_mode": str(getattr(self, "exec_mode", "") or ""),
                        "order_id": str(oid),
                        "asset_id": str(asset_id or ""),
                        "side": str(side or "").upper().strip(),
                        "origin": str(origin or ""),
                        "source": "ORDER_SUBMIT",
                        "price": float(px_limit) if px_limit is not None else None,
                        "qty": float(size) if size is not None else None,
                        "signal_key": str(rec.get("signal_key") or ""),
                        "signal_direction": str(rec.get("signal_direction") or ""),
                        "signal_provider": str(rec.get("signal_provider") or ""),
                        "signal_market_slug": str(rec.get("signal_market_slug") or ""),
                        "signal_received_ts": float(sig_ts) if float(sig_ts or 0.0) > 0 else None,
                        "decision_ts": float(decide_ts) if float(decide_ts or 0.0) > 0 else None,
                        "post_start_ts": float(send_ts) if float(send_ts or 0.0) > 0 else None,
                        "post_end_ts": float(submit_ts) if float(submit_ts or 0.0) > 0 else None,
                        "order_submit_ts": float(submit_ts),
                        "fill_ts": None,
                        "sign_ms": sign_ms,
                        "decision_to_post_start_ms": decide_to_send_ms,
                        "post_start_to_post_end_ms": send_to_ack_ms,
                        "decision_to_post_end_ms": decide_to_ack_ms,
                        "signal_to_decision_ms": sig_to_decide_ms,
                        "signal_to_post_start_ms": sig_to_send_ms,
                        "signal_to_post_end_ms": sig_to_ack_ms,
                        "signal_to_submit_ms": sig_to_submit_ms,
                        "signal_to_fill_ms": None,
                        "post_start_to_fill_ms": None,
                        "decision_to_fill_ms": None,
                        "submit_to_fill_ms": None,
                        "meta": rec,
                    }
                )
        except Exception:
            pass
    def _get_order_execution_context(self, order_id: str) -> Optional[dict]:
        oid = str(order_id or "").strip()
        if not oid:
            return None
        try:
            with self._order_exec_ctx_lock:
                rec = self._order_exec_ctx.get(oid)
            return dict(rec) if isinstance(rec, dict) else None
        except Exception:
            return None

    def _log_execution_latency_on_fill(
        self,
        order_id: Optional[str],
        asset_id: str,
        side: str,
        price: float,
        qty: float,
        source: str = "TRADE_EVT",
    ) -> None:
        """Log detailed latency breakdown when we observe an execution (fill).

        Breakdown terms:
          - decide->send: time from local decision to the first byte of the HTTP submit (post_order call start)
          - send->ack: HTTP submit RTT until we got an order_id back
          - ack->fill: delay from ack until we observed the WS fill/order event
          - send->fill / decide->fill: end-to-end from send/decision to observed fill
        """
        if not bool(getattr(self, "exec_latency_log_enabled", False)):
            return

        oid = str(order_id or "").strip()
        if not oid:
            return

        now_ts = float(time.time())
        now_ns = None
        try:
            now_ns = int(time.perf_counter_ns())
        except Exception:
            now_ns = None

        rec = self._get_order_execution_context(oid)

        # Primary timestamps
        submit_ts = 0.0  # ack time (post_order returned)
        send_ts = 0.0    # when we started post_order (HTTP)
        decide_ts = 0.0  # when we decided to place this order
        sig_ts = 0.0

        # Signal metadata
        sig_key = ""
        sig_dir = ""

        # Submit breakdown metrics (ms) if already computed
        decide_to_send_ms = None
        send_to_ack_ms = None
        decide_to_ack_ms = None
        sign_ms = None

        # Extra context
        origin_val = ""
        sig_provider_val = ""
        sig_market_slug_val = ""
        px_limit_val = None
        order_size_val = None

        # Pull from tracked order context
        if isinstance(rec, dict):
            try:
                submit_ts = float(rec.get("order_submit_ts", 0.0) or 0.0)
            except Exception:
                submit_ts = 0.0
            try:
                send_ts = float(rec.get("post_start_ts", 0.0) or 0.0)
            except Exception:
                send_ts = 0.0
            try:
                decide_ts = float(rec.get("decision_ts", 0.0) or 0.0)
            except Exception:
                decide_ts = 0.0
            try:
                sig_ts = float(rec.get("signal_received_ts", 0.0) or 0.0)
            except Exception:
                sig_ts = 0.0

            sig_key = str(rec.get("signal_key") or "")
            sig_dir = str(rec.get("signal_direction") or "")

            try:
                decide_to_send_ms = rec.get("decision_to_post_start_ms")
            except Exception:
                decide_to_send_ms = None
            try:
                send_to_ack_ms = rec.get("post_start_to_post_end_ms")
            except Exception:
                send_to_ack_ms = None
            try:
                decide_to_ack_ms = rec.get("decision_to_post_end_ms")
            except Exception:
                decide_to_ack_ms = None
            try:
                sign_ms = rec.get("sign_ms")
            except Exception:
                sign_ms = None

            try:
                origin_val = str(rec.get("origin") or "")
                sig_provider_val = str(rec.get("signal_provider") or "")
                sig_market_slug_val = str(rec.get("signal_market_slug") or "")
            except Exception:
                pass
            try:
                px_limit_val = float(rec.get("px_limit")) if rec.get("px_limit") is not None else None
            except Exception:
                px_limit_val = None
            try:
                order_size_val = float(rec.get("size")) if rec.get("size") is not None else None
            except Exception:
                order_size_val = None

        # If context was pruned or never tracked, try best-effort fallback from recent taker record.
        if submit_ts <= 0:
            try:
                with self._recent_taker_lock:
                    r2 = self._recent_taker_orders.get(oid)
                if isinstance(r2, dict):
                    submit_ts = float(r2.get("ts", 0.0) or 0.0)
                    if send_ts <= 0:
                        send_ts = float(r2.get("post_start_ts", 0.0) or 0.0)
                    if decide_ts <= 0:
                        decide_ts = float(r2.get("decision_ts", 0.0) or 0.0)
                    if sig_ts <= 0:
                        sig_ts = float(r2.get("signal_received_ts", 0.0) or 0.0)
                    if not sig_key:
                        sig_key = str(r2.get("signal_key") or "")
                    if not sig_dir:
                        sig_dir = str(r2.get("signal_direction") or "")
            except Exception:
                pass

        # If send_ts is missing, assume send ~= ack (still lets us compute ack->fill)
        if send_ts <= 0 and submit_ts > 0:
            send_ts = submit_ts
        if decide_ts <= 0 and send_ts > 0:
            decide_ts = send_ts

        # End-to-end metrics (ms)
        sig_to_fill_ms = self._lat_ms(now_ts, sig_ts) if sig_ts > 0 else None
        sig_to_submit_ms = self._lat_ms(submit_ts, sig_ts) if (sig_ts > 0 and submit_ts > 0) else None

        ack_to_fill_ms = self._lat_ms(now_ts, submit_ts) if submit_ts > 0 else None  # legacy submit->fill
        send_to_fill_ms = self._lat_ms(now_ts, send_ts) if send_ts > 0 else None
        decide_to_fill_ms = self._lat_ms(now_ts, decide_ts) if decide_ts > 0 else None

        # If breakdown ms missing, try to compute from raw ns stored in context (more stable than wall clock).
        try:
            if isinstance(rec, dict):
                def _diff_ms_ns(a, b):
                    try:
                        if a is None or b is None:
                            return None
                        a = int(a); b = int(b)
                        if b < a:
                            return None
                        return int(round((b - a) / 1_000_000.0))
                    except Exception:
                        return None

                if decide_to_send_ms is None:
                    decide_to_send_ms = _diff_ms_ns(rec.get("decision_ns"), rec.get("post_start_ns"))
                if send_to_ack_ms is None:
                    send_to_ack_ms = _diff_ms_ns(rec.get("post_start_ns"), rec.get("post_end_ns"))
                if decide_to_ack_ms is None:
                    decide_to_ack_ms = _diff_ms_ns(rec.get("decision_ns"), rec.get("post_end_ns"))
        except Exception:
            pass

        # Derive signal->decision/send/ack if possible
        sig_to_decide_ms = self._lat_ms(decide_ts, sig_ts) if (sig_ts > 0 and decide_ts > 0) else None
        sig_to_send_ms = self._lat_ms(send_ts, sig_ts) if (sig_ts > 0 and send_ts > 0) else None
        sig_to_ack_ms = self._lat_ms(submit_ts, sig_ts) if (sig_ts > 0 and submit_ts > 0) else None

        missing_context = False
        if (sig_ts <= 0) and (submit_ts <= 0) and (send_ts <= 0) and (decide_ts <= 0):
            missing_context = True

        # Console log
        try:
            aid_tail = str(asset_id)[-6:] if asset_id else ""
            parts = []

            # Breakdown first (what you need to optimize execution)
            if decide_to_send_ms is not None:
                parts.append(f"decide->send={decide_to_send_ms}ms")
            if send_to_ack_ms is not None:
                parts.append(f"send->ack={send_to_ack_ms}ms")
            if ack_to_fill_ms is not None:
                parts.append(f"ack->fill={ack_to_fill_ms}ms")
            if send_to_fill_ms is not None:
                parts.append(f"send->fill={send_to_fill_ms}ms")
            if decide_to_fill_ms is not None:
                parts.append(f"decide->fill={decide_to_fill_ms}ms")

            # Signal end-to-end (if present)
            if sig_to_decide_ms is not None:
                parts.append(f"signal->decide={sig_to_decide_ms}ms")
            if sig_to_send_ms is not None:
                parts.append(f"signal->send={sig_to_send_ms}ms")
            if sig_to_ack_ms is not None:
                parts.append(f"signal->ack={sig_to_ack_ms}ms")
            if sig_to_fill_ms is not None:
                parts.append(f"signal->fill={sig_to_fill_ms}ms")

            if not parts:
                parts.append("no_timing_ctx")

            self.logger.info(
                f"[LATENCY][FILL] {' '.join(parts)} key={sig_key or 'N/A'} dir={sig_dir or ''} "
                f"oid={oid[:10]}.. asset={aid_tail} side={str(side or '').upper()} px={float(price):.4f} qty={float(qty):.4f} src={source}"
            )
        except Exception:
            pass

        # File log (CSV/JSONL): FILL event (always best-effort)
        try:
            self._latency_file_append(
                {
                    "event": "FILL",
                    "ts": float(now_ts),
                    "ts_utc": self._utc_iso(now_ts),
                    "market_slug": str(getattr(self, "market_slug", "") or ""),
                    "exec_mode": str(getattr(self, "exec_mode", "") or ""),
                    "order_id": str(oid),
                    "asset_id": str(asset_id or ""),
                    "side": str(side or "").upper().strip(),
                    "origin": str(origin_val or ""),
                    "source": str(source or ""),
                    "price": float(price) if price is not None else None,
                    "qty": float(qty) if qty is not None else None,
                    "signal_key": str(sig_key or ""),
                    "signal_direction": str(sig_dir or ""),
                    "signal_provider": str(sig_provider_val or ""),
                    "signal_market_slug": str(sig_market_slug_val or ""),
                    "signal_received_ts": float(sig_ts) if float(sig_ts or 0.0) > 0 else None,
                    "decision_ts": float(decide_ts) if float(decide_ts or 0.0) > 0 else None,
                    "post_start_ts": float(send_ts) if float(send_ts or 0.0) > 0 else None,
                    "post_end_ts": float(submit_ts) if float(submit_ts or 0.0) > 0 else None,
                    "order_submit_ts": float(submit_ts) if float(submit_ts or 0.0) > 0 else None,
                    "fill_ts": float(now_ts),
                    "sign_ms": sign_ms,
                    "decision_to_post_start_ms": decide_to_send_ms,
                    "post_start_to_post_end_ms": send_to_ack_ms,
                    "decision_to_post_end_ms": decide_to_ack_ms,
                    "signal_to_decision_ms": sig_to_decide_ms,
                    "signal_to_post_start_ms": sig_to_send_ms,
                    "signal_to_post_end_ms": sig_to_ack_ms,
                    "signal_to_submit_ms": sig_to_submit_ms,
                    "signal_to_fill_ms": sig_to_fill_ms,
                    "post_start_to_fill_ms": send_to_fill_ms,
                    "decision_to_fill_ms": decide_to_fill_ms,
                    "submit_to_fill_ms": ack_to_fill_ms,
                    "px_limit": px_limit_val,
                    "order_size": order_size_val,
                    "missing_context": bool(missing_context),
                }
            )
        except Exception:
            pass
    def _remember_taker_order(self, order_id: str, asset_id: str, size: float, px_limit: float, side: str = "BUY") -> None:
        """Remember a taker order we sent.

        Stores intended asset, intended size, and limit price so we can:
          - attribute WS trade events to the correct asset
          - cap fills to the size we actually sent (prevents qYES/qNO drift)
        """
        if not order_id:
            return
        now = time.time()
        try:
            size_f = float(size)
        except Exception:
            size_f = 0.0
        try:
            px_f = float(px_limit)
        except Exception:
            px_f = 0.0

        with self._recent_taker_lock:
            rec = {
                "ts": now,
                "asset_id": str(asset_id),
                "size": size_f,
                "px_limit": px_f,
                "applied": 0.0,
                "side": str(side or "BUY").upper().strip(),
            }

            # Attach signal metadata (if we're currently acting on a signal)
            try:
                sig_ctx = self._get_active_signal_context()
                if isinstance(sig_ctx, dict) and float(sig_ctx.get("signal_received_ts", 0.0) or 0.0) > 0:
                    rec["signal_key"] = str(sig_ctx.get("signal_key") or "")
                    rec["signal_received_ts"] = float(sig_ctx.get("signal_received_ts") or 0.0)
                    rec["signal_direction"] = str(sig_ctx.get("signal_direction") or "")
            except Exception:
                pass

            self._recent_taker_orders[order_id] = rec
            # prune old
            cutoff = now - float(self.taker_order_ttl_seconds)
            self._recent_taker_orders = {
                k: v
                for k, v in self._recent_taker_orders.items()
                if float((v or {}).get("ts", 0.0)) >= cutoff
            }

    def _is_recent_taker_order(self, order_id: str) -> bool:
        if not order_id:
            return False
        now = time.time()
        with self._recent_taker_lock:
            rec = self._recent_taker_orders.get(order_id)
        if not rec:
            return False
        ts = float(rec.get("ts", 0.0))
        return (now - ts) <= float(self.taker_order_ttl_seconds)

    def _has_pending_taker_order(self, side: str, asset_id: Optional[str] = None) -> bool:
        """True if we still expect fills/cancel for a recent taker order (prevents duplicate hedges/unwinds)."""
        side_u = str(side or "").upper().strip()
        now = time.time()
        with self._recent_taker_lock:
            items = list(self._recent_taker_orders.items())
        for _oid, rec in items:
            try:
                if side_u and str(rec.get("side", "")).upper() != side_u:
                    continue
                if asset_id is not None and str(rec.get("asset_id")) != str(asset_id):
                    continue
                ts = float(rec.get("ts", 0.0))
                if (now - ts) > float(self.taker_order_ttl_seconds):
                    continue
                size = float(rec.get("size", 0.0))
                applied = float(rec.get("applied", 0.0))
                if (size - applied) > 1e-6:
                    return True
            except Exception:
                continue
        return False


    def _pending_taker_notional_usd(
        self,
        side: Optional[str] = None,
        asset_id: Optional[str] = None,
        max_age_seconds: float = 0.0,
    ) -> float:
        """Sum notional of outstanding recent taker orders: (remaining_size * limit_price).

        This is primarily used by SNIPER mode to enforce SNIPER_MAX_NOTIONAL_USD *strictly* even when
        multiple entry orders are in-flight (or when WS fill events are slightly delayed).

        Args:
            side: Optional 'BUY'/'SELL' filter.
            asset_id: Optional token_id filter.
            max_age_seconds: If > 0, only include orders younger than this many seconds.
                This is important for IOC-like orders (FOK/FAK): if we miss a cancellation event,
                we don't want to block the bot for the full TAKER_ORDER_TTL_SECONDS.
        """
        now = time.time()
        side_u = str(side).upper().strip() if side else ""
        max_age = float(max_age_seconds or 0.0)

        total = 0.0
        with self._recent_taker_lock:
            items = list(self._recent_taker_orders.items())

        for _oid, rec in items:
            try:
                if side_u and str(rec.get("side", "")).upper().strip() != side_u:
                    continue
                if asset_id is not None and str(rec.get("asset_id")) != str(asset_id):
                    continue

                ts = float(rec.get("ts", 0.0) or 0.0)
                age = now - ts
                if age < 0:
                    age = 0.0

                # Hard TTL guard
                if age > float(self.taker_order_ttl_seconds):
                    continue

                # Optional sniper-style pending reservation window
                if max_age > 0.0 and age > max_age:
                    continue

                size = float(rec.get("size", 0.0) or 0.0)
                applied = float(rec.get("applied", 0.0) or 0.0)
                remaining = max(0.0, size - applied)
                if remaining <= 1e-9:
                    continue

                px = float(rec.get("px_limit", 0.0) or 0.0)
                if px <= 0:
                    continue

                total += remaining * px
            except Exception:
                continue

        return float(total)

    def _has_pending_taker_order_recent(self, side: str, asset_id: Optional[str] = None, max_age_seconds: float = 0.0) -> bool:
        """True if we have a *recent* taker order that still has un-applied remaining size.

        We intentionally gate this by age (max_age_seconds) rather than the longer TTL used by
        _has_pending_taker_order(), because IOC-style taker orders (FOK/FAK) can be accepted and
        then killed quickly, and we don't want missed order-events to block trading for minutes.
        """

        try:
            side_u = (side or "").upper().strip()
            if side_u not in {"BUY", "SELL"}:
                return False
            now = time.time()

            with self._recent_taker_lock:
                items = list(self._recent_taker_orders.items())

            for _oid, rec in items:
                try:
                    if (rec.get("side") or "").upper() != side_u:
                        continue
                    if asset_id and str(rec.get("asset_id")) != str(asset_id):
                        continue
                    ts = float(rec.get("ts") or 0.0)
                    if max_age_seconds and (now - ts) > float(max_age_seconds):
                        continue
                    size = float(rec.get("size") or 0.0)
                    applied = float(rec.get("applied") or 0.0)
                    if (size - applied) > 1e-9:
                        return True
                except Exception:
                    continue

            return False
        except Exception:
            return False

    def _get_balance_allowance_conditional_cached(self, token_id: str, max_age_seconds: float = 2.0) -> Optional[Tuple[float, float]]:
        """Fetch (balance_shares, allowance_shares) for a CONDITIONAL token_id, with a small local cache.

        Polymarket conditional token balances/allowances are typically returned in base-units
        (micro-shares where 1 share == 1_000_000 units). The sniper strategy operates in "share" units,
        so we normalize into shares here.

        Override (rare): if your client already returns *share* units, set:
            POLY_CONDITIONAL_UNITS_PER_SHARE=1
        """

        tid = str(token_id)
        now = time.time()

        units_per_share = float(os.getenv("POLY_CONDITIONAL_UNITS_PER_SHARE", "1000000") or 1000000)
        if units_per_share <= 0:
            units_per_share = 1000000.0

        # Fast cache
        try:
            with self._ba_cache_lock:
                rec = self._ba_cache.get(tid)
            if rec and (now - float(rec[0])) <= float(max_age_seconds):
                return (float(rec[1]), float(rec[2]))
        except Exception:
            pass

        try:
            # Keep the client's internal cache updated too (some versions use this for balance checks)
            try:
                self.client.update_balance_allowance(BalanceAllowanceParams(asset_type=AssetType.CONDITIONAL, token_id=tid))
            except Exception:
                pass

            resp = self.client.get_balance_allowance(BalanceAllowanceParams(asset_type=AssetType.CONDITIONAL, token_id=tid))
            bal_raw = float(resp.get("balance", 0) or 0)
            allo_raw = float(resp.get("allowance", 0) or 0)

            # Normalize base-units -> shares
            bal = bal_raw / units_per_share
            allo = allo_raw / units_per_share

            try:
                with self._ba_cache_lock:
                    self._ba_cache[tid] = (now, bal, allo)
            except Exception:
                pass
            return (bal, allo)
        except Exception as e:
            try:
                self.logger.warning(f"[BAL] get_balance_allowance failed token={tid[-6:]} err={e}")
            except Exception:
                pass
            return None


    def _taker_order_fallback_on_order_event(self, msg: dict) -> None:
        """Best-effort: apply taker fills using user WS 'order' events if trade events are missed.

        Why: we've seen cases where SELL fills were not observed via trade events, causing the bot to
        think it's still exposed and retry hedges/unwinds (leading to balance errors). Order events
        include size_matched, which we can use as an idempotent fallback.

        """




        if not getattr(self, "taker_fill_fallback_from_order_events", True):
            return

        oid = msg.get("id") or msg.get("order_id") or msg.get("orderID") or msg.get("orderId")
        if not oid:
            return

        # Only care about taker orders we've recently submitted
        with self._recent_taker_lock:
            rec = self._recent_taker_orders.get(oid)
        if not rec:
            return

        def _f(x) -> float:
            try:
                return float(x)
            except Exception:
                return 0.0

        matched_total = _f(msg.get("size_matched"))
        if matched_total <= 0:
            # Some payloads may use alternative keys
            matched_total = _f(msg.get("matched_size")) or _f(msg.get("filled_size"))

        size_limit = _f(rec.get("size"))
        if size_limit > 0:
            matched_total = min(matched_total, size_limit)

        applied_so_far = _f(rec.get("applied"))
        if matched_total < applied_so_far:
            matched_total = applied_so_far

        inc = matched_total - applied_so_far

        # Determine completion hints
        typ = str(msg.get("type") or "").upper()
        status = str(msg.get("status") or "").upper()
        done_hint = typ in {"CANCELLATION", "CANCELLED", "CANCELED", "REJECTED", "EXPIRED", "KILLED", "KILL"} or status in {"CANCELLED", "CANCELED", "REJECTED", "EXPIRED", "FAILED"}

        # Use order price as a conservative fallback (trade event would be more accurate)
        px = _f(msg.get("price")) or _f(rec.get("px_limit"))
        side = str(rec.get("side") or msg.get("side") or "BUY").upper()
        asset_id = str(rec.get("asset_id") or msg.get("asset_id") or "")

        # If we observed new matched size, apply incremental fill.
        if inc > 1e-9 and px > 0 and asset_id:
            # Update applied under lock first (idempotent)
            with self._recent_taker_lock:
                rec2 = self._recent_taker_orders.get(oid) or rec
                rec2["applied"] = float(matched_total)
                self._recent_taker_orders[oid] = rec2
                fully_done = size_limit > 0 and float(matched_total) >= (size_limit - 1e-6)
                if fully_done:
                    self._recent_taker_orders.pop(oid, None)

            try:
                self.logger.info(
                    f"[FILL][ORDER_EVT] {side} asset={asset_id[-6:]} price={px:.2f} qty={inc:.2f} order={str(oid)[:10]}.. matched_total={matched_total:.2f}"
                )
            except Exception:
                pass

            # Latency telemetry: signal->fill / submit->fill (best-effort)
            try:
                self._log_execution_latency_on_fill(
                    order_id=str(oid),
                    asset_id=str(asset_id),
                    side=str(side),
                    price=float(px),
                    qty=float(inc),
                    source="ORDER_EVT",
                )
            except Exception:
                pass

            self._apply_fill(asset_id, float(px), float(inc), trade_key=f"order_evt:{oid}:{matched_total:.8f}", side=side)
            return

        # No new matched size. If the order is definitively done (canceled/rejected), drop it so we can retry safely.
        if done_hint:
            with self._recent_taker_lock:
                self._recent_taker_orders.pop(oid, None)

    def _handle_user_trade_event(self, msg: dict):
        # User trade events can arrive multiple times for the same fill as they progress through
        # MATCHED -> MINED -> CONFIRMED. We want to apply the fill exactly once.
        if msg.get("event_type") != "trade":
            return

        status = str(msg.get("status") or "").upper()
        if status not in {"MATCHED", "MINED", "CONFIRMED"}:
            return

        trade_id = msg.get("id") or msg.get("trade_id") or msg.get("tradeId") or msg.get("tradeID")
        # Build a stable, status-independent key so we don't double-count status updates.
        base_key = str(trade_id) if trade_id else None

        trader_side = str(msg.get("trader_side") or msg.get("traderSide") or "").upper()
        maker_orders = msg.get("maker_orders") or msg.get("makerOrders")

        wallet = (self.wallet_address or "").lower()

        def _f(x) -> float:
            try:
                return float(x)
            except Exception:
                return 0.0

        # Detect if we are the MAKER by looking for our wallet in maker_orders
        maker_leg = None
        if isinstance(maker_orders, list) and maker_orders and wallet:
            for mo in maker_orders:
                try:
                    mo_maker_addr = str(mo.get("maker_address") or mo.get("makerAddress") or "").lower()
                    if mo_maker_addr == wallet:
                        maker_leg = mo
                        break
                except Exception:
                    continue

        if maker_leg is None and isinstance(maker_orders, list) and maker_orders and self.user_api_key:
            # Fallback for older payloads that key off owner id
            for mo in maker_orders:
                try:
                    if str(mo.get("owner") or "") == self.user_api_key:
                        maker_leg = mo
                        break
                except Exception:
                    continue

        # -----------------------------
        # CASE A: We are MAKER
        # -----------------------------
        if maker_leg is not None or trader_side == "MAKER":
            if maker_leg is None and isinstance(maker_orders, list) and maker_orders:
                maker_leg = maker_orders[0]  # best-effort fallback

            if not isinstance(maker_leg, dict):
                return

            maker_oid = str(maker_leg.get("order_id") or maker_leg.get("orderId") or "")
            token_id = str(maker_leg.get("asset_id") or maker_leg.get("assetId") or "")
            side = str(maker_leg.get("side") or "").upper()
            qty = _f(maker_leg.get("matched_amount") or maker_leg.get("matchedAmount"))
            px_exec = _f(maker_leg.get("price"))

            if not token_id or side not in {"BUY", "SELL"} or qty <= 0 or px_exec <= 0:
                return

            trade_key = (
                f"{base_key}:maker"
                if base_key
                else f"trade_fallback:maker:{maker_oid}:{token_id}:{side}:{qty:.8f}:{px_exec:.8f}"
            )

            applied = self._apply_fill(
                asset_id=token_id,
                price=px_exec,
                filled=qty,
                trade_key=trade_key,
                side=side,
            )
            if applied:
                self.logger.info(
                    f"[FILL] MAKER asset={token_id[-6:]} price={px_exec:.2f} qty={qty:.6f} maker_order={maker_oid}"
                )
                # Latency telemetry: signal->fill / submit->fill (best-effort)
                try:
                    self._log_execution_latency_on_fill(
                        order_id=str(maker_oid),
                        asset_id=str(token_id),
                        side=str(side),
                        price=float(px_exec),
                        qty=float(qty),
                        source="TRADE_EVT_MAKER",
                    )
                except Exception:
                    pass
            return

        # -----------------------------
        # CASE B: Assume we are TAKER
        # -----------------------------
        taker_oid = str(msg.get("taker_order_id") or msg.get("takerOrderId") or msg.get("taker_orderId") or "")
        token_id = str(msg.get("asset_id") or msg.get("assetId") or msg.get("token_id") or msg.get("tokenId") or "")
        side = str(msg.get("side") or "").upper()
        qty = _f(msg.get("size") or msg.get("matched_amount") or msg.get("amount"))
        px_exec = _f(msg.get("price"))

        if (not token_id) or side not in {"BUY", "SELL"} or qty <= 0 or px_exec <= 0:
            # Best-effort fallback:
            # - Recover token_id/side from our recent taker order record (if we have it)
            # - Use maker_orders VWAP; if the maker leg is on the opposite outcome (binary mint),
            #   convert price via (1 - vwap) to get the taker-side executed price.
            if (not token_id or side not in {"BUY", "SELL"}) and taker_oid:
                rec = self._recent_taker_orders.get(taker_oid)
                if isinstance(rec, dict):
                    if not token_id:
                        token_id = str(rec.get("asset_id") or "")
                    if side not in {"BUY", "SELL"}:
                        side = str(rec.get("side") or side).upper()

            if not (isinstance(maker_orders, list) and maker_orders):
                return

            total_qty = 0.0
            total_cost = 0.0
            first_asset = ""
            for idx, mo in enumerate(maker_orders):
                if idx == 0:
                    first_asset = str(mo.get("asset_id") or mo.get("assetId") or "")
                mo_qty = _f(mo.get("matched_amount") or mo.get("matchedAmount"))
                mo_px = _f(mo.get("price"))
                total_qty += mo_qty
                total_cost += mo_qty * mo_px

            if total_qty <= 0:
                return

            vwap = (total_cost / total_qty) if total_cost > 0 else 0.0

            # If maker_orders are on the opposite outcome, convert via complement price.
            if token_id and first_asset and first_asset != token_id:
                px_exec = 1.0 - vwap
            else:
                px_exec = vwap

            try:
                tick = float(self.cfg.tick) if float(self.cfg.tick) > 0 else 0.01
            except Exception:
                tick = 0.01
            px_exec = clamp(px_exec, tick, 0.99)

            qty = total_qty

        trade_key = (
            f"{base_key}:taker"
            if base_key
            else f"trade_fallback:taker:{taker_oid}:{token_id}:{side}:{qty:.8f}:{px_exec:.8f}"
        )

        applied = self._apply_fill(
            asset_id=token_id,
            price=px_exec,
            filled=qty,
            trade_key=trade_key,
            side=side,
        )
        if not applied:
            return

        # Update pending taker order record (if present) – tolerate FOK/FAK overfills
        if taker_oid:
            rec = self._recent_taker_orders.get(taker_oid)
            if rec is not None:
                rec["applied"] = float(rec.get("applied", 0.0)) + float(qty)
                rec["ts"] = time.time()
                self._recent_taker_orders[taker_oid] = rec

                if rec["applied"] >= float(rec.get("size", 0.0)) - 1e-9:
                    self._recent_taker_orders.pop(taker_oid, None)

        self.logger.info(
            f"[FILL] TAKER asset={token_id[-6:]} price={px_exec:.2f} qty={qty:.6f} taker_order={taker_oid}"
        )
        # Latency telemetry: signal->fill / submit->fill (best-effort)
        try:
            self._log_execution_latency_on_fill(
                order_id=str(taker_oid),
                asset_id=str(token_id),
                side=str(side),
                price=float(px_exec),
                qty=float(qty),
                source="TRADE_EVT_TAKER",
            )
        except Exception:
            pass
        return

    def _handle_user_order_event(self, msg: dict):
        if msg.get("event_type") != "order":
            return

        # ---- Taker fill fallback (BUY/SELL) ----
        # Trade events are ideal, but we've seen cases where SELL fills were not observed.
        # Order events contain size_matched and are a reliable fallback to keep positions in sync.
        try:
            self._taker_order_fallback_on_order_event(msg)
        except Exception as e:
            if getattr(self, "DEBUG_MODE", False):
                self.logger.warning(f"[DBG] taker order fallback error: {e}")

        # ---- Maker open-order tracking (BUY only) ----
        asset_id = str(msg.get("asset_id") or "")
        if asset_id not in (self.yes_asset, self.no_asset):
            return

        side = (msg.get("side") or "").upper()
        if side != "BUY":
            return

        oid = msg.get("id") or msg.get("order_id") or msg.get("orderID") or msg.get("orderId")
        if not oid:
            return

        typ = (msg.get("type") or "").upper()
        price = float(msg.get("price") or 0)
        original = float(msg.get("original_size") or 0)
        matched = float(msg.get("size_matched") or 0)
        remaining = max(0.0, original - matched) if original else 0.0

        with self.state_lock:
            if typ == "CANCELLATION":
                oo = self.state["open_orders"].get(asset_id)
                if oo and oo.get("order_id") == oid:
                    self.state["open_orders"].pop(asset_id, None)
                    save_state(self.state_file, self.state)
                return

            if remaining <= 0:
                oo = self.state["open_orders"].get(asset_id)
                if oo and oo.get("order_id") == oid:
                    self.state["open_orders"].pop(asset_id, None)
                    save_state(self.state_file, self.state)
                return

            # NOTE: This may briefly track non-GTC orders too, but they generally clear quickly.
            self.state["open_orders"][asset_id] = {
                "order_id": oid,
                "price": price,
                "size": remaining,
                "ts": time.time(),
            }
            save_state(self.state_file, self.state)

    def _handle_user_event(self, msg: dict):
        et = msg.get("event_type")
        if et == "trade":
            self._handle_user_trade_event(msg)
        elif et == "order":
            self._handle_user_order_event(msg)

    def on_user_message(self, ws, message: str):
        try:
            payload = json.loads(message)
        except Exception:
            return
        if isinstance(payload, list):
            for item in payload:
                if isinstance(item, dict):
                    self._handle_user_event(item)
        elif isinstance(payload, dict):
            self._handle_user_event(payload)

    # ---------------- Order management ----------------
    def _cancel(self, order_id: str):
        if not order_id:
            return
        if self.cfg.dry_run:
            self.logger.info(f"[DRY] cancel {order_id}")
            return
        try:
            self.client.cancel(order_id)
            return
        except Exception:
            pass
        try:
            self.client.cancel_order(order_id)
        except Exception as e:
            self.logger.error(f"Cancel failed: {e}")

    def _cancel_open_order_local(self, asset_id: str, reason: str = ""):
        with self.state_lock:
            oo = self.state.get("open_orders", {}).get(asset_id)
        if not oo:
            return
        oid = oo.get("order_id")
        if not oid:
            return
        if reason:
            self.logger.info(f"🧹 Cancel {asset_id[-6:]} ({reason})")
        self._cancel(oid)
        with self.state_lock:
            self.state["open_orders"].pop(asset_id, None)
            save_state(self.state_file, self.state)

    def cancel_all_open_orders_local(self, reason: str = ""):
        with self.state_lock:
            oo = dict(self.state.get("open_orders") or {})
        if not oo:
            return
        if reason:
            self.logger.info(f"🧹 Cancel local open orders: {reason}")
        for aid, row in oo.items():
            oid = (row or {}).get("order_id")
            if oid:
                self._cancel(oid)
        with self.state_lock:
            self.state["open_orders"] = {}
            save_state(self.state_file, self.state)



    def cancel_all_open_orders_local_except(self, keep_asset_id: str, reason: str = ""):
        """
        Cancel all locally-tracked open orders except one asset (typically the missing-side hedge order).
        This prevents us from accidentally canceling the protective hedge we just placed.
        """
        with self.state_lock:
            oo = dict(self.state.get("open_orders") or {})
        if not oo:
            return

        # If the only tracked order is the one we're keeping, do nothing (avoid log spam).
        if len(oo) == 1 and any(str(k) == str(keep_asset_id) for k in oo.keys()):
            return

        to_cancel = []
        for aid, row in oo.items():
            if str(aid) == str(keep_asset_id):
                continue
            oid = (row or {}).get("order_id")
            if oid:
                to_cancel.append(oid)

        if not to_cancel:
            return

        if reason:
            tail = str(keep_asset_id)[-6:] if keep_asset_id is not None else str(keep_asset_id)
            self.logger.info(f"🧹 Cancel local open orders (except {tail}): {reason}")

        for oid in to_cancel:
            self._cancel(oid)

        with self.state_lock:
            kept = self.state.get("open_orders", {}).get(keep_asset_id)
            self.state["open_orders"] = {}
            if kept:
                self.state["open_orders"][keep_asset_id] = kept
            save_state(self.state_file, self.state)


    def cancel_all_orders_exchange(self, reason: str = ""):
        if reason:
            self.logger.info(f"🧹 Cancel-all (exchange): {reason}")
        orders = []
        try:
            orders = self.client.get_orders(OpenOrderParams(market=self.condition_id)) or []
        except Exception as e:
            self.logger.error(f"get_orders failed during cancel_all: {e}")
        for o in orders:
            oid = o.get("id") or o.get("order_id")
            if oid:
                self._cancel(oid)
        with self.state_lock:
            self.state["open_orders"] = {}
            save_state(self.state_file, self.state)


    # ---------------- Exchange reconciliation (anti-duplicate-orders) ----------------
    def _extract_order_id(self, o: dict) -> Optional[str]:
        return (
            o.get("id")
            or o.get("order_id")
            or o.get("orderID")
            or o.get("orderId")
        )

    def _extract_order_token_id(self, o: dict) -> Optional[str]:
        return (
            o.get("token_id")
            or o.get("tokenId")
            or o.get("asset_id")
            or o.get("assetId")
        )

    def _extract_order_side(self, o: dict) -> str:
        return str(o.get("side") or "").upper()

    def _extract_order_price(self, o: dict) -> float:
        try:
            return float(o.get("price") or 0.0)
        except Exception:
            return 0.0

    def _extract_order_remaining_size(self, o: dict) -> float:
        for k in ("size", "remaining_size", "remainingSize", "original_size", "originalSize"):
            if k in o and o.get(k) is not None:
                try:
                    return float(o.get(k) or 0.0)
                except Exception:
                    pass
        return 0.0

    def _list_open_orders_exchange(self) -> list:
        """Fetch open orders for this market from the exchange (best-effort)."""
        try:
            return self.client.get_orders(OpenOrderParams(market=self.condition_id)) or []
        except Exception as e:
            self.logger.error(f"get_orders failed during reconcile: {e}")
            return []

    def _cancel_exchange_orders_for_assets(self, asset_ids: list, reason: str = "") -> None:
        """Cancel ANY exchange open orders for the given asset_ids (best-effort)."""
        if self.cfg.dry_run:
            return
        aset = {str(a) for a in (asset_ids or [])}
        if not aset:
            return
        orders = self._list_open_orders_exchange()
        for o in orders:
            aid = self._extract_order_token_id(o)
            if aid is None:
                continue
            if str(aid) not in aset:
                continue
            oid = self._extract_order_id(o)
            if not oid:
                continue
            if reason:
                self.logger.info(f"🧹 Cancel exchange order {str(oid)[:10]}.. for {str(aid)[-6:]} ({reason})")
            self._cancel(oid)

    def _reconcile_exchange_orders_for_asset(self, asset_id: str, intended_price: Optional[float] = None, force: bool = False) -> None:
        """
        Ensure at most ONE live exchange order exists for this asset.
        If duplicates exist (common when cancels lag / feed flaps), cancel extras and keep one.

        Also attempts to adopt an existing exchange order into local state if we lost track.
        """
        if not self.reconcile_exchange_orders or self.cfg.dry_run:
            return

        now = time.time()
        last = float(self._last_reconcile_ts.get(str(asset_id), 0.0))
        if (not force) and (now - last) < float(self.reconcile_interval_seconds):
            return
        self._last_reconcile_ts[str(asset_id)] = now

        orders = self._list_open_orders_exchange()
        mine = []
        for o in orders:
            aid = self._extract_order_token_id(o)
            if aid is None or str(aid) != str(asset_id):
                continue
            # We only ever place BUY orders in this bot.
            side = self._extract_order_side(o)
            if side and side != "BUY":
                continue
            oid = self._extract_order_id(o)
            if not oid:
                continue
            mine.append(o)

        if not mine:
            return

        # If only one exchange order exists, adopt it into local state if missing.
        if len(mine) == 1:
            o = mine[0]
            oid = self._extract_order_id(o)
            if not oid:
                return
            with self.state_lock:
                local = (self.state.get("open_orders") or {}).get(str(asset_id))
            if not local or str(local.get("order_id")) != str(oid):
                p = self._extract_order_price(o)
                sz = self._extract_order_remaining_size(o)
                with self.state_lock:
                    self.state.setdefault("open_orders", {})
                    self.state["open_orders"][str(asset_id)] = {
                        "order_id": oid,
                        "price": p,
                        "size": sz,
                        "ts": now,
                    }
                    save_state(self.state_file, self.state)
            return

        # More than one order: keep one, cancel the rest.
        with self.state_lock:
            local = (self.state.get("open_orders") or {}).get(str(asset_id))
        keep_id = str(local.get("order_id")) if local and local.get("order_id") else None

        keep_order = None
        if keep_id:
            for o in mine:
                if str(self._extract_order_id(o)) == keep_id:
                    keep_order = o
                    break

        if keep_order is None:
            # Choose the order closest to intended_price (if provided), otherwise the most aggressive (highest price).
            def score(o: dict) -> float:
                p = self._extract_order_price(o)
                if intended_price is not None and float(intended_price) > 0:
                    return -abs(p - float(intended_price))
                return p
            keep_order = max(mine, key=score)
            keep_id = str(self._extract_order_id(keep_order))

        # Cancel extras
        for o in mine:
            oid = self._extract_order_id(o)
            if not oid:
                continue
            if str(oid) == str(keep_id):
                continue
            self.logger.info(f"🧹 Reconcile: cancel extra order {str(oid)[:10]}.. for {str(asset_id)[-6:]}")
            self._cancel(oid)

        # Update local tracking to the kept order
        p = self._extract_order_price(keep_order)
        sz = self._extract_order_remaining_size(keep_order)
        with self.state_lock:
            self.state.setdefault("open_orders", {})
            self.state["open_orders"][str(asset_id)] = {
                "order_id": keep_id,
                "price": p,
                "size": sz,
                "ts": now,
            }
            save_state(self.state_file, self.state)

    def _post_order_compat(self, signed_order, order_type, post_only: Optional[bool]):
        """
        Compatibility wrapper because py_clob_client versions differ:
          - post_order(order, OrderType.GTC, True)
          - post_order(order, order_type=..., post_only=...)
        """
        if self.cfg.dry_run:
            return None

        # Try common call shapes
        if post_only is None:
            # For FAK/FOK we avoid post_only parameter where possible
            try:
                return self.client.post_order(signed_order, order_type)
            except TypeError:
                try:
                    return self.client.post_order(signed_order, order_type=order_type)
                except TypeError:
                    return self.client.post_order(signed_order, orderType=order_type)

        # With post_only explicit
        try:
            return self.client.post_order(signed_order, order_type, post_only)
        except TypeError:
            try:
                return self.client.post_order(signed_order, order_type=order_type, post_only=post_only)
            except TypeError:
                return self.client.post_order(signed_order, orderType=order_type, postOnly=post_only)


    def _post_orders_compat(self, signed_orders, order_type, post_only: Optional[bool] = None):
        """
        Best-effort batch submit for multiple orders.
        Falls back to sequential post_order if the client/lib doesn't support batching.

        Returns a list of per-order responses (or None entries) aligned to signed_orders.
        """
        if self.cfg.dry_run:
            return [None for _ in (signed_orders or [])]

        signed_orders = list(signed_orders or [])
        if not signed_orders:
            return []

        # Prefer a true batch endpoint if available
        for meth_name in ("post_orders", "post_order_list", "postOrderList", "postOrders"):
            fn = getattr(self.client, meth_name, None)
            if not callable(fn):
                continue
            try:
                # Try common call shapes
                if post_only is None:
                    try:
                        resp = fn(signed_orders, order_type=order_type)
                    except TypeError:
                        try:
                            resp = fn(signed_orders, orderType=order_type)
                        except TypeError:
                            resp = fn(signed_orders, order_type)
                else:
                    try:
                        resp = fn(signed_orders, order_type=order_type, post_only=post_only)
                    except TypeError:
                        try:
                            resp = fn(signed_orders, orderType=order_type, postOnly=post_only)
                        except TypeError:
                            resp = fn(signed_orders, order_type, post_only)

                # Normalize response to list
                if isinstance(resp, list):
                    return resp
                if isinstance(resp, dict):
                    if isinstance(resp.get("responses"), list):
                        return resp["responses"]
                    if isinstance(resp.get("data"), list):
                        return resp["data"]
                # Unknown shape; just broadcast the response
                return [resp for _ in signed_orders]
            except Exception as e:
                self.logger.error(f"⚠️ batch submit via {meth_name} failed: {e}")
                break

        # Fallback sequential
        out = []
        for so in signed_orders:
            try:
                out.append(self._post_order_compat(so, order_type, post_only=post_only))
            except Exception as e:
                self.logger.error(f"post_order failed in fallback: {e}")
                out.append(None)
        return out

    def _place_postonly_bid(self, asset_id: str, price: float, size: float) -> Optional[str]:

        # Latency timing: decision moment for this order placement call
        _decide_ts = float(time.time())
        _decide_ns = None
        try:
            _decide_ns = int(time.perf_counter_ns())
        except Exception:
            _decide_ns = None

        # Respect market tick size (price) and allow fractional sizes (size precision is configurable).
        try:
            tick = float(self.cfg.tick) if float(self.cfg.tick) > 0 else 0.01
        except Exception:
            tick = 0.01
        try:
            dp = int(getattr(self, "size_decimals", 6) or 6)
        except Exception:
            dp = 6
        dp = max(0, min(8, dp))

        size = q_down(float(size), dp)
        price = round_down(float(price), tick)

        if size < self.cfg.min_shares or price <= 0:
            return None

        # enforce min maker notional ($1)
        if (price * size) < self.min_maker_notional:
            # raise size to meet notional (still must be >= min_shares)
            need_size = self.min_maker_notional / price
            need_size = q_up(need_size, dp)
            size = max(size, need_size)
            if size < self.cfg.min_shares:
                size = self.cfg.min_shares
            # re-check notional again
            if (price * size) < self.min_maker_notional:
                return None

        # re-check maker constraint with freshest ask
        ba = self._best_bid_ask(asset_id)
        if not ba:
            return None
        _, ask = ba
        maker_max = ask - self.cfg.maker_buffer_ticks * self.cfg.tick
        maker_max = round_down(maker_max, self.cfg.tick)
        if price > maker_max:
            # would be marketable now -> skip instead of getting 400
            return None

        if self.cfg.dry_run:
            self.logger.info(
                f"[DRY] POSTONLY BID asset={asset_id[-6:]} price={price:.2f} size={size:.2f} notional={price * size:.2f}")
            return None

        args = OrderArgs(price=float(price), size=float(size), side=BUY, token_id=str(asset_id))
        sign_start_ns = None
        sign_end_ns = None
        post_start_ts = None
        post_end_ts = None
        post_start_ns = None
        post_end_ns = None
        try:
            try:
                sign_start_ns = int(time.perf_counter_ns())
            except Exception:
                sign_start_ns = None
            signed = self.client.create_order(args)
            try:
                sign_end_ns = int(time.perf_counter_ns())
            except Exception:
                sign_end_ns = None

            post_start_ts = float(time.time())
            try:
                post_start_ns = int(time.perf_counter_ns())
            except Exception:
                post_start_ns = None

            resp = self._post_order_compat(signed, OrderType.GTC, post_only=True)

            post_end_ts = float(time.time())
            try:
                post_end_ns = int(time.perf_counter_ns())
            except Exception:
                post_end_ns = None
        except Exception as e:
            self.logger.error(f"post_only bid failed: {e}")
            return None

        oid = None
        if isinstance(resp, dict):
            oid = resp.get("orderID") or resp.get("order_id") or resp.get("id")

        # Track context for later fill-latency measurement.
        if oid:
            try:
                self._track_order_execution_context(
                    order_id=str(oid),
                    asset_id=str(asset_id),
                    side="BUY",
                    px_limit=float(price),
                    size=float(size),
                    decision_ts=_decide_ts,
                    post_start_ts=post_start_ts,
                    post_end_ts=post_end_ts,
                    decision_ns=_decide_ns,
                    sign_start_ns=sign_start_ns,
                    sign_end_ns=sign_end_ns,
                    post_start_ns=post_start_ns,
                    post_end_ns=post_end_ns,
                    origin="MAKER_POSTONLY_GTC",
                )
            except Exception:
                pass
        return oid

    def _place_limit_bid_gtc(
        self,
        asset_id: str,
        price: float,
        size: float,
        post_only: Optional[bool] = None,
    ) -> Optional[str]:
        """Place a resting limit BUY (GTC) at `price`.

        Behaviour:
          - If price >= current ask: it can fill immediately (taker) and any remainder may rest.
          - If price < current ask: it rests and may fill later (maker).

        This is used by SNIPER/SIGNAL_SNIPPER when *_ENTRY_ORDER_TYPE is GTC/LIMIT.

        Safety notes:
          - Uses integer share sizes (avoids makerAmount precision issues).
          - Tracks the order in local open_orders state so the bot can avoid duplicate entries.
        """

        # Latency timing: decision moment for this order placement call
        _decide_ts = float(time.time())
        _decide_ns = None
        try:
            _decide_ns = int(time.perf_counter_ns())
        except Exception:
            _decide_ns = None

        try:
            tick = float(self.cfg.tick) if float(self.cfg.tick) > 0 else 0.01
        except Exception:
            tick = 0.01

        # Clamp + tick-quantize price
        px = float(price)
        px = clamp(px, tick, 0.99)
        # Round DOWN so we never exceed the intended limit due to float noise.
        px = round_down(px, tick)
        px = clamp(px, tick, 0.99)

        # Integer shares for safety/compat
        min_int = max(1, int(math.ceil(float(self.cfg.min_shares) - 1e-12)))
        sz_int = int(math.floor(float(size) + 1e-12))

        if sz_int < min_int:
            sz_int = min_int
        # Keep sizes as multiples of min_shares (helps avoid dust)
        if sz_int >= min_int:
            sz_int = (sz_int // min_int) * min_int
        if sz_int < min_int:
            sz_int = min_int

        # DRY run: simulate an order id and track it locally to avoid spam.
        if self.cfg.dry_run:
            oid = f"DRY_LIMIT_GTC_{int(time.time() * 1000)}"
            try:
                now = time.time()
                with self.state_lock:
                    self.state.setdefault("open_orders", {})
                    self.state["open_orders"][str(asset_id)] = {
                        "order_id": oid,
                        "price": float(px),
                        "size": float(sz_int),
                        "ts": now,
                    }
                    save_state(self.state_file, self.state)
            except Exception:
                pass
            self.logger.info(
                f"[DRY] limit bid GTC asset={str(asset_id)[-6:]} px={px:.3f} size={sz_int} post_only={post_only}"
            )
            return oid

        try:
            args = OrderArgs(
                price=px,
                size=float(sz_int),
                side=BUY,
                token_id=str(asset_id),
            )
            sign_start_ns = None
            sign_end_ns = None
            post_start_ts = None
            post_end_ts = None
            post_start_ns = None
            post_end_ns = None

            try:
                sign_start_ns = int(time.perf_counter_ns())
            except Exception:
                sign_start_ns = None
            signed = self.client.create_order(args)
            try:
                sign_end_ns = int(time.perf_counter_ns())
            except Exception:
                sign_end_ns = None

            post_start_ts = float(time.time())
            try:
                post_start_ns = int(time.perf_counter_ns())
            except Exception:
                post_start_ns = None

            resp = self._post_order_compat(
                signed,
                OrderType.GTC,
                post_only=post_only if post_only is not None else None,
            )

            post_end_ts = float(time.time())
            try:
                post_end_ns = int(time.perf_counter_ns())
            except Exception:
                post_end_ns = None
        except Exception as e:
            self.logger.error(f"limit bid GTC failed: {e}")
            return None

        oid = None
        if isinstance(resp, dict):
            oid = resp.get("orderID") or resp.get("order_id") or resp.get("id")

        if oid:
            # Best-effort local tracking (WS order events will refine remaining size).
            try:
                now = time.time()
                with self.state_lock:
                    self.state.setdefault("open_orders", {})
                    self.state["open_orders"][str(asset_id)] = {
                        "order_id": oid,
                        "price": float(px),
                        "size": float(sz_int),
                        "ts": now,
                    }
                    save_state(self.state_file, self.state)
            except Exception:
                pass

            try:
                tail = str(asset_id)[-6:]
            except Exception:
                tail = str(asset_id)
            self.logger.info(
                f"[LIMIT] placed GTC BUY asset={tail} px={px:.3f} size={sz_int} post_only={post_only}"
            )

            # Track context for later fill-latency measurement.
            try:
                self._track_order_execution_context(
                    order_id=str(oid),
                    asset_id=str(asset_id),
                    side="BUY",
                    px_limit=float(px),
                    size=float(sz_int),
                    decision_ts=_decide_ts,
                    post_start_ts=post_start_ts,
                    post_end_ts=post_end_ts,
                    decision_ns=_decide_ns,
                    sign_start_ns=sign_start_ns,
                    sign_end_ns=sign_end_ns,
                    post_start_ns=post_start_ns,
                    post_end_ns=post_end_ns,
                    origin=("LIMIT_GTC" + ("_POSTONLY" if bool(post_only) else "")),
                )
            except Exception:
                pass

        return oid


    def _resolve_order_type(self, name: str):
        """
        Map env string to OrderType enum safely.

        Notes:
          - Polymarket CLOB uses time-in-force values (GTC/FOK/FAK). Orders are always LIMIT orders.
          - We accept a few human-friendly aliases (e.g. LIMIT -> GTC, IOC -> FAK).
        """
        name_u = str(name or "").upper().strip()

        # Human-friendly aliases
        if name_u in {"LIMIT", "LIMIT_GTC", "GTC_LIMIT"}:
            name_u = "GTC"
        if name_u in {"IOC", "IOK", "FILL_AND_KILL", "FILLANDKILL"}:
            name_u = "FAK"
        if name_u in {"FILL_OR_KILL", "FILLORKILL"}:
            name_u = "FOK"

        name = name_u

        try:
            return getattr(OrderType, name)
        except Exception:
            try:
                return OrderType[name]
            except Exception:
                # Last resort: fall back to GTC but log loudly.
                self.logger.warning(f"⚠️ Unknown OrderType '{name}'. Falling back to GTC.")
                return OrderType.GTC


    def _place_taker_bid_fak(self, asset_id: str, price: float, size: float, order_type_name: Optional[str] = None) -> Optional[str]:
        """
        Emergency hedge (taker): FAK recommended.
        IMPORTANT: For marketable BUY, Polymarket requires derived maker amount to be <= 2dp.
        With 2dp price, that implies integer share size.
        """

        # Latency timing: decision moment for this order placement call
        _decide_ts = float(time.time())
        _decide_ns = None
        try:
            _decide_ns = int(time.perf_counter_ns())
        except Exception:
            _decide_ns = None


        # price tick – round UP to stay marketable (must respect market tick_size)
        try:
            tick = float(self.cfg.tick) if float(self.cfg.tick) > 0 else 0.01
        except Exception:
            tick = 0.01
        price = round_up(float(price), tick)
        price = clamp(price, tick, 0.99)

        # Integer shares (safer under WS congestion / makerAmount precision rules)
        size_int = int(math.floor(float(size) + 1e-12))
        min_int = max(1, int(math.ceil(float(self.cfg.min_shares) - 1e-12)))
        if size_int < min_int:
            return None
        size = float(size_int)

        if self.cfg.dry_run:
            ot_name = str(order_type_name or self.hedge_taker_order_type).upper().strip()
            self.logger.info(
                f"[DRY] TAKER HEDGE asset={asset_id[-6:]} price={price:.2f} size={size:.0f} type={ot_name}")
            return None

        ot_name = str(order_type_name or self.hedge_taker_order_type).upper().strip()
        ot = self._resolve_order_type(ot_name)
        args = OrderArgs(price=float(price), size=float(size), side=BUY, token_id=str(asset_id))
        self.logger.info(f"forced taker hedge: {args} type={ot}")

        sign_start_ns = None
        sign_end_ns = None
        post_start_ts = None
        post_end_ts = None
        post_start_ns = None
        post_end_ns = None
        try:
            try:
                sign_start_ns = int(time.perf_counter_ns())
            except Exception:
                sign_start_ns = None
            signed = self.client.create_order(args)
            try:
                sign_end_ns = int(time.perf_counter_ns())
            except Exception:
                sign_end_ns = None

            post_start_ts = float(time.time())
            try:
                post_start_ns = int(time.perf_counter_ns())
            except Exception:
                post_start_ns = None

            resp = self._post_order_compat(signed, ot, post_only=None)

            post_end_ts = float(time.time())
            try:
                post_end_ns = int(time.perf_counter_ns())
            except Exception:
                post_end_ns = None
        except Exception as e:
            self._last_taker_error = str(e)
            self._last_taker_error_ts = time.time()
            print("taker hedge failed:", e)
            # optional: pause a bit to avoid spamming
            self._taker_fail_pause_until = time.time() + 2.0
            return None

        oid = None
        if isinstance(resp, dict):
            oid = resp.get("orderID") or resp.get("order_id") or resp.get("id")

        if oid:
            self.logger.info(
                f"[TAKER {ot_name}] sent asset={asset_id[-6:]} px={price:.2f} sz={size:.0f} oid={oid}")
            self._remember_taker_order(oid, asset_id, size=size, px_limit=price, side="BUY")
            try:
                self._track_order_execution_context(
                    order_id=str(oid),
                    asset_id=str(asset_id),
                    side="BUY",
                    px_limit=float(price),
                    size=float(size),
                    decision_ts=_decide_ts,
                    post_start_ts=post_start_ts,
                    post_end_ts=post_end_ts,
                    decision_ns=_decide_ns,
                    sign_start_ns=sign_start_ns,
                    sign_end_ns=sign_end_ns,
                    post_start_ns=post_start_ns,
                    post_end_ns=post_end_ns,
                    origin=f"TAKER_{ot_name}_BUY",
                )
            except Exception:
                pass
        return oid

    def _place_taker_ask_fak(self, asset_id: str, price: float, size: float, order_type_name: Optional[str] = None) -> Optional[str]:
        """Emergency flatten (taker SELL): FAK recommended.

        Uses integer shares for precision safety.
        For a marketable SELL, price must be <= best_bid (we'll round DOWN).
        """

        # Latency timing: decision moment for this order placement call
        _decide_ts = float(time.time())
        _decide_ns = None
        try:
            _decide_ns = int(time.perf_counter_ns())
        except Exception:
            _decide_ns = None

        # price tick – round DOWN for SELL (must respect market tick_size)
        try:
            tick = float(self.cfg.tick) if float(self.cfg.tick) > 0 else 0.01
        except Exception:
            tick = 0.01
        price = round_down(float(price), tick)
        price = clamp(price, tick, 0.99)

        # Integer shares
        size_int = int(math.floor(float(size) + 1e-12))
        min_int = max(1, int(math.ceil(float(self.cfg.min_shares) - 1e-12)))
        if size_int < min_int:
            return None
        size = float(size_int)

        if self.cfg.dry_run:
            ot_name = str(order_type_name or self.hedge_taker_order_type).upper().strip()
            self.logger.info(f"[DRY] TAKER SELL asset={asset_id[-6:]} price={price:.2f} size={size:.0f} type={ot_name}")
            return None

        ot_name = str(order_type_name or self.hedge_taker_order_type).upper().strip()
        ot = self._resolve_order_type(ot_name)
        args = OrderArgs(price=float(price), size=float(size), side=SELL, token_id=str(asset_id))
        self.logger.info(f"forced taker sell: {args} type={ot}")

        sign_start_ns = None
        sign_end_ns = None
        post_start_ts = None
        post_end_ts = None
        post_start_ns = None
        post_end_ns = None
        try:
            try:
                sign_start_ns = int(time.perf_counter_ns())
            except Exception:
                sign_start_ns = None
            signed = self.client.create_order(args)
            try:
                sign_end_ns = int(time.perf_counter_ns())
            except Exception:
                sign_end_ns = None

            post_start_ts = float(time.time())
            try:
                post_start_ns = int(time.perf_counter_ns())
            except Exception:
                post_start_ns = None

            resp = self._post_order_compat(signed, ot, post_only=None)

            post_end_ts = float(time.time())
            try:
                post_end_ns = int(time.perf_counter_ns())
            except Exception:
                post_end_ns = None
        except Exception as e:
            self._last_taker_error = str(e)
            self._last_taker_error_ts = time.time()
            print("taker sell failed:", e)
            self._taker_fail_pause_until = time.time() + 2.0
            return None

        oid = None
        if isinstance(resp, dict):
            oid = resp.get("orderID") or resp.get("order_id") or resp.get("id")

        if oid:
            self.logger.info(f"[TAKER {ot_name}] sent SELL asset={asset_id[-6:]} px={price:.2f} sz={size:.0f} oid={oid}")
            self._remember_taker_order(oid, asset_id, size=size, px_limit=price, side="SELL")
            try:
                self._track_order_execution_context(
                    order_id=str(oid),
                    asset_id=str(asset_id),
                    side="SELL",
                    px_limit=float(price),
                    size=float(size),
                    decision_ts=_decide_ts,
                    post_start_ts=post_start_ts,
                    post_end_ts=post_end_ts,
                    decision_ns=_decide_ns,
                    sign_start_ns=sign_start_ns,
                    sign_end_ns=sign_end_ns,
                    post_start_ns=post_start_ns,
                    post_end_ns=post_end_ns,
                    origin=f"TAKER_{ot_name}_SELL",
                )
            except Exception:
                pass
        return oid


    # ============================
    # TAKER_PAIR (pair arbitrage) logic
    # ============================
    def _pair_arb_required_total(self) -> float:
        """Maximum acceptable (ask_yes + ask_no) to attempt a taker-pair buy.

        We require:
            total_px <= 1 - (min_profit + est_fees + safety)

        Notes:
          - pair_arb_fee_rate is treated as a conservative *per-complete-set* fee fraction of $1.
            If you pay 0.5% taker fee per leg, set this to ~0.01 to be conservative.
        """
        min_profit = float(self.pair_arb_min_profit_ticks) * float(self.cfg.tick)
        safety = float(self.pair_arb_safety_ticks) * float(self.cfg.tick)
        fees_buf = float(self.pair_arb_fee_rate) * 1.0  # conservative upper bound
        req = 1.0 - min_profit - safety - fees_buf - 1e-9
        return clamp(req, 0.0, 1.0)

    def _taker_pair_submit(self, size_int: int, y_px: float, n_px: float) -> Tuple[Optional[str], Optional[str]]:
        """Submit both taker BUY orders (YES and NO) as close together as possible."""

        # Latency timing: decision moment for this pair submit call
        _decide_ts = float(time.time())
        _decide_ns = None
        try:
            _decide_ns = int(time.perf_counter_ns())
        except Exception:
            _decide_ns = None

        if size_int <= 0:
            return None, None

        ot = self._resolve_order_type(self.pair_arb_order_type)

        # Safety: if order type resolves to GTC, we are no longer "atomic". Stop unless explicitly overridden.
        if (ot == OrderType.GTC) and (not self.pair_arb_allow_gtc):
            self.logger.info(
                f"🛑 PAIR_ARB_ORDER_TYPE='{self.pair_arb_order_type}' resolved to GTC. "
                f"This is UNSAFE for atomic pair-arb. Set PAIR_ARB_ALLOW_GTC=true to override."
            )
            self.exit_reason = "PAIR_ARB_UNSAFE_GTC"
            self.cancel_all_orders_exchange(reason="pair arb unsafe order type")
            self.stop_event.set()
            return None, None

        args_y = OrderArgs(price=float(y_px), size=float(size_int), side=BUY, token_id=str(self.yes_asset))
        args_n = OrderArgs(price=float(n_px), size=float(size_int), side=BUY, token_id=str(self.no_asset))

        if self.cfg.dry_run:
            self.logger.info(f"[DRY] TAKER_PAIR {self.pair_arb_order_type} size={size_int} y_px={y_px:.2f} n_px={n_px:.2f}")
            return None, None

        sign_y_start_ns = None
        sign_y_end_ns = None
        sign_n_start_ns = None
        sign_n_end_ns = None
        try:
            try:
                sign_y_start_ns = int(time.perf_counter_ns())
            except Exception:
                sign_y_start_ns = None
            signed_y = self.client.create_order(args_y)
            try:
                sign_y_end_ns = int(time.perf_counter_ns())
            except Exception:
                sign_y_end_ns = None

            try:
                sign_n_start_ns = int(time.perf_counter_ns())
            except Exception:
                sign_n_start_ns = None
            signed_n = self.client.create_order(args_n)
            try:
                sign_n_end_ns = int(time.perf_counter_ns())
            except Exception:
                sign_n_end_ns = None
        except Exception as e:
            self.logger.error(f"pair create_order failed: {e}")
            self._taker_fail_pause_until = time.time() + float(self.pair_arb_pause_on_error_seconds)
            return None, None

        post_start_ts = None
        post_end_ts = None
        post_start_ns = None
        post_end_ns = None
        try:
            post_start_ts = float(time.time())
            try:
                post_start_ns = int(time.perf_counter_ns())
            except Exception:
                post_start_ns = None

            resps = self._post_orders_compat([signed_y, signed_n], ot, post_only=None)

            post_end_ts = float(time.time())
            try:
                post_end_ns = int(time.perf_counter_ns())
            except Exception:
                post_end_ns = None
        except Exception as e:
            self.logger.error(f"pair post_orders failed: {e}")
            self._taker_fail_pause_until = time.time() + float(self.pair_arb_pause_on_error_seconds)
            return None, None

        # Parse order IDs from responses
        y_oid = None
        n_oid = None
        if isinstance(resps, list):
            if len(resps) >= 1 and isinstance(resps[0], dict):
                y_oid = resps[0].get("orderID") or resps[0].get("order_id") or resps[0].get("id")
            if len(resps) >= 2 and isinstance(resps[1], dict):
                n_oid = resps[1].get("orderID") or resps[1].get("order_id") or resps[1].get("id")

        # Remember order IDs (helps WS fill accounting / debugging)
        if y_oid:
            self._remember_taker_order(y_oid, self.yes_asset, size=float(size_int), px_limit=float(y_px), side="BUY")
            try:
                self._track_order_execution_context(
                    order_id=str(y_oid),
                    asset_id=str(self.yes_asset),
                    side="BUY",
                    px_limit=float(y_px),
                    size=float(size_int),
                    decision_ts=_decide_ts,
                    post_start_ts=post_start_ts,
                    post_end_ts=post_end_ts,
                    decision_ns=_decide_ns,
                    sign_start_ns=sign_y_start_ns,
                    sign_end_ns=sign_y_end_ns,
                    post_start_ns=post_start_ns,
                    post_end_ns=post_end_ns,
                    origin=f"TAKER_PAIR_{str(self.pair_arb_order_type or '').upper()}_YES",
                )
            except Exception:
                pass
        if n_oid:
            self._remember_taker_order(n_oid, self.no_asset, size=float(size_int), px_limit=float(n_px), side="BUY")
            try:
                self._track_order_execution_context(
                    order_id=str(n_oid),
                    asset_id=str(self.no_asset),
                    side="BUY",
                    px_limit=float(n_px),
                    size=float(size_int),
                    decision_ts=_decide_ts,
                    post_start_ts=post_start_ts,
                    post_end_ts=post_end_ts,
                    decision_ns=_decide_ns,
                    sign_start_ns=sign_n_start_ns,
                    sign_end_ns=sign_n_end_ns,
                    post_start_ns=post_start_ns,
                    post_end_ns=post_end_ns,
                    origin=f"TAKER_PAIR_{str(self.pair_arb_order_type or '').upper()}_NO",
                )
            except Exception:
                pass

        return y_oid, n_oid

    def _wait_for_pair_fills(self, qy0: float, qn0: float, target_size: int, timeout_s: float) -> Tuple[float, float]:
        """Wait (event-driven) for fills to be reflected in state up to timeout."""
        deadline = time.time() + float(max(0.01, timeout_s))
        while time.time() < deadline and (not self.stop_event.is_set()):
            with self.state_lock:
                qy = float(self.state.get("q_yes", 0.0))
                qn = float(self.state.get("q_no", 0.0))
            fy = max(0.0, qy - float(qy0))
            fn = max(0.0, qn - float(qn0))

            if fy >= float(target_size) and fn >= float(target_size):
                return fy, fn

            remaining = deadline - time.time()
            if remaining <= 0:
                break

            # Wait for WS-driven fill updates
            try:
                self.position_update_event.wait(timeout=min(0.05, remaining))
                self.position_update_event.clear()
            except Exception:
                time.sleep(min(0.05, remaining))

        with self.state_lock:
            qy = float(self.state.get("q_yes", 0.0))
            qn = float(self.state.get("q_no", 0.0))
        return max(0.0, qy - float(qy0)), max(0.0, qn - float(qn0))

    def _handle_exposure_mismatch(self, filled_yes: float, filled_no: float) -> None:
        """Handle the case where one leg fills and the other doesn't."""
        delta = float(filled_yes) - float(filled_no)  # + means excess YES
        if abs(delta) < 1e-9:
            return


        # WS congestion safety: reconcile balances before acting (avoids false mismatch -> unnecessary unwind)
        try:
            self._reconcile_state_from_balances(reason="exposure_mismatch")
        except Exception:
            pass

        # If delta is below min_shares, we cannot reliably fix with Polymarket min size rules -> stop.
        if abs(delta) < float(self.cfg.min_shares):
            self.logger.info(
                f"🛑 Exposure mismatch below min_shares. filled_yes={filled_yes:.2f} filled_no={filled_no:.2f} "
                f"delta={delta:.2f} -> STOP"
            )
            self.exit_reason = "DUST_EXPOSURE"
            self.cancel_all_orders_exchange(reason="dust exposure")
            self.stop_event.set()
            return

        policy = str(self.exposure_policy or "UNWIND").upper().strip()
        self.logger.info(
            f"⚠️ EXPOSURE mismatch: filled_yes={filled_yes:.0f} filled_no={filled_no:.0f} "
            f"delta={delta:.0f} policy={policy}"
        )

        # Always cancel any remaining orders for safety
        self.cancel_all_open_orders_local(reason="exposure mismatch cleanup")
        self._cancel_exchange_orders_for_assets([self.yes_asset, self.no_asset], reason="exposure mismatch cleanup")

        if policy == "WAIT":
            # Let the normal hedge loop handle it (NOT recommended).
            return

        if policy == "HEDGE":
            # Try to buy the missing side using the existing hedge-cap logic.
            self._emergency_taker_hedge_step(delta, reason="pair_arb_mismatch")

            # Optional: after a short grace period, if still imbalanced, unwind anyway.
            if self.exposure_hedge_then_unwind:
                time.sleep(max(0.05, float(self.exposure_hedge_grace_seconds)))
                with self.state_lock:
                    qy2 = float(self.state.get("q_yes", 0.0))
                    qn2 = float(self.state.get("q_no", 0.0))
                delta2 = qy2 - qn2
                if abs(delta2) >= float(self.cfg.min_shares):
                    self.logger.info(f"⚠️ Exposure still present after hedge grace. delta={delta2:.2f} -> UNWIND heavy.")
                    # fall through to unwind logic below (recompute delta)
                    delta = delta2
                else:
                    return
            else:
                return

        # Default: UNWIND heavy leg (fastest risk removal) – chunked + depth-aware
        self._chunked_unwind_heavy_leg(delta, reason="pair_arb_mismatch")
        return

    def _normalize_exposure_policy(self, policy: str) -> str:
        p = (policy or "").upper().strip()
        if p in ("FLATTEN", "UNWIND", "SELL", "SELL_HEAVY", "EXIT"):
            return "UNWIND"
        if p in ("HEDGE_THEN_UNWIND", "HEDGE+UNWIND", "HEDGE_UNWIND", "HEDGE-UNWIND", "HTU"):
            return "HEDGE_THEN_UNWIND"
        if p in ("HEDGE", "BUY_MISSING"):
            return "HEDGE"
        if p in ("WAIT", "HOLD"):
            return "WAIT"
        return p or "HEDGE"

    def _unwind_heavy_leg(self, delta: float, reason: str) -> None:
        """Taker-sell the heavy leg to flatten exposure (best-effort, fast)."""
        if abs(float(delta)) < float(self.cfg.min_shares):
            return


        # Deterministic throttle: don't spam repeated SELLs while a prior taker order is inflight
        now = time.time()
        if now < float(getattr(self, "_taker_inflight_until", 0.0) or 0.0):
            return
        if now < float(getattr(self, "_taker_fail_pause_until", 0.0) or 0.0):
            return

        heavy_asset = self.yes_asset if float(delta) > 0 else self.no_asset

        # Strict inflight gating: if we already have a pending taker SELL for this asset, wait for WS/order events
        if getattr(self, "taker_strict_inflight", True) and self._has_pending_taker_order("SELL", heavy_asset):
            return

        ba = self._best_bid_ask(heavy_asset)
        if not ba:
            self.logger.info(f"🛑 UNWIND failed: missing best bid for heavy asset={str(heavy_asset)[-6:]} ({reason})")
            self.exit_reason = "UNWIND_NO_BID"
            self.stop_event.set()
            return

        heavy_bid = float(ba[0] or 0.0)
        if heavy_bid <= 0:
            self.logger.info(f"🛑 UNWIND failed: heavy bid<=0 for asset={str(heavy_asset)[-6:]} ({reason})")
            self.exit_reason = "UNWIND_NO_BID"
            self.stop_event.set()
            return

        # Use chunked unwind to reduce slippage and avoid false unwinds under WS lag
        self._chunked_unwind_heavy_leg(delta, reason=str(reason or "unwind"))
        return

    def _maker_exposure_step(self, delta: float, unhedged_age: float) -> None:
        """Handle imbalanced state in MAKER mode according to MAKER_EXPOSURE_POLICY."""
        pol_raw = str(getattr(self, "maker_exposure_policy", "HEDGE"))
        policy = self._normalize_exposure_policy(pol_raw)

        # Hard time stop: if configured, force unwind after X seconds regardless.
        max_s = float(getattr(self, "maker_exposure_max_seconds", 0.0) or 0.0)
        if max_s > 0 and unhedged_age >= max_s:
            self.logger.info(f"⏱️ Exposure age {unhedged_age:.2f}s >= max {max_s:.2f}s -> UNWIND heavy (policy hard max)")
            self.cancel_all_open_orders_local(reason="maker exposure hard max -> unwind")
            self._cancel_exchange_orders_for_assets([self.yes_asset, self.no_asset], reason="maker exposure hard max -> unwind")
            self._unwind_heavy_leg(delta, reason="maker_exposure_max_seconds")
            return

        if policy == "UNWIND":
            self.cancel_all_open_orders_local(reason="maker exposure policy=UNWIND")
            self._cancel_exchange_orders_for_assets([self.yes_asset, self.no_asset], reason="maker exposure policy=UNWIND")
            self._unwind_heavy_leg(delta, reason="maker_policy_unwind")
            return

        missing_asset = self.no_asset if float(delta) > 0 else self.yes_asset

        # Compute the no-loss hedge cap (settlement break-even)
        cap_now = float(self._hedge_price_cap())

        # If cap is already <= 0, we cannot buy missing without locking a loss.
        # In strict safety mode, we prefer to flatten rather than sit exposed.
        if cap_now <= 0:
            self.logger.info(f"🛑 Hedge cap<=0 (cap={cap_now:.2f}) delta={delta:.2f} policy={policy} -> FLATTEN/STOP")
            info = self._flatten_now_best(delta)
            if info:
                self._force_flatten_and_stop(delta, info)
            else:
                self.exit_reason = "CAP_LOCKED_LOSS"
                self.cancel_all_orders_exchange(reason="cap<=0 locked loss")
                self.stop_event.set()
            return

        # Need current ask for missing side to know whether hedge is feasible without locking loss
        m_ba = self._best_bid_ask(missing_asset)
        if not m_ba:
            return
        missing_ask = float(m_ba[1] or 0.0)
        if missing_ask <= 0:
            return

        # If we are cap-blocked (ask > cap), a taker hedge is impossible without loss.
        cap_blocked = (missing_ask > cap_now + 1e-12)

        # Optional strict safety: if cap-blocked beyond grace, unwind heavy
        grace = float(getattr(self, "maker_exposure_hedge_grace_seconds", self.exposure_hedge_grace_seconds) or 0.0)
        want_then_unwind = (
            policy == "HEDGE_THEN_UNWIND"
            or bool(getattr(self, "maker_exposure_hedge_then_unwind", False))
        )

        self._dbg_maker(
            f"[DBG][MAKER][EXPOSURE] delta={delta:.2f} age={unhedged_age:.2f}s cap={cap_now:.2f} "
            f"missing_ask={missing_ask:.2f} cap_blocked={cap_blocked} policy={policy}",
            key="maker_exposure",
            throttle_s=0.5,
        )

        if cap_blocked:
            # Keep a passive maker hedge working at (<=cap), but don't chase with taker.
            maker_max = self._maker_max_price(missing_asset)
            if maker_max is not None:
                target_price = min(cap_now, maker_max)
                target_price = round_down(target_price, self.cfg.tick)
                size = min(abs(float(delta)), float(self.cfg.clip_shares))
                try:
                    if self.first_hedge_full and self._first_cycle_started and (not self._first_cycle_done):
                        size = abs(float(delta))
                except Exception:
                    pass
                if size >= float(self.cfg.min_shares) and target_price > 0:
                    self._maybe_replace(missing_asset, target_price, float(size), stale_seconds=self.hedge_stale_seconds)

            # Cancel any other resting orders but keep the hedge order working
            self.cancel_all_open_orders_local_except(missing_asset, reason="cap-blocked (keep maker hedge)")

            if want_then_unwind and (unhedged_age >= grace):
                self.logger.info(f"⚠️ Cap-blocked for {unhedged_age:.2f}s (grace={grace:.2f}s) -> UNWIND heavy (policy)")
                self.cancel_all_open_orders_local(reason="cap-blocked -> unwind")
                self._cancel_exchange_orders_for_assets([self.yes_asset, self.no_asset], reason="cap-blocked -> unwind")
                self._unwind_heavy_leg(delta, reason="cap_blocked_policy_unwind")
            return

        # Not cap-blocked: we can hedge without locking a loss.
        # If we waited too long, force a taker hedge for speed.
        if unhedged_age >= float(self.unhedged_timeout_seconds):
            self._emergency_taker_hedge_step(delta, reason=f"maker_unhedged>{self.unhedged_timeout_seconds}s")
            return

        # Otherwise, maker hedge within cap (fastest maker fill while staying no-loss)
        maker_max = self._maker_max_price(missing_asset)
        if maker_max is None:
            return

        target_price = min(cap_now, maker_max)
        target_price = round_down(target_price, self.cfg.tick)
        if target_price <= 0:
            return

        size = min(abs(float(delta)), float(self.cfg.clip_shares))
        try:
            if self.first_hedge_full and self._first_cycle_started and (not self._first_cycle_done):
                size = abs(float(delta))
        except Exception:
            pass

        if size >= float(self.cfg.min_shares):
            self._maybe_replace(missing_asset, target_price, float(size), stale_seconds=self.hedge_stale_seconds)
    def _taker_pair_arb_step(self, remaining_budget: float) -> None:
        """Attempt taker-pair arbitrage with retries + hard timeout (DEBUG_MODE shows why we skip)."""
        now = time.time()
        dbg = bool(getattr(self, "pair_arb_debug", False))

        # Global cooldown (prevents spamming)
        if (now - float(self._pair_arb_last_attempt_ts)) < float(self.pair_arb_cooldown_seconds):
            if dbg:
                self._dbg(
                    f"[DBG][TAKER_PAIR] skip cooldown "
                    f"dt={(now - float(self._pair_arb_last_attempt_ts)):.3f}s < "
                    f"{float(self.pair_arb_cooldown_seconds):.3f}s",
                    key="pair_cooldown"
                )
            return

        # If recent taker failure (e.g., 400/5xx), pause a bit
        if now < float(getattr(self, "_taker_fail_pause_until", 0.0)):
            if dbg:
                self._dbg(
                    f"[DBG][TAKER_PAIR] skip fail-pause "
                    f"remain={(float(self._taker_fail_pause_until) - now):.3f}s",
                    key="pair_failpause"
                )
            return

        if remaining_budget <= 0:
            if dbg:
                self._dbg("[DBG][TAKER_PAIR] skip no remaining budget", key="pair_budget")
            return

        # Optional stability gate (warmup/spread/parity) before attempting pair arb
        if self.pair_arb_use_stability_gate:
            ok, why = self._accumulate_allowed()
            if not ok:
                if dbg:
                    self._dbg(f"[DBG][TAKER_PAIR] skip stability gate: {why}", key=f"pair_gate_{why}")
                return

        # Fetch current asks
        yq = self._best_bid_ask(self.yes_asset)
        nq = self._best_bid_ask(self.no_asset)
        if not yq or not nq:
            if dbg:
                self._dbg("[DBG][TAKER_PAIR] skip missing quotes", key="pair_missing_quotes")
            return

        _, y_ask = yq
        _, n_ask = nq

        # Guard: avoid absurd legs (optional)
        if (y_ask <= 0) or (n_ask <= 0):
            if dbg:
                self._dbg(
                    f"[DBG][TAKER_PAIR] skip non-positive ask y_ask={y_ask} n_ask={n_ask}",
                    key="pair_zero_ask"
                )
            return

        # Guard: max leg price
        if y_ask > float(self.pair_arb_max_leg_price) or n_ask > float(self.pair_arb_max_leg_price):
            if dbg:
                self._dbg(
                    f"[DBG][TAKER_PAIR] skip max_leg_price y_ask={y_ask:.2f} "
                    f"n_ask={n_ask:.2f} max={float(self.pair_arb_max_leg_price):.2f}",
                    key="pair_max_leg"
                )
            return

        # Guard: avoid extreme skew between legs (optional)
        try:
            skew = abs(float(y_ask) - float(n_ask))
            if skew > (float(self.pair_arb_max_skew_ticks) * float(self.cfg.tick)):
                if dbg:
                    self._dbg(
                        f"[DBG][TAKER_PAIR] skip skew {skew:.4f} > "
                        f"max={float(self.pair_arb_max_skew_ticks) * float(self.cfg.tick):.4f}",
                        key="pair_skew"
                    )
                return
        except Exception:
            skew = float("nan")

        # Apply optional slippage (in ticks) for better fill probability
        y_px = float(y_ask) + float(self.pair_arb_slippage_ticks) * float(self.cfg.tick)
        n_px = float(n_ask) + float(self.pair_arb_slippage_ticks) * float(self.cfg.tick)
        y_px = clamp(y_px, self.cfg.tick, 0.99)
        n_px = clamp(n_px, self.cfg.tick, 0.99)

        # Marketable BUY limits: round UP to tick_size
        y_px = round_up(y_px, self.cfg.tick)
        n_px = round_up(n_px, self.cfg.tick)

        total_px = float(y_px) + float(n_px)
        req = float(self._pair_arb_required_total())

        if dbg:
            self._dbg(
                f"[DBG][TAKER_PAIR] quotes y_ask={y_ask:.2f} n_ask={n_ask:.2f} "
                f"y_px={y_px:.2f} n_px={n_px:.2f} sum={total_px:.2f} "
                f"req<={req:.2f} budget={remaining_budget:.2f}",
                key="pair_summary"
            )

        if total_px > req:
            if dbg:
                self._dbg(
                    f"[DBG][TAKER_PAIR] skip asksum {total_px:.2f} > req {req:.2f} "
                    f"(min_profit_ticks={int(self.pair_arb_min_profit_ticks)} "
                    f"fee_rate={float(self.pair_arb_fee_rate):.4f} "
                    f"safety_ticks={int(self.pair_arb_safety_ticks)} "
                    f"slippage_ticks={int(self.pair_arb_slippage_ticks)})",
                    key="pair_asksum"
                )
            return

        # Determine integer size based on budget + per-order notional constraints
        min_shares_int = max(1, int(math.ceil(float(self.cfg.min_shares) - 1e-12)))
        max_shares = int(max(min_shares_int, int(self.pair_arb_max_shares)))

        max_affordable = int(math.floor(float(remaining_budget) / float(total_px) + 1e-12))
        size_int = min(max_affordable, max_shares)

        # Enforce minimum order notional per leg (conservative)
        try:
            min_notional = float(self.min_taker_notional)
        except Exception:
            min_notional = 1.0

        need_y = int(math.ceil(min_notional / float(y_px) - 1e-12)) if y_px > 0 else 0
        need_n = int(math.ceil(min_notional / float(n_px) - 1e-12)) if n_px > 0 else 0
        min_needed = max(min_shares_int, need_y, need_n)

        if size_int < min_needed:
            if (min_needed <= max_affordable) and (min_needed <= max_shares):
                size_int = int(min_needed)
            else:
                if dbg:
                    self._dbg(
                        f"[DBG][TAKER_PAIR] skip size too small size_int={size_int} "
                        f"min_needed={min_needed} max_affordable={max_affordable} "
                        f"max_shares={max_shares}",
                        key="pair_size"
                    )
                return

        if size_int < min_shares_int:
            if dbg:
                self._dbg(
                    f"[DBG][TAKER_PAIR] skip below min_shares size_int={size_int} "
                    f"min_shares={min_shares_int}",
                    key="pair_min_shares"
                )
            return

        # We're going to attempt now
        self._pair_arb_last_attempt_ts = now

        # Cancel any resting orders before attempting (remove interference)
        if self.pair_arb_cancel_before_attempt:
            self.cancel_all_open_orders_local(reason="before pair arb")
            self._cancel_exchange_orders_for_assets([self.yes_asset, self.no_asset], reason="before pair arb")

        # Snapshot state before submission
        with self.state_lock:
            qy0 = float(self.state.get("q_yes", 0.0))
            qn0 = float(self.state.get("q_no", 0.0))

        self.logger.info(
            f"⚡ TAKER_PAIR attempt size={size_int} y_px={y_px:.2f} n_px={n_px:.2f} "
            f"total={total_px:.2f} req<={req:.2f} budget={remaining_budget:.2f} type={self.pair_arb_order_type}"
        )

        # Retry loop: only retry when BOTH legs fill 0
        for attempt in range(max(1, int(self.pair_arb_max_retries))):
            if self.stop_event.is_set():
                return

            y_oid, n_oid = self._taker_pair_submit(int(size_int), float(y_px), float(n_px))

            # If submission errored, _taker_pair_submit sets a fail-pause to avoid spamming.
            if (y_oid is None and n_oid is None) and (time.time() < float(getattr(self, "_taker_fail_pause_until", 0.0))):
                return

            # Hard timeout waiting for fills to hit our WS-based state
            fy, fn = self._wait_for_pair_fills(qy0, qn0, int(size_int), float(self.pair_arb_timeout_seconds))

            # Cleanup any lingering open orders (safety)
            if self.pair_arb_reconcile_after_timeout:
                try:
                    if y_oid:
                        self._cancel(y_oid)
                    if n_oid:
                        self._cancel(n_oid)
                except Exception:
                    pass
                self._cancel_exchange_orders_for_assets([self.yes_asset, self.no_asset], reason="pair arb cleanup")

            
            # WS congestion safety: reconcile from balances before deciding mismatch/unwind
            try:
                if (fy <= 0 and fn <= 0) or (abs(float(fy) - float(fn)) >= 1e-6):
                    if self._reconcile_state_from_balances(reason="pair_arb_post_wait"):
                        with self.state_lock:
                            qy2 = float(self.state.get("q_yes", 0.0))
                            qn2 = float(self.state.get("q_no", 0.0))
                        fy = max(0.0, qy2 - float(qy0))
                        fn = max(0.0, qn2 - float(qn0))
            except Exception:
                pass

# Evaluate results
            if fy <= 0 and fn <= 0:
                if dbg:
                    self._dbg(
                        f"[DBG][TAKER_PAIR] attempt {attempt+1} no fills (fy=0 fn=0)",
                        key="pair_nofill"
                    )
                if attempt < (int(self.pair_arb_max_retries) - 1):
                    backoff_ms = random.randint(
                        int(self.pair_arb_retry_backoff_ms_min),
                        int(self.pair_arb_retry_backoff_ms_max)
                    )
                    time.sleep(max(0.0, backoff_ms / 1000.0))
                    continue
                return

            if abs(fy - fn) < 1e-6:
                self.logger.info(f"✅ TAKER_PAIR filled YES={fy:.0f} NO={fn:.0f} (total_px≈{total_px:.2f})")
                return

            # Otherwise mismatch: handle exposure and stop retrying
            self._handle_exposure_mismatch(fy, fn)
            return


    def _desired_maker_bid(self, asset_id: str) -> Optional[float]:
        """Compute our intended maker bid for an asset.

        Logic:
          - Start from best bid (optionally improve by IMPROVE_BID_TICKS)
          - Enforce maker constraint: bid <= ask - MAKER_BUFFER_TICKS*tick
        """
        ba = self._best_bid_ask(asset_id)
        if not ba:
            self._dbg_maker(f"[DBG][MAKER] desired_bid missing BBO asset={str(asset_id)[-6:]}", key=f"maker_desired_missing_{str(asset_id)[-6:]}")
            return None

        bid, ask = ba
        bid = float(bid or 0.0)
        ask = float(ask or 0.0)

        if bid <= 0 or ask <= 0:
            self._dbg_maker(
                f"[DBG][MAKER] desired_bid invalid BBO asset={str(asset_id)[-6:]} bid={bid:.4f} ask={ask:.4f}",
                key=f"maker_desired_invalid_{str(asset_id)[-6:]}",
            )
            return None

        # Start at best bid, optionally improve
        p = bid + float(self.cfg.improve_bid_ticks) * float(self.cfg.tick)
        p = round_down(p, self.cfg.tick)

        # Ensure we remain maker: p <= ask - buffer*tick
        maker_max = ask - float(self.cfg.maker_buffer_ticks) * float(self.cfg.tick)
        maker_max = round_down(maker_max, self.cfg.tick)
        if maker_max <= 0:
            self._dbg_maker(
                f"[DBG][MAKER] desired_bid maker_max<=0 asset={str(asset_id)[-6:]} ask={ask:.4f} maker_max={maker_max:.4f}",
                key=f"maker_desired_makermax_{str(asset_id)[-6:]}",
            )
            return None

        p2 = min(p, maker_max)
        p2 = round_down(p2, self.cfg.tick)
        if p2 <= 0:
            self._dbg_maker(
                f"[DBG][MAKER] desired_bid p<=0 asset={str(asset_id)[-6:]} bid={bid:.2f} ask={ask:.2f} p={p:.2f} maker_max={maker_max:.2f}",
                key=f"maker_desired_p0_{str(asset_id)[-6:]}",
            )
            return None

        self._dbg_maker(
            f"[DBG][MAKER] desired_bid asset={str(asset_id)[-6:]} bid={bid:.2f} ask={ask:.2f} improve={int(self.cfg.improve_bid_ticks)} maker_buf={int(self.cfg.maker_buffer_ticks)} -> p={p2:.2f}",
            key=f"maker_desired_{str(asset_id)[-6:]}",
            throttle_s=1.0,
        )
        return p2
    def _maker_max_price(self, asset_id: str) -> Optional[float]:
        """
        Maximum price we can bid and still be maker: ask - buffer*tick
        """
        ba = self._best_bid_ask(asset_id)
        if not ba:
            return None
        _, ask = ba
        if ask <= 0:
            return None
        maker_max = ask - self.cfg.maker_buffer_ticks * self.cfg.tick
        maker_max = round_down(maker_max, self.cfg.tick)
        return maker_max if maker_max > 0 else None


    def _maker_bid_cross_ask_safe(self, asset_id: str, other_asset_id: str, edge: float) -> Optional[float]:
        """Compute a maker bid that is cross-ask safe.

        Cross-ask safety:
            bid(asset) + ask(other) <= 1 - edge

        This prevents quoting an apparently-attractive bid on one side while the other side's ask has
        already moved against us (i.e., we'd be unable to hedge without locking a loss).
        """
        desired = self._desired_maker_bid(asset_id)
        if desired is None:
            return None

        other = self._best_bid_ask(other_asset_id)
        if not other:
            self._dbg_maker(
                f"[DBG][MAKER] cross_safe missing other BBO asset={str(asset_id)[-6:]} other={str(other_asset_id)[-6:]}",
                key=f"maker_cross_missing_{str(asset_id)[-6:]}_{str(other_asset_id)[-6:]}",
            )
            return None
        _, other_ask = other
        other_ask = float(other_ask or 0.0)
        if other_ask <= 0:
            self._dbg_maker(
                f"[DBG][MAKER] cross_safe other_ask<=0 asset={str(asset_id)[-6:]} other={str(other_asset_id)[-6:]} other_ask={other_ask}",
                key=f"maker_cross_other0_{str(asset_id)[-6:]}_{str(other_asset_id)[-6:]}",
            )
            return None

        safe_cap = (1.0 - float(edge)) - other_ask
        safe_cap = round_down(safe_cap, self.cfg.tick)

        p2 = min(float(desired), float(safe_cap))
        p2 = round_down(p2, self.cfg.tick)

        if p2 <= 0:
            self._dbg_maker(
                f"[DBG][MAKER] cross_safe p<=0 asset={str(asset_id)[-6:]} desired={desired:.2f} other_ask={other_ask:.2f} edge={edge:.4f} safe_cap={safe_cap:.2f}",
                key=f"maker_cross_p0_{str(asset_id)[-6:]}_{str(other_asset_id)[-6:]}",
            )
            return None

        self._dbg_maker(
            f"[DBG][MAKER] cross_safe asset={str(asset_id)[-6:]} desired={desired:.2f} other_ask={other_ask:.2f} edge={edge:.2f} safe_cap={safe_cap:.2f} -> bid={p2:.2f}",
            key=f"maker_cross_{str(asset_id)[-6:]}_{str(other_asset_id)[-6:]}",
            throttle_s=0.8,
        )
        return p2


    def _maybe_replace(self, asset_id: str, price: float, size: float, stale_seconds: Optional[int] = None):
        now = time.time()
        aid = str(asset_id)

        # Cancel/replace guard: avoid overlapping orders when cancels lag.
        if now < float(self._cancel_pending_until.get(aid, 0.0)):
            return

        # Reconcile with exchange to ensure we don't have multiple live orders for this asset.
        self._reconcile_exchange_orders_for_asset(aid, intended_price=price)

        stale = int(stale_seconds) if stale_seconds is not None else int(self.cfg.stale_seconds)
        with self.state_lock:
            oo = (self.state.get("open_orders") or {}).get(aid)

        need_new = False
        if not oo:
            need_new = True
        else:
            old_price = float(oo.get("price") or 0)
            old_size = float(oo.get("size") or 0)
            age = now - float(oo.get("ts") or now)
            moved_ticks = abs(price - old_price) / self.cfg.tick
            size_changed = (old_size <= 0) or (abs(size - old_size) >= max(0.25 * old_size, self.cfg.min_shares))

            if age >= stale or moved_ticks >= self.cfg.replace_if_price_moves_ticks or size_changed:
                if age < self.reprice_min_seconds and not size_changed and age < stale and moved_ticks < (self.cfg.replace_if_price_moves_ticks * 3):
                    # Too soon to churn quotes; wait a bit to reduce adverse selection.
                    return

                self.logger.info(
                    f"[REPLACE] {aid[-6:]} old={old_price:.2f} new={price:.2f} "
                    f"moved={moved_ticks:.1f} age={age:.1f}s"
                )

                oid_old = oo.get("order_id")
                if oid_old:
                    self._cancel(oid_old)

                # Drop local tracking immediately; place the replacement only after a short guard.
                with self.state_lock:
                    (self.state.get("open_orders") or {}).pop(aid, None)
                    save_state(self.state_file, self.state)

                self._cancel_pending_until[aid] = now + float(self.cancel_replace_guard_seconds)
                return

        if not need_new:
            return

        # Before placing a new order, do a forced reconcile to cancel any lingering duplicates.
        self._reconcile_exchange_orders_for_asset(aid, intended_price=price, force=True)

        oid = self._place_postonly_bid(aid, price, size)
        if not oid:
            return

        with self.state_lock:
            self.state.setdefault("open_orders", {})
            self.state["open_orders"][aid] = {"order_id": oid, "price": price, "size": size, "ts": now}
            save_state(self.state_file, self.state)

    # ---------------- Hedge cap logic (settlement no-loss) ----------------
    def _hedge_price_cap(self) -> float:
        """
        If we're imbalanced, returns the maximum price we are allowed to pay for the missing side
        such that after buying exactly 'need' shares to balance, locked profit at settlement is >= 0.
        """
        with self.state_lock:
            qy = float(self.state["q_yes"])
            qn = float(self.state["q_no"])
            total_cost = float(self.state["c_yes"]) + float(self.state["c_no"])

        delta = qy - qn
        need = abs(delta)
        if need <= 0:
            return float("inf")

        heavy = qy if delta > 0 else qn
        # break-even: heavy - (total_cost + p*need) >= 0  => p <= (heavy - total_cost)/need
        p_max = (heavy - total_cost) / need

        # safety buffer
        p_max -= self.cfg.hedge_buffer_ticks * self.cfg.tick
        p_max = round_down(p_max, self.cfg.tick)
        return max(0.0, p_max)

    def _cancel_heavy_side_orders(self):
        """
        After any fill: never keep buying the heavy side while unhedged.
        """
        with self.state_lock:
            qy = float(self.state["q_yes"])
            qn = float(self.state["q_no"])
            oo = dict(self.state.get("open_orders") or {})

        delta = qy - qn
        if abs(delta) < self.cfg.min_shares:
            return  # close enough; allow accumulation logic handle it

        heavy_asset = self.yes_asset if delta > 0 else self.no_asset
        heavy_order = oo.get(heavy_asset, {})
        oid = heavy_order.get("order_id")
        if oid:
            self.logger.info(f"🧹 Cancel heavy-side order asset={heavy_asset[-6:]} (delta={delta:.2f})")
            self._cancel(oid)
            with self.state_lock:
                self.state["open_orders"].pop(heavy_asset, None)
                save_state(self.state_file, self.state)

    # ---------------- Logging ----------------

    def _log_status(self):
        with self.state_lock:
            lp = locked_profit(self.state)
            cpp = cost_per_pair(self.state)
            total = float(self.state["c_yes"]) + float(self.state["c_no"])
            qy = float(self.state["q_yes"])
            qn = float(self.state["q_no"])

        line = (
            f"LP={lp:+.4f} CPP={cpp:.6f} TotalCost={total:.4f} "
            f"qYES={qy:.2f} qNO={qn:.2f} "
            f"(mode={self.exec_mode} fsm={getattr(self, 'fsm_state', '-') } mkt_ws={self.market_connected} user_ws={self.user_connected})"
        )

        # Extra visibility for TAKER_PAIR / debug mode
        if self.exec_mode == "TAKER_PAIR" or getattr(self, "debug_mode", False):
            yq = self._best_bid_ask(self.yes_asset)
            nq = self._best_bid_ask(self.no_asset)
            if yq and nq:
                yb, ya = yq
                nb, na = nq
                ask_sum = float(ya) + float(na)
                try:
                    req = float(self._pair_arb_required_total())
                except Exception:
                    req = float("nan")
                try:
                    remaining_budget = max(0.0, float(self.cfg.max_total_cost) - total - float(self.cfg.reserve_usd))
                except Exception:
                    remaining_budget = float("nan")
                line += (
                    f" | BBO YES {yb:.2f}/{ya:.2f} NO {nb:.2f}/{na:.2f} "
                    f"ask_sum={ask_sum:.2f} req<={req:.2f} "
                    f"budget≈{remaining_budget:.2f}"
                )
            else:
                line += " | BBO missing"

        self.logger.info(line)

    def trade_metrics_snapshot(self) -> dict:
        """Lightweight metrics snapshot used by DB logging.

        MAKER/TAKER_PAIR: lp = locked profit (complete-set arbitrage).
        SNIPER: lp = mark-to-market PnL (conservative, uses best bids).
        """
        with self.state_lock:
            qy = float(self.state.get("q_yes", 0.0))
            qn = float(self.state.get("q_no", 0.0))
            cy = float(self.state.get("c_yes", 0.0))
            cn = float(self.state.get("c_no", 0.0))

        total_cost = cy + cn

        if getattr(self, "sniper_mode", False):
            y_bid, _, n_bid, _ = self._sniper_best_snapshot()
            pnl = (qy * float(y_bid) - cy) + (qn * float(n_bid) - cn)
            return {"lp": float(pnl), "total_cost": float(total_cost), "q_yes": qy, "q_no": qn, "cpp": 0.0}

        lp = locked_profit(self.state)
        cpp = cost_per_pair(self.state)
        return {"lp": lp, "total_cost": float(total_cost), "q_yes": qy, "q_no": qn, "cpp": cpp}


    def _flatten_now_best(self, delta: float) -> Optional[dict]:
        """Compute the best (least-loss) immediate flatten option.

        Returns dict with:
          - action: 'BUY_MISSING' or 'SELL_HEAVY'
          - lp: resulting locked_profit if executed now (using current bid/ask)
          - loss: max(0, -lp)
          - cap_now, missing_ask, heavy_bid, gap
        """
        with self.state_lock:
            qy = float(self.state.get("q_yes", 0.0))
            qn = float(self.state.get("q_no", 0.0))
            total_cost = float(self.state.get("c_yes", 0.0)) + float(self.state.get("c_no", 0.0))

        need = abs(float(delta))
        if need <= 0:
            return None

        heavy_asset = self.yes_asset if delta > 0 else self.no_asset
        missing_asset = self.no_asset if delta > 0 else self.yes_asset
        heavy_qty = max(qy, qn)
        light_qty = min(qy, qn)

        heavy_ba = self._best_bid_ask(heavy_asset)
        miss_ba = self._best_bid_ask(missing_asset)
        if not heavy_ba or not miss_ba:
            return None

        heavy_bid = float(heavy_ba[0] or 0.0)
        missing_ask = float(miss_ba[1] or 0.0)
        if heavy_bid <= 0 or missing_ask <= 0:
            return None

        cap_now = float(self._hedge_price_cap())
        gap = float(missing_ask - cap_now)

        # Option A: BUY missing at ask (complete sets)
        lp_buy = heavy_qty - (total_cost + need * missing_ask)

        # Option B: SELL heavy at bid (reduce to light qty)
        lp_sell = light_qty - (total_cost - need * heavy_bid)

        if lp_sell >= lp_buy:
            best_lp = lp_sell
            action = "SELL_HEAVY"
        else:
            best_lp = lp_buy
            action = "BUY_MISSING"

        loss = max(0.0, -best_lp)
        return {
            "action": action,
            "lp": float(best_lp),
            "loss": float(loss),
            "cap_now": float(cap_now),
            "missing_ask": float(missing_ask),
            "heavy_bid": float(heavy_bid),
            "gap": float(gap),
            "need": float(need),
            "heavy_asset": str(heavy_asset),
            "missing_asset": str(missing_asset),
        }

    def _maybe_trigger_max_loss(self, delta: float, unhedged_age: float) -> bool:
        """Circuit breaker: if flatten-now loss is too large for too long, force flatten and stop.

        This prevents rare tail events (expire unhedged) from wiping many small wins.
        """
        if not self.max_loss_enabled:
            self._max_loss_breach_since = None
            return False

        if abs(delta) < float(self.cfg.min_shares):
            self._max_loss_breach_since = None
            return False

        if unhedged_age < float(self.max_loss_grace_seconds):
            self._max_loss_breach_since = None
            return False

        info = self._flatten_now_best(delta)
        if not info:
            self._max_loss_breach_since = None
            return False

        runaway_gap = float(self.max_loss_runaway_gap_ticks) * float(self.cfg.tick)
        if info["gap"] <= runaway_gap:
            # Not a runaway; likely normal temporary imbalance.
            self._max_loss_breach_since = None
            return False

        if info["loss"] <= float(self.max_loss_usd_per_market):
            # Within tolerated bound.
            self._max_loss_breach_since = None
            return False

        now = time.time()
        if self._max_loss_breach_since is None:
            self._max_loss_breach_since = now
            return False

        if (now - float(self._max_loss_breach_since)) < float(self.max_loss_confirm_seconds):
            return False

        self.logger.info(
            f"🧯 CIRCUIT BREAKER: flatten-now loss={info['loss']:.2f} > max_loss={self.max_loss_usd_per_market:.2f} "
            f"gap={info['gap']:.2f} (ask={info['missing_ask']:.2f}, cap={info['cap_now']:.2f}) "
            f"unhedged_age={unhedged_age:.1f}s -> FLATTEN + STOP"
        )
        self._force_flatten_and_stop(delta, info)
        return True

    def _force_flatten_and_stop(self, delta: float, info: dict) -> None:
        """Execute the best immediate flatten action (BUY missing or SELL heavy), then stop."""
        # Cancel resting orders first to free balance and avoid re-imbalance.
        self.cancel_all_open_orders_local(reason="circuit breaker")
        self._cancel_exchange_orders_for_assets([self.yes_asset, self.no_asset], reason="circuit breaker")

        need_int = int(math.floor(abs(float(delta)) + 1e-12))
        if need_int < max(1, int(math.ceil(float(self.cfg.min_shares) - 1e-12))):
            self.exit_reason = "CIRCUIT_BREAKER_TOO_SMALL"
            self.stop_event.set()
            return

        heavy_asset = str(info.get("heavy_asset"))
        missing_asset = str(info.get("missing_asset"))
        missing_ask = float(info.get("missing_ask") or 0.0)
        heavy_bid = float(info.get("heavy_bid") or 0.0)
        action = str(info.get("action") or "BUY_MISSING")

        # Use slippage ticks for marketability, but keep reasonable clamps.
        slip = float(self.hedge_slippage_ticks) * float(self.cfg.tick)

        oid = None

        if action == "BUY_MISSING":
            # Allow spending full remaining budget (ignore reserve) for circuit breaker.
            with self.state_lock:
                total_cost = float(self.state.get("c_yes", 0.0)) + float(self.state.get("c_no", 0.0))
            remaining = float(self.cfg.max_total_cost) - float(total_cost)
            px = clamp(round_up(missing_ask + slip, self.cfg.tick), self.cfg.tick, 0.99)
            max_affordable = int(math.floor(remaining / px + 1e-12)) if px > 0 else 0
            size_int = min(need_int, max_affordable) if max_affordable > 0 else 0

            if size_int >= max(1, int(math.ceil(float(self.cfg.min_shares) - 1e-12))):
                self.logger.info(f"🧯 Flatten BUY missing {missing_asset[-6:]} need={need_int} do={size_int} px={px:.2f} remaining={remaining:.2f}")
                oid = self._place_taker_bid_fak(missing_asset, px, float(size_int))
            else:
                self.logger.info(f"🧯 Flatten BUY missing not affordable (need={need_int}, max_affordable={max_affordable}) -> fallback SELL heavy")
                action = "SELL_HEAVY"

        if action == "SELL_HEAVY":
            # Marketable SELL: price at (bid - slip)
            px = clamp(round_down(heavy_bid - slip, self.cfg.tick), self.cfg.tick, 0.99)
            self.logger.info(f"🧯 Flatten SELL heavy {heavy_asset[-6:]} size={need_int} px={px:.2f}")
            oid = self._place_taker_ask_fak(heavy_asset, px, float(need_int))

        # Stop regardless; we tried to flatten.
        self.exit_reason = "CIRCUIT_BREAKER_FLATTEN"

        # Allow WS fills to arrive briefly.
        time.sleep(2)

        self.cancel_all_orders_exchange(reason="circuit breaker stop")
        self.stop_event.set()
    # ---------------- Emergency taker hedge ----------------
    # ---------------- Emergency taker hedge ----------------
    def _emergency_taker_hedge_step(self, delta: float, reason: str) -> None:
        """
        Force a taker hedge (FAK by default) for the missing side.

        HARD SAFETY:
          - respects max_total_cost + reserve_usd (will NOT spend above cap)
          - throttles repeated taker hedges
          - uses integer shares for marketable BUY precision rules
        """
        now = time.time()

        # Optional "inflight" throttle: if we recently fired a taker hedge, wait for WS fills
        if time.time() < getattr(self, "_taker_inflight_until", 0.0):
            return

        if (now - self._last_taker_hedge_ts) < self._taker_hedge_min_interval:
            return
        self._last_taker_hedge_ts = now

        missing_asset = self.no_asset if delta > 0 else self.yes_asset

        # Strict inflight gating: if a prior taker BUY hedge for this asset is still pending, wait
        if getattr(self, "taker_strict_inflight", True) and self._has_pending_taker_order("BUY", missing_asset):
            return

        ba = self._best_bid_ask(missing_asset)
        if not ba:
            self.logger.info(f"⚠️ Emergency hedge: missing best_bid_ask for {missing_asset[-6:]} ({reason})")
            return
        _, ask = ba
        if ask <= 0:
            self.logger.info(f"⚠️ Emergency hedge: missing ask for {missing_asset[-6:]} ({reason})")
            return

        # ===== NO-LOSS HEDGE CAP =====
        # Never taker-hedge above the break-even cap, otherwise we lock in a guaranteed loss.
        cap = self._hedge_price_cap()

        # Aggressive limit: ask + slippage_ticks*tick (rounded UP so it stays marketable)
        px_candidate = ask + float(self.hedge_slippage_ticks) * self.cfg.tick
        px_candidate = round_up(px_candidate, self.cfg.tick)
        px_candidate = clamp(px_candidate, self.cfg.tick, 0.99)

        # HARD FIX: clamp taker hedge price to cap so slippage cannot push it above cap.
        # If cap is below the current ask, we cannot taker-hedge without locking a loss.
        px = min(float(cap), float(px_candidate))
        px = round_down(px, self.cfg.tick)

        if cap <= 0:
            self.logger.info(f"🛑 Hedge cap <= 0 (cap={cap:.2f}) -> STOP")
            self.exit_reason = "CAP_LOCKED_LOSS"
            self.cancel_all_orders_exchange(reason="cap<=0 locked loss")
            self.stop_event.set()
            return

        if (ask > cap) or (px + 1e-9 < ask):
            # If we've very recently entered a cap-blocked state for the same asset/cap, avoid spamming logs/actions.
            if (
                now < float(self._cap_blocked_until)
                and str(self._cap_blocked_asset) == str(missing_asset)
                and self._cap_blocked_cap is not None
                and abs(float(self._cap_blocked_cap) - float(cap)) <= (2 * self.cfg.tick)
            ):
                return

            self._cap_blocked_until = now + float(self.cap_blocked_cooldown_seconds)
            self._cap_blocked_asset = str(missing_asset)
            self._cap_blocked_cap = float(cap)

            self.logger.info(f"🛑 Emergency hedge blocked: ask={ask:.2f} cap={cap:.2f} (px={px:.2f}) ({reason}).")
            # Best-effort: place/keep a passive maker hedge at the cap. Do NOT taker-chase into a guaranteed loss.
            size_try = max(float(self.cfg.min_shares), float(min(abs(delta), self.cfg.clip_shares)))
            self._maybe_replace(missing_asset, cap, size_try, stale_seconds=self.hedge_stale_seconds)

            # Cancel any other resting orders (especially the heavy side) but keep the hedge order working.
            self.cancel_all_open_orders_local_except(missing_asset, reason="hedge cap blocked (keep hedge)")

            # Do not stop the bot here; keep running so we can potentially get filled at cap or manage near-expiry.
            return

        # ===== HARD BUDGET CAP (APPLIES TO HEDGE TOO) =====
        with self.state_lock:
            total_cost = float(self.state["c_yes"]) + float(self.state["c_no"])

        remaining_usd = self.cfg.max_total_cost - total_cost - self.cfg.reserve_usd
        if remaining_usd <= 0:
            self.logger.info(
                f"🛑 No remaining budget to hedge. total_cost={total_cost:.2f} "
                f"cap={self.cfg.max_total_cost:.2f} reserve={self.cfg.reserve_usd:.2f} -> STOP"
            )
            self.exit_reason = "NO_BUDGET"
            self.cancel_all_orders_exchange(reason="no budget to hedge")
            self.stop_event.set()
            return

        # Desired hedge shares (integer for marketable BUY precision)
        need_int = int(math.floor(abs(delta) + 1e-12))

        # Max shares we can afford at px within remaining budget
        max_affordable = int(math.floor(remaining_usd / px + 1e-12))

        # Final hedge size (integer)
        size_int = min(need_int, max_affordable)

        if size_int < max(1, int(math.ceil(float(self.cfg.min_shares) - 1e-12))):
            self.logger.info(
                f"🛑 Hedge too expensive for remaining budget. remaining={remaining_usd:.2f} px={px:.2f} "
                f"need={need_int} max_affordable={max_affordable} -> STOP"
            )
            self.exit_reason = "HEDGE_TOO_EXPENSIVE"
            self.cancel_all_orders_exchange(reason="hedge too expensive")
            self.stop_event.set()
            return

        partial = (size_int < need_int)
        need = float(size_int)

        self.logger.info(
            f"🚑 EMERGENCY HEDGE ({reason}) delta={delta:.4f} need={need:.0f} "
            f"buy={missing_asset[-6:]} ask={ask:.2f} px={px:.2f} "
            f"remaining_usd={remaining_usd:.2f} type={self.hedge_taker_order_type}"
        )

        # Cancel local resting orders first (avoid accidental re-imbalance while hedging)
        self.cancel_all_open_orders_local(reason="before emergency taker hedge")

        # Mark inflight to avoid spamming; cleared when fills are processed (if you implement that)
        self._taker_inflight_until = time.time() + 2.0

        oid = self._place_taker_bid_fak(missing_asset, px, need)

        # If we could only hedge partially due to budget, stop after the taker attempt to avoid further exposure growth.
        if partial and (oid or self.cfg.dry_run):
            self.logger.info(f"🛑 Partial hedge executed ({int(size_int)}/{int(need_int)} shares) due to budget. Stopping.")
            time.sleep(1)
            self.exit_reason = "PARTIAL_HEDGE_BUDGET"
            self.cancel_all_orders_exchange(reason="partial hedge stop")
            self.stop_event.set()
            return

    
    # ============================
    # SNIPER (high-probability) logic
    # ============================
    def _sniper_best_snapshot(self) -> Tuple[float, float, float, float]:
        """Return (yes_bid, yes_ask, no_bid, no_ask) from cached best quotes.

        Robust to different cache formats:
          - {"bid": x, "ask": y}
          - (bid, ask)
        """

        def _unpack(v) -> Tuple[float, float]:
            try:
                if isinstance(v, dict):
                    return float(v.get("bid") or 0.0), float(v.get("ask") or 0.0)
                if isinstance(v, (list, tuple)) and len(v) >= 2:
                    return float(v[0] or 0.0), float(v[1] or 0.0)
            except Exception:
                pass
            return 0.0, 0.0

        with self.best_lock:
            y = self.best.get(self.yes_asset)
            n = self.best.get(self.no_asset)

        yb, ya = _unpack(y)
        nb, na = _unpack(n)
        return float(yb), float(ya), float(nb), float(na)

    def _sniper_mark_to_market_pnl(self) -> float:
        """Conservative MTM PnL using *realizable* exit prices (bid - slippage)."""
        y_bid, _, n_bid, _ = self._sniper_best_snapshot()
        y_px = self._sniper_est_exit_price(float(y_bid))
        n_px = self._sniper_est_exit_price(float(n_bid))
        with self.state_lock:
            qy = float(self.state.get("q_yes", 0.0))
            qn = float(self.state.get("q_no", 0.0))
            cy = float(self.state.get("c_yes", 0.0))
            cn = float(self.state.get("c_no", 0.0))
        return float((qy * y_px - cy) + (qn * n_px - cn))
    def _sniper_position(self) -> Optional[dict]:
        """Return current directional position (prefers the larger leg if both exist)."""
        y_bid, y_ask, n_bid, n_ask = self._sniper_best_snapshot()
        with self.state_lock:
            qy = float(self.state.get("q_yes", 0.0))
            qn = float(self.state.get("q_no", 0.0))
            cy = float(self.state.get("c_yes", 0.0))
            cn = float(self.state.get("c_no", 0.0))

        min_sh = float(self.cfg.min_shares)

        # Choose the "dominant" leg if both exist (shouldn't happen in pure sniper mode, but safe).
        if qy >= min_sh and (qy >= qn or qn < min_sh):
            avg = (cy / qy) if qy > 0 else 0.0
            return {"side": "YES", "asset_id": self.yes_asset, "qty": qy, "cost": cy, "avg": avg, "bid": y_bid, "ask": y_ask}
        if qn >= min_sh:
            avg = (cn / qn) if qn > 0 else 0.0
            return {"side": "NO", "asset_id": self.no_asset, "qty": qn, "cost": cn, "avg": avg, "bid": n_bid, "ask": n_ask}
        return None

    def _sniper_est_entry_price(self, ask: float) -> float:
        """Estimate the actual BUY limit price we'll send for sniper entries.

        Uses the same mechanics as the real entry order:
          - ask + entry_slippage_ticks * tick
          - round UP to tick (avoid underbidding due to float rounding)
          - clamp to sniper_hard_max_price (safety cap)
        """
        if ask <= 0:
            return 0.0
        try:
            tick = float(self.cfg.tick) if float(self.cfg.tick) > 0 else 0.01
        except Exception:
            tick = 0.01

        hard_max = float(getattr(self, "sniper_hard_max_price", self.sniper_price_max) or self.sniper_price_max)

        px_raw = float(ask) + float(self.sniper_entry_slippage_ticks) * tick
        px = min(px_raw, hard_max)
        px = round_up(px, tick)
        px = clamp(px, tick, hard_max)
        return float(px)

    def _sniper_est_exit_price(self, bid: float, extra_slip_ticks: float = 0.0) -> float:
        """Estimate a realizable SELL limit price for exits (bid - slippage, tick-rounded).

        This is intentionally *conservative* and is used for TP/SL trigger calculations.
        The actual exit routine may widen slippage slightly on retries.
        """
        if bid <= 0:
            return 0.0
        try:
            tick = float(self.cfg.tick) if float(self.cfg.tick) > 0 else 0.01
        except Exception:
            tick = 0.01

        slip = float(self.sniper_exit_slippage_ticks) + float(extra_slip_ticks or 0.0)
        px_raw = float(bid) - slip * tick
        px = max(tick, px_raw)

        # Round DOWN so we don't accidentally post above bid (maker) due to float rounding.
        px = round_down(px, tick)

        # Keep behavior consistent with actual exit order placement (avoid accidental 1.00 asks).
        px = clamp(px, tick, 0.99)
        return float(px)

    def _sniper_maybe_endgame_blind_post(self, seconds_left: float, now_ts: float) -> bool:
        """Attempt a last-second 'blind' resting limit BUY even if the WS feed is stale/disconnected.

        This exists for extremely competitive endgame markets where:
          - losing-side bids often go to 0.00 (making a 2-sided snapshot unavailable), and/or
          - market/user websockets can disconnect in the final seconds.

        When enabled (SNIPER_ENDGAME_BLIND_POST=true), the bot will try to place ONE resting GTC BUY
        at a fixed price (usually 0.99) during the final N seconds before expiry (and optionally for
        a short grace period after expiry). This bypasses the normal SNIPER entry gates that require
        a fresh 2-sided BBO snapshot.

        Returns True if an endgame order is already resting or was submitted by this call.
        """
        if not bool(getattr(self, "sniper_endgame_blind_post", False)):
            return False

        try:
            win_s = float(getattr(self, "sniper_endgame_blind_post_window_seconds", 0.0) or 0.0)
        except Exception:
            win_s = 0.0
        if win_s <= 0:
            return False

        # Window relative to expiry (seconds_left decreases to 0 at expiry, negative after)
        grace = float(getattr(self, "sniper_expiry_grace_seconds", 0.0) or 0.0)
        if float(seconds_left) > (win_s + 1e-9):
            return False
        if float(seconds_left) < (-grace - 1e-9):
            return False

        # Only attempt when flat (no position yet).
        with self.state_lock:
            qy = float(self.state.get("q_yes", 0.0))
            qn = float(self.state.get("q_no", 0.0))
            trade_count = int(self.state.get("sniper_trade_count", 0))

        min_sh = float(self.cfg.min_shares)
        if qy >= (min_sh - 1e-9) or qn >= (min_sh - 1e-9):
            return False

        if trade_count >= int(getattr(self, "sniper_max_trades_per_market", 1) or 1):
            return False

        # Throttle endgame attempts (avoid spamming if the exchange rejects due to closure).
        if (float(now_ts) - float(getattr(self, "_sniper_endgame_post_last_attempt_ts", 0.0) or 0.0)) < 0.20:
            return False
        self._sniper_endgame_post_last_attempt_ts = float(now_ts)

        # Determine side to post.
        side_cfg = str(getattr(self, "sniper_endgame_side", "AUTO") or "AUTO").upper().strip()
        side: Optional[str] = None

        if side_cfg in ("YES", "NO"):
            side = side_cfg
        else:
            # AUTO: use last-known asks (even if the losing side bid is 0).
            yb, ya, nb, na = self._sniper_best_snapshot()

            max_stale = float(getattr(self, "sniper_endgame_blind_post_max_stale_seconds", 60.0) or 60.0)
            with self.best_lock:
                yts = float(self.best_ts.get(self.yes_asset, 0.0) or 0.0)
                nts = float(self.best_ts.get(self.no_asset, 0.0) or 0.0)

            def _ok_quote(a: float, ts: float) -> bool:
                if float(a) <= 0.0:
                    return False
                if float(ts) <= 0.0:
                    return False
                if max_stale > 0 and (float(now_ts) - float(ts)) > max_stale:
                    return False
                return True

            opts = []
            if _ok_quote(float(ya), float(yts)):
                opts.append(("YES", float(ya)))
            if _ok_quote(float(na), float(nts)):
                opts.append(("NO", float(na)))

            if not opts:
                # We don't have a usable last-known ask for either side.
                return False

            # Safety: prefer a side already at/above price_min (implied high-prob).
            price_min = float(getattr(self, "sniper_price_min", 0.0) or 0.0)
            eps = float(getattr(self, "sniper_price_max_epsilon", 0.0) or 0.0)
            require_min = bool(getattr(self, "sniper_endgame_require_price_min", True))

            good = [o for o in opts if float(o[1]) + 1e-12 >= (price_min - eps)]
            if len(good) == 1:
                side = str(good[0][0])
            elif len(good) > 1:
                # Rare: both sides look "high". Choose the higher ask (more likely winner).
                side = str(max(good, key=lambda x: float(x[1]))[0])
            else:
                if require_min and price_min > 0:
                    return False
                # If user disabled the price_min requirement, just pick the higher ask.
                side = str(max(opts, key=lambda x: float(x[1]))[0])

        if side not in ("YES", "NO"):
            return False

        asset_id = self.yes_asset if side == "YES" else self.no_asset

        # If we already have a resting order for this asset, don't post again.
        with self.state_lock:
            oo = (self.state.get("open_orders") or {}).get(str(asset_id))
        if oo and oo.get("order_id"):
            return True

        # Target price: default to hard_max/price_max (user usually sets this to 0.99).
        try:
            tick = float(self.cfg.tick) if float(self.cfg.tick) > 0 else 0.01
        except Exception:
            tick = 0.01

        px = float(getattr(self, "sniper_endgame_blind_post_price", 0.0) or 0.0)
        if px <= 0.0:
            px = float(getattr(self, "sniper_hard_max_price", getattr(self, "sniper_price_max", 0.99)) or getattr(self, "sniper_price_max", 0.99))

        hard_max = float(getattr(self, "sniper_hard_max_price", px) or px)
        if hard_max <= 0.0:
            hard_max = float(px)

        # Round DOWN so we never exceed the intended limit due to float noise.
        px = min(float(px), float(hard_max))
        px = round_down(px, tick)
        px = clamp(px, tick, float(hard_max))

        # Size: use normal sniper budgeting unless overridden.
        try:
            size_target = int(self._sniper_calc_entry_size(float(px)))
        except Exception:
            size_target = 0
        if size_target <= 0:
            return False

        size_override = int(getattr(self, "sniper_endgame_blind_post_size_shares", 0) or 0)
        if size_override > 0:
            size_target = min(int(size_target), int(size_override))

        # Round down to multiple of min_shares (avoid dust)
        min_int = max(1, int(math.ceil(float(self.cfg.min_shares) - 1e-12)))
        if size_target < min_int:
            return False
        size_target = (int(size_target) // min_int) * min_int
        if size_target < min_int:
            return False

        post_only = True if bool(getattr(self, "sniper_entry_post_only", False)) else None

        # Log (note: we intentionally do NOT require fresh data here)
        try:
            fresh = "Y" if self._market_data_fresh() else "N"
        except Exception:
            fresh = "?"
        self.logger.info(
            f"⚡ [SNIPER] ENDGAME blind-post side={side} px={px:.3f} sz={size_target} t_left={float(seconds_left):.2f}s fresh={fresh}"
        )

        oid = self._place_limit_bid_gtc(
            asset_id=str(asset_id),
            price=float(px),
            size=float(size_target),
            post_only=post_only,
        )
        if not oid:
            return False

        # Record submission metadata (fills may arrive later).
        try:
            with self.state_lock:
                self.state["sniper_last_entry_ts"] = float(time.time())
                self.state["sniper_last_side"] = str(side)
                self.state["sniper_last_entry_order_id"] = str(oid)
                save_state(self.state_file, self.state)
        except Exception:
            pass

        return True


    def _sniper_entry_candidate(self, seconds_left: float, ignore_roi_gate: bool = False) -> Optional[dict]:
        """Check if there's a high-probability side worth sniping right now.

        IMPORTANT: price/ROI gates use the *realizable entry price* (ask + slippage, tick-rounded),
        not the raw ask snapshot. This prevents 'looks good' signals that cannot actually be executed.
        """
        y_bid, y_ask, n_bid, n_ask = self._sniper_best_snapshot()
        if y_ask <= 0 or n_ask <= 0 or y_bid <= 0 or n_bid <= 0:
            return None

        y_mid = 0.5 * (y_bid + y_ask)
        n_mid = 0.5 * (n_bid + n_ask)
        parity = abs((y_mid + n_mid) - 1.0)
        if parity > float(self.sniper_parity_tolerance):
            return None

        # Pick the favored (higher mid) side.
        side = "YES" if y_mid >= n_mid else "NO"
        asset_id = self.yes_asset if side == "YES" else self.no_asset
        bid = y_bid if side == "YES" else n_bid
        ask = y_ask if side == "YES" else n_ask

        # Thin spread filter
        try:
            tick = float(self.cfg.tick) if float(self.cfg.tick) > 0 else 0.01
        except Exception:
            tick = 0.01

        entry_type_name = str(getattr(self, "sniper_entry_order_type", "FOK") or "FOK").upper().strip()
        limit_entry = entry_type_name in ("GTC", "LIMIT")

        spread_ticks = int(round((ask - bid) / tick))
        if spread_ticks > int(self.sniper_max_spread_ticks):
            return None

        # High-prob (price ~= probability) gate (inclusive, with tiny epsilon for float/tick rounding)
        price_min = float(self.sniper_price_min)
        price_max = float(self.sniper_price_max)
        eps = float(getattr(self, "sniper_price_max_epsilon", 0.0) or 0.0)
        hard_max = float(getattr(self, "sniper_hard_max_price", price_max) or price_max)

        # Estimate the *actual* entry price we'd have to pay.
        entry_px = self._sniper_est_entry_price(float(ask))
        if entry_px <= 0:
            return None

        # If our capped/rounded entry px is *below* the current ask, the order is not marketable → won't fill.
        if (not limit_entry) and (entry_px + 1e-12 < float(ask)):
            return None

        # HARD cap check: if ask is meaningfully above hard_max, don't even try.
        # (Allow a very tiny float tolerance; do NOT use eps here because eps may be larger than tick.)
        if (not limit_entry) and (float(ask) > (hard_max + 1e-9)):
            return None

        # Price gates use realizable entry px (not raw ask)
        if entry_px < (price_min - eps) or entry_px > (price_max + eps):
            return None

        if not bool(ignore_roi_gate):


            # ROI gate: ensure TP is *achievable* given max possible exit price.
            # If we plan to exit before expiry, realistic ceiling is ~0.99 (top of the book range).
            max_exit_price = 0.99 if bool(self.sniper_exit_before_expiry) else 1.00
            max_roi = (max_exit_price - entry_px) / entry_px if entry_px > 0 else 0.0

            # Approx fee allowance: buy+sell (2 legs) if exiting; buy only if holding to settlement.
            fee_allow = (2.0 if bool(self.sniper_exit_before_expiry) else 1.0) * float(self.sniper_fee_rate)
            required_roi = float(self.sniper_take_profit_pct) + fee_allow + float(self.sniper_min_edge_over_fees)
            if max_roi + 1e-9 < required_roi:
                return None

        return {
            "side": side,
            "asset_id": asset_id,
            "bid": float(bid),
            "ask": float(ask),
            "entry_px": float(entry_px),
            "spread_ticks": int(spread_ticks),
            "parity": float(parity),
            "seconds_left": float(seconds_left),
        }

    def _sniper_entry_confirmed(self, cand: dict, now_ts: float) -> bool:
        """Debounce entry signals to reduce whipsaw entries.

        If SNIPER_ENTRY_CONFIRM_SECONDS > 0, the bot will only attempt an entry after the
        *entry candidate remains valid continuously* for that many seconds.

        This is intentionally conservative:
        - if the candidate disappears (price leaves gates / spread widens / parity breaks), the timer resets
        - if the preferred side flips (YES↔NO), the timer resets
        """
        confirm_s = float(getattr(self, "sniper_entry_confirm_seconds", 0.0) or 0.0)
        if confirm_s <= 0:
            return True

        side = str(cand.get("side") or "")
        if side not in ("YES", "NO"):
            # Defensive: treat unknown as not confirmed and reset.
            self._sniper_entry_gate_since = None
            self._sniper_entry_gate_side = None
            return False

        if self._sniper_entry_gate_since is None or self._sniper_entry_gate_side != side:
            self._sniper_entry_gate_since = float(now_ts)
            self._sniper_entry_gate_side = side
            return False

        return (float(now_ts) - float(self._sniper_entry_gate_since)) >= confirm_s

    def _sniper_calc_entry_size(self, entry_price: float) -> int:
        """Compute integer share size respecting budget + min order constraints.

        IMPORTANT: This enforces SNIPER_MAX_NOTIONAL_USD strictly even when multiple taker BUY orders are
        in-flight (e.g., repeated FOK attempts on a thin book). We reserve notional for recent pending
        taker BUY orders so we never *potentially* exceed the configured cap if more than one order fills.

        Note: We only reserve pending notional for a short age window (SNIPER_PENDING_ORDER_MAX_AGE_SECONDS)
        because FOK/FAK are IOC-like and should resolve quickly. This avoids the bot getting stuck if a
        cancellation event is missed.
        """
        if entry_price <= 0:
            return 0
        min_sh = max(1, int(math.ceil(float(self.cfg.min_shares) - 1e-12)))

        with self.state_lock:
            net_spend = float(self.state.get("c_yes", 0.0)) + float(self.state.get("c_no", 0.0))

        # Reserve budget for recent in-flight taker BUY orders (prevents accidental over-buying).
        pending_buy = 0.0
        try:
            pending_age = float(getattr(self, "sniper_pending_order_max_age_seconds", 0.0) or 0.0)
            pending_buy = float(self._pending_taker_notional_usd(side="BUY", max_age_seconds=pending_age))
        except Exception:
            pending_buy = 0.0

        # Reserve budget for locally tracked open BUY orders (e.g., resting LIMIT/GTC entries).
        open_buy = 0.0
        try:
            with self.state_lock:
                oo = dict(self.state.get("open_orders") or {})
            for _aid, row in oo.items():
                px_o = float((row or {}).get("price", 0.0) or 0.0)
                sz_o = float((row or {}).get("size", 0.0) or 0.0)
                if px_o > 0 and sz_o > 0:
                    open_buy += px_o * sz_o
        except Exception:
            open_buy = 0.0

        # Budget: max_total_cost - reserve - already_spent - pending - open_orders,
        # additionally capped by sniper_max_notional_usd - pending - open_orders.
        remaining_budget = float(self.cfg.max_total_cost) - float(self.cfg.reserve_usd) - net_spend - pending_buy - open_buy
        cap_remaining = float(self.sniper_max_notional_usd) - pending_buy - open_buy

        notional = min(max(0.0, cap_remaining), max(0.0, remaining_budget))
        if notional < float(self.min_taker_notional) - 1e-9:
            return 0

        size = int(math.floor(notional / float(entry_price) + 1e-12))

        # Round down to a multiple of min_shares to avoid leaving remainders we can't exit.
        if size >= min_sh:
            size = (size // min_sh) * min_sh

        # Final guards
        if size < min_sh:
            return 0
        if size * float(entry_price) < float(self.min_taker_notional) - 1e-9:
            return 0
        return int(size)

    def _log_status_sniper(self, seconds_left: float):
        pos = self._sniper_position()
        pnl = self._sniper_mark_to_market_pnl()
        with self.state_lock:
            tc = float(self.state.get("sniper_trade_count", 0))
        if pos is None:
            print(f"[SNIPER] t_left={seconds_left:6.1f}s trades={int(tc)} pnl(mtm)={pnl:+.4f} (flat)")
            return

        cost = float(pos["cost"])
        qty = float(pos["qty"])
        bid = float(pos["bid"])
        avg = float(pos["avg"])

        # Use realizable exit price for displayed PnL% (matches TP/SL triggers).
        exit_px = self._sniper_est_exit_price(bid)
        pnl_est = qty * exit_px - cost
        pnl_pct = pnl_est / cost if cost > 1e-12 else 0.0

        print(
            f"[SNIPER] t_left={seconds_left:6.1f}s trades={int(tc)} side={pos['side']} "
            f"qty={qty:.2f} avg={avg:.3f} bid={bid:.3f} ex={exit_px:.3f} pnl={pnl_est:+.4f} ({pnl_pct*100:+.2f}%)"
        )
    def _sniper_try_enter(self, cand: dict) -> bool:
            """Attempt a sniper entry (taker BUY on favored side).

            Improvements:
              - uses realizable entry price (ask + slippage, tick-rounded) consistently with ROI/price gates
              - adds a safe reliability fallback: if FOK fails, shrink size and retry (still no partial fills)
              - optional final fallback to a different order type (e.g. FAK) if configured
            """
            now = time.time()
            if now < getattr(self, "_taker_fail_pause_until", 0.0):
                return False

            # If an entry/exit order was just submitted, allow WS fills to land before attempting again.
            if now < float(getattr(self, "_taker_inflight_until", 0.0) or 0.0):
                return False

            # Simple throttle against duplicate rapid-fire signals
            if now - float(self.sniper_last_signal_ts) < 0.25:
                return False
            self.sniper_last_signal_ts = now

            ask = float(cand.get("ask", 0.0) or 0.0)
            if ask <= 0:
                return False

            entry_type_name = str(getattr(self, "sniper_entry_order_type", "FOK") or "FOK").upper().strip()
            limit_entry = entry_type_name in ("GTC", "LIMIT")

            # Realizable BUY px (for IOC types it must be >= ask to fill; for LIMIT/GTC we may rest below ask)
            px = float(cand.get("entry_px", 0.0) or 0.0)
            if px <= 0:
                px = self._sniper_est_entry_price(ask)
            if px <= 0:
                return False
            if (not limit_entry) and (px + 1e-12 < ask):
                # Not marketable (typically due to a too-low hard cap)
                return False

            try:
                tick = float(self.cfg.tick) if float(self.cfg.tick) > 0 else 0.01
            except Exception:
                tick = 0.01

            hard_max = float(getattr(self, "sniper_hard_max_price", self.sniper_price_max) or self.sniper_price_max)
            px = clamp(round_up(px, tick), tick, hard_max)

            # Minimum integer lot (exchange enforces a min size; we also round sizes to multiples of min_shares)
            min_sh = max(1, int(math.ceil(float(self.cfg.min_shares) - 1e-12)))

            # Size based on the *worst case* limit we might pay.
            size_int = self._sniper_calc_entry_size(px)
            if size_int <= 0:
                # Common cause in SIGNAL_SNIPPER: max_notional too small to meet min_shares at this price.
                if bool(getattr(self, "signal_sniper_mode", False)) and bool(getattr(self, "signal_debug", False)):
                    try:
                        req_usd = float(min_sh) * float(px)
                    except Exception:
                        req_usd = 0.0
                    try:
                        cap = float(getattr(self, "sniper_max_notional_usd", 0.0) or 0.0)
                    except Exception:
                        cap = 0.0
                    self.logger.info(
                        f"[SIGNAL] skip entry: cannot meet min_shares with current budget. "
                        f"min_shares={min_sh} px={px:.4f} -> min_required≈{req_usd:.2f} USD "
                        f"but SIGNAL_MAX_NOTIONAL_USD={cap:.2f}."
                    )
                return False

            # Inflight window (let fills land; prevents spam-chasing / duplicate entries)
            inflight_s = float(getattr(self, "sniper_entry_inflight_seconds", 1.5) or 1.5)
            self._taker_inflight_until = time.time() + max(0.25, inflight_s)

            asset_id = str(cand["asset_id"])

            # If a recent BUY taker order for this asset is still "pending" (or we simply haven't
            # received the WS fill/cancel yet), do not submit another. This prevents accidental
            # double-buys when the loop is fast and exchange/websocket events lag.
            pending_age_s = float(getattr(self, "sniper_pending_order_max_age_seconds", 0.0) or 0.0)
            if (not limit_entry) and pending_age_s > 0 and self._has_pending_taker_order_recent(
                "BUY", asset_id=asset_id, max_age_seconds=pending_age_s
            ):
                return False

            target = int(size_int)

            # Optional chunking: submit multiple smaller taker orders (typically FOK) to improve fill probability on thin books.
            # Enable by setting SNIPER_ENTRY_CHUNK_SHARES (must be a multiple of min_shares).
            chunk_cfg = int(getattr(self, "sniper_entry_chunk_shares", 0) or 0)
            chunk = int(target) if chunk_cfg <= 0 else int(chunk_cfg)
            if chunk < min_sh:
                chunk = min_sh
            chunk = (chunk // min_sh) * min_sh
            if chunk < min_sh:
                chunk = min_sh

            max_orders = int(getattr(self, "sniper_entry_max_orders", 3) or 3)
            if max_orders < 1:
                max_orders = 1

            primary_type = str(self.sniper_entry_order_type or "FOK").upper().strip()
            fallback_type = str(getattr(self, "sniper_entry_order_type_fallback", "") or "").upper().strip()

            # Safe reliability fallback for FOK: shrink size and retry.
            shrink_factor = float(getattr(self, "sniper_entry_shrink_factor", 0.5) or 0.5)
            if not (0.05 <= shrink_factor < 1.0):
                shrink_factor = 0.5

            shrink_min = int(getattr(self, "sniper_entry_shrink_min_chunk_shares", 0) or 0)
            if shrink_min <= 0:
                shrink_min = min_sh
            if shrink_min < min_sh:
                shrink_min = min_sh
            shrink_min = (shrink_min // min_sh) * min_sh
            if shrink_min < min_sh:
                shrink_min = min_sh

            # IMPORTANT: To prevent accidental double-buys, we submit *at most one* entry order per
            # sniper loop cycle. If WS fill/cancel events are delayed, bursting multiple FOK orders
            # back-to-back can result in over-buying. If you want larger entries, increase
            # SNIPER_ENTRY_CHUNK_SHARES (or disable chunking) rather than relying on multi-order bursts.
            orders_sent = 0
            any_submitted = False

            desired_chunk = int(min(chunk, target))
            desired_chunk = (desired_chunk // min_sh) * min_sh
            if desired_chunk < min_sh:
                return False

            # LIMIT/GTC entry: place one resting order and treat submission as success.
            if primary_type in ("GTC", "LIMIT"):
                # Avoid duplicate resting orders on fast loops.
                with self.state_lock:
                    oo = (self.state.get("open_orders") or {}).get(asset_id)
                if oo and oo.get("order_id"):
                    return True

                post_only = True if bool(getattr(self, "sniper_entry_post_only", False)) else None
                oid_lim = self._place_limit_bid_gtc(
                    asset_id=asset_id,
                    price=float(px),
                    size=float(desired_chunk),
                    post_only=post_only,
                )
                if not oid_lim:
                    return False

                # Record submission metadata (fills may arrive later).
                with self.state_lock:
                    self.state["sniper_last_entry_ts"] = float(time.time())
                    self.state["sniper_last_side"] = str(cand.get("side", ""))
                    self.state["sniper_last_entry_order_id"] = str(oid_lim)
                    save_state(self.state_file, self.state)

                return True

            # Build a list of sizes to try for this single entry attempt.
            sizes_to_try = [desired_chunk]
            if primary_type == "FOK":
                # Shrink on failures down to shrink_min (still FOK → no partial fills)
                s = desired_chunk
                while s > shrink_min:
                    s2 = int(math.floor(s * shrink_factor + 1e-12))
                    s2 = max(shrink_min, s2)
                    s2 = (s2 // min_sh) * min_sh
                    if s2 < shrink_min:
                        s2 = shrink_min
                    if s2 >= s:
                        break
                    sizes_to_try.append(int(s2))
                    s = s2

                # Ensure we always attempt at shrink_min at least once.
                if sizes_to_try[-1] != shrink_min:
                    sizes_to_try.append(int(shrink_min))

            submitted_primary = False
            for this_chunk in sizes_to_try:
                if orders_sent >= max_orders:
                    break

                oid = self._place_taker_bid_fak(
                    asset_id=asset_id,
                    price=float(px),
                    size=float(this_chunk),
                    order_type_name=primary_type,
                )
                orders_sent += 1

                if oid:
                    any_submitted = True
                    submitted_primary = True
                    break

            # If primary failed, optionally try a fallback type (e.g. FAK) ONCE, then stop.
            if (not submitted_primary) and fallback_type and fallback_type != primary_type and orders_sent < max_orders:
                fb_chunk = int(max(shrink_min, desired_chunk))
                fb_chunk = (fb_chunk // min_sh) * min_sh
                if fb_chunk < min_sh:
                    fb_chunk = min_sh

                oid2 = self._place_taker_bid_fak(
                    asset_id=asset_id,
                    price=float(px),
                    size=float(fb_chunk),
                    order_type_name=fallback_type,
                )
                orders_sent += 1
                if oid2:
                    any_submitted = True

            if not any_submitted:
                return False

            # Wait briefly for fill events to land (trade/order WS).
            try:
                # Wait long enough for WS fills to arrive (but keep it bounded).
                wait_s = max(1.0, min(4.0, float(inflight_s)))
                self.position_update_event.wait(timeout=wait_s)
                self.position_update_event.clear()
            except Exception:
                pass

            # Verify whether we actually got a position (some venues accept FOK/FAK orders and then kill them
            # without raising an exception; in that case we must not assume we entered).
            filled = False
            try:
                filled = self._sniper_position() is not None
            except Exception:
                filled = False

            if not filled:
                # No fills observed – treat as a failed attempt (common on thin books with FOK/FAK).
                # Pause briefly before retrying to reduce order spam / rate-limit risk.
                pause_s = float(getattr(self, "sniper_entry_retry_pause_seconds", 0.0) or 0.0)
                if pause_s > 0:
                    self._taker_fail_pause_until = time.time() + pause_s
                return False

            # Record successful entry metadata.
            with self.state_lock:
                self.state["sniper_last_entry_ts"] = float(time.time())
                self.state["sniper_last_side"] = str(cand.get("side", ""))
                save_state(self.state_file, self.state)

            return True
    
    def _sniper_try_exit(self, pos: dict, reason: str) -> bool:
        """Attempt to exit a sniper position.

        Default behaviour (SNIPER_STOP_LOSS_MODE=MARKET):
            - Aggressive exit via marketable taker SELLs (FOK → optional FAK fallback),
              widening slippage a tick per retry pass.

        Optional stop-limit behaviour (SNIPER_STOP_LOSS_MODE=LIMIT) for STOP_LOSS exits only:
            - Never sells below the configured stop floor
                floor = entry_ref_price * (1 - SNIPER_STOP_LOSS_PCT)
            - Places/keeps a GTC limit SELL at that floor (may not fill in a fast crash).
        """
        now = time.time()
        if now < getattr(self, "_taker_fail_pause_until", 0.0):
            return False

        asset_id = str(pos.get("asset_id") or "")
        if not asset_id:
            return False

        reason_u = str(reason or "").upper().strip()

        # Normalize mode string
        mode = str(getattr(self, "sniper_stop_loss_mode", "MARKET") or "MARKET").upper().strip()
        if mode in ("STOP_LIMIT", "STOPLIMIT"):
            mode = "LIMIT"
        if mode in ("STOP_MARKET", "STOPMARKET", "TAKER", "AGGRESSIVE"):
            mode = "MARKET"

        stop_limit_mode = (reason_u == "STOP_LOSS") and (mode == "LIMIT")

        remaining = float(pos.get("qty") or 0.0)
        if remaining <= 0.0:
            return True

        try:
            tick = float(self.cfg.tick) if float(self.cfg.tick) > 0 else 0.01
        except Exception:
            tick = 0.01

        min_int = max(1, int(math.ceil(float(self.cfg.min_shares) - 1e-12)))
        remaining_int = int(math.floor(float(remaining) + 1e-12))
        if remaining_int < min_int:
            return True

        # Best-effort balance/allowance snapshot to avoid "not enough balance/allowance" spam.
        balance_avail_int = remaining_int
        allow_int = remaining_int
        try:
            ba = self._get_balance_allowance_conditional_cached(asset_id, max_age_seconds=2.0)
            if ba:
                balance_avail_int = int(math.floor(float(ba[0]) + 1e-12))
                allow_int = int(math.floor(float(ba[1]) + 1e-12))
        except Exception:
            pass

        if allow_int < min_int:
            self.logger.warning(
                "⚠️ [SNIPER] exit failed: allowance too low. Approve conditional tokens for selling, then the bot will retry."
            )
            print(f"ba=({balance_avail_int}, {allow_int}) min_int={min_int} allow_int={allow_int} bal_int={balance_avail_int}")
            self._taker_fail_pause_until = time.time() + 60.0
            return False

        # ==========================
        # STOP_LIMIT stop-loss exits
        # ==========================
        if stop_limit_mode:
            stop_pct = float(getattr(self, "sniper_stop_loss_pct", 0.0) or 0.0)
            ref_px = float(getattr(self, "_sniper_entry_ref_price", 0.0) or 0.0)
            if ref_px <= 0.0:
                try:
                    ref_px = float(pos.get("avg", 0.0) or 0.0)
                except Exception:
                    ref_px = 0.0

            # If we can't compute a sane floor, fall back to MARKET behaviour.
            if ref_px <= 0.0 or stop_pct <= 0.0:
                stop_limit_mode = False
            else:
                floor_raw = float(ref_px) * (1.0 - float(stop_pct))
                # Round UP so we never go below the configured stop threshold.
                floor_px = clamp(round_up(float(floor_raw), float(tick)), float(tick), 0.99)

                # If we already placed the same stop-limit recently, just wait for fills.
                resubmit_s = float(getattr(self, "sniper_stop_limit_resubmit_seconds", 5.0) or 0.0)
                last_oid = getattr(self, "_sniper_stop_limit_order_id", None)
                last_ts = float(getattr(self, "_sniper_stop_limit_order_ts", 0.0) or 0.0)
                last_px = float(getattr(self, "_sniper_stop_limit_order_px", 0.0) or 0.0)

                if (
                    last_oid
                    and abs(float(last_px) - float(floor_px)) <= float(tick) / 2.0
                    and resubmit_s > 0.0
                    and (now - last_ts) < resubmit_s
                ):
                    return False

                # Best effort: cancel prior stop-limit (if any) only when resubmitting.
                if last_oid:
                    try:
                        self._cancel(str(last_oid))
                    except Exception:
                        pass

                # Cancel any open orders for this asset once per (re)submit
                try:
                    self.cancel_all_open_orders_local(reason=f"sniper stop-limit {reason_u}")
                    self._cancel_exchange_orders_for_assets([asset_id], reason=f"sniper stop-limit {reason_u}")
                    time.sleep(0.15)
                except Exception:
                    pass

                # Size: try to sell as much as possible without exceeding balance/allowance.
                sell_int = min(remaining_int, balance_avail_int, allow_int)
                sell_int = (sell_int // min_int) * min_int
                if sell_int < min_int:
                    return False

                stop_ot = str(getattr(self, "sniper_stop_limit_order_type", "GTC") or "GTC").upper().strip()
                oid = self._place_taker_ask_fak(
                    asset_id=asset_id,
                    price=float(floor_px),
                    size=float(sell_int),
                    order_type_name=stop_ot,
                )
                if oid:
                    self._sniper_stop_limit_order_id = str(oid)
                    self._sniper_stop_limit_order_ts = float(time.time())
                    self._sniper_stop_limit_order_px = float(floor_px)

                    # If it's marketable (bid >= floor), we may fill immediately.
                    try:
                        self.position_update_event.wait(timeout=1.0)
                        self.position_update_event.clear()
                    except Exception:
                        pass
                    if self._sniper_position() is None:
                        return True

                return False  # keep waiting / resubmitting later

        # ==========================
        # MARKET exits (default)
        # ==========================

        # Optionally cancel any existing exit orders to avoid locked balance errors.
        if bool(getattr(self, "sniper_cancel_exit_orders_before_retry", True)):
            self.cancel_all_open_orders_local(reason=f"sniper exit {reason_u}")
            self._cancel_exchange_orders_for_assets([asset_id], reason=f"sniper exit {reason_u}")
            time.sleep(0.2)

        chunk = int(getattr(self, "sniper_exit_chunk_shares", min_int) or min_int)
        if chunk < min_int:
            chunk = min_int

        max_passes = 4
        for pass_i in range(max_passes):
            cur = self._sniper_position()
            if cur is None:
                return True

            remaining = float(cur.get("qty") or 0.0)
            remaining_int = int(math.floor(float(remaining) + 1e-12))
            if remaining_int < min_int:
                return True

            bid = float(cur.get("bid") or 0.0)
            px = float(bid) - float(int(self.sniper_exit_slippage_ticks) + int(pass_i)) * float(tick)
            px = clamp(round_down(float(px), float(tick)), float(tick), 0.99)

            sell_int = min(remaining_int, chunk)
            sell_int = (sell_int // min_int) * min_int
            if sell_int < min_int:
                sell_int = min_int

            # Clamp to available balance/allowance best-effort (prevents oversize / allowance errors)
            sell_int = min(sell_int, balance_avail_int, allow_int)
            sell_int = (sell_int // min_int) * min_int
            if sell_int < min_int:
                return False

            ot = str(self.sniper_exit_order_type or "FOK").upper().strip()
            oid = self._place_taker_ask_fak(
                asset_id=asset_id,
                price=float(px),
                size=float(sell_int),
                order_type_name=ot,
            )

            # Optional fallback order type (e.g., FAK if FOK fails)
            if (not oid) and getattr(self, "sniper_exit_order_type_fallback", ""):
                fb = str(getattr(self, "sniper_exit_order_type_fallback", "") or "").upper().strip()
                if fb and fb != ot:
                    oid = self._place_taker_ask_fak(
                        asset_id=asset_id,
                        price=float(px),
                        size=float(sell_int),
                        order_type_name=fb,
                    )

            if not oid:
                continue

            # Wait for WS fill update
            try:
                self.position_update_event.wait(timeout=1.5)
                self.position_update_event.clear()
            except Exception:
                pass

            cur2 = self._sniper_position()
            if cur2 is None:
                return True

            rem2 = float(cur2.get("qty") or 0.0)
            if rem2 <= remaining - 1e-9:
                # Made progress; try again if needed.
                continue

            # No progress: cancel and pause briefly (avoid free-option + rate-limit spam)
            try:
                self._cancel(str(oid))
            except Exception:
                pass
            time.sleep(0.35)

        return False


    def _signal_direction_to_side(self, direction: str) -> Optional[str]:
        d = str(direction or "").strip().upper()
        if d in ("YES", "UP", "LONG", "BUY", "BULL"):
            return "YES"
        if d in ("NO", "DOWN", "SHORT", "SELL", "BEAR"):
            return "NO"
        return None

    def _signal_seen(self, key: str) -> bool:
        k = str(key or "").strip()
        if not k:
            return True
        with self.state_lock:
            seen = set(self.state.get("seen_signal_keys", []))
        return k in seen

    def _signal_mark_seen(self, sig: SignalTrade) -> None:
        try:
            k = str(sig.key or "").strip()
            if not k:
                return
            with self.state_lock:
                self.state.setdefault("seen_signal_keys", []).append(k)
                save_state(self.state_file, self.state)
        except Exception:
            pass

    def _ensure_signal_hub(self) -> None:
        """If the caller did not provide a SignalHub, create a local one (WS -> inbox)."""
        if not bool(getattr(self, "signal_sniper_mode", False)):
            return
        if self.signal_hub is not None:
            return
        prov = str(getattr(self, "signal_provider", "WEBSOCKET") or "").upper().strip()
        if prov != "WEBSOCKET":
            return

        # Local hub uses the bot's stop_event, so it will stop when this bot stops.
        try:
            file_log_raw = env_bool("SIGNAL_FILE_LOG_RAW", False)
        except Exception:
            file_log_raw = False

        fs = None
        try:
            fs = JsonlFileService(getattr(self, "signal_file_path", ""), enabled=True)
        except Exception:
            fs = None

        inbox = SignalInbox(stop_event=self.stop_event, maxlen=10000)
        hub = SignalHub(
            ws_url=str(getattr(self, "signal_ws_url", "") or "").strip(),
            inbox=inbox,
            stop_event=self.stop_event,
            file_service=fs,
            logger=self.logger,
            reconnect_min=float(getattr(self, "signal_ws_reconnect_min", 1.0) or 1.0),
            reconnect_max=float(getattr(self, "signal_ws_reconnect_max", 30.0) or 30.0),
            ping_interval=float(getattr(self, "signal_ws_ping_interval", 10.0) or 10.0),
            ping_timeout=float(getattr(self, "signal_ws_ping_timeout", 7.0) or 7.0),
            tls_min=float(getattr(self, "signal_ws_tls_min", 1.2) or 1.2),
            insecure=bool(getattr(self, "signal_ws_insecure", False)),
            ws_debug=bool(getattr(self, "signal_ws_debug", False)),
            log_raw=bool(file_log_raw),
        )
        hub.start()
        self.signal_hub = hub
        self._owns_signal_hub = True

    def _signal_entry_candidate_from_signal(self, sig: SignalTrade, seconds_left: float) -> Optional[dict]:
        """Build a SNIPER-like entry candidate from an external signal + current order book."""
        side = self._signal_direction_to_side(sig.direction)
        if side not in ("YES", "NO"):
            if getattr(self, "signal_debug", False):
                self.logger.info(f"[SIGNAL] drop: unknown direction={sig.direction!r} key={sig.key}")
            return None

        y_bid, y_ask, n_bid, n_ask = self._sniper_best_snapshot()
        if y_ask <= 0 or n_ask <= 0 or y_bid <= 0 or n_bid <= 0:
            return None

        y_mid = 0.5 * (y_bid + y_ask)
        n_mid = 0.5 * (n_bid + n_ask)
        parity = abs((y_mid + n_mid) - 1.0)
        if parity > float(self.sniper_parity_tolerance):
            if getattr(self, "signal_debug", False):
                self.logger.info(f"[SIGNAL] drop: parity {parity:.4f} > tol {float(self.sniper_parity_tolerance):.4f}")
            return None

        asset_id = self.yes_asset if side == "YES" else self.no_asset
        bid = y_bid if side == "YES" else n_bid
        ask = y_ask if side == "YES" else n_ask

        try:
            tick = float(self.cfg.tick) if float(self.cfg.tick) > 0 else 0.01
        except Exception:
            tick = 0.01

        entry_type_name = str(getattr(self, "sniper_entry_order_type", "FOK") or "FOK").upper().strip()
        limit_entry = entry_type_name in ("GTC", "LIMIT")

        spread_ticks = int(round((ask - bid) / tick))
        if spread_ticks > int(self.sniper_max_spread_ticks):
            if getattr(self, "signal_debug", False):
                self.logger.info(f"[SIGNAL] drop: spread_ticks {spread_ticks} > max {int(self.sniper_max_spread_ticks)}")
            return None

        # Realizable marketable BUY px (ask + slippage, tick-rounded, capped)
        entry_px = float(self._sniper_est_entry_price(float(ask)) or 0.0)
        if entry_px <= 0:
            return None
        if (not limit_entry) and (entry_px + 1e-12 < float(ask)):
            # Not marketable (usually hard cap too low)
            if getattr(self, "signal_debug", False):
                self.logger.info(
                    f"[SIGNAL] drop: entry_px {entry_px:.4f} < ask {float(ask):.4f} (hard_max={float(getattr(self,'sniper_hard_max_price',0.0) or 0.0):.4f})"
                )
            return None

        price_min = float(self.sniper_price_min)
        price_max = float(self.sniper_price_max)
        eps = float(getattr(self, "sniper_price_max_epsilon", 0.0) or 0.0)
        hard_max = float(getattr(self, "sniper_hard_max_price", price_max) or price_max)

        if (not limit_entry) and (float(ask) > (hard_max + 1e-9)):
            if getattr(self, "signal_debug", False):
                self.logger.info(f"[SIGNAL] drop: ask {float(ask):.4f} > hard_max {hard_max:.4f}")
            return None

        if entry_px < (price_min - eps) or entry_px > (price_max + eps):
            if getattr(self, "signal_debug", False):
                self.logger.info(f"[SIGNAL] drop: entry_px {entry_px:.4f} outside [{price_min:.4f},{price_max:.4f}]")
            return None

        # Drift check: don't chase if live ask moved too far from the signal's reference price
        max_drift_ticks = float(getattr(self, "signal_price_drift_max_ticks", 0.0) or 0.0)
        ref_px = float(sig.entry_price or 0.0)
        if max_drift_ticks > 0 and ref_px > 0:
            drift = abs(entry_px - ref_px)
            if drift > (max_drift_ticks * tick + 1e-9):
                if getattr(self, "signal_debug", False):
                    self.logger.info(
                        f"[SIGNAL] drop: drift {drift:.4f} > {max_drift_ticks} ticks (tick={tick:.4f}) | live={entry_px:.4f} ref={ref_px:.4f}"
                    )
                return None

        return {
            "side": side,
            "asset_id": asset_id,
            "bid": float(bid),
            "ask": float(ask),
            "entry_px": float(entry_px),
            "spread_ticks": int(spread_ticks),
            "parity": float(parity),
            "seconds_left": float(seconds_left),
        }

    def _log_status_signal(self, seconds_left: float):
        pos = self._sniper_position()
        pnl = self._sniper_mark_to_market_pnl()
        with self.state_lock:
            tc = float(self.state.get("sniper_trade_count", 0))
        if pos is None:
            print(f"[SIGNAL] t_left={seconds_left:6.1f}s trades={int(tc)} pnl(mtm)={pnl:+.4f} (flat)")
            return

        cost = float(pos["cost"])
        qty = float(pos["qty"])
        bid = float(pos["bid"])
        avg = float(pos["avg"])

        exit_px = self._sniper_est_exit_price(bid)
        pnl_est = qty * exit_px - cost
        pnl_pct = pnl_est / cost if cost > 1e-12 else 0.0

        print(
            f"[SIGNAL] t_left={seconds_left:6.1f}s trades={int(tc)} side={pos['side']} "
            f"qty={qty:.2f} avg={avg:.3f} bid={bid:.3f} ex={exit_px:.3f} pnl={pnl_est:+.4f} ({pnl_pct*100:+.2f}%)"
        )

    def _run_signal_sniper_loop(self) -> str:
        """Directional strategy loop driven by external signals (SignalHub)."""
        self._ensure_signal_hub()
        hub = self.signal_hub

        if hub is None or not hasattr(hub, "inbox"):
            self.exit_reason = "SIGNAL_NO_HUB"
            try:
                self.stop()
            except Exception:
                pass
            return self.exit_reason

        print(
            f"🚦 SIGNAL_SNIPPER enabled | provider={getattr(self, 'signal_provider', '')} "
            f"price∈[{self.sniper_price_min:.2f},{self.sniper_price_max:.2f}] hard_max={float(getattr(self,'sniper_hard_max_price',self.sniper_price_max)):.2f} "
            f"TP={self.sniper_take_profit_pct*100:.1f}% SL={self.sniper_stop_loss_pct*100:.1f}% "
            f"max_trades={int(self.sniper_max_trades_per_market)} max_notional={float(self.sniper_max_notional_usd):.2f} "
            f"conf_min={float(getattr(self,'signal_confidence_min',0.0) or 0.0):.2f} "
            f"follow_slug={bool(getattr(self,'signal_follow_slug',False))} require_match={bool(getattr(self,'signal_require_slug_match',True))} "
            f"ws_connected={bool(getattr(hub,'is_connected',lambda: False)())}"
        )

        last_log = 0.0

        while not self.stop_event.is_set():
            wait_s = max(0.01, float(self.loop_wait_seconds_sniper))
            try:
                self.wake_event.wait(timeout=wait_s)
                self.wake_event.clear()
                self.position_update_event.clear()
            except Exception:
                pass

            now = time.time()
            seconds_left = float(self.expiry_ts - now)





            if seconds_left <= 0:
                self.exit_reason = "SIGNAL_MARKET_EXPIRED"
                break

            if now - last_log >= float(self.log_every_seconds):
                self._log_status_signal(seconds_left)
                last_log = now

            if not self._market_data_fresh():
                continue

            # Do not spam during taker-failure cooloff
            if now < getattr(self, "_taker_fail_pause_until", 0.0):
                continue

            pos = self._sniper_position()

            # Track position open/close (for stop-loss persistence / min-hold)
            # Also track an "entry reference price" used by stop-limit exits.
            if pos is None:
                if getattr(self, "_sniper_in_pos", False):
                    self._sniper_in_pos = False
                    self._sniper_pos_open_ts = 0.0
                    self._sniper_stop_breach_since = None
                    self._sniper_entry_ref_price = 0.0
                    self._sniper_stop_limit_order_id = None
                    self._sniper_stop_limit_order_ts = 0.0
                    self._sniper_stop_limit_order_px = 0.0
            else:
                if not getattr(self, "_sniper_in_pos", False):
                    self._sniper_in_pos = True
                    self._sniper_pos_open_ts = float(now)
                    self._sniper_stop_breach_since = None
                    try:
                        self._sniper_entry_ref_price = float(pos.get("avg", 0.0) or 0.0)
                    except Exception:
                        self._sniper_entry_ref_price = 0.0
                    self._sniper_stop_limit_order_id = None
                    self._sniper_stop_limit_order_ts = 0.0
                    self._sniper_stop_limit_order_px = 0.0
                else:
                    if float(getattr(self, "_sniper_entry_ref_price", 0.0) or 0.0) <= 0.0:
                        try:
                            self._sniper_entry_ref_price = float(pos.get("avg", 0.0) or 0.0)
                        except Exception:
                            self._sniper_entry_ref_price = 0.0

            # -------- FLAT: wait for a matching signal --------
            if pos is None:
                with self.state_lock:
                    trade_count = int(self.state.get("sniper_trade_count", 0))
                if trade_count >= int(self.sniper_max_trades_per_market):
                    self.exit_reason = "SIGNAL_MAX_TRADES_REACHED"
                    break

                # Optional time-window gate (off by default; SIGNAL_IGNORE_TIME_WINDOW=true)
                if not bool(getattr(self, "signal_ignore_time_window", True)):
                    if seconds_left > float(self.sniper_entry_max_seconds) or seconds_left < float(self.sniper_entry_min_seconds):
                        continue

                follow_slug = bool(getattr(self, "signal_follow_slug", False))

                # If following slugs, we *peek* so we can SWITCH without consuming.
                if follow_slug:
                    sig = None
                    try:
                        sig = hub.inbox.peek(timeout=0.2)
                    except Exception:
                        sig = None
                    if sig is None:
                        continue

                    # Slug mismatch -> switch markets (do NOT consume the signal).
                    if str(sig.market_slug) != str(self.market_slug):
                        self.exit_reason = f"SWITCH:{sig.market_slug}"
                        try:
                            # best effort: cancel any stray orders (should be none when flat)
                            self.cancel_all_orders_exchange(reason="signal switch")
                        except Exception:
                            pass
                        break

                    # Slug matches: consume it now.
                    try:
                        sig = hub.inbox.get(timeout=0.0) or sig
                    except Exception:
                        pass
                else:
                    # Not following slugs: pop the first signal that matches this market_slug without dropping others.
                    sig = None
                    try:
                        sig = hub.inbox.get_for_slug(self.market_slug, timeout=0.2)
                    except Exception:
                        sig = None
                    if sig is None:
                        continue

                # Dedup: per-bot persistent state
                if self._signal_seen(sig.key):
                    if getattr(self, "signal_debug", False):
                        self.logger.info(f"[SIGNAL] skip: already seen key={sig.key}")
                    continue

                # Confidence gate
                try:
                    conf = float(sig.confidence or 0.0)
                except Exception:
                    conf = 0.0
                if conf + 1e-12 < float(getattr(self, "signal_confidence_min", 0.0) or 0.0):
                    if getattr(self, "signal_debug", False):
                        self.logger.info(f"[SIGNAL] drop: conf {conf:.3f} < min {float(getattr(self,'signal_confidence_min',0.0) or 0.0):.3f}")
                    self._signal_mark_seen(sig)
                    continue

                cand = self._signal_entry_candidate_from_signal(sig, seconds_left)
                if not cand:
                    # Mark as seen to avoid re-processing, unless explicitly disabled
                    if bool(getattr(self, "signal_use_once", True)):
                        self._signal_mark_seen(sig)
                    continue

                # Attempt entry (taker BUY)
                ok = False
                try:
                    # Set active signal context so order submits/fills can be attributed to this signal.
                    self._set_active_signal_context(sig, purpose="SIGNAL_ENTRY")
                    ok = bool(self._sniper_try_enter(cand))
                except Exception as e:
                    self.logger.error(f"[SIGNAL] entry exception: {repr(e)}")
                    ok = False
                finally:
                    # Always clear so unrelated orders won't inherit this signal.
                    self._clear_active_signal_context()

                # Mark as used (use-once semantics)
                if bool(getattr(self, "signal_use_once", True)):
                    self._signal_mark_seen(sig)

                if not ok:
                    if getattr(self, "signal_debug", False):
                        self.logger.info(f"[SIGNAL] entry failed key={sig.key} side={cand.get('side')} px={cand.get('entry_px')}")
                    continue

                # Best-effort summary latency (ms) from signal reception -> observed entry.
                try:
                    entry_ms = self._lat_ms(time.time(), float(getattr(sig, "received_ts", 0.0) or 0.0))
                except Exception:
                    entry_ms = None

                if entry_ms is not None:
                    self.logger.info(
                        f"[SIGNAL] ENTERED key={sig.key} side={cand.get('side')} ask={cand.get('ask'):.4f} px={cand.get('entry_px'):.4f} conf={conf:.3f} latency_ms={entry_ms}"
                    )
                else:
                    self.logger.info(
                        f"[SIGNAL] ENTERED key={sig.key} side={cand.get('side')} ask={cand.get('ask'):.4f} px={cand.get('entry_px'):.4f} conf={conf:.3f}"
                    )
                continue

            # -------- IN POSITION: manage exit (same logic as sniper) --------
            cost = float(pos["cost"])
            qty = float(pos["qty"])
            bid = float(pos["bid"])

            exit_px = self._sniper_est_exit_price(bid)
            pnl = qty * exit_px - cost
            pnl_pct = pnl / cost if cost > 1e-12 else 0.0

            # Force exit before expiry (optional safety)
            if bool(self.sniper_exit_before_expiry) and seconds_left <= float(self.sniper_force_exit_seconds):
                if self._sniper_try_exit(pos, reason="FORCE_EXIT"):
                    break
                continue

            # Take profit
            if cost > 1e-12 and pnl_pct >= float(self.sniper_take_profit_pct):
                if self._sniper_try_exit(pos, reason="TAKE_PROFIT"):
                    break
                continue

            # Stop loss (with persistence)
            stop_pct = float(self.sniper_stop_loss_pct)
            if cost > 1e-12 and stop_pct > 0:
                if pnl_pct <= -stop_pct:
                    held_s = float(now) - float(getattr(self, "_sniper_pos_open_ts", 0.0) or 0.0)
                    if held_s >= float(getattr(self, "sniper_min_hold_seconds", 0.0) or 0.0):
                        if getattr(self, "_sniper_stop_breach_since", None) is None:
                            self._sniper_stop_breach_since = float(now)
                        if (float(now) - float(self._sniper_stop_breach_since)) >= float(getattr(self, "sniper_stop_confirm_seconds", 0.0) or 0.0):
                            if self._sniper_try_exit(pos, reason="STOP_LOSS"):
                                break
                            continue
                else:
                    self._sniper_stop_breach_since = None

            # Stop buffer flattening
            if bool(self.sniper_exit_before_expiry) and seconds_left <= float(self.cfg.stop_buffer_seconds):
                if self._sniper_try_exit(pos, reason="STOP_BUFFER_EXIT"):
                    break

        # Cleanup
        try:
            self.stop()
        except Exception:
            pass
        return self.exit_reason


    def _run_sniper_loop(self) -> str:
        """Directional high-probability 'sniping' strategy loop."""
        print(
            f"🚀 SNIPER mode enabled | price∈[{self.sniper_price_min:.2f},{self.sniper_price_max:.2f}] "
            f"TP={self.sniper_take_profit_pct*100:.1f}% SL={self.sniper_stop_loss_pct*100:.1f}% "
            f"entry_window=[{self.sniper_entry_min_seconds}s..{self.sniper_entry_max_seconds}s] "
            f"force_exit={self.sniper_force_exit_seconds}s exit_before_expiry={self.sniper_exit_before_expiry} "
            f"force_entry_min={getattr(self, 'sniper_force_entry_min_price', 0.0):.2f} "
            f"force_entry_max_age={getattr(self, 'sniper_force_entry_max_age_seconds', 0)}s "
            f"entry_confirm={getattr(self, 'sniper_entry_confirm_seconds', 0.0):.2f}s"
        )

        # Optional: repeat trades in the same market after a TP exit.
        repeat_mode = bool(getattr(self, "sniper_repeat_mode", False))
        repeat_cooldown_s = float(getattr(self, "sniper_repeat_cooldown_seconds", 0.0) or 0.0)
        repeat_stop_after_sl = bool(getattr(self, "sniper_repeat_stop_after_stop_loss", True))
        if repeat_mode:
            print(
                f"🔁 SNIPER repeat enabled | max_trades={int(self.sniper_max_trades_per_market)} "
                f"cooldown={repeat_cooldown_s:.2f}s stop_after_stop_loss={repeat_stop_after_sl}"
            )

        last_log = 0.0

        while not self.stop_event.is_set():
            wait_s = max(0.01, float(self.loop_wait_seconds_sniper))
            try:
                self.wake_event.wait(timeout=wait_s)
                self.wake_event.clear()
                self.position_update_event.clear()

                now = time.time()
                seconds_left = float(self.expiry_ts - now)

                grace = float(getattr(self, "sniper_expiry_grace_seconds", 0.0) or 0.0)

                if seconds_left <= (-grace):
                    self.exit_reason = "SNIPER_MARKET_EXPIRED"
                    break

                # status logs (decoupled from loop frequency)
                if now - last_log >= float(self.log_every_seconds):
                    self._log_status_sniper(seconds_left)
                    last_log = now

                # ENDGAME blind post: last-second resting LIMIT/GTC bid even if the feed is stale/disconnected.
                # Opt-in via SNIPER_ENDGAME_BLIND_POST=true.
                try:
                    if self._sniper_maybe_endgame_blind_post(seconds_left=float(seconds_left), now_ts=float(now)):
                        # Once we have a resting endgame order, we can wait for fills.
                        # (If a fill arrives, normal position management will take over on the next loop.)
                        continue
                except Exception as _e:
                    try:
                        self.logger.error(f"⚠️ endgame blind-post error: {_e}")
                    except Exception:
                        pass

                # Need fresh data
                if not self._market_data_fresh():
                    continue

                # Do not spam during taker-failure cooloff
                if now < getattr(self, "_taker_fail_pause_until", 0.0):
                    continue

                # Current position?
                pos = self._sniper_position()

                # Track position open/close (for stop-loss persistence / min-hold)
                # Also track an "entry reference price" used by stop-limit exits.
                if pos is None:
                    if getattr(self, "_sniper_in_pos", False):
                        self._sniper_in_pos = False
                        self._sniper_pos_open_ts = 0.0
                        self._sniper_stop_breach_since = None
                        self._sniper_entry_ref_price = 0.0
                        self._sniper_stop_limit_order_id = None
                        self._sniper_stop_limit_order_ts = 0.0
                        self._sniper_stop_limit_order_px = 0.0
                        # Reset entry confirmation state after closing a position.
                        self._sniper_entry_gate_since = None
                        self._sniper_entry_gate_side = None
                else:
                    if not getattr(self, "_sniper_in_pos", False):
                        self._sniper_in_pos = True
                        self._sniper_pos_open_ts = float(now)
                        self._sniper_stop_breach_since = None
                        try:
                            self._sniper_entry_ref_price = float(pos.get("avg", 0.0) or 0.0)
                        except Exception:
                            self._sniper_entry_ref_price = 0.0
                        self._sniper_stop_limit_order_id = None
                        self._sniper_stop_limit_order_ts = 0.0
                        self._sniper_stop_limit_order_px = 0.0
                        # Reset entry confirmation state once we are in position.
                        self._sniper_entry_gate_since = None
                        self._sniper_entry_gate_side = None
                    else:
                        # If we restarted the bot mid-position, backfill the reference price once.
                        if float(getattr(self, "_sniper_entry_ref_price", 0.0) or 0.0) <= 0.0:
                            try:
                                self._sniper_entry_ref_price = float(pos.get("avg", 0.0) or 0.0)
                            except Exception:
                                self._sniper_entry_ref_price = 0.0

                # -------- FLAT: consider entry --------
                if pos is None:
                    with self.state_lock:
                        trade_count = int(self.state.get("sniper_trade_count", 0))

                    if trade_count >= int(self.sniper_max_trades_per_market):
                        self.exit_reason = "SNIPER_MAX_TRADES_REACHED"
                        break

                    # Repeat-mode stop condition: once we're flat and the safe entry window is closed,
                    # stop this market (no new trades).
                    if repeat_mode:
                        # Hard stop buffer: no new risk too close to expiry.
                        if float(seconds_left) <= float(self.cfg.stop_buffer_seconds):
                            self.exit_reason = "SNIPER_STOP_BUFFER"
                            break
                        # If we force-exit before expiry, don't keep scanning once we're inside that window.
                        if bool(self.sniper_exit_before_expiry) and float(seconds_left) <= float(self.sniper_force_exit_seconds) + 1.0:
                            self.exit_reason = "SNIPER_FORCE_EXIT_WINDOW"
                            break
                        # Entry window closed.
                        if float(seconds_left) < float(self.sniper_entry_min_seconds):
                            self.exit_reason = "SNIPER_ENTRY_WINDOW_CLOSED"
                            break

                        # Cooldown after a completed exit (anti-churn / machine-gun prevention)
                        if repeat_cooldown_s > 0.0:
                            with self.state_lock:
                                last_exit_ts = float(self.state.get("sniper_last_exit_ts", 0.0) or 0.0)
                            if last_exit_ts > 0.0 and (float(now) - last_exit_ts) < repeat_cooldown_s:
                                continue

                    # Only enter late in the market window (reduces sudden reversal risk).
                    # Optional override: if the favored side is already very high early, allow a "force entry".
                    if seconds_left > float(self.sniper_entry_max_seconds):
                        force_min = float(getattr(self, "sniper_force_entry_min_price", 0.0) or 0.0)
                        if force_min > 0.0:
                            # Restrict to early market age if configured (0 = no limit)
                            age_s = float(now) - float(getattr(self, "start_ts", now))
                            max_age = float(getattr(self, "sniper_force_entry_max_age_seconds", 0) or 0)
                            if (max_age <= 0.0) or (age_s <= max_age):
                                cand = self._sniper_entry_candidate(
                                    seconds_left,
                                    ignore_roi_gate=bool(getattr(self, "sniper_force_entry_ignore_roi_gate", False)),
                                )
                                ask_ok = bool(cand) and float(cand.get("ask", 0.0) or 0.0) + 1e-12 >= float(force_min)
                                if not ask_ok:
                                    # Reset confirmation timer if the force-entry condition isn't continuously true.
                                    if float(getattr(self, "sniper_entry_confirm_seconds", 0.0) or 0.0) > 0.0:
                                        self._sniper_entry_gate_since = None
                                        self._sniper_entry_gate_side = None
                                else:
                                    # Debounce entry: require the force-entry signal to persist.
                                    if not self._sniper_entry_confirmed(cand, float(now)):
                                        continue
                                    self.logger.info(
                                        f"⚡ [SNIPER] FORCE-ENTRY triggered "
                                        f"side={cand['side']} ask={cand['ask']:.3f} entry_px={cand.get('entry_px', 0.0):.3f}>=min={force_min:.3f} "
                                        f"age={age_s:.1f}s t_left={seconds_left:.1f}s "
                                        f"spread_ticks={cand.get('spread_ticks')} parity={cand.get('parity'):.4f}"
                                    )
                                    self._sniper_try_enter(cand)
                        continue

                    # Too close to expiry to open fresh risk.
                    if seconds_left < float(self.sniper_entry_min_seconds):
                        if float(seconds_left) <= float(self.cfg.stop_buffer_seconds):
                            self.exit_reason = "SNIPER_TOO_LATE_TO_ENTER"
                            break
                        continue

                    # If we intend to force-exit before expiry, don't enter after that point.
                    if bool(self.sniper_exit_before_expiry) and seconds_left <= float(self.sniper_force_exit_seconds) + 1.0:
                        continue

                    cand = self._sniper_entry_candidate(
                        seconds_left,
                        ignore_roi_gate=bool(getattr(self, "sniper_entry_ignore_roi_gate", False)),
                    )
                    if not cand:
                        # Entry condition broke; reset confirmation timer.
                        self._sniper_entry_gate_since = None
                        self._sniper_entry_gate_side = None
                    else:
                        if not self._sniper_entry_confirmed(cand, float(now)):
                            continue
                        self._sniper_try_enter(cand)
                    continue

                # -------- IN POSITION: manage exit --------
                cost = float(pos["cost"])
                qty = float(pos["qty"])
                bid = float(pos["bid"])

                # IMPORTANT: compute TP/SL triggers on a *realizable* exit price, not raw bid.
                exit_px = self._sniper_est_exit_price(bid)
                pnl = qty * exit_px - cost
                pnl_pct = pnl / cost if cost > 1e-12 else 0.0

                # Force exit before expiry (optional safety)
                if bool(self.sniper_exit_before_expiry) and seconds_left <= float(self.sniper_force_exit_seconds):
                    if self._sniper_try_exit(pos, reason="FORCE_EXIT"):
                        break
                    continue

                # Take profit
                if cost > 1e-12 and pnl_pct >= float(self.sniper_take_profit_pct):
                    if self._sniper_try_exit(pos, reason="TAKE_PROFIT"):
                        # In repeat-mode we continue scanning (until max trades / entry window closes).
                        if repeat_mode:
                            continue
                        break
                    continue

                # Stop loss (with persistence to avoid thin-book fakeouts)
                stop_pct = float(self.sniper_stop_loss_pct)
                if cost > 1e-12 and stop_pct > 0:
                    if pnl_pct <= -stop_pct:
                        held_s = float(now) - float(getattr(self, "_sniper_pos_open_ts", 0.0) or 0.0)
                        if held_s >= float(getattr(self, "sniper_min_hold_seconds", 0.0) or 0.0):
                            if getattr(self, "_sniper_stop_breach_since", None) is None:
                                self._sniper_stop_breach_since = float(now)
                            if (float(now) - float(self._sniper_stop_breach_since)) >= float(getattr(self, "sniper_stop_confirm_seconds", 0.0) or 0.0):
                                if self._sniper_try_exit(pos, reason="STOP_LOSS"):
                                    # Repeat-mode risk control: usually stop after a stop-loss exit.
                                    if repeat_mode and not repeat_stop_after_sl:
                                        continue
                                    break
                                continue
                    else:
                        self._sniper_stop_breach_since = None

                # If we are inside rollover stop buffer, and configured to exit before expiry, attempt flatten.
                if bool(self.sniper_exit_before_expiry) and seconds_left <= float(self.cfg.stop_buffer_seconds):
                    if self._sniper_try_exit(pos, reason="STOP_BUFFER_EXIT"):
                        break

            except Exception as e:
                self.logger.error(f"SNIPER loop error: {repr(e)}")
                self.exit_reason = "SNIPER_EXCEPTION"
                break

        # Cleanup
        try:
            self.stop()
        except Exception:
            pass
        return self.exit_reason


# ---------------- Main loop ----------------

    def run(self) -> str:
        t1 = threading.Thread(target=self._ws_runner, args=("market", self.on_market_message), daemon=True)
        t2 = threading.Thread(target=self._ws_runner, args=("user", self.on_user_message), daemon=True)
        t1.start()
        t2.start()

        if getattr(self, "signal_sniper_mode", False):
            return self._run_signal_sniper_loop()

        if getattr(self, "sniper_mode", False):
            return self._run_sniper_loop()

        while not self.stop_event.is_set():
            # Event-driven: wake on best_bid_ask updates (max wait bounded by LOOP_WAIT_SECONDS_*)
            wait_s = float(self.loop_wait_seconds_taker) if self.exec_mode == "TAKER_PAIR" else float(self.loop_wait_seconds_maker)
            wait_s = max(0.01, wait_s)
            try:
                # Wake on either market updates *or* position/fill updates.
                self.wake_event.wait(timeout=wait_s)
                self.wake_event.clear()
                # clear legacy events too (avoid backlog when switching modes)
                self.market_update_event.clear()
                self.position_update_event.clear()
            except Exception:
                time.sleep(min(wait_s, 0.5))

            self._ticks += 1
            now_ts = time.time()

            # Status log (seconds-based; avoids log spam when loop frequency increases)
            try:
                every = max(0.5, float(self.log_every_seconds))
            except Exception:
                every = 5.0
            if (now_ts - float(getattr(self, "_last_status_log_ts", 0.0))) >= every:
                self._last_status_log_ts = now_ts
                self._log_status()

            with self.state_lock:
                total_cost = float(self.state["c_yes"]) + float(self.state["c_no"])

            if total_cost >= self.cfg.max_total_cost:
                self.logger.info(
                    f"🛑 HARD SPEND CAP HIT total_cost={total_cost:.2f} >= {self.cfg.max_total_cost:.2f} -> CANCEL + STOP")
                self.exit_reason = "HARD_SPEND_CAP"
                self.cancel_all_orders_exchange(reason="hard spend cap")
                break

            # expiry stop / rollover
            seconds_left = self.expiry_ts - time.time()
            seconds_left -= 10.0  # small buffer to account for execution latency (esp. if we are late to wake up after expiry)
            if seconds_left < self.cfg.stop_buffer_seconds:
                # Before we stop, try to flatten if imbalanced (hedge preference)
                with self.state_lock:
                    qy = float(self.state["q_yes"])
                    qn = float(self.state["q_no"])
                delta = qy - qn
                if abs(delta) >= self.cfg.min_shares:
                    self.logger.info(f"⏳ Near expiry ({seconds_left:.0f}s). Forcing emergency hedge before stopping.")
                    self._emergency_taker_hedge_step(delta, reason="near_expiry")
                    time.sleep(1)  # allow WS fills
                self.logger.info(f"⏳ Expiring in {seconds_left:.0f}s -> stopping for rollover.")
                self.cancel_all_orders_exchange(reason="expiry")
                break

            # feed safety
            if not self._market_data_fresh():
                if not self._in_feed_pause:
                    self.logger.info("⚠️ FEED STALE/DOWN -> cancel all + pause.")
                    self.cancel_all_orders_exchange(reason="feed stale")
                    self._in_feed_pause = True
                continue
            else:
                if self._in_feed_pause:
                    self.logger.info("✅ FEED OK -> resume.")
                    self._in_feed_pause = False

            # Always compute delta early (so hedging is never blocked by reserve/spend cap)
            with self.state_lock:
                total_cost = float(self.state["c_yes"]) + float(self.state["c_no"])
                qy = float(self.state["q_yes"])
                qn = float(self.state["q_no"])
            delta = qy - qn

            # Maintain unhedged timer
            if abs(delta) >= self.cfg.min_shares:
                if self._unhedged_since is None:
                    self._unhedged_since = time.time()
            else:
                self._unhedged_since = None

            # ============================
            # Profit lock mode (only when flat-ish)
            # ============================
            lp = locked_profit(self.state)
            if abs(delta) < 0.25 and lp >= self.cfg.lock_profit_target:
                self.logger.info(f"✅ Target hit. Canceling all orders first. lp={lp:.4f}")
                self.cancel_all_orders_exchange(reason="locked profit target")
                self.exit_reason = "TARGET_HIT"

                time.sleep(2)  # let any in-flight fills arrive via WS

                with self.state_lock:
                    qy2 = float(self.state["q_yes"])
                    qn2 = float(self.state["q_no"])
                lp2 = locked_profit(self.state)

                if abs(qy2 - qn2) < 0.25 and lp2 >= self.cfg.lock_profit_target:
                    self.logger.info(f"✅ Still flat after cancel. Stopping. lp={lp2:.4f}")
                    break
                else:
                    self.logger.info(
                        f"⚠️ Fill occurred during cancel. Continuing hedge. "
                        f"qy={qy2:.2f} qn={qn2:.2f} lp={lp2:.4f}"
                    )
                    # update local delta
                    delta = qy2 - qn2


            # ==============================
            # Deterministic mini state-machine (FSM)
            # ==============================
            if getattr(self, "fsm_enabled", False):
                # ---- transitions ----
                if self.fsm_state == "BALANCED":
                    if abs(delta) >= self.cfg.min_shares:
                        self._fsm_set_state("EXPOSED", reason=f"delta={delta:.2f}")
                elif self.fsm_state == "EXPOSED":
                    if abs(delta) < self.cfg.min_shares:
                        self._fsm_set_state("COOLDOWN", reason=f"delta={delta:.2f}")
                elif self.fsm_state == "COOLDOWN":
                    if abs(delta) >= self.cfg.min_shares:
                        self._fsm_set_state("EXPOSED", reason=f"delta={delta:.2f}")
                    elif time.time() >= float(getattr(self, "_fsm_cooldown_until", 0.0) or 0.0):
                        self._fsm_set_state("BALANCED", reason="cooldown done")

                # ---- state actions ----
                if self.fsm_state == "EXPOSED":
                    # Never keep buying the heavy side while exposed (prevents runaway imbalance)
                    if bool(getattr(self, "fsm_dont_add_to_heavy", True)):
                        self._cancel_heavy_side_orders()

                    # How long we've been unhedged (seconds)
                    if self._unhedged_since is not None:
                        unhedged_age = time.time() - float(self._unhedged_since)
                    else:
                        unhedged_age = 0.0

                    if self._maybe_trigger_max_loss(delta, unhedged_age):
                        continue

                    # Hard guard: if exposure is too large, unwind immediately (configurable)
                    max_expo = float(getattr(self, "fsm_max_exposure_shares", 0.0) or 0.0)
                    if max_expo > 0 and abs(float(delta)) > max_expo:
                        self.logger.info(
                            f"🧯 FSM max exposure hit abs(delta)={abs(float(delta)):.2f} > {max_expo:.2f} -> UNWIND heavy"
                        )
                        self.cancel_all_open_orders_local(reason="fsm max exposure -> unwind")
                        self._cancel_exchange_orders_for_assets([self.yes_asset, self.no_asset], reason="fsm max exposure -> unwind")
                        self._unwind_heavy_leg(delta, reason="fsm_max_exposure")
                        continue

                    # Exposure resolution depends on mode:
                    if self.exec_mode == "TAKER_PAIR":
                        # Pair-arb: prioritize fast taker hedge (still cap-clamped inside)
                        self._emergency_taker_hedge_step(delta, reason="fsm_exposed_taker_pair")
                    else:
                        # Maker mode: apply maker exposure policy (HEDGE / UNWIND / HEDGE_THEN_UNWIND)
                        self._maker_exposure_step(delta, unhedged_age)

                    continue  # never quote both sides while EXPOSED

                if self.fsm_state == "COOLDOWN":
                    # No new quotes during cooldown.
                    continue

            # ==============================
            # MODE 1: HEDGE (imbalanced) [legacy, FSM disabled]
            # ==============================
            if abs(delta) >= self.cfg.min_shares:
                # Always enforce heavy-side cancellation when imbalanced
                self._cancel_heavy_side_orders()

                # How long we've been unhedged (seconds)
                if self._unhedged_since is not None:
                    unhedged_age = time.time() - float(self._unhedged_since)
                else:
                    unhedged_age = 0.0

                if self._maybe_trigger_max_loss(delta, unhedged_age):
                    continue

                # Apply maker exposure policy (HEDGE / UNWIND / HEDGE_THEN_UNWIND)
                self._maker_exposure_step(delta, unhedged_age)
                continue  # never quote both sides while imbalanced

            # ==============================
            # MODE 2: BALANCED (accumulate / pair-arb)
            # ==============================

            # Spend cap: only blocks NEW accumulation (we still hedge above).
            remaining = self.cfg.max_total_cost - total_cost
            if remaining <= 0:
                self.logger.info("🛑 spend cap hit (balanced) -> stop")
                self.exit_reason = "SPEND_CAP"
                self.cancel_all_orders_exchange(reason="spend cap")
                break

            # Reserve cap: just means “don’t start new cycles”
            if remaining <= self.cfg.reserve_usd:
                # Also cancel any resting quotes if we are no longer accumulating
                self.cancel_all_open_orders_local(reason="reserve reached (balanced)")
                continue

            # ------------------------------
            # EXEC_MODE: TAKER_PAIR
            # ------------------------------
            if self.exec_mode == "TAKER_PAIR":
                # In TAKER_PAIR mode we do NOT leave maker quotes resting.
                # We only buy complete sets when the asks already sum to < 1 (minus buffers).
                budget = max(0.0, float(remaining) - float(self.cfg.reserve_usd))
                self._taker_pair_arb_step(remaining_budget=budget)
                continue

            # ------------------------------
            # EXEC_MODE: MAKER (default)
            # ------------------------------

            # Stability gate (warmup + spread + parity), but only for accumulate
            ok, why = self._accumulate_allowed()
            if not ok:
                if getattr(self, "maker_debug", False):
                    yq = self._best_bid_ask(self.yes_asset)
                    nq = self._best_bid_ask(self.no_asset)
                    if yq and nq:
                        yb, ya = yq
                        nb, na = nq
                        self._dbg_maker(
                            f"[DBG][MAKER] skip accumulate gate: {why} | "
                            f"BBO YES {yb:.2f}/{ya:.2f} NO {nb:.2f}/{na:.2f}",
                            key=f"maker_gate_{why}",
                            throttle_s=0.8,
                        )
                    else:
                        self._dbg_maker(f"[DBG][MAKER] skip accumulate gate: {why}", key=f"maker_gate_{why}", throttle_s=0.8)

                # Cancel resting quotes during warmup/unstable periods so we don't leave stale orders behind
                self.cancel_all_open_orders_local(reason=f"accumulate gate: {why}")
                continue

            # Quote invalidation (remove the taker's free option):
            # If resting quotes have become unhedgeable (given opposite ASK), cancel them before they get picked off.
            now_ts2 = time.time()
            if now_ts2 < float(self._quote_pause_until):
                continue

            invalid, inv_reason = self._quotes_invalidated()
            if invalid:
                if getattr(self, "maker_debug", False):
                    self._dbg_maker(
                        f"[DBG][MAKER] quote invalidated: {inv_reason}",
                        key="maker_quote_invalidated",
                        throttle_s=0.5,
                    )
                self.cancel_all_open_orders_local(reason=f"quote invalidated: {inv_reason}")
                self._cancel_exchange_orders_for_assets([self.yes_asset, self.no_asset], reason="quote invalidated")
                self._quote_pause_until = time.time() + float(self.quote_invalidation_pause_seconds)
                continue

            # Entry condition + cross-ask-safe quoting:
            effective_edge_ticks = max(int(self.cfg.entry_edge_ticks), int(self.min_entry_edge_ticks))
            entry_edge = effective_edge_ticks * self.cfg.tick

            y_bid = self._maker_bid_cross_ask_safe(self.yes_asset, self.no_asset, edge=entry_edge)
            n_bid = self._maker_bid_cross_ask_safe(self.no_asset, self.yes_asset, edge=entry_edge)
            if y_bid is None or n_bid is None:
                if getattr(self, "maker_debug", False):
                    yq = self._best_bid_ask(self.yes_asset)
                    nq = self._best_bid_ask(self.no_asset)
                    if yq and nq:
                        yb, ya = yq
                        nb, na = nq
                        self._dbg_maker(
                            f"[DBG][MAKER] no safe bids (edge={entry_edge:.2f}) | "
                            f"YES {yb:.2f}/{ya:.2f} NO {nb:.2f}/{na:.2f}",
                            key="maker_no_safe_bids",
                            throttle_s=0.7,
                        )
                    else:
                        self._dbg_maker(f"[DBG][MAKER] no safe bids (edge={entry_edge:.2f})", key="maker_no_safe_bids", throttle_s=0.7)
                self.cancel_all_open_orders_local(reason="no safe bids")
                continue

            # Paired-entry gate (no-loss hedgeability):
            yq = self._best_bid_ask(self.yes_asset)
            nq = self._best_bid_ask(self.no_asset)
            if not yq or not nq:
                self.cancel_all_open_orders_local(reason="missing quotes for paired gate")
                continue
            _, y_ask = yq
            _, n_ask = nq

            # Use tick-integer comparisons to avoid float edge-cases like "0.32 > 0.32"
            # due to binary rounding. This preserves safety while preventing false rejections.
            try:
                tick = float(self.cfg.tick) if float(self.cfg.tick) > 0 else 0.01
            except Exception:
                tick = 0.01

            buf = float(self.paired_entry_buffer_ticks) * tick

            def _tix(p: float) -> int:
                try:
                    return int(round(float(p) / float(tick) + 1e-9))
                except Exception:
                    return int(round(float(p) / 0.01 + 1e-9))

            thr_no_ticks = _tix(1.0 - float(y_bid) - float(buf))
            thr_yes_ticks = _tix(1.0 - float(n_bid) - float(buf))
            n_ask_ticks = _tix(float(n_ask))
            y_ask_ticks = _tix(float(y_ask))

            thr_no = float(thr_no_ticks) * float(tick)
            thr_yes = float(thr_yes_ticks) * float(tick)

            # If we bid YES at y_bid, ensure NO ask is hedgeable.
            if n_ask_ticks > thr_no_ticks:

                if getattr(self, "maker_debug", False):
                    self._dbg_maker(
                        f"[DBG][MAKER] paired gate fail: NO ask {n_ask:.2f} > {thr_no:.2f} "
                        f"(y_bid={y_bid:.2f} buf={buf:.2f})",
                        key="maker_paired_gate_no",
                        throttle_s=0.7,
                    )
                self.cancel_all_open_orders_local(reason=f"paired gate fail (NO ask {n_ask:.2f} > {thr_no:.2f})")
                continue

            # If we bid NO at n_bid, ensure YES ask is hedgeable.
            if y_ask_ticks > thr_yes_ticks:
                if getattr(self, "maker_debug", False):
                    self._dbg_maker(
                        f"[DBG][MAKER] paired gate fail: YES ask {y_ask:.2f} > {thr_yes:.2f} "
                        f"(n_bid={n_bid:.2f} buf={buf:.2f})",
                        key="maker_paired_gate_yes",
                        throttle_s=0.7,
                    )
                self.cancel_all_open_orders_local(reason=f"paired gate fail (YES ask {y_ask:.2f} > {thr_yes:.2f})")
                continue

            # Still require combined entry edge for profitability:
            if (y_bid + n_bid) > (1.0 - entry_edge):
                if getattr(self, "maker_debug", False):
                    self._dbg_maker(
                        f"[DBG][MAKER] entry edge fail sum={y_bid + n_bid:.2f} > req={1.0 - entry_edge:.2f} "
                        f"(entry_edge={entry_edge:.2f})",
                        key="maker_entry_edge_fail",
                        throttle_s=0.7,
                    )
                # If we no longer have edge, cancel existing quotes to avoid adverse selection.
                self.cancel_all_open_orders_local(reason="entry edge fail")
                continue

            # Place both sides at clip size.
            size = self.cfg.clip_shares
            try:
                if (not self._first_cycle_done) and (not self._first_cycle_started):
                    if float(self.first_clip_shares) >= float(self.cfg.min_shares):
                        size = float(self.first_clip_shares)
            except Exception:
                pass
            if size < self.cfg.min_shares:
                size = self.cfg.min_shares

            # Depth gate: ensure opposite-side ASK liquidity exists to hedge one-leg fills (reduces bad exposure on thin books)
            if getattr(self, "depth_gate_enabled", False):
                okd, whyd = self._depth_gate_accumulate(size=float(size), y_bid=float(y_bid), n_bid=float(n_bid), buf=float(buf))
                if not okd:
                    if getattr(self, "maker_debug", False):
                        self._dbg_maker(f"[DBG][MAKER] depth gate fail: {whyd}", key="maker_depth_gate", throttle_s=0.7)
                    if not getattr(self, "depth_gate_warn_only", False):
                        self.cancel_all_open_orders_local(reason=f"depth gate: {whyd}")
                        continue

            # Budget cap per tick: approximate cost if both fill at our bid
            est = size * (y_bid + n_bid)
            avail = float(remaining) - float(self.cfg.reserve_usd)
            if est > avail:
                if getattr(self, "maker_debug", False):
                    self._dbg_maker(
                        f"[DBG][MAKER] skip budget est={est:.2f} > avail={avail:.2f} "
                        f"(size={size:.0f} sum={y_bid + n_bid:.2f} remaining={remaining:.2f} reserve={self.cfg.reserve_usd:.2f})",
                        key="maker_budget_skip",
                        throttle_s=0.7,
                    )
                continue

            if getattr(self, "maker_debug", False):
                self._dbg_maker(
                    f"[DBG][MAKER] place/keep quotes y_bid={y_bid:.2f} n_bid={n_bid:.2f} "
                    f"sum={y_bid + n_bid:.2f} req<={1.0 - entry_edge:.2f} size={size:.0f}",
                    key="maker_quote_place",
                    throttle_s=0.7,
                )

            self._maybe_replace(self.yes_asset, y_bid, size)
            self._maybe_replace(self.no_asset, n_bid, size)

        self.stop()
        return self.exit_reason

    def stop(self):
        self.stop_event.set()
        # Only stop a SignalHub if this bot instance created it (otherwise it is global / shared).
        try:
            if bool(getattr(self, "_owns_signal_hub", False)) and getattr(self, "signal_hub", None) is not None:
                self.signal_hub.close()
        except Exception:
            pass
        try:
            if self.market_ws:
                self.market_ws.close()
        except Exception:
            pass
        try:
            if self.user_ws:
                self.user_ws.close()
        except Exception:
            pass


def print_pnl_metrics(s, bot_id: str, logger: Optional[Any] = None):
    repo = BotRepository(s)

    today = date_jakarta()
    week_start = week_start_date_jakarta()
    month_start = month_start_date_jakarta()

    b_today = repo.pnl_and_trade_count_for_bot(bot_id, today, today)
    b_week = repo.pnl_and_trade_count_for_bot(bot_id, week_start, today)
    b_month = repo.pnl_and_trade_count_for_bot(bot_id, month_start, today)

    a_today = repo.pnl_and_trade_count_all_bots(today, today)
    a_week = repo.pnl_and_trade_count_all_bots(week_start, today)
    a_month = repo.pnl_and_trade_count_all_bots(month_start, today)

    msg = (
        f"📊 PNL Summary (Asia/Jakarta)\n"
        f"  Bot {bot_id} Today  : PNL={b_today[0]:+.4f} | Trades={b_today[1]}\n"
        f"  Bot {bot_id} Weekly : PNL={b_week[0]:+.4f} | Trades={b_week[1]}\n"
        f"  Bot {bot_id} Monthly: PNL={b_month[0]:+.4f} | Trades={b_month[1]}\n"
        f"  ALL bots Today      : PNL={a_today[0]:+.4f} | Trades={a_today[1]}\n"
        f"  ALL bots Weekly     : PNL={a_week[0]:+.4f} | Trades={a_week[1]}\n"
        f"  ALL bots Monthly    : PNL={a_month[0]:+.4f} | Trades={a_month[1]}"
    )
    if logger:
        logger.info(msg)
    else:
        print(msg)


# ============================================================
# main
# ============================================================
def main():
    cfg = BotConfig(
        clob_host=os.getenv("CLOB_HOST", "https://clob.polymarket.com"),
        ws_base=os.getenv("WS_BASE", "wss://ws-subscriptions-clob.polymarket.com"),
        chain_id=int(os.getenv("CHAIN_ID", "137")),
        private_key=os.getenv("POLYMARKET_PRIVATE_KEY", "").strip(),
        dry_run=os.getenv("DRY_RUN", "false").lower() == "true",
    )

    # ---------------- Market segment selection ----------------
    seg = _segment(os.getenv("MARKET_SEGMENT", "15M"))
    d = SEGMENT_DEFAULTS.get(seg, SEGMENT_DEFAULTS["15M"])
    cfg.market_segment = seg
    cfg.market_duration_seconds = int(os.getenv("MARKET_DURATION_SECONDS", str(d["duration"])))
    cfg.market_step_seconds = int(os.getenv("MARKET_STEP_SECONDS", str(d["step"])))
    os.environ["MARKET_DURATION_SECONDS"] = str(cfg.market_duration_seconds)
    os.environ["MARKET_STEP_SECONDS"] = str(cfg.market_step_seconds)
    cfg.stop_buffer_seconds = int(os.getenv("STOP_BUFFER_SECONDS", str(d["stop_buffer"])))

    if not cfg.private_key:
        raise SystemExit("Missing POLYMARKET_PRIVATE_KEY")

    db_url = os.getenv("DB_URL", "sqlite:///./bot.sqlite3")
    engine = make_engine(db_url)
    SessionLocal = make_session_factory(engine)
    
    for _i in range(5):
        try:
            BotRepository.init_schema(engine)
            break
        except OperationalError as e:
            logger.error(f"⚠️ DB Init Error: {e}. Retrying...")
            time.sleep(2)

    bot_id = os.getenv("BOT_ID", "maker_hedgecap_bot")
    bot_description = os.getenv("BOT_DESCRIPTION", "Maker+HedgeCap Polymarket bot")
    account_name = os.getenv("ACCOUNT_NAME", "default")

    sig = os.getenv("SIGNATURE_TYPE", "1").strip()
    funder = os.getenv("POLYMARKET_FUNDER", "").strip()

    if sig and funder:
        cfg.signature_type = int(sig)
        cfg.funder = funder

    if not cfg.funder:
        raise SystemExit("Missing POLYMARKET_FUNDER")

    # ---------------- Signal hub (optional, for SIGNAL_SNIPPER) ----------------
    exec_mode = os.getenv("EXEC_MODE", "MAKER").upper().strip()
    signal_mode = exec_mode in ("SIGNAL_SNIPPER", "SIGNAL_SNIPER", "SIGNAL_SNIPE", "SIGNAL")
    signal_hub: Optional[SignalHub] = None
    signal_stop_event: Optional[threading.Event] = None

    if signal_mode:
        provider = os.getenv("SIGNAL_PROVIDER", "WEBSOCKET").upper().strip()
        if provider == "WEBSOCKET":
            signal_stop_event = threading.Event()
            inbox = SignalInbox(stop_event=signal_stop_event, maxlen=10000)

            signal_file_dir = os.getenv("SIGNAL_FILE_DIR", "./signals").strip() or "./signals"
            os.makedirs(signal_file_dir, exist_ok=True)
            signal_file_path = os.getenv("SIGNAL_FILE_PATH", "").strip()
            if not signal_file_path:
                # global log (all signals). Individual bot instances can also log per-market if they create their own hub.
                signal_file_path = os.path.join(signal_file_dir, "signal_ws_global.jsonl")

            file_log_raw = env_bool("SIGNAL_FILE_LOG_RAW", False)
            fs = JsonlFileService(signal_file_path, enabled=True)

            ws_url = os.getenv("SIGNAL_WS_URL", "").strip()
            if not ws_url:
                raise SystemExit("Missing SIGNAL_WS_URL for SIGNAL_PROVIDER=WEBSOCKET")

            hub_logger = setup_item_logger("signal_hub")
            signal_hub = SignalHub(
                ws_url=ws_url,
                inbox=inbox,
                stop_event=signal_stop_event,
                file_service=fs,
                logger=hub_logger,
                reconnect_min=env_float("SIGNAL_WS_RECONNECT_MIN", 1.0),
                reconnect_max=env_float("SIGNAL_WS_RECONNECT_MAX", 30.0),
                ping_interval=env_float("SIGNAL_WS_PING_INTERVAL", 10.0),
                ping_timeout=env_float("SIGNAL_WS_PING_TIMEOUT", 7.0),
                tls_min=env_float("SIGNAL_WS_TLS_MIN", 1.2),
                insecure=env_bool("SIGNAL_WS_INSECURE", False),
                ws_debug=env_bool("SIGNAL_WS_DEBUG", False),
                log_raw=file_log_raw,
            )
            signal_hub.start()
            hub_logger.info(f"[SIGNAL_HUB] started provider=WEBSOCKET url={os.getenv('SIGNAL_WS_URL','').strip()} file={signal_file_path}")

    slug = os.getenv("MARKET_SLUG", "").strip()
    if not slug:
        # Allow starting without MARKET_SLUG when using SIGNAL_FOLLOW_SLUG.
        if signal_mode and env_bool("SIGNAL_FOLLOW_SLUG", False) and signal_hub is not None:
            wait_logger = setup_item_logger("signal_wait")
            wait_logger.info("MARKET_SLUG is empty; waiting for first signal (SIGNAL_FOLLOW_SLUG=true)...")
            first = signal_hub.inbox.peek(timeout=None)
            if first is None:
                raise SystemExit("Missing MARKET_SLUG and no signal received from SIGNAL_WS_URL")
            slug = str(first.market_slug)
            wait_logger.info(f"Using initial market_slug from signal: {slug}")
        else:
            raise SystemExit("Missing MARKET_SLUG")

    # --- Safe defaults for you ---
    # This configuration already working fine,
    # making 0.05 - 0.30 profit every trade.
    cfg.min_shares = 5.0
    cfg.clip_shares = 5.0

    cfg.entry_edge_ticks = 6          # require bids sum <= 0.94 (stricter, safer) (plus cross-ask safety)
    cfg.hedge_buffer_ticks = 2        # cap minus 2 ticks safety (avoid trap)
    cfg.maker_buffer_ticks = 1        # stay maker

    cfg.improve_bid_ticks = 0

    cfg.stale_seconds = 5
    cfg.replace_if_price_moves_ticks = 3

    cfg.max_total_cost = float(os.getenv("MAX_TOTAL_COST", "15.0"))
    cfg.reserve_usd = float(os.getenv("RESERVE_USD", "2.0"))
    cfg.market_data_stale_seconds = 8
    cfg.cancel_all_on_start = True
    cfg.log_every = 5

    # NOTE: New safety knobs are env-based (no DB schema changes):
    #   WARMUP_SECONDS=15
    #   MAX_SPREAD_TICKS=10
    #   PARITY_TOLERANCE=0.03
    #   UNHEDGED_TIMEOUT_SECONDS=5
    #   HEDGE_SLIPPAGE_TICKS=3
    #   HEDGE_TAKER_ORDER_TYPE=FAK
    #   TAKER_ORDER_TTL_SECONDS=120
    #   TAKER_HEDGE_MIN_INTERVAL=1.0

    current_slug = slug

    while True:
        bot_logger = setup_item_logger(current_slug)
        bot_logger.info(f"\n🚀 STARTING MARKET: {current_slug}")

        bot = None  # ensure defined for signal handler + finally

        def handle_sig(*_):
            bot_logger.warning("\nSignal received. Stopping...")
            try:
                if bot is not None:
                    bot.cancel_all_orders_exchange(reason="signal stop")
                    bot.stop()
            except Exception:
                pass
            # Stop global signal hub (if any)
            try:
                if signal_stop_event is not None:
                    signal_stop_event.set()
                if signal_hub is not None:
                    signal_hub.close()
            except Exception:
                pass
            raise SystemExit(0)

        signal.signal(signal.SIGINT, handle_sig)
        signal.signal(signal.SIGTERM, handle_sig)

        # --- FETCHING CONFIGURATION ---
        bot_row = None
        cfg_row = None
        config_id = None
        should_skip_market = False

        for _db_attempt in range(5):
            try:
                with SessionLocal() as s:
                    repo = BotRepository(s)
                    bot_row = repo.get_bot(bot_id)

                    if bot_row is None:
                        # recreate if deleted
                        bootstrap_cfg_id = repo.upsert_configuration(cfg)
                        repo.upsert_bot(bot_id, bot_description, account_name, "ACTIVE", bootstrap_cfg_id)
                        bot_row = repo.get_bot(bot_id)

                    if bot_row.status != "ACTIVE":
                        should_skip_market = True
                        break

                    cfg_row = repo.get_configuration(bot_row.configuration_id) if bot_row.configuration_id else None
                    if cfg_row is None:
                        # fallback to env cfg and ensure it is stored
                        config_id = repo.upsert_configuration(cfg)
                    else:
                        config_id = cfg_row.configuration_id
                        # build cfg object from row
                        cfg = BotConfig(
                            clob_host=cfg_row.clob_host,
                            ws_base=cfg_row.ws_base,
                            chain_id=int(cfg_row.chain_id),
                            private_key=cfg_row.private_key,
                            signature_type=(int(cfg_row.signature_type) if cfg_row.signature_type is not None else None),
                            funder=cfg_row.funder,

                            tick=float(cfg_row.tick),
                            min_shares=float(cfg_row.min_shares),
                            lock_profit_target=float(cfg_row.lock_profit_target),

                            clip_shares=float(cfg_row.clip_shares),
                            improve_bid_ticks=int(cfg_row.improve_bid_ticks),
                            maker_buffer_ticks=int(cfg_row.maker_buffer_ticks),
                            replace_if_price_moves_ticks=int(cfg_row.replace_if_price_moves_ticks),
                            stale_seconds=int(cfg_row.stale_seconds),

                            entry_edge_ticks=int(cfg_row.entry_edge_ticks),
                            hedge_buffer_ticks=int(cfg_row.hedge_buffer_ticks),
                            max_total_cost=float(cfg_row.max_total_cost),
                            reserve_usd=float(cfg_row.reserve_usd),

                            cancel_all_on_start=bool(cfg_row.cancel_all_on_start),
                            dry_run=bool(cfg_row.dry_run),
                            log_every=int(cfg_row.log_every),

                            market_data_stale_seconds=int(cfg_row.market_data_stale_seconds),
                            ws_reconnect_min=float(cfg_row.ws_reconnect_min),
                            ws_reconnect_max=float(cfg_row.ws_reconnect_max),

                            stop_buffer_seconds=int(cfg_row.stop_buffer_seconds),
                        )
                break
            except OperationalError as e:
                bot_logger.error(f"⚠️ DB Error (fetching config): {e}. Retrying {_db_attempt+1}/5...")
                time.sleep(2)
            except Exception as e:
                # Catch generic if it looks like connection drop
                if "closed the connection" in str(e) or "OperationalError" in str(e):
                    bot_logger.error(f"⚠️ DB Error (fetching config): {e}. Retrying {_db_attempt+1}/5...")
                    time.sleep(2)
                else:
                    raise e
        
        if should_skip_market:
            bot_logger.warning(f"🛑 Bot DISABLED in DB. Skipping {current_slug}.")
            time.sleep(2)
            current_slug = get_next_slug(current_slug)
            continue

        exit_reason = "UNKNOWN"
        trade_id = None
        run_reason = None
        
        # Create pending trade record BEFORE running
        try:
            for _db_attempt in range(5):
                try:
                    with SessionLocal() as s:
                        repo = BotRepository(s)
                        # We need start_iso. Bot creates it in __init__, but we can preempt it or pass it.
                        # Bot init is fast, let's init bot first to get proper start time, or just allow small drift.
                        # Better: Init bot first, then record, then run.
                        bot = MakerHedgeCapBot(cfg, current_slug, bot_logger, signal_hub=signal_hub)

                        trade_id, status = repo.create_pending_trade(
                            bot_id=bot_id,
                            slug=current_slug,
                            configuration_id=config_id,
                            start_trade_iso=bot.start_trade_iso,
                        )
                        bot_logger.info(f"Created pending trade record: {trade_id} status={status}")
                    break
                except OperationalError as e:
                    bot_logger.error(f"⚠️ DB Error (create_trade): {e}. Retrying {_db_attempt+1}/5...")
                    time.sleep(2)
                except Exception as e:
                    if "closed the connection" in str(e) or "OperationalError" in str(e):
                        bot_logger.error(f"⚠️ DB Error (create_trade): {e}. Retrying {_db_attempt+1}/5...")
                        time.sleep(2)
                    else:
                        raise e
            
            if status != "INITIALIZED":
                bot_logger.info(f"⏭️ Trade {trade_id} already exists with status={status}. Skipping {current_slug}.")
                time.sleep(1)
                current_slug = get_next_slug(current_slug)
                continue

            run_reason = bot.run()
            exit_reason = run_reason
        except RuntimeError as e:
            if str(e) == "NO_MARKET":
                bot_logger.info(f"⏭️ No market yet for {current_slug}. Skipping.")
                time.sleep(2)
                current_slug = get_next_slug(current_slug)
                continue
            else:
                raise
        except Exception as e:
            bot_logger.warning(f"Bot crashed: {e}. Moving to next slug.")
            exit_reason = f"CRASH:{type(e).__name__}"
        finally:
            if bot is not None and trade_id is not None:
                metrics = bot.trade_metrics_snapshot()
                end_trade_iso = now_iso_jakarta()
                exit_reason = "FINALIZED"
                for _db_attempt in range(5):
                    try:
                        with SessionLocal() as s:
                            repo = BotRepository(s)
                            repo.update_trade_result(
                                trade_id=trade_id,
                                end_trade_iso=end_trade_iso,
                                lp=metrics["lp"],
                                total_cost=metrics["total_cost"],
                                q_yes=metrics["q_yes"],
                                q_no=metrics["q_no"],
                                cpp=metrics.get("cpp", 0.0),
                                exit_reason=exit_reason
                            )
                        break
                    except OperationalError as e:
                        bot_logger.error(f"⚠️ DB Error (update_trade): {e}. Retrying {_db_attempt+1}/5...")
                        time.sleep(2)
                    except Exception as e:
                        if "closed the connection" in str(e) or "OperationalError" in str(e):
                            bot_logger.error(f"⚠️ DB Error (update_trade): {e}. Retrying {_db_attempt+1}/5...")
                            time.sleep(2)
                        else:
                            raise e

                bot_logger.info(f"💾 Updated trade row {trade_id}. reason={exit_reason} lp={metrics['lp']:.4f} cost={metrics['total_cost']:.4f}")

        time.sleep(2)
        bot_logger.info(f"Ending this market {current_slug}")
        flow_reason = run_reason or exit_reason
        if isinstance(flow_reason, str) and flow_reason.startswith("SWITCH:"):
            next_slug = flow_reason.split(":", 1)[1].strip()
            bot_logger.info(f"🔁 Switching market due to signal: {current_slug} -> {next_slug}")
        else:
            next_slug = get_next_slug(current_slug)
        if next_slug == current_slug and not current_slug.split('-')[-1].isdigit():
            bot_logger.info(f"🛑 Non-timestamp slug '{current_slug}' -> no auto-roll. Stopping.")
            break
        current_slug = next_slug

        # ---- PNL summary (before next market wait) ----
        for _db_attempt in range(5):
            try:
                with SessionLocal() as s:
                    # Assuming _pnl_metrics is a function that returns a string to be logged
                    # and that bot_logger is the correct logger to use in this context.
                    # The original instruction mentioned `self.logger.info` but `self` is not available here.
                    # Using `bot_logger.info` as it's the available logger.
                    bot_logger.info(
                        print_pnl_metrics(s, bot_id, bot_logger)
                    )
                break
            except OperationalError as e:
                 bot_logger.info(f"⚠️ DB Error (pnl_metrics): {e}. Retrying {_db_attempt+1}/5...")
                 time.sleep(2)
            except Exception as e:
                if "closed the connection" in str(e) or "OperationalError" in str(e):
                    bot_logger.info(f"⚠️ DB Error (pnl_metrics): {e}. Retrying {_db_attempt+1}/5...")
                    time.sleep(2)
                else:
                    # just skip pnl if it fails
                    bot_logger.info(f"⚠️ PNL print failed: {e}")
                    break
        # ----------------------------------------------

        # rollover to next 15m
        bot_logger.info(f"💤 Waiting 2s before next market... {current_slug}")
        time.sleep(2)


if __name__ == "__main__":
    main()
