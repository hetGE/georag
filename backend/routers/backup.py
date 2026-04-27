"""Backup/restore API endpoints."""
import asyncio
import logging
from pathlib import Path

from fastapi import APIRouter, UploadFile, File, HTTPException
from fastapi.responses import FileResponse

from backend.config import DATA_DIR
from backend.services.backup_service import exporter, importer

logger = logging.getLogger(__name__)
router = APIRouter(tags=["backup"])

UPLOAD_DIR = DATA_DIR / "exports"


def _check_busy():
    """Raise 409 if any background operation is running."""
    # Lazy imports to avoid circular deps
    from backend.routers.processing import _processor
    from backend.services import wiki_service

    if _processor.is_running:
        raise HTTPException(409, "Document processing is running — try again later")
    if wiki_service._ingest_running:
        raise HTTPException(409, "Wiki ingest is running — try again later")


# ── Export ────────────────────────────────────────────────────────────────────

@router.post("/backup/export")
async def start_export(include_wiki: bool = True, include_rag: bool = True):
    """Start a background backup export."""
    if exporter.is_running:
        raise HTTPException(409, "Export already running")
    if importer.is_running:
        raise HTTPException(409, "Import is running")
    _check_busy()

    asyncio.create_task(exporter.run(include_wiki=include_wiki, include_rag=include_rag))
    return {"ok": True}


@router.get("/backup/export/status")
async def export_status():
    """Poll export progress."""
    return exporter.get_status()


@router.get("/backup/export/download")
async def download_export():
    """Download the completed .georag archive."""
    if exporter.is_running:
        raise HTTPException(409, "Export still running")
    path = exporter.output_path
    if not path or not path.exists():
        raise HTTPException(404, "No export available")
    return FileResponse(
        path=str(path),
        media_type="application/zip",
        filename=path.name,
    )


@router.post("/backup/export/stop")
async def stop_export():
    """Cancel a running export."""
    exporter.stop()
    return {"ok": True}


# ── Import ────────────────────────────────────────────────────────────────────

@router.post("/backup/import/validate")
async def validate_import(file: UploadFile = File(...)):
    """Upload and validate a .georag archive without importing."""
    UPLOAD_DIR.mkdir(parents=True, exist_ok=True)
    upload_path = UPLOAD_DIR / (file.filename or "upload.georag")
    with open(upload_path, "wb") as f:
        while chunk := await file.read(8192):
            f.write(chunk)

    result = importer.validate_archive(upload_path)
    # Keep the file around for a subsequent import call
    result["upload_path"] = str(upload_path)
    return result


@router.post("/backup/import")
async def start_import(file: UploadFile = File(...)):
    """Upload a .georag archive and start importing."""
    if importer.is_running:
        raise HTTPException(409, "Import already running")
    if exporter.is_running:
        raise HTTPException(409, "Export is running")
    _check_busy()

    UPLOAD_DIR.mkdir(parents=True, exist_ok=True)
    upload_path = UPLOAD_DIR / (file.filename or "upload.georag")
    with open(upload_path, "wb") as f:
        while chunk := await file.read(8192):
            f.write(chunk)

    asyncio.create_task(importer.run(upload_path))
    return {"ok": True}


@router.get("/backup/import/status")
async def import_status():
    """Poll import progress."""
    return importer.get_status()


@router.post("/backup/import/stop")
async def stop_import():
    """Cancel a running import."""
    importer.stop()
    return {"ok": True}
