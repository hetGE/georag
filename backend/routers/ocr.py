"""Start/stop/status of OCR processing for failed PDF files."""
import asyncio
from fastapi import APIRouter

from backend.services.ocr_processor import OCRProcessor

router = APIRouter(tags=["ocr"])

# Global OCR processor instance
_ocr_processor = OCRProcessor()


@router.post("/ocr/start")
async def start_ocr():
    """Start OCR on all failed PDF files."""
    from backend.routers.processing import _processor
    from backend.routers.explore import _explorer

    if _ocr_processor.is_running:
        return {"error": "OCR already in progress"}
    if _processor.is_running:
        return {"error": "Cannot OCR while processing is running"}
    if _explorer.is_running:
        return {"error": "Cannot OCR while tag exploration is running"}

    asyncio.create_task(_ocr_processor.run())
    return {"status": "started"}


@router.post("/ocr/stop")
async def stop_ocr():
    """Stop OCR after current file."""
    _ocr_processor.stop()
    return {"status": "stopping"}


@router.get("/ocr/status")
async def ocr_status():
    """Get current OCR processing status."""
    return {
        "is_running": _ocr_processor.is_running,
        "total_files": _ocr_processor.total_files,
        "processed_files": _ocr_processor.processed_files,
        "ocr_success": _ocr_processor.ocr_success,
        "ocr_failed": _ocr_processor.ocr_failed,
        "current_file": _ocr_processor.current_file,
        "errors": _ocr_processor.errors[-20:],
    }
