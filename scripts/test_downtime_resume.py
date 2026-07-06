"""Regression test for downtime-pause auto-resume across a process restart.

Bug (pre-fix): auto-resume was gated on a TRANSIENT in-process flag, so if the
app restarted at any point during the downtime window, the window would end but
the paused wiki build never resumed. The fix persists the authorisation as
AppSettings.scheduled_paused_for_downtime, so a fresh process still resumes.

This drives the REAL scheduler._tick() at the window's end, in a simulated fresh
process (module transients at their launch defaults). Services + settings I/O are
faked, so it touches no DB / llama / Chroma. Run:

    PYTHONPATH=. venv/bin/python scripts/test_downtime_resume.py

Exits non-zero if any case fails.
"""
import asyncio
import sys
import types

# ── Fake the two service modules BEFORE importing the real scheduler ──────────
rec = {"llama_started": False, "wiki_resumed": False, "llama_up": False}

llama = types.ModuleType("backend.services.llama_supervisor")
async def _is_running(name):      return rec["llama_up"]
async def _start_all(wait=False, timeout=None):
    rec["llama_started"] = True; rec["llama_up"] = True; return True
async def _stop_all():            rec["llama_up"] = False
def _is_user_paused():            return False
llama.is_running, llama.start_all = _is_running, _start_all
llama.stop_all, llama.is_user_paused = _stop_all, _is_user_paused

wiki = types.ModuleType("backend.services.wiki_service")
def _get_ingest_status():         return {"is_running": False, "phase": "stopped"}
async def _stop_ingest():         pass
async def _ingest_sources(tag_names=None, file_ids=None):  rec["wiki_resumed"] = True
wiki.get_ingest_status, wiki.stop_ingest, wiki.ingest_sources = (
    _get_ingest_status, _stop_ingest, _ingest_sources)

sys.modules["backend.services.llama_supervisor"] = llama
sys.modules["backend.services.wiki_service"] = wiki

from backend.services import scheduler  # noqa: E402  binds to the fakes above

# ── Isolate settings I/O to an in-memory row ──────────────────────────────────
SETTINGS = {}
def _patched_update(**kw):
    for k, v in kw.items():
        SETTINGS[k] = v
    return dict(SETTINGS)
scheduler.get_settings = lambda: dict(SETTINGS)
scheduler.update_settings = _patched_update
scheduler.is_in_downtime = lambda s=None: SETTINGS.get("_in_downtime", False)


async def drive_tick(*, active, paused_for_dt, in_downtime=False):
    """Simulate a fresh process (reset transients) with the given persisted row,
    then run one real scheduler tick at/after the window boundary."""
    SETTINGS.clear()
    SETTINGS.update(
        schedule_enabled=True, downtime_start="06:30", downtime_end="08:30",
        scheduled_run_active=active, scheduled_paused_for_downtime=paused_for_dt,
        scheduled_tag_names=[], scheduled_file_ids=[1, 2, 3],
        scheduled_processed_file_ids=[], _in_downtime=in_downtime,
    )
    scheduler._pending_resume_after_downtime = False   # launch defaults
    scheduler._force_uptime_until = None
    rec.update(llama_started=False, wiki_resumed=False, llama_up=False)
    await scheduler._tick()
    await asyncio.sleep(0.05)  # let create_task(ingest_sources) run


def check(name, cond):
    print(f"  {'PASS' if cond else 'FAIL'}  {name}")
    return cond


async def main():
    ok = True

    # 1. THE FIX: clean downtime-pause + restart during window; window has ended.
    #    Fresh process, but the persisted marker survives -> must resume.
    await drive_tick(active=True, paused_for_dt=True)
    ok &= check("restart during window -> build resumes", rec["wiki_resumed"])
    ok &= check("restart during window -> marker cleared on resume",
                SETTINGS["scheduled_paused_for_downtime"] is False)

    # 2. REGRESSION GUARD: build killed mid-flight OUTSIDE downtime leaves the
    #    marker False. A cold launch must NOT auto-start the (huge) ingest.
    await drive_tick(active=True, paused_for_dt=False)
    ok &= check("mid-build kill -> does NOT auto-start", not rec["wiki_resumed"])

    # 3. Run cancelled/stopped while paused (active cleared, marker stale True):
    #    no resume, and the stale marker is cleared.
    await drive_tick(active=False, paused_for_dt=True)
    ok &= check("cancelled-while-paused -> no resume", not rec["wiki_resumed"])
    ok &= check("cancelled-while-paused -> stale marker cleared",
                SETTINGS["scheduled_paused_for_downtime"] is False)

    # 4. No active build at all -> nothing happens.
    await drive_tick(active=False, paused_for_dt=False)
    ok &= check("no active build -> no resume", not rec["wiki_resumed"])

    print("\nRESULT:", "ALL PASS" if ok else "FAILURES ABOVE")
    sys.exit(0 if ok else 1)

asyncio.run(main())
