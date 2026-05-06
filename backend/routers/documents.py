"""Document list, search, filter, pagination."""
import logging
import os
import platform
import subprocess
from pathlib import Path
from fastapi import APIRouter, Depends, Query, HTTPException
from pydantic import BaseModel
from sqlalchemy.orm import Session, joinedload
from sqlalchemy import func

from backend.config import ENGINEERING_ROOT
from backend.models.database import get_db
from backend.models.schemas import File, Tag, FileTag
from backend.models.pydantic_models import BatchTagRequest
from backend.services.embedding_client import embed_text
from backend.services.vector_store import search_file_paths

logger = logging.getLogger(__name__)

router = APIRouter(tags=["documents"])


@router.get("/documents")
async def list_documents(
    search: str = Query("", description="Search filename or path"),
    search_contents: bool = Query(False, description="Also search file contents via embeddings"),
    extension: str = Query("", description="Filter by extension"),
    tag: str = Query("", description="Filter by tag name"),
    status: str = Query("", description="Filter by scan_status"),
    page: int = Query(1, ge=1),
    per_page: int = Query(50, ge=1, le=200),
    db: Session = Depends(get_db),
):
    query = db.query(File).options(joinedload(File.tags).joinedload(FileTag.tag))

    file_scores = {}
    if search and search_contents:
        # Semantic content search: embed query, find matching files in ChromaDB
        content_paths = []
        try:
            query_embedding = await embed_text(search)
            # Search specific tag or all tags
            if tag and tag != "__none__":
                search_tags = [tag]
            else:
                search_tags = [t[0] for t in db.query(Tag.name).all()]
            file_scores = search_file_paths(query_embedding, search_tags)
            content_paths = list(file_scores.keys())
            logger.info("Content search for '%s': %d files matched", search, len(content_paths))
        except Exception as e:
            logger.warning("Content search failed (is the embedding llama-server on :8002 running?): %s", e)
        # Match by filename/path OR by content similarity
        query = query.filter(
            (File.filename.ilike(f"%{search}%"))
            | (File.relative_path.ilike(f"%{search}%"))
            | (File.relative_path.in_(content_paths))
        )
    elif search:
        query = query.filter(
            (File.filename.ilike(f"%{search}%")) | (File.relative_path.ilike(f"%{search}%"))
        )
    if extension:
        query = query.filter(File.extension == extension.lower().lstrip("."))
    if tag == "__none__":
        query = query.filter(~File.tags.any())
    elif tag:
        query = query.join(File.tags).join(FileTag.tag).filter(Tag.name == tag)
    if status == "failed":
        # "Failed" filter covers both regular failures and OCR-specific failures
        query = query.filter(File.scan_status.in_(["failed", "ocr_failed"]))
    elif status:
        query = query.filter(File.scan_status == status)

    if file_scores:
        # Semantic search: fetch all matches, sort by relevance, paginate in Python
        all_files = query.distinct().all()
        all_files.sort(key=lambda f: (-file_scores.get(f.relative_path, -1), f.relative_path))
        total = len(all_files)
        files = all_files[(page - 1) * per_page : page * per_page]
    else:
        total = query.distinct().count()
        files = (
            query.distinct()
            .order_by(File.relative_path)
            .offset((page - 1) * per_page)
            .limit(per_page)
            .all()
        )

    def file_dict(f):
        d = {
            "id": f.id,
            "relative_path": f.relative_path,
            "filename": f.filename,
            "extension": f.extension,
            "size_bytes": f.size_bytes,
            "parent_directory": f.parent_directory,
            "scan_status": f.scan_status,
            "chunk_count": f.chunk_count,
            "tags": [
                {"name": ft.tag.name, "display_name": ft.tag.display_name, "color": ft.tag.color}
                for ft in f.tags
            ],
        }
        if f.relative_path in file_scores:
            d["relevance_score"] = round(file_scores[f.relative_path], 3)
        return d

    return {
        "total": total,
        "page": page,
        "per_page": per_page,
        "pages": (total + per_page - 1) // per_page if per_page else 1,
        "files": [file_dict(f) for f in files],
    }


@router.get("/documents/stats")
def document_stats(db: Session = Depends(get_db)):
    total = db.query(File).count()
    by_status = dict(
        db.query(File.scan_status, func.count(File.id))
        .group_by(File.scan_status)
        .all()
    )
    # Fold ocr_failed into failed so the summary and filter stay consistent
    if "ocr_failed" in by_status:
        by_status["failed"] = by_status.get("failed", 0) + by_status.pop("ocr_failed")
    by_ext = dict(
        db.query(File.extension, func.count(File.id))
        .group_by(File.extension)
        .order_by(func.count(File.id).desc())
        .limit(20)
        .all()
    )
    tagged_ids = db.query(FileTag.file_id).distinct()
    untagged = db.query(File).filter(~File.id.in_(tagged_ids)).count()
    return {"total": total, "by_status": by_status, "by_extension": by_ext, "untagged": untagged}


@router.post("/documents/{file_id}/open")
def open_file(file_id: int, db: Session = Depends(get_db)):
    """Open a file locally with the system default application."""
    file = db.get(File, file_id)
    if not file:
        raise HTTPException(status_code=404, detail="File not found")

    full_path = ENGINEERING_ROOT / file.relative_path
    if not full_path.exists():
        raise HTTPException(status_code=404, detail="File not found on disk")

    # Security: ensure the resolved path is still under ENGINEERING_ROOT
    resolved = full_path.resolve()
    if not str(resolved).startswith(str(ENGINEERING_ROOT.resolve())):
        raise HTTPException(status_code=403, detail="Access denied")

    system = platform.system()
    if system == "Darwin":
        subprocess.Popen(["open", str(resolved)])
    elif system == "Windows":
        os.startfile(str(resolved))
    else:
        subprocess.Popen(["xdg-open", str(resolved)])
    return {"status": "opened"}


