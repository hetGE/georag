"""CRUD tags, auto-tag endpoint."""
from fastapi import APIRouter, Depends
from sqlalchemy import func
from sqlalchemy.orm import Session

from backend.models.database import get_db
from backend.models.schemas import Tag, FileTag
from backend.models.pydantic_models import TagCreate, TagUpdate
from backend.routers.processing import _processor
from backend.services import vector_store

router = APIRouter(tags=["tags"])


@router.get("/tags")
def list_tags(db: Session = Depends(get_db)):
    count_sub = (
        db.query(FileTag.tag_id, func.count().label("cnt"))
        .group_by(FileTag.tag_id)
        .subquery()
    )
    rows = (
        db.query(Tag, func.coalesce(count_sub.c.cnt, 0).label("live_count"))
        .outerjoin(count_sub, Tag.id == count_sub.c.tag_id)
        .order_by(Tag.display_name)
        .all()
    )
    return [
        {
            "id": t.id,
            "name": t.name,
            "display_name": t.display_name,
            "description": t.description,
            "color": t.color,
            "file_count": live_count,
        }
        for t, live_count in rows
    ]


@router.post("/tags")
def create_tag(tag_data: TagCreate, db: Session = Depends(get_db)):
    if _processor.is_running and not _processor._stop_flag:
        return {"error": "Cannot create tags while processing is running. Stop processing first."}

    existing = db.query(Tag).filter(Tag.name == tag_data.name).first()
    if existing:
        return {"error": "Tag already exists", "tag": existing.name}

    tag = Tag(**tag_data.model_dump())
    db.add(tag)
    db.commit()
    db.refresh(tag)
    return {"id": tag.id, "name": tag.name, "display_name": tag.display_name}


@router.put("/tags/{tag_id}")
def update_tag(tag_id: int, tag_data: TagUpdate, db: Session = Depends(get_db)):
    tag = db.query(Tag).filter(Tag.id == tag_id).first()
    if not tag:
        return {"error": "Tag not found"}
    if tag_data.display_name is not None:
        tag.display_name = tag_data.display_name
    if tag_data.description is not None:
        tag.description = tag_data.description
    if tag_data.color is not None:
        tag.color = tag_data.color
    db.commit()
    db.refresh(tag)
    return {
        "id": tag.id,
        "name": tag.name,
        "display_name": tag.display_name,
        "description": tag.description,
        "color": tag.color,
        "file_count": tag.file_count,
    }


@router.delete("/tags/{tag_name}")
def delete_tag(tag_name: str, db: Session = Depends(get_db)):
    if _processor.is_running and not _processor._stop_flag:
        return {"error": "Cannot delete tags while processing is running. Stop processing first."}

    tag = db.query(Tag).filter(Tag.name == tag_name).first()
    if not tag:
        return {"error": "Tag not found"}

    db.query(FileTag).filter(FileTag.tag_id == tag.id).delete()
    db.delete(tag)
    db.commit()

    # Remove the ChromaDB collection for this tag
    try:
        vector_store.delete_collection(tag_name)
    except Exception:
        pass

    return {"status": "deleted"}
