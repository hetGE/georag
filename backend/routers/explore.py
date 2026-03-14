"""Tag exploration: discover new categories from untagged files."""
import asyncio
from fastapi import APIRouter, Depends
from pydantic import BaseModel
from sqlalchemy.orm import Session

from backend.models.database import get_db
from backend.models.schemas import Tag, File, FileTag
from backend.services.tag_explorer import TagExplorer

router = APIRouter(tags=["explore"])

# Global explorer instance
_explorer = TagExplorer()


class AcceptTagsRequest(BaseModel):
    tags: list[dict]


@router.post("/explore/start")
async def start_exploring():
    """Start tag exploration on untagged processed files."""
    from backend.routers.processing import _processor
    from backend.routers.ocr import _ocr_processor
    if _processor.is_running:
        return {"error": "Cannot explore while processing is running."}
    if _ocr_processor.is_running:
        return {"error": "Cannot explore while OCR is running."}
    if _explorer.is_running:
        return {"error": "Exploration already in progress."}

    asyncio.create_task(_explorer.run())
    return {"status": "started"}


@router.post("/explore/stop")
async def stop_exploring():
    """Stop exploration after current batch."""
    _explorer.stop()
    return {"status": "stopping"}


@router.get("/explore/status")
async def explore_status():
    """Get current exploration status and results."""
    return {
        "is_running": _explorer.is_running,
        "total_files": _explorer.total_files,
        "processed_files": _explorer.processed_files,
        "current_batch": _explorer.current_batch,
        "total_batches": _explorer.total_batches,
        "errors": _explorer.errors[-10:],
        "candidates": _explorer.candidates if not _explorer.is_running else [],
        "has_results": _explorer.has_results(),
        "existing_tags_assigned": _explorer.existing_tags_assigned,
        "existing_files_tagged": _explorer.existing_files_tagged,
    }


@router.post("/explore/accept")
async def accept_tags(request: AcceptTagsRequest, db: Session = Depends(get_db)):
    """Accept selected candidate tags -- create them in the database."""
    from backend.routers.processing import _processor
    if _processor.is_running:
        return {"error": "Cannot create tags while processing is running."}

    created = []
    total_file_tags = 0

    # Build filename -> File lookup once
    all_filenames = set()
    for tag_data in request.tags:
        for fn in tag_data.get("filenames", []):
            all_filenames.add(fn)
    file_lookup = {
        f.filename: f
        for f in db.query(File).filter(File.filename.in_(all_filenames)).all()
    } if all_filenames else {}

    for tag_data in request.tags:
        existing = db.query(Tag).filter(Tag.name == tag_data["name"]).first()
        if existing:
            continue
        tag = Tag(
            name=tag_data["name"],
            display_name=tag_data["display_name"],
            description=tag_data.get("description", ""),
            color=tag_data.get("color", "#6c757d"),
        )
        db.add(tag)
        db.flush()  # get tag.id

        # Create FileTag associations for discovered files
        tag_file_count = 0
        for fn in tag_data.get("filenames", []):
            file_rec = file_lookup.get(fn)
            if not file_rec:
                continue
            db.add(FileTag(
                file_id=file_rec.id,
                tag_id=tag.id,
                source="auto",
                confidence=0.8,
            ))
            tag_file_count += 1

        tag.file_count = tag_file_count
        total_file_tags += tag_file_count
        created.append(tag_data["name"])

    db.commit()

    # Clear exploration results
    _explorer.candidates = []
    _explorer._raw_suggestions = {}

    return {"status": "accepted", "created": created, "file_tags_created": total_file_tags}


@router.post("/explore/dismiss")
async def dismiss_results():
    """Dismiss all exploration results without creating tags."""
    _explorer.candidates = []
    _explorer._raw_suggestions = {}
    return {"status": "dismissed"}
