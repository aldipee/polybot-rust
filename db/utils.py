# db/utils.py
import json
import hashlib
import uuid
from dataclasses import asdict
from datetime import datetime, timedelta
from zoneinfo import ZoneInfo

JAKARTA_TZ = ZoneInfo("Asia/Jakarta")

def now_iso_jakarta() -> str:
    return datetime.now(tz=JAKARTA_TZ).isoformat(timespec="seconds")

def date_jakarta() -> str:
    return datetime.now(tz=JAKARTA_TZ).date().isoformat()

def cfg_hash(cfg) -> str:
    payload = json.dumps(asdict(cfg), sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()

def new_uuid() -> str:
    return str(uuid.uuid4())


def week_start_date_jakarta() -> str:
    """Monday as start of week."""
    d = datetime.now(tz=JAKARTA_TZ).date()
    start = d - timedelta(days=d.weekday())  # Mon=0
    return start.isoformat()

def month_start_date_jakarta() -> str:
    d = datetime.now(tz=JAKARTA_TZ).date()
    return d.replace(day=1).isoformat()
