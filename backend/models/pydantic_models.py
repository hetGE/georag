"""Pydantic models for API request/response validation."""
from pydantic import BaseModel
from typing import Optional
from datetime import datetime


class TagCreate(BaseModel):
    name: str
    display_name: str
    description: Optional[str] = None
    color: Optional[str] = "#6c757d"


class TagResponse(BaseModel):
    id: int
    name: str
    display_name: str
    description: Optional[str]
    color: str
    file_count: int

    class Config:
        from_attributes = True


class FileResponse(BaseModel):
    id: int
    relative_path: str
    filename: str
    extension: Optional[str]
    size_bytes: Optional[int]
    parent_directory: Optional[str]
    scan_status: str
    chunk_count: int
    tags: list[TagResponse] = []

    class Config:
        from_attributes = True


class ChatRequest(BaseModel):
    message: str
    conversation_id: Optional[int] = None
    tag_names: list[str] = []


class ConversationResponse(BaseModel):
    id: int
    title: str
    created_at: datetime
    updated_at: datetime
    selected_tags: list[str]

    class Config:
        from_attributes = True


class MessageResponse(BaseModel):
    id: int
    role: str
    content: str
    sources: list = []
    created_at: datetime

    class Config:
        from_attributes = True


class ProcessingRequest(BaseModel):
    tag_names: list[str] = []
    file_ids: list[int] = []
    reprocess: bool = False


class ProcessingStatus(BaseModel):
    is_running: bool
    total_files: int = 0
    processed_files: int = 0
    failed_files: int = 0
    current_file: Optional[str] = None
    errors: list[str] = []
