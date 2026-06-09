"""Downtime-window scheduler + AppSettings I/O.

Owns:
  - reading/writing AppSettings (single row, id=1)
  - computing whether we are currently in the daily downtime window
  - the periodic loop that drives the wiki-stop / llama-stop / llama-start /
    wiki-resume cycle
  - the "End downtime now" forced-uptime override

The wiki and llama lifecycle modules are kept dependency-free of this module
where possible; wiki_service writes directly to AppSettings (via SessionLocal)
to clear scheduled_run_active on natural completion.
"""
import asyncio
import datetime
import logging
from typing import Optional

from backend.models.database import SessionLocal
from backend.models.schemas import AppSettings
from backend.services import llama_supervisor, wiki_service

logger = logging.getLogger(__name__)

_task: Optional[asyncio.Task] = None
_force_uptime_until: Optional[datetime.datetime] = None  # "End downtime" override

# True between enter_downtime() and a successful exit_downtime(). Replaces the
# old "schedule_enabled && llama_down ⇒ restart" rule, which also restarted
# llama after a user-initiated auto-shutdown.
_pending_resume_after_downtime: bool = False

# Armed by enter_downtime() ONLY when it pauses a build that was actually
# running in this process. It is the sole authorisation for auto-resuming a
# wiki build. A persisted scheduled_run_active alone must never start a build —
# otherwise every cold app launch would kick off the (huge) ingest. This flag
# is transient (lost on restart), so a fresh launch never auto-resumes.
_wiki_pending_downtime_resume: bool = False


# ── Settings I/O ─────────────────────────────────────────────────────────

def _row_to_dict(s: AppSettings) -> dict:
    return {
        "schedule_enabled": bool(s.schedule_enabled),
        "downtime_start": s.downtime_start or "06:30",
        "downtime_end": s.downtime_end or "09:30",
        "scheduled_run_active": bool(s.scheduled_run_active),
        "scheduled_tag_names": list(s.scheduled_tag_names or []),
        "scheduled_file_ids": list(s.scheduled_file_ids or []),
        "scheduled_processed_file_ids": list(s.scheduled_processed_file_ids or []),
    }


def _ensure_row(db) -> AppSettings:
    s = db.query(AppSettings).filter(AppSettings.id == 1).first()
    if s is None:
        s = AppSettings(id=1)
        db.add(s)
        db.commit()
        db.refresh(s)
    return s


def get_settings() -> dict:
    db = SessionLocal()
    try:
        return _row_to_dict(_ensure_row(db))
    finally:
        db.close()


def update_settings(**kwargs) -> dict:
    """Update one or more setting fields. Unknown keys are ignored."""
    allowed = {
        "schedule_enabled", "downtime_start", "downtime_end",
        "scheduled_run_active",
        "scheduled_tag_names", "scheduled_file_ids",
        "scheduled_processed_file_ids",
    }
    db = SessionLocal()
    try:
        s = _ensure_row(db)
        for k, v in kwargs.items():
            if k in allowed:
                setattr(s, k, v)
        db.commit()
        db.refresh(s)
        return _row_to_dict(s)
    finally:
        db.close()


def clear_scheduled_run():
    """Called when the wiki build finishes naturally or is manually cancelled."""
    update_settings(
        scheduled_run_active=False,
        scheduled_tag_names=[],
        scheduled_file_ids=[],
        scheduled_processed_file_ids=[],
    )


# ── Time window math ─────────────────────────────────────────────────────

def _parse_hhmm(s: str) -> int:
    h, m = s.split(":")
    return int(h) * 60 + int(m)


def _now_minutes() -> int:
    now = datetime.datetime.now()
    return now.hour * 60 + now.minute


def is_in_downtime(settings: Optional[dict] = None) -> bool:
    s = settings or get_settings()
    if not s["schedule_enabled"]:
        return False
    if _force_uptime_until and datetime.datetime.now() < _force_uptime_until:
        return False
    try:
        start = _parse_hhmm(s["downtime_start"])
        end = _parse_hhmm(s["downtime_end"])
    except (ValueError, AttributeError):
        return False
    if start == end:
        return False
    now = _now_minutes()
    if start < end:
        return start <= now < end
    # Wraps past midnight (e.g. 22:00 → 06:00)
    return now >= start or now < end


