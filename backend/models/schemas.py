"""SQLAlchemy ORM models."""
import datetime
from sqlalchemy import (
    Column, Integer, String, Float, DateTime, Text, ForeignKey, JSON, Index, Boolean
)
from sqlalchemy.orm import relationship
from backend.models.database import Base


class File(Base):
    __tablename__ = "files"

    id = Column(Integer, primary_key=True, autoincrement=True)
    relative_path = Column(String, unique=True, nullable=False, index=True)
    filename = Column(String, nullable=False, index=True)
    extension = Column(String, index=True)
    size_bytes = Column(Integer)
    modified_time = Column(Float)
    parent_directory = Column(String, index=True)
    content_hash = Column(String)
    scan_status = Column(String, default="new", index=True)  # new/processed/failed/skipped
    extracted_text_preview = Column(Text)
    chunk_count = Column(Integer, default=0)
    processed_at = Column(DateTime)
    auto_tagged = Column(Integer, default=0)  # 0=no, 1=yes
    auto_tag_confidence = Column(Float)

    tags = relationship("FileTag", back_populates="file", cascade="all, delete-orphan")

    __table_args__ = (
        Index("ix_files_status_ext", "scan_status", "extension"),
    )


class Tag(Base):
    __tablename__ = "tags"

    id = Column(Integer, primary_key=True, autoincrement=True)
    name = Column(String, unique=True, nullable=False, index=True)
    display_name = Column(String, nullable=False)
    description = Column(Text)
    color = Column(String, default="#6c757d")
    file_count = Column(Integer, default=0)

    files = relationship("FileTag", back_populates="tag", cascade="all, delete-orphan")


class FileTag(Base):
    __tablename__ = "file_tags"

    id = Column(Integer, primary_key=True, autoincrement=True)
    file_id = Column(Integer, ForeignKey("files.id", ondelete="CASCADE"), nullable=False)
    tag_id = Column(Integer, ForeignKey("tags.id", ondelete="CASCADE"), nullable=False)
    source = Column(String, default="manual")  # auto/manual/folder_hint
    confidence = Column(Float)
    created_at = Column(DateTime, default=datetime.datetime.utcnow)

    file = relationship("File", back_populates="tags")
    tag = relationship("Tag", back_populates="files")

    __table_args__ = (
        Index("ix_file_tags_unique", "file_id", "tag_id", unique=True),
    )


class WikiPage(Base):
    __tablename__ = "wiki_pages"

    id = Column(Integer, primary_key=True, autoincrement=True)
    slug = Column(String, unique=True, nullable=False, index=True)
    title = Column(String, nullable=False)
    content = Column(Text, nullable=False, default="")
    category = Column(String, default="general", index=True)  # entity, concept, source_summary, comparison, topic, index, log
    summary = Column(Text)
    source_files = Column(JSON, default=list)
    backlinks = Column(JSON, default=list)
    created_at = Column(DateTime, default=datetime.datetime.utcnow)
    updated_at = Column(DateTime, default=datetime.datetime.utcnow, onupdate=datetime.datetime.utcnow)


class WikiLog(Base):
    __tablename__ = "wiki_log"

    id = Column(Integer, primary_key=True, autoincrement=True)
    operation = Column(String, nullable=False)  # ingest, query, lint, chat_growth
    detail = Column(Text)
    pages_affected = Column(JSON, default=list)
    created_at = Column(DateTime, default=datetime.datetime.utcnow)


class Conversation(Base):
    __tablename__ = "conversations"

    id = Column(Integer, primary_key=True, autoincrement=True)
    title = Column(String, default="New Conversation")
    created_at = Column(DateTime, default=datetime.datetime.utcnow)
    updated_at = Column(DateTime, default=datetime.datetime.utcnow, onupdate=datetime.datetime.utcnow)
    deleted_at = Column(DateTime, nullable=True, default=None)
    selected_tags = Column(JSON, default=list)

    messages = relationship("Message", back_populates="conversation", cascade="all, delete-orphan",
                            order_by="Message.created_at")


class Message(Base):
    __tablename__ = "messages"

    id = Column(Integer, primary_key=True, autoincrement=True)
    conversation_id = Column(Integer, ForeignKey("conversations.id", ondelete="CASCADE"), nullable=False)
    role = Column(String, nullable=False)  # user/assistant/system
    content = Column(Text, nullable=False)
    sources = Column(JSON, default=list)
    created_at = Column(DateTime, default=datetime.datetime.utcnow)

    conversation = relationship("Conversation", back_populates="messages")


class AppSettings(Base):
    """Single-row config (id=1) for the scheduled-downtime + auto-shutdown features."""
    __tablename__ = "app_settings"

    id = Column(Integer, primary_key=True)  # always 1
    schedule_enabled = Column(Boolean, default=False, nullable=False)
    downtime_start = Column(String, default="06:30", nullable=False)  # local "HH:MM"
    downtime_end = Column(String, default="09:30", nullable=False)
    auto_shutdown_on_manual_pause = Column(Boolean, default=False, nullable=False)
    scheduled_run_active = Column(Boolean, default=False, nullable=False)
    scheduled_tag_names = Column(JSON, default=list, nullable=False)
    scheduled_file_ids = Column(JSON, default=list, nullable=False)
    updated_at = Column(DateTime, default=datetime.datetime.utcnow,
                        onupdate=datetime.datetime.utcnow)