@router.post("/documents/{file_id}/open-folder")
def open_folder(file_id: int, db: Session = Depends(get_db)):
    """Open the containing folder of a file with the file selected."""
    file = db.get(File, file_id)
    if not file:
        raise HTTPException(status_code=404, detail="File not found")

    full_path = ENGINEERING_ROOT / file.relative_path
    if not full_path.exists():
        raise HTTPException(status_code=404, detail="File not found on disk")

    resolved = full_path.resolve()
    if not str(resolved).startswith(str(ENGINEERING_ROOT.resolve())):
        raise HTTPException(status_code=403, detail="Access denied")

    system = platform.system()
    if system == "Darwin":
        subprocess.Popen(["open", "-R", str(resolved)])
    elif system == "Windows":
        subprocess.Popen(["explorer", "/select,", str(resolved)])
    else:
        subprocess.Popen(["xdg-open", str(resolved.parent)])
    return {"status": "opened"}


class OpenByPathRequest(BaseModel):
    file_path: str


@router.post("/documents/open-by-path")
def open_file_by_path(req: OpenByPathRequest):
    """Open a file by its relative path with the system default application."""
    full_path = ENGINEERING_ROOT / req.file_path
    if not full_path.exists():
        raise HTTPException(status_code=404, detail="File not found on disk")

    resolved = full_path.resolve()
    if not str(resolved).startswith(str(ENGINEERING_ROOT.resolve())):
        raise HTTPException(status_code=403, detail="Access denied")

    system = platform.system()
    if system == "Darwin":
        subprocess.Popen(["open", str(resolved)])
    elif system == "Windows":
        os.startfile(str(resolved))
    else:
        subprocess.Popen(["xdg-open", str(resolved)])
    return {"status": "opened"}


@router.post("/documents/batch/tags/{tag_name}")
def batch_add_tag(tag_name: str, req: BatchTagRequest, db: Session = Depends(get_db)):
    tag = db.query(Tag).filter(Tag.name == tag_name).first()
    if not tag:
        return {"error": "Tag not found"}

    for file_id in req.file_ids:
        file = db.get(File, file_id)
        if not file:
            continue
        existing = db.query(FileTag).filter(FileTag.file_id == file_id, FileTag.tag_id == tag.id).first()
        if existing:
            continue
        db.add(FileTag(file_id=file_id, tag_id=tag.id, source="manual"))
        if file.scan_status != "processed":
            file.scan_status = "processed"

    tag.file_count = db.query(FileTag).filter(FileTag.tag_id == tag.id).count()
    db.commit()
    return {"status": "tagged", "count": len(req.file_ids)}


@router.delete("/documents/batch/tags/{tag_name}")
def batch_remove_tag(tag_name: str, req: BatchTagRequest, db: Session = Depends(get_db)):
    tag = db.query(Tag).filter(Tag.name == tag_name).first()
    if not tag:
        return {"error": "Tag not found"}

    db.query(FileTag).filter(
        FileTag.file_id.in_(req.file_ids),
        FileTag.tag_id == tag.id,
    ).delete(synchronize_session=False)

    tag.file_count = db.query(FileTag).filter(FileTag.tag_id == tag.id).count()
    db.commit()
    return {"status": "removed", "count": len(req.file_ids)}


@router.post("/documents/batch/mark-new")
def batch_mark_new(req: BatchTagRequest, db: Session = Depends(get_db)):
    """Mark selected files as 'new' so they will be reprocessed."""
    count = 0
    for file_id in req.file_ids:
        file = db.get(File, file_id)
        if file and file.scan_status != "new":
            file.scan_status = "new"
            count += 1
    db.commit()
    return {"status": "marked", "count": count}


@router.post("/documents/batch/mark-skipped")
def batch_mark_skipped(req: BatchTagRequest, db: Session = Depends(get_db)):
    """Manually skip selected files so they won't be processed."""
    count = 0
    for file_id in req.file_ids:
        file = db.get(File, file_id)
        if file and file.scan_status != "skipped":
            file.scan_status = "skipped"
            count += 1
    db.commit()
    return {"status": "marked", "count": count}


@router.post("/documents/{file_id}/tags/{tag_name}")
def add_tag_to_file(file_id: int, tag_name: str, db: Session = Depends(get_db)):
    file = db.get(File, file_id)
    tag = db.query(Tag).filter(Tag.name == tag_name).first()
    if not file or not tag:
        return {"error": "File or tag not found"}

    existing = db.query(FileTag).filter(FileTag.file_id == file_id, FileTag.tag_id == tag.id).first()
    if existing:
        return {"status": "already_tagged"}

    db.add(FileTag(file_id=file_id, tag_id=tag.id, source="manual"))
    tag.file_count = db.query(FileTag).filter(FileTag.tag_id == tag.id).count() + 1
    if file.scan_status != "processed":
        file.scan_status = "processed"
    db.commit()
    return {"status": "tagged"}


@router.delete("/documents/{file_id}/tags/{tag_name}")
def remove_tag_from_file(file_id: int, tag_name: str, db: Session = Depends(get_db)):
    tag = db.query(Tag).filter(Tag.name == tag_name).first()
    if not tag:
        return {"error": "Tag not found"}

    ft = db.query(FileTag).filter(FileTag.file_id == file_id, FileTag.tag_id == tag.id).first()
    if ft:
        db.delete(ft)
        tag.file_count = max(0, db.query(FileTag).filter(FileTag.tag_id == tag.id).count() - 1)
        db.commit()
    return {"status": "removed"}
