"""Document list, search, filter, pagination."""
import subprocess
from fastapi import APIRouter, Depends, Query, HTTPException
from sqlalchemy.orm import Session, joinedload
from sqlalchemy import func

from backend.config import ENGINEERING_ROOT
from backend.models.database import get_db
from backend.models.schemas import File, Tag, FileTag

router = APIRouter(tags=["documents"])


@router.get("/documents")
def list_documents(
    search: str = Query("", description="Search filename or path"),
    extension: str = Query("", description="Filter by extension"),
    tag: str = Query("", description="Filter by tag name"),
    status: str = Query("", description="Filter by scan_status"),
    page: int = Query(1, ge=1),
    per_page: int = Query(50, ge=1, le=200),
    db: Session = Depends(get_db),
):
    query = db.query(File).options(joinedload(File.tags).joinedload(FileTag.tag))

    if search:
        query = query.filter(
            (File.filename.ilike(f"%{search}%")) | (File.relative_path.ilike(f"%{search}%"))
        )
    if extension:
        query = query.filter(File.extension == extension.lower().lstrip("."))
    if tag == "__none__":
        query = query.filter(~File.tags.any())
    elif tag:
        query = query.join(File.tags).join(FileTag.tag).filter(Tag.name == tag)
    if status:
        query = query.filter(File.scan_status == status)

    total = query.distinct().count()
    files = (
        query.distinct()
        .order_by(File.relative_path)
        .offset((page - 1) * per_page)
        .limit(per_page)
        .all()
    )

    return {
        "total": total,
        "page": page,
        "per_page": per_page,
        "pages": (total + per_page - 1) // per_page if per_page else 1,
        "files": [
            {
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
            for f in files
        ],
    }


@router.get("/documents/stats")
def document_stats(db: Session = Depends(get_db)):
    total = db.query(File).count()
    by_status = dict(
        db.query(File.scan_status, func.count(File.id))
        .group_by(File.scan_status)
        .all()
    )
    by_ext = dict(
        db.query(File.extension, func.count(File.id))
        .group_by(File.extension)
        .order_by(func.count(File.id).desc())
        .limit(20)
        .all()
    )
    return {"total": total, "by_status": by_status, "by_extension": by_ext}


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

    subprocess.Popen(["open", str(resolved)])
    return {"status": "opened"}


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
