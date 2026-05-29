"""SQLAlchemy engine and session management."""
from sqlalchemy import create_engine, text, inspect
from sqlalchemy.orm import sessionmaker, declarative_base

from backend.config import DATABASE_URL

engine = create_engine(DATABASE_URL, connect_args={"check_same_thread": False})
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

    # Migration: add per-build attempted-file tracking to an existing app_settings table
    app_cols = [c['name'] for c in insp.get_columns('app_settings')]
    if 'scheduled_processed_file_ids' not in app_cols:
        with engine.begin() as conn:
            conn.execute(text(
                "ALTER TABLE app_settings ADD COLUMN scheduled_processed_file_ids JSON DEFAULT '[]'"))

    # Seed AppSettings row (id=1) if missing
    with engine.begin() as conn:
        row = conn.execute(text("SELECT id FROM app_settings WHERE id=1")).first()
        if row is None:
            conn.execute(text(
                "INSERT INTO app_settings (id, schedule_enabled, downtime_start, downtime_end, "
                "auto_shutdown_on_manual_pause, scheduled_run_active, "
                "scheduled_tag_names, scheduled_file_ids, scheduled_processed_file_ids) "
                "VALUES (1, 0, '06:30', '09:30', 0, 0, '[]', '[]', '[]')"
            ))
