import os
import sys
from pathlib import Path
from loguru import logger

from services.storage_service import R2LogUploader

# --- your existing env ---
LOG_DIR = os.getenv("LOG_DIR", "output")
STATE_DIR = os.getenv("STATE_DIR", ".")
BOT_ID = os.getenv("BOT_ID")

logger.remove()
logger.add(sys.stderr, level="INFO")


def ensure_dir(p: Path) -> Path:
    p.mkdir(parents=True, exist_ok=True)
    return p


# -----------------------------
# R2 uploader setup (optional)
# -----------------------------
_uploader = None

def init_r2_uploader():
    global _uploader

    account_id = os.getenv("R2_ACCOUNT_ID")
    bucket = os.getenv("R2_BUCKET")
    access_key_id = os.getenv("R2_ACCESS_KEY_ID")
    secret_access_key = os.getenv("R2_SECRET_ACCESS_KEY")

    if not all([account_id, bucket, access_key_id, secret_access_key]):
        logger.info("R2 uploader disabled (missing R2 env vars).")
        return None

    from boto3 import __version__ as _  # ensure boto3 installed

    prefix = os.getenv("R2_PREFIX", "logs")
    scan_interval = int(os.getenv("R2_SCAN_INTERVAL", "5"))
    min_age = int(os.getenv("R2_MIN_AGE", "10"))

    # Import the class from wherever you put it
    # from your_module import R2LogUploader

    _uploader = R2LogUploader(
        account_id=account_id,
        bucket=bucket,
        access_key_id=access_key_id,
        secret_access_key=secret_access_key,
        bot_id=BOT_ID or "unknown",
        state_dir=Path(STATE_DIR),
        prefix=prefix,
        scan_interval=scan_interval,
        min_age_seconds=min_age,
    )
    _uploader.start()
    logger.info(f"R2 uploader enabled: bucket={bucket}, prefix={prefix}, bot_id={BOT_ID}")
    return _uploader


# Call this once at process start (e.g., in main)
# init_r2_uploader()


# Global registry of handlers added by setup_item_logger to prevent duplicates/leaks
_active_handlers = []

def setup_item_logger(item_id: str):
    """
    Creates per-item sinks:
      <LOG_DIR>/<item_id>/app.log
      <LOG_DIR>/<item_id>/app.json
    Uploads rotated/compressed (*.zip) logs to R2 under:
      <R2_PREFIX>/<BOT_ID>/<item_id>/
    """
    global _active_handlers

    # Cleanup previous handlers from this helper to avoid "Handler #N" leaks and rotation races
    if _active_handlers:
        for h_id in _active_handlers:
            try:
                logger.remove(h_id)
            except ValueError:
                pass
        _active_handlers.clear()

    item_dir = ensure_dir(Path(LOG_DIR) / str(item_id))

    # Track this directory for upload (if uploader enabled)
    if _uploader is not None:
        _uploader.track(str(item_id), item_dir)

    # Create a "child" logger with contextual data
    l = logger.bind(item_id=item_id, item_dir=str(item_dir))

    # Important: filter routes records to only this item's sinks
    def only_this_item(record):
        return record["extra"].get("item_id") == item_id

    # Text .log sink
    h1 = l.add(
        item_dir / "app.log",
        level="DEBUG",
        rotation="10 MB",
        retention="14 days",
        compression="zip",
        backtrace=True,
        diagnose=False,
        enqueue=True,
        filter=only_this_item,
        format="{time:YYYY-MM-DD HH:mm:ss}|{level}| {message}",
    )

    # JSON sink (NDJSON)
    h2 = l.add(
        item_dir / "app.json",
        level="DEBUG",
        rotation="10 MB",
        retention="14 days",
        compression="zip",
        enqueue=True,
        filter=only_this_item,
        serialize=True,
    )

    _active_handlers.extend([h1, h2])
    return l
