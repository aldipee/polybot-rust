# db/models.py
from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column
from sqlalchemy import String, Integer, Float, Text, CheckConstraint, UniqueConstraint

class Base(DeclarativeBase):
    pass

class Bot(Base):
    __tablename__ = "bot"

    bot_id: Mapped[str] = mapped_column(String, primary_key=True)
    bot_description: Mapped[str | None] = mapped_column(Text, nullable=True)
    account_name: Mapped[str | None] = mapped_column(String, nullable=True)

    status: Mapped[str] = mapped_column(String, nullable=False)  # ACTIVE / DISABLED
    configuration_id: Mapped[str | None] = mapped_column(String, nullable=True)

    created_at: Mapped[str] = mapped_column(String, nullable=False)
    updated_at: Mapped[str] = mapped_column(String, nullable=False)

    __table_args__ = (
        CheckConstraint("status IN ('ACTIVE','DISABLED')", name="ck_bot_status"),
    )

class Configuration(Base):
    __tablename__ = "configuration"

    configuration_id: Mapped[str] = mapped_column(String, primary_key=True)  # UUID
    config_hash: Mapped[str] = mapped_column(String, nullable=False, unique=True)

    clob_host: Mapped[str] = mapped_column(String, nullable=False)
    ws_base: Mapped[str] = mapped_column(String, nullable=False)
    chain_id: Mapped[int] = mapped_column(Integer, nullable=False)
    private_key: Mapped[str] = mapped_column(String, nullable=False)
    signature_type: Mapped[int | None] = mapped_column(Integer, nullable=True)
    funder: Mapped[str | None] = mapped_column(String, nullable=True)

    tick: Mapped[float] = mapped_column(Float, nullable=False)
    min_shares: Mapped[float] = mapped_column(Float, nullable=False)
    lock_profit_target: Mapped[float] = mapped_column(Float, nullable=False)

    clip_shares: Mapped[float] = mapped_column(Float, nullable=False)
    improve_bid_ticks: Mapped[int] = mapped_column(Integer, nullable=False)
    maker_buffer_ticks: Mapped[int] = mapped_column(Integer, nullable=False)
    replace_if_price_moves_ticks: Mapped[int] = mapped_column(Integer, nullable=False)
    stale_seconds: Mapped[int] = mapped_column(Integer, nullable=False)

    entry_edge_ticks: Mapped[int] = mapped_column(Integer, nullable=False)
    hedge_buffer_ticks: Mapped[int] = mapped_column(Integer, nullable=False)
    max_total_cost: Mapped[float] = mapped_column(Float, nullable=False)
    reserve_usd: Mapped[float] = mapped_column(Float, nullable=False)

    cancel_all_on_start: Mapped[int] = mapped_column(Integer, nullable=False)  # 0/1
    dry_run: Mapped[int] = mapped_column(Integer, nullable=False)              # 0/1
    log_every: Mapped[int] = mapped_column(Integer, nullable=False)

    market_data_stale_seconds: Mapped[int] = mapped_column(Integer, nullable=False)
    ws_reconnect_min: Mapped[float] = mapped_column(Float, nullable=False)
    ws_reconnect_max: Mapped[float] = mapped_column(Float, nullable=False)

    stop_buffer_seconds: Mapped[int] = mapped_column(Integer, nullable=False)

    created_at: Mapped[str] = mapped_column(String, nullable=False)

class Trade(Base):
    __tablename__ = "trade"

    trade_id: Mapped[str] = mapped_column(String, primary_key=True)
    exit_reason: Mapped[str] = mapped_column(String, nullable=False)

    bot_id: Mapped[str] = mapped_column(String, nullable=False)
    slug: Mapped[str] = mapped_column(String, nullable=False)
    configuration_id: Mapped[str] = mapped_column(String, nullable=False)

    date: Mapped[str] = mapped_column(String, nullable=False)       # YYYY-MM-DD (Jakarta)
    start_trade: Mapped[str] = mapped_column(String, nullable=False) # ISO
    end_trade: Mapped[str] = mapped_column(String, nullable=False)   # ISO

    lp: Mapped[float] = mapped_column(Float, nullable=False)
    total_cost: Mapped[float] = mapped_column(Float, nullable=False)
    q_yes: Mapped[float] = mapped_column(Float, nullable=False)
    q_no: Mapped[float] = mapped_column(Float, nullable=False)
    cpp: Mapped[float] = mapped_column(Float, nullable=False, default=0.0)
    status: Mapped[str | None] = mapped_column(String, nullable=True)
    claim_status: Mapped[str | None] = mapped_column(String, nullable=True)
    meta_data: Mapped[str | None] = mapped_column(Text, nullable=True)
    # realized_lp: Mapped[float] = mapped_column(Float, nullable=False)
    # locked_lp: Mapped[float] = mapped_column(Float, nullable=False)
    # gross_buy: Mapped[float] = mapped_column(Float, nullable=False)
    # gross_sell: Mapped[float] = mapped_column(Float, nullable=False)
    # num_buy: Mapped[float] = mapped_column(Float, nullable=False)
    # num_sell: Mapped[float] = mapped_column(Float, nullable=False)
    # max_abs_exposure: Mapped[float] = mapped_column(Float, nullable=False)
