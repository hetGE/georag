"""System-wide settings, status, and downtime overrides."""
import asyncio
import logging
from typing import Optional

from fastapi import APIRouter, HTTPException
from pydantic import BaseModel, Field

from backend.config import (
    CHAT_MODELS, DEFAULT_CHAT_DEPTH, chat_depth_level, chat_depth_levels,
)
from backend.services import llama_supervisor, scheduler
from backend.routers import processing as _processing  # _processor
from backend.routers import ocr as _ocr  # _ocr_processor
from backend.routers import explore as _explore  # _explorer
from backend.routers import chat as _chat  # _chat_streaming flag

logger = logging.getLogger(__name__)
router = APIRouter(tags=["system"])


def _busy_reason() -> Optional[str]:
    """Returns a human-readable reason if any LLM-using work is active,
    or None if it's safe to pause llama-servers."""
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

    # Active operations
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
        "in_downtime": in_dt,
        "next_boundary": scheduler.next_boundary_iso(),
        "force_uptime_until": scheduler.force_uptime_until(),
        "llama_chat_up": srv["chat_up"],
        "llama_embed_up": srv["embed_up"],
        "llama_state": llama_state,
        "state_label": label,
        "state_severity": severity,
    }


@router.post("/system/end-downtime")
async def end_downtime():
    """User-triggered: end the current downtime period immediately.
    Saved schedule stays — tomorrow's window still applies."""
    scheduler.end_downtime_now()
    # Trigger an immediate tick so llama comes up without waiting for the next
    # 30s tick.
    asyncio.create_task(scheduler._tick())
    return {"ok": True}


class ChatDepthPayload(BaseModel):
    depth: str


class ChatModelPayload(BaseModel):
    model: str
    depth: Optional[str] = None


def _chat_models() -> list[dict]:
    models = []
    for key, model in CHAT_MODELS.items():
        reason = llama_supervisor.chat_model_unavailable(key)
        models.append({
            "key": key, "label": model["label"], "detail": model["detail"],
            "backend": model["backend"], "vision": model["vision"],
            "available": reason is None, "unavailable_reason": reason,
        })
    return models


def _chat_levels(key: str) -> list[dict]:
    """The model's Retrieval Depth levels, each marked with whether it fits on
    this machine where that is known in advance (`fits`: true, false or null)."""
    levels = []
    for level in chat_depth_levels(key):
        fits, reason = llama_supervisor.chat_ctx_fit(level["ctx"])
        levels.append({**level, "fits": fits,
                       "fit_error": _chat.depth_refusal(level, reason) if fits is False else None})
    return levels


@router.get("/system/chat-depths")
async def get_chat_depths():
    """The chat models, the one in use, its Retrieval Depth levels, and the
    context window the chat server has now."""
    key = llama_supervisor.active_chat_model_key()
    return {
        "default": DEFAULT_CHAT_DEPTH,
        "model": key,
        "models": _chat_models(),
        "levels": _chat_levels(key),
        "server_ctx": await llama_supervisor.chat_ctx(),
    }


async def _llms_running() -> bool:
    s = scheduler.get_settings()
    return not scheduler.is_in_downtime(s) and (await llama_supervisor.status())["chat_up"]


@router.post("/system/chat-depth")
async def apply_chat_depth(payload: ChatDepthPayload):
    """Prepare the chat server for a Retrieval Depth level: if the level needs a
    larger context window than the one loaded, reload the model with it and
    confirm it fits in memory. Returns {ok: false, error} and keeps the previous
    context when it does not fit."""
    key = llama_supervisor.active_chat_model_key()
    if payload.depth not in {level["key"] for level in chat_depth_levels(key)}:
        raise HTTPException(status_code=404, detail=f"Unknown depth '{payload.depth}'.")
    level = chat_depth_level(key, payload.depth)
    if not await _llms_running():
        # LLMs are paused or down: don't start them just to check. The level's
        # context is used on the next launch and verified when a chat starts.
        llama_supervisor.set_chat_ctx_target(level["ctx"])
        return {"ok": True, "checked": False}
    ok, error = await llama_supervisor.ensure_chat_ctx(level["ctx"], _busy_reason)
    if not ok:
        return {"ok": False, "error": _chat.depth_refusal(level, error)}
    return {"ok": True, "checked": True}


@router.post("/system/chat-model")
async def apply_chat_model(payload: ChatModelPayload):
    """Switch the chat model. The chat server is restarted with the new model's
    backend; if it cannot be loaded the previous model is restored and
    {ok: false, error} returned. `depth` is the Retrieval Depth level to carry
    over: when the new model cannot run it on this machine, the response names
    the level to fall back to and why."""
    if payload.model not in CHAT_MODELS:
        raise HTTPException(status_code=404, detail=f"Unknown chat model '{payload.model}'.")
    label = CHAT_MODELS[payload.model]["label"]
    ok, error = await llama_supervisor.switch_chat_model(payload.model, _busy_reason)
    if not ok:
        return {"ok": False, "error": f"Cannot switch to {label}. {error}"}

    key = llama_supervisor.active_chat_model_key()
    level = chat_depth_level(key, payload.depth)
    notice = None
    if await _llms_running():
        ok, error = await llama_supervisor.ensure_chat_ctx(level["ctx"], _busy_reason)
        if not ok:
            notice = _chat.depth_refusal(level, error)
            level = chat_depth_level(key, DEFAULT_CHAT_DEPTH)
            await llama_supervisor.ensure_chat_ctx(level["ctx"], _busy_reason)
    else:
        llama_supervisor.set_chat_ctx_target(level["ctx"])
    return {
        "ok": True,
        "model": key,
        "depth": level["key"],
        "notice": notice,
        "levels": _chat_levels(key),
    }


@router.get("/system/llama/memory")
async def llama_memory():
    """What is loaded right now and how much memory it holds (so how much a
    pause frees). llama-server sizes come from its own load logs, the MLX
    server's from its process footprint; a server that is not running counts
    as nothing."""
    srv = await llama_supervisor.status()
    model = llama_supervisor.active_chat_model()
    chat_mib = embed_mib = None
    if srv["chat_up"]:
        chat_mib = await asyncio.to_thread(llama_supervisor.loaded_memory_mib, "chat")
    if srv["embed_up"]:
        embed_mib = await asyncio.to_thread(llama_supervisor.loaded_memory_mib, "embed")
    return {
        "chat_up": srv["chat_up"],
        "embed_up": srv["embed_up"],
        "chat_model": model["label"],
        "chat_backend": model["backend"],
        "chat_ctx": await llama_supervisor.chat_ctx() if srv["chat_up"] else None,
        "chat_mib": chat_mib,
        "embed_mib": embed_mib,
        "total_mib": (chat_mib or 0) + (embed_mib or 0) if (chat_mib or embed_mib) else None,
    }


@router.post("/system/llama/pause")
async def pause_llama():
    """Manually pause both llama-servers to free memory. Refuses if any
    LLM-using work is currently active."""
    reason = _busy_reason()
    if reason:
        raise HTTPException(status_code=409, detail=f"Cannot pause: {reason}.")
    await llama_supervisor.stop_all(user_initiated=True)
    return {"ok": True, "state": "paused"}


@router.post("/system/llama/start")
async def start_llama():
    """Manually start both llama-servers. Returns immediately; the pill will
    poll its way through the "Starting LLMs" → "LLMs idle" transition."""
    # wait=False launches a background _wait_ready that clears _starting.
    await llama_supervisor.start_all(wait=False)
    return {"ok": True, "state": "starting"}
