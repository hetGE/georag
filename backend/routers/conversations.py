"""Conversation history management."""
import datetime
from fastapi import APIRouter, Depends
from sqlalchemy.orm import Session

from backend.models.database import get_db
from backend.models.schemas import Conversation, Message

router = APIRouter(tags=["conversations"])


@router.get("/conversations/trash")
def list_trash(db: Session = Depends(get_db)):
    convos = (
        db.query(Conversation)
        .filter(Conversation.deleted_at.isnot(None))
        .order_by(Conversation.deleted_at.desc())
        .limit(50)
        .all()
    )
    return [
        {
            "id": c.id,
            "title": c.title,
            "created_at": c.created_at.isoformat() if c.created_at else None,
            "deleted_at": c.deleted_at.isoformat() if c.deleted_at else None,
        }
        for c in convos
    ]


@router.get("/conversations")
def list_conversations(db: Session = Depends(get_db)):
    convos = (
        db.query(Conversation)
        .filter(Conversation.deleted_at.is_(None))
        .order_by(Conversation.updated_at.desc())
        .limit(50)
        .all()
    )
    return [
        {
            "id": c.id,
            "title": c.title,
            "created_at": c.created_at.isoformat() if c.created_at else None,
            "updated_at": c.updated_at.isoformat() if c.updated_at else None,
            "selected_tags": c.selected_tags or [],
        }
        for c in convos
    ]


@router.get("/conversations/{conv_id}")
def get_conversation(conv_id: int, db: Session = Depends(get_db)):
    conv = db.get(Conversation, conv_id)
    if not conv:
        return {"error": "Conversation not found"}

    messages = (
        db.query(Message)
        .filter(Message.conversation_id == conv_id)
        .order_by(Message.created_at)
        .all()
    )
    return {
        "id": conv.id,
        "title": conv.title,
        "selected_tags": conv.selected_tags or [],
        "messages": [
            {
                "id": m.id,
                "role": m.role,
                "content": m.content,
                "sources": m.sources or [],
                "created_at": m.created_at.isoformat() if m.created_at else None,
            }
            for m in messages
        ],
    }


@router.delete("/conversations/{conv_id}")
def delete_conversation(conv_id: int, db: Session = Depends(get_db)):
    """Soft-delete: move conversation to trash."""
    conv = db.get(Conversation, conv_id)
    if not conv:
        return {"error": "Conversation not found"}

    conv.deleted_at = datetime.datetime.utcnow()
    db.commit()
    return {"status": "trashed"}


@router.delete("/conversations/{conv_id}/permanent")
def permanent_delete_conversation(conv_id: int, db: Session = Depends(get_db)):
    """Hard-delete: permanently remove conversation and its messages."""
    conv = db.get(Conversation, conv_id)
    if not conv:
        return {"error": "Conversation not found"}

    db.query(Message).filter(Message.conversation_id == conv_id).delete()
    db.delete(conv)
    db.commit()
    return {"status": "deleted"}


@router.post("/conversations/{conv_id}/restore")
def restore_conversation(conv_id: int, db: Session = Depends(get_db)):
    """Restore a trashed conversation."""
    conv = db.get(Conversation, conv_id)
    if not conv:
        return {"error": "Conversation not found"}

    conv.deleted_at = None
    db.commit()
    return {"status": "restored"}