def next_boundary_iso() -> Optional[str]:
    """ISO timestamp of the next state change (window start or end), or None."""
    s = get_settings()
    if not s["schedule_enabled"]:
        return None
    in_dt = is_in_downtime(s)
    target_hhmm = s["downtime_end"] if in_dt else s["downtime_start"]
    if _force_uptime_until and datetime.datetime.now() < _force_uptime_until:
        # During forced uptime, "next" is when the override expires (= end-of-window)
        return _force_uptime_until.isoformat()
    h, m = (int(x) for x in target_hhmm.split(":"))
    now = datetime.datetime.now()
    target = now.replace(hour=h, minute=m, second=0, microsecond=0)
    if target <= now:
        target += datetime.timedelta(days=1)
    return target.isoformat()


def end_downtime_now():
    """Forced-uptime override active until the current window's end (then expires)."""
    global _force_uptime_until
    s = get_settings()
    if not s["schedule_enabled"]:
        return
    h, m = (int(x) for x in s["downtime_end"].split(":"))
    now = datetime.datetime.now()
    end_dt = now.replace(hour=h, minute=m, second=0, microsecond=0)
    if end_dt <= now:
        end_dt += datetime.timedelta(days=1)
    _force_uptime_until = end_dt
    logger.info("End-downtime override active until %s", _force_uptime_until.isoformat())


def force_uptime_until() -> Optional[str]:
    """For diagnostics — current override deadline or None."""
    if _force_uptime_until and datetime.datetime.now() < _force_uptime_until:
        return _force_uptime_until.isoformat()
    return None


# ── Transitions ─────────────────────────────────────────────────────────

async def _wait_for_phase(target: str, timeout: float = 300.0) -> bool:
    loop = asyncio.get_event_loop()
    deadline = loop.time() + timeout
    while loop.time() < deadline:
        phase = wiki_service.get_ingest_status()["phase"]
        if phase == target:
            return True
        if phase in ("idle", "done"):
            return True
        await asyncio.sleep(2.0)
    return False


async def enter_downtime():
    global _pending_resume_after_downtime, _wiki_pending_downtime_resume
    logger.info("Scheduler: entering downtime window")
    _pending_resume_after_downtime = True
    if wiki_service.get_ingest_status()["is_running"]:
        # A build was actively running when downtime hit — arm its resume for
        # when the window ends. This is the only path that authorises an
        # auto-resume; a cold launch (flag defaults False) never starts a build.
        _wiki_pending_downtime_resume = True
        logger.info("Scheduler: stopping wiki ingest gracefully")
        await wiki_service.stop_ingest()
        reached = await _wait_for_phase("stopped", timeout=600.0)
        logger.info("Scheduler: wiki drain complete (reached_stopped=%s, phase=%s)",
                    reached, wiki_service.get_ingest_status()["phase"])
    logger.info("Scheduler: stopping llama-servers")
    await llama_supervisor.stop_all()


async def _ensure_llama_up_if_needed(s: dict, chat_up: bool) -> bool:
    """Start llama if the scheduler should bring it back up after a downtime
    or to service a queued scheduled run. Returns True iff llama is up after."""
    global _pending_resume_after_downtime
    if chat_up:
        return True
    if llama_supervisor.is_user_paused():
        logger.info("Scheduler: llama stays down (user-paused)")
        return False
    if not (_pending_resume_after_downtime or s["scheduled_run_active"]):
        logger.info("Scheduler: llama stays down (no pending resume, no scheduled run)")
        return False
    logger.info(
        "Scheduler: starting llama-servers (pending_resume=%s scheduled_run=%s)",
        _pending_resume_after_downtime, s["scheduled_run_active"],
    )
    ok = await llama_supervisor.start_all(wait=True)
    if not ok:
        logger.error("Scheduler: llama-servers did not start; will retry next tick")
        return False
    _pending_resume_after_downtime = False
    return True


