"""Start/stop/status of document processing pipeline."""
import asyncio
from fastapi import APIRouter, Depends
from sqlalchemy.orm import Session

from backend.models.database import get_db
from backend.models.pydantic_models import ProcessingRequest, ProcessingStatus
from backend.services.document_processor import DocumentProcessor

router = APIRouter(tags=["processing"])

# Global processor instance
_processor = DocumentProcessor()


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
async def processing_status():
    return {
        "is_running": _processor.is_running,
        "total_files": _processor.total_files,
        "processed_files": _processor.processed_files,
        "failed_files": _processor.failed_files,
        "current_file": _processor.current_file,
        "errors": _processor.errors[-20:],  # Last 20 errors
    }


@router.post("/processing/scan")
async def scan_files():
    """Trigger file system scan."""
    from backend.services.scanner import scan_engineering_directory
    count = scan_engineering_directory()
    return {"status": "complete", "files_found": count}
