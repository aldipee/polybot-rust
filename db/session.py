# db/session.py
from sqlalchemy import create_engine
from sqlalchemy.orm import sessionmaker, Session

def make_engine(db_url: str):
    # For SQLite: db_url = "sqlite:///./bot.sqlite3"
    return create_engine(db_url, future=True)

def make_session_factory(engine):
    return sessionmaker(bind=engine, class_=Session, expire_on_commit=False, future=True)
