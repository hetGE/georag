"""CRUD tags, auto-tag endpoint."""
from fastapi import APIRouter, Depends
from sqlalchemy.orm import Session

from backend.models.database import get_db
from backend.models.schemas import Tag, FileTag
from backend.models.pydantic_models import TagCreate

router = APIRouter(tags=["tags"])


@router.get("/tags")
def list_tags(db: Session = Depends(get_db)):
    tags = db.query(Tag).order_by(Tag.display_name).all()
    return [
        {
            "id": t.id,
            "name": t.name,
            "display_name": t.display_name,
            "description": t.description,
            "color": t.color,
            "file_count": t.file_count,
        }
        for t in tags
    ]


@router.post("/tags")
def create_tag(tag_data: TagCreate, db: Session = Depends(get_db)):
    existing = db.query(Tag).filter(Tag.name == tag_data.name).first()
    if existing:
        return {"error": "Tag already exists", "tag": existing.name}

    tag = Tag(**tag_data.model_dump())
    db.add(tag)
    db.commit()
    db.refresh(tag)
    return {"id": tag.id, "name": tag.name, "display_name": tag.display_name}


@router.delete("/tags/{tag_name}")
def delete_tag(tag_name: str, db: Session = Depends(get_db)):
    tag = db.query(Tag).filter(Tag.name == tag_name).first()
    if not tag:
        return {"error": "Tag not found"}

    db.query(FileTag).filter(FileTag.tag_id == tag.id).delete()
    db.delete(tag)
    db.commit()
    return {"status": "deleted"}