async def _resume_scheduled_wiki_if_needed(s: dict, chat_up: bool, wiki_running: bool):
    """Resume a build that THIS process paused for downtime — never auto-start
    one off a merely-persisted scheduled_run_active. A cold app launch must not
    kick off a wiki build; the user starts/resumes builds explicitly. Idempotent
    — if the wiki is still draining or llama isn't up yet, retry next tick."""
    global _wiki_pending_downtime_resume
    if not _wiki_pending_downtime_resume:
        return
    if not s["scheduled_run_active"]:
        # The run was cancelled/finished while paused — nothing to resume.
        _wiki_pending_downtime_resume = False
        return
    if not chat_up or wiki_running:
        logger.info(
            "Scheduler: downtime build pending resume but not yet "
            "(chat_up=%s wiki_running=%s)", chat_up, wiki_running,
        )
        return
    tag_names = s["scheduled_tag_names"]
    file_ids = s["scheduled_file_ids"]
    logger.info(
        "Scheduler: auto-resuming wiki ingest paused for downtime (tags=%s file_ids=%d)",
        tag_names, len(file_ids),
    )
    _wiki_pending_downtime_resume = False
    asyncio.create_task(
        wiki_service.ingest_sources(tag_names=tag_names, file_ids=file_ids)
    )


async def exit_downtime():
    """Legacy entry-point kept for the /system/end-downtime path: drives
    one full reconcile cycle (start llama + maybe resume wiki)."""
    s = get_settings()
    chat_up = await llama_supervisor.is_running("chat")
    chat_up = await _ensure_llama_up_if_needed(s, chat_up)
    wiki_running = wiki_service.get_ingest_status()["is_running"]
    await _resume_scheduled_wiki_if_needed(s, chat_up, wiki_running)


# ── Periodic loop ───────────────────────────────────────────────────────

async def _tick():
    """Reconcile state with desired state (idempotent — safe to run repeatedly)."""
    global _force_uptime_until
    if _force_uptime_until and datetime.datetime.now() >= _force_uptime_until:
        _force_uptime_until = None

    s = get_settings()
    in_dt = is_in_downtime(s)
    chat_up, embed_up = await asyncio.gather(
        llama_supervisor.is_running("chat"),
        llama_supervisor.is_running("embed"),
    )
    wiki_running = wiki_service.get_ingest_status()["is_running"]

    logger.info(
        "Scheduler tick: in_dt=%s chat_up=%s embed_up=%s wiki_running=%s "
        "scheduled_run_active=%s pending_resume=%s",
        in_dt, chat_up, embed_up, wiki_running,
        s["scheduled_run_active"], _pending_resume_after_downtime,
    )

    if in_dt:
        # Should be paused. Stop wiki + llama if anything is still up. Check BOTH
        # servers — not just chat — so an embed-only survivor (e.g. an orphan from
        # a uvicorn --reload) can't sit idle through downtime eating memory.
        if chat_up or embed_up or wiki_running:
            await enter_downtime()
        return

    # Outside the downtime window. Two independent concerns, both reconciled
    # every tick so we self-heal if a previous attempt didn't fully take.
    chat_up = await _ensure_llama_up_if_needed(s, chat_up)
    await _resume_scheduled_wiki_if_needed(s, chat_up, wiki_running)


async def _scheduler_loop():
    logger.info("Scheduler started")
    while True:
        try:
            await _tick()
        except asyncio.CancelledError:
            raise
        except Exception:
            logger.exception("Scheduler tick error")
        await asyncio.sleep(30.0)


def start_scheduler():
    global _task
    if _task and not _task.done():
        return
    _task = asyncio.create_task(_scheduler_loop())


async def stop_scheduler():
    global _task
    if _task and not _task.done():
        _task.cancel()
        try:
            await _task
        except asyncio.CancelledError:
            pass
    _task = None
