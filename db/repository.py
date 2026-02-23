# db/repository.py
from sqlalchemy.orm import Session
from sqlalchemy import select, func
from .models import Bot, Configuration, Trade
from .utils import now_iso_jakarta, date_jakarta, cfg_hash, new_uuid

class BotRepository:
    def __init__(self, session: Session):
        self.s = session

    # ---------- schema ----------
    @staticmethod
    def init_schema(engine):
        from .models import Base
        Base.metadata.create_all(engine)

    # ---------- bot ----------
    def upsert_bot(self, bot_id: str, bot_description: str, account_name: str, status: str, configuration_id: str):
        now = now_iso_jakarta()
        bot = self.s.get(Bot, bot_id)
        if bot:
            bot.bot_description = bot_description
            bot.account_name = account_name
            bot.status = status
            bot.configuration_id = configuration_id
            bot.updated_at = now
        else:
            bot = Bot(
                bot_id=bot_id,
                bot_description=bot_description,
                account_name=account_name,
                status=status,
                configuration_id=configuration_id,
                created_at=now,
                updated_at=now,
            )
            self.s.add(bot)
        self.s.commit()

    def get_bot(self, bot_id: str) -> Bot | None:
        return self.s.get(Bot, bot_id)

    # ---------- configuration ----------
    def upsert_configuration(self, cfg) -> str:
        h = cfg_hash(cfg)

        existing = self.s.execute(
            select(Configuration).where(Configuration.config_hash == h)
        ).scalar_one_or_none()

        if existing:
            return existing.configuration_id

        cid = new_uuid()
        now = now_iso_jakarta()

        c = Configuration(
            configuration_id=cid,
            config_hash=h,

            clob_host=cfg.clob_host,
            ws_base=cfg.ws_base,
            chain_id=int(cfg.chain_id),
            private_key=cfg.private_key,
            signature_type=(int(cfg.signature_type) if cfg.signature_type is not None else None),
            funder=cfg.funder,

            tick=float(cfg.tick),
            min_shares=float(cfg.min_shares),
            lock_profit_target=float(cfg.lock_profit_target),

            clip_shares=float(cfg.clip_shares),
            improve_bid_ticks=int(cfg.improve_bid_ticks),
            maker_buffer_ticks=int(cfg.maker_buffer_ticks),
            replace_if_price_moves_ticks=int(cfg.replace_if_price_moves_ticks),
            stale_seconds=int(cfg.stale_seconds),

            entry_edge_ticks=int(cfg.entry_edge_ticks),
            hedge_buffer_ticks=int(cfg.hedge_buffer_ticks),
            max_total_cost=float(cfg.max_total_cost),
            reserve_usd=float(cfg.reserve_usd),

            cancel_all_on_start=1 if cfg.cancel_all_on_start else 0,
            dry_run=1 if cfg.dry_run else 0,
            log_every=int(cfg.log_every),

            market_data_stale_seconds=int(cfg.market_data_stale_seconds),
            ws_reconnect_min=float(cfg.ws_reconnect_min),
            ws_reconnect_max=float(cfg.ws_reconnect_max),

            stop_buffer_seconds=int(cfg.stop_buffer_seconds),

            created_at=now,
        )
        self.s.add(c)
        self.s.commit()
        return cid

    def get_configuration(self, configuration_id: str) -> Configuration | None:
        return self.s.get(Configuration, configuration_id)

    def pnl_and_trade_count_for_bot(self, bot_id: str, start_date: str, end_date: str) -> tuple[float, int]:
        """
        PNL = SUM(trade.lp) for a specific bot_id
        Total trade = COUNT(*)
        Uses Trade.date (YYYY-MM-DD Jakarta) inclusive range.
        """
        stmt = (
            select(
                func.coalesce(func.sum(Trade.lp), 0.0),
                func.count(Trade.trade_id),
            )
            .where(Trade.bot_id == bot_id)
            .where(Trade.date >= start_date)
            .where(Trade.date <= end_date)
            .where(Trade.status.in_(["WON", "LOSS", "DRAW"]))
            .where(
                ~(
                    (Trade.status == "DRAW")
                    & (func.coalesce(Trade.total_cost, 0.0) <= 1e-9)
                    & (func.coalesce(Trade.q_yes, 0.0) <= 1e-9)
                    & (func.coalesce(Trade.q_no, 0.0) <= 1e-9)
                )
            )
        )
        pnl, cnt = self.s.execute(stmt).one()
        return float(pnl), int(cnt)

    def pnl_and_trade_count_all_bots(self, start_date: str, end_date: str) -> tuple[float, int]:
        """
        PNL = SUM(trade.lp) across ALL bots
        Total trade = COUNT(*)
        Uses Trade.date (YYYY-MM-DD Jakarta) inclusive range.
        """
        stmt = (
            select(
                func.coalesce(func.sum(Trade.lp), 0.0),
                func.count(Trade.trade_id),
            )
            .where(Trade.date >= start_date)
            .where(Trade.date <= end_date)
            .where(Trade.status.in_(["WON", "LOSS", "DRAW"]))
            .where(
                ~(
                    (Trade.status == "DRAW")
                    & (func.coalesce(Trade.total_cost, 0.0) <= 1e-9)
                    & (func.coalesce(Trade.q_yes, 0.0) <= 1e-9)
                    & (func.coalesce(Trade.q_no, 0.0) <= 1e-9)
                )
            )
        )
        pnl, cnt = self.s.execute(stmt).one()
        return float(pnl), int(cnt)

    # ---------- trade ----------
    def create_pending_trade(
        self,
        bot_id: str,
        slug: str,
        configuration_id: str,
        start_trade_iso: str,
    ) -> tuple[str, str]:
        # Check if exists
        existing = self.s.execute(
            select(Trade).where(Trade.bot_id == bot_id).where(Trade.slug == slug)
        ).scalar_one_or_none()
        
        if existing:
            return existing.trade_id, existing.status

        tid = new_uuid()
        t = Trade(
            trade_id=tid,
            bot_id=bot_id,
            slug=slug,
            configuration_id=configuration_id,
            date=date_jakarta(),
            start_trade=start_trade_iso,
            end_trade="",  # updated later
            lp=0.0,
            total_cost=0.0,
            q_yes=0.0,
            q_no=0.0,
            cpp=0.0,
            status="INITIALIZED",
            claim_status=None,
            meta_data=None,
            exit_reason="RUNNING"
        )
        self.s.add(t)
        self.s.commit()
        return tid, "INITIALIZED"

    def update_trade_result(
        self,
        trade_id: str,
        end_trade_iso: str,
        lp: float,
        total_cost: float,
        cpp: float,
        q_yes: float,
        q_no: float,
        exit_reason: str,
    ):
        t = self.s.get(Trade, trade_id)
        if t:
            t.end_trade = end_trade_iso
            t.lp = float(lp)
            t.total_cost = float(total_cost)
            t.cpp = float(cpp)
            t.q_yes = float(q_yes)
            t.q_no = float(q_no)
            t.exit_reason = exit_reason
            
            # WON/LOSS logic
            if t.lp > 0:
                t.status = "WON"
            elif t.lp < 0:
                t.status = "LOSS"
            else:
                t.status = "DRAW"

            self.s.commit()

