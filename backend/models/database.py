"""SQLAlchemy engine and session management."""
from sqlalchemy import create_engine, text, inspect, event
from sqlalchemy.orm import sessionmaker, declarative_base

from backend.config import DATABASE_URL

engine = create_engine(DATABASE_URL, connect_args={"check_same_thread": False})


@event.listens_for(engine, "connect")
def _set_sqlite_pragmas(dbapi_conn, _conn_record):
    """WAL lets readers and a writer run concurrently, and a busy_timeout makes
    any remaining contention wait briefly instead of raising "database is
    locked"."""
    cur = dbapi_conn.cursor()
    cur.execute("PRAGMA journal_mode=WAL")
    cur.execute("PRAGMA busy_timeout=5000")
    cur.execute("PRAGMA synchronous=NORMAL")
    cur.close()


SessionLocal = sessionmaker(autocommit=False, autoflush=False, bind=engine)
Base = declarative_base()


def get_db():
    """Dependency that yields a database session."""
    db = SessionLocal()
    try:
        yield db
    finally:
        db.close()


def init_db():
    """Create all tables."""
    from backend.models.schemas import (  # noqa: F401
        File, Tag, FileTag, Conversation, Message, AppSettings
    )
    Base.metadata.create_all(bind=engine)

    # Migration: add deleted_at column to existing conversations table
    insp = inspect(engine)
    columns = [c['name'] for c in insp.get_columns('conversations')]
    if 'deleted_at' not in columns:
        with engine.begin() as conn:
            conn.execute(text("ALTER TABLE conversations ADD COLUMN deleted_at DATETIME"))

    # Seed AppSettings row (id=1) if missing
    with engine.begin() as conn:
        row = conn.execute(text("SELECT id FROM app_settings WHERE id=1")).first()
        if row is None:
            conn.execute(text(
                "INSERT INTO app_settings (id, schedule_enabled, downtime_start, downtime_end, "
                "auto_shutdown_on_manual_pause) "
                "VALUES (1, 0, '06:30', '09:30', 0)"
            ))
