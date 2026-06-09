"""Full real integration test for the downtime llama-shutdown fix.

Launches the REAL llama-servers via the supervisor, simulates the uvicorn
--reload orphan leak (clears tracking so the pids are no longer known), then:
  1. shows the OLD path (_stop_one alone) leaves the orphans alive  -> the bug
  2. drives a REAL scheduler._tick() inside a downtime window       -> the fix
and asserts both ports are freed, both processes dead, and RAM reclaimed.

Isolated: monkeypatches get_settings so it never touches the prod DB and never
starts the scheduled wiki ingest.
"""
import asyncio
import datetime
import logging
import subprocess
import sys

from backend.services import llama_supervisor as sup
from backend.services import scheduler as sch

logging.basicConfig(
    level=logging.INFO, stream=sys.stdout,
    format="    %(levelname)-7s [%(name)s] %(message)s",
)
log = logging.getLogger("IT")


def listeners() -> str:
    out = subprocess.run(
        ["lsof", "-nP", "-iTCP:8001", "-iTCP:8002", "-sTCP:LISTEN"],
        capture_output=True, text=True,
    )
    return out.stdout.strip()


def rss_mb(pid) -> float:
    if not pid:
        return 0.0
    out = subprocess.run(["ps", "-o", "rss=", "-p", str(pid)],
                         capture_output=True, text=True)
    try:
        return int(out.stdout.strip()) / 1024.0
    except ValueError:
        return 0.0


def banner(msg):
    print(f"\n{'='*70}\n{msg}\n{'='*70}")


async def main() -> int:
    banner("STEP 1 — launch BOTH real llama-servers via the supervisor")
    ok = await sup.start_all(wait=True, timeout=300.0)
    st = await sup.status()
    chat_pid = sup._read_pid_file(sup._servers["chat"].pid_file)
    embed_pid = sup._read_pid_file(sup._servers["embed"].pid_file)
    print(f"  start_all ok={ok}  status={st}")
    print(f"  chat pid={chat_pid} ({rss_mb(chat_pid):.0f} MB RSS)")
    print(f"  embed pid={embed_pid} ({rss_mb(embed_pid):.0f} MB RSS)")
    print(f"  listeners:\n{listeners()}")
    assert ok and st["chat_up"] and st["embed_up"], "servers did not come up"
    baseline_mb = rss_mb(chat_pid) + rss_mb(embed_pid)

    banner("STEP 2 — simulate the --reload orphan leak (drop all tracking)")
    for s in sup._servers.values():
        s.process = None
        try:
            s.pid_file.unlink()
        except OSError:
            pass
    print("  cleared spec.process + removed pid files; real servers still listening")

    banner("STEP 3 — OLD behavior: _stop_one alone (no port sweep)")
    await asyncio.gather(*(sup._stop_one(s) for s in sup._servers.values()))
    after_old = listeners()
    chat_alive_old = sup._is_pid_alive(chat_pid)
    embed_alive_old = sup._is_pid_alive(embed_pid)
    print(f"  chat alive={chat_alive_old}  embed alive={embed_alive_old}")
    print(f"  listeners after _stop_one-only:\n{after_old or '  (none)'}")
    assert chat_alive_old and embed_alive_old and after_old, \
        "expected the OLD path to leave orphans alive (bug repro)"
    print("  ==> CONFIRMED: the old shutdown path leaves both orphans resident.")

    banner("STEP 4 — THE FIX: real scheduler._tick() inside a downtime window")
    now = datetime.datetime.now()
    start = (now - datetime.timedelta(minutes=1)).strftime("%H:%M")
    end = (now + datetime.timedelta(hours=1)).strftime("%H:%M")
    settings = {
        "schedule_enabled": True, "downtime_start": start, "downtime_end": end,
        "scheduled_run_active": False, "scheduled_tag_names": [],
        "scheduled_file_ids": [], "scheduled_processed_file_ids": [],
    }
    sch.get_settings = lambda: settings  # isolate from prod DB
    print(f"  downtime window {start}-{end} (now {now.strftime('%H:%M:%S')}) -> in_dt expected True")
    await sch._tick()
    await asyncio.sleep(1.0)
    after_new = listeners()
    chat_alive_new = sup._is_pid_alive(chat_pid)
    embed_alive_new = sup._is_pid_alive(embed_pid)
    reclaimed = baseline_mb - (rss_mb(chat_pid) + rss_mb(embed_pid))
    print(f"  chat alive={chat_alive_new}  embed alive={embed_alive_new}")
    print(f"  listeners after fix:\n{after_new or '  (none)'}")
    print(f"  RAM reclaimed ~= {reclaimed:.0f} MB")

    banner("RESULT")
    passed = (not chat_alive_new) and (not embed_alive_new) and (after_new == "")
    if passed:
        print("  PASS — scheduler drained on embed/chat probe and reaped both orphans;")
        print("         both ports free, both processes dead, memory reclaimed.")
    else:
        print("  FAIL — see state above")
    return 0 if passed else 1


if __name__ == "__main__":
    rc = 2
    try:
        rc = asyncio.run(main())
    finally:
        # Safety net: never leave a llama-server behind on a failed run.
        subprocess.run("for p in $(lsof -t -nP -iTCP:8001 -iTCP:8002 -sTCP:LISTEN 2>/dev/null); "
                       "do kill -9 $p 2>/dev/null; done", shell=True)
    sys.exit(rc)
