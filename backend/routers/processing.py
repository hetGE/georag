"""Start/stop/status of document processing pipeline."""
import asyncio
from pathlib import Path
from fastapi import APIRouter, Depends
from sqlalchemy.orm import Session
from sqlalchemy import func

from backend.models.database import get_db
from backend.models.schemas import File
from backend.models.pydantic_models import ProcessingRequest, ProcessingStatus
from backend.services.document_processor import DocumentProcessor

router = APIRouter(tags=["processing"])

# Global processor instance
_processor = DocumentProcessor()

_ONBOARDING_DISMISSED_PATH = Path("data/onboarding_dismissed")


@router.post("/processing/start")
async def start_processing(request: ProcessingRequest, db: Session = Depends(get_db)):
    if _processor.is_running:
        return {"error": "Processing already in progress"}

    asyncio.create_task(_processor.run(
        tag_names=request.tag_names,
        file_ids=request.file_ids,
        reprocess=request.reprocess,
    ))
    return {"status": "started"}


@router.post("/processing/stop")
async def stop_processing():
    _processor.stop()
    return {"status": "stopping"}


@router.get("/processing/status")
async def processing_status(db: Session = Depends(get_db)):
    status_counts = dict(
        db.query(File.scan_status, func.count(File.id))
        .group_by(File.scan_status)
        .all()
    )
    total_files = sum(status_counts.values())
    processed_files = status_counts.get("processed", 0)
    failed_files = status_counts.get("failed", 0)
    skipped_files = status_counts.get("skipped", 0)

    return {
        "is_running": _processor.is_running,
        "total_files": total_files,
        "processed_files": processed_files,
        "failed_files": failed_files,
        "skipped_files": skipped_files,
        "current_file": _processor.current_file,
        "errors": _processor.errors[-20:],  # Last 20 errors
    }


@router.post("/processing/scan")
async def scan_files():
    """Trigger file system scan."""
    from backend.services.scanner import scan_engineering_directory
    result = scan_engineering_directory()
    return {"status": "complete", **result}


@router.get("/processing/onboarding-status")
def onboarding_status(db: Session = Depends(get_db)):
    """Return current onboarding state for the wizard."""
    total_files = db.query(File).count()
    status_counts = dict(
        db.query(File.scan_status, func.count(File.id))
        .group_by(File.scan_status)
        .all()
    )
    new_files = status_counts.get("new", 0) + status_counts.get("failed", 0)
    processed_files = status_counts.get("processed", 0)
    failed_files = status_counts.get("failed", 0)
    skipped_files = status_counts.get("skipped", 0)

    # Extension breakdown for display
    by_extension = dict(
        db.query(File.extension, func.count(File.id))
        .group_by(File.extension)
        .order_by(func.count(File.id).desc())
        .limit(10)
        .all()
    )

    if total_files == 0:
        phase = "not_started"
    elif _processor.is_running and _processor._stop_flag:
        phase = "stopping"
    elif _processor.is_running:
        phase = "processing"
    elif new_files > 0:
        phase = "scanned"
    else:
        phase = "complete"

    dismissed = _ONBOARDING_DISMISSED_PATH.exists()

    return {
        "phase": phase,
        "total_files": total_files,
        "new_files": new_files,
        "processed_files": processed_files,
        "failed_files": failed_files,
        "skipped_files": skipped_files,
        "is_processing": _processor.is_running,
        "dismissed": dismissed,
        "by_extension": by_extension,
        "current_file": _processor.current_file,
    }


@router.post("/processing/onboarding-dismiss")
def dismiss_onboarding():
    """Dismiss the onboarding wizard."""
    _ONBOARDING_DISMISSED_PATH.parent.mkdir(parents=True, exist_ok=True)
    _ONBOARDING_DISMISSED_PATH.touch()
    return {"status": "dismissed"}


@router.delete("/processing/onboarding-dismiss")
def reset_onboarding():
    """Reset the onboarding wizard (show it again)."""
    if _ONBOARDING_DISMISSED_PATH.exists():
        _ONBOARDING_DISMISSED_PATH.unlink()
    return {"status": "reset"}
