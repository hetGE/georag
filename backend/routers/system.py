"""System-wide settings, status, and downtime overrides."""
import asyncio
import logging
from typing import Optional

from fastapi import APIRouter, HTTPException
from pydantic import BaseModel, Field

from backend.services import llama_supervisor, scheduler, wiki_service
from backend.routers import processing as _processing  # _processor
from backend.routers import ocr as _ocr  # _ocr_processor
from backend.routers import explore as _explore  # _explorer
from backend.routers import chat as _chat  # _chat_streaming flag

logger = logging.getLogger(__name__)
router = APIRouter(tags=["system"])


def _busy_reason() -> Optional[str]:
    """Returns a human-readable reason if any LLM-using work is active,
    or None if it's safe to pause llama-servers."""
    if wiki_service.get_ingest_status().get("is_running"):
        return "wiki build is running"
    if wiki_service.get_lint_fix_status().get("is_running"):
        return "wiki health check is running"
    if getattr(_processing._processor, "is_running", False):
        return "library processing is running"
    if getattr(_ocr._ocr_processor, "is_running", False):
        return "OCR is running"
    if getattr(_explore._explorer, "is_running", False):
        return "tag exploration is running"
    if getattr(_chat, "_chat_streaming", False):
        return "a chat reply is streaming"
    return None


def _compute_state(in_downtime: bool, llama_state: str) -> tuple[str, str]:
    """Returns (label, severity). Severity ∈ {ok, busy, warn, down}.

    `llama_state` ∈ {running, starting, paused, down} from llama_supervisor.
    """
    if in_downtime:
        return ("Scheduled pause", "down")
    if llama_state == "starting":
        return ("Starting LLMs", "busy")

    # Wiki has explicit stopping phases — surface those before the running ones.
    if wiki_service.get_ingest_status().get("phase") == "stopping":
        return ("Wiki stopping", "busy")
    if wiki_service.get_lint_fix_status().get("phase") == "stopping":
        return ("Wiki health check stopping", "busy")

    # Active operations
    if wiki_service.get_ingest_status().get("is_running"):
        return ("Wiki building", "busy")
    if wiki_service.get_lint_fix_status().get("is_running"):
        return ("Wiki health check", "busy")
    if getattr(_processing._processor, "is_running", False):
        return ("Library processing", "busy")
    if getattr(_ocr._ocr_processor, "is_running", False):
        return ("OCR running", "busy")
    if getattr(_explore._explorer, "is_running", False):
        return ("Tag exploration", "busy")
    if getattr(_chat, "_chat_streaming", False):
        return ("Chat responding", "busy")

    if llama_state in ("paused", "down"):
        # We don't surface "down" as a separate state — the user-visible
        # difference is always "click the pill to start", so collapse both
        # into "LLMs paused".
        return ("LLMs paused", "ok")
    return ("LLMs idle", "ok")


class SettingsPayload(BaseModel):
    schedule_enabled: Optional[bool] = None
    downtime_start: Optional[str] = Field(default=None, description='"HH:MM"')
    downtime_end: Optional[str] = Field(default=None, description='"HH:MM"')
    auto_shutdown_on_manual_pause: Optional[bool] = None


def _validate_hhmm(value: str) -> bool:
    try:
        h, m = value.split(":")
        return 0 <= int(h) <= 23 and 0 <= int(m) <= 59
    except (ValueError, AttributeError):
        return False


@router.get("/system/settings")
async def get_settings():
    return scheduler.get_settings()


@router.put("/system/settings")
async def put_settings(payload: SettingsPayload):
    updates = payload.model_dump(exclude_none=True)
    for key in ("downtime_start", "downtime_end"):
        if key in updates and not _validate_hhmm(updates[key]):
            return {"ok": False, "error": f"Invalid {key} (expected HH:MM)"}
    return {"ok": True, "settings": scheduler.update_settings(**updates)}


@router.get("/system/status")
async def get_status():
    s = scheduler.get_settings()
    in_dt = scheduler.is_in_downtime(s)
    srv = await llama_supervisor.status()
    llama_state = llama_supervisor.get_lifecycle_state(srv["chat_up"], srv["embed_up"])
    label, severity = _compute_state(in_dt, llama_state)
    return {
        "schedule_enabled": s["schedule_enabled"],
        "downtime_start": s["downtime_start"],
        "downtime_end": s["downtime_end"],
        "auto_shutdown_on_manual_pause": s["auto_shutdown_on_manual_pause"],
        "in_downtime": in_dt,
        "next_boundary": scheduler.next_boundary_iso(),
        "force_uptime_until": scheduler.force_uptime_until(),
        "llama_chat_up": srv["chat_up"],
        "llama_embed_up": srv["embed_up"],
        "llama_state": llama_state,
        "scheduled_run_active": s["scheduled_run_active"],
        "wiki_phase": wiki_service.get_ingest_status()["phase"],
        "wiki_is_running": wiki_service.get_ingest_status()["is_running"],
        "state_label": label,
        "state_severity": severity,
    }


@router.post("/system/end-downtime")
async def end_downtime():
    """User-triggered: end the current downtime period immediately.
    Saved schedule stays — tomorrow's window still applies."""
    scheduler.end_downtime_now()
    # Trigger an immediate tick so llama comes up + wiki resumes without waiting
    # for the next 30s tick.
    asyncio.create_task(scheduler._tick())
    return {"ok": True}


@router.post("/system/llama/pause")
async def pause_llama():
    """Manually pause both llama-servers to free memory. Refuses if any
    LLM-using work is currently active."""
    reason = _busy_reason()
    if reason:
        raise HTTPException(status_code=409, detail=f"Cannot pause: {reason}.")
    await llama_supervisor.stop_all()
    return {"ok": True, "state": "paused"}


@router.post("/system/llama/start")
async def start_llama():
    """Manually start both llama-servers. Returns immediately; the pill will
    poll its way through the "Starting LLMs" → "LLMs idle" transition."""
    # wait=False launches a background _wait_ready that clears _starting.
    await llama_supervisor.start_all(wait=False)
    return {"ok": True, "state": "starting"}
