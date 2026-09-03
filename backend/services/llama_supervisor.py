"""Manage llama-server subprocesses for chat (:8001) and embedding (:8002).

The FastAPI app owns these processes so the scheduler can stop/start them
during a downtime window. Idempotent: start_all() is a no-op if a server is
already responding on its port (so it survives uvicorn --reload).
"""
import asyncio
import logging
import os
import signal
import subprocess
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional

import httpx

from backend.config import (
    CHAT_BASE_URL,
    EMBEDDING_BASE_URL,
    LLAMA_CHAT_LAUNCH,
    LLAMA_EMBED_LAUNCH,
    LOG_DIR,
    DATA_DIR,
)

logger = logging.getLogger(__name__)


@dataclass
class _ServerSpec:
    name: str
    launch: list[str]
    base_url: str
    pid_file: Path
    log_file: Path
    process: Optional[subprocess.Popen] = field(default=None, repr=False)


_servers: dict[str, _ServerSpec] = {
    "chat": _ServerSpec(
        name="chat",
        launch=LLAMA_CHAT_LAUNCH,
        base_url=CHAT_BASE_URL,
        pid_file=DATA_DIR / "llama-chat.pid",
        log_file=LOG_DIR / "llama-chat.log",
    ),
    "embed": _ServerSpec(
        name="embed",
        launch=LLAMA_EMBED_LAUNCH,
        base_url=EMBEDDING_BASE_URL,
        pid_file=DATA_DIR / "llama-embed.pid",
        log_file=LOG_DIR / "llama-embed.log",
    ),
}

# Lifecycle intent flags. Transient (lost on uvicorn reload); the pill falls
# back to "Servers down" after a restart if llama hasn't been re-launched, which
# is the safer reading.
_starting: bool = False
_intentionally_paused: bool = False
# Latched by a manual pause (POST /system/llama/pause). Unlike _intentionally_paused
# (also set by the scheduler's downtime stop), this means "the user wants llama
# down" — the scheduler honours it so a manual pause survives the next reconcile
# tick instead of being undone while a scheduled run is still queued.
_user_paused: bool = False

# Serialise start_all/stop_all so a click + scheduler tick don't race.
_lifecycle_lock = asyncio.Lock()


def _is_pid_alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
        return True
    except (OSError, ProcessLookupError):
        return False


def _read_pid_file(pid_file: Path) -> Optional[int]:
    if not pid_file.exists():
        return None
    try:
        return int(pid_file.read_text().strip())
    except (ValueError, OSError):
        return None


async def _port_alive(base_url: str, timeout: float = 1.5) -> bool:
    try:
        async with httpx.AsyncClient(timeout=timeout) as client:
            r = await client.get(f"{base_url}/v1/models")
            return r.status_code == 200
    except Exception:
        return False


async def _port_alive_tolerant(base_url: str, attempts: int = 3, timeout: float = 4.0) -> bool:
    """Liveness check that tolerates a momentarily busy server. A single 1.5s
    probe frequently times out while llama is mid-generation; treating that as
    "down" made the scheduler relaunch an already-running server (and risk a
    duplicate on the same port). Retry a few times with a longer timeout before
    concluding it is really down."""
    for attempt in range(attempts):
        if await _port_alive(base_url, timeout=timeout):
            return True
        if attempt < attempts - 1:
            await asyncio.sleep(0.5)
    return False


def _pid_listening_on(base_url: str) -> Optional[int]:
    """Look up the PID of the process listening on the port in base_url. macOS/Linux only."""
    try:
        from urllib.parse import urlparse
        port = urlparse(base_url).port
        if not port:
            return None
        out = subprocess.run(
            ["lsof", "-t", "-i", f":{port}", "-sTCP:LISTEN"],
            capture_output=True, text=True, timeout=3,
        )
        if out.returncode != 0:
            return None
        first = out.stdout.strip().splitlines()
        return int(first[0]) if first else None
    except (subprocess.SubprocessError, ValueError, OSError):
        return None


async def is_running(name: str) -> bool:
    # Tolerant probe: this feeds the scheduler's restart decision, and a single
    # 1.5s probe routinely times out on a busy (but alive) server.
    return await _port_alive_tolerant(_servers[name].base_url)


def is_user_paused() -> bool:
    """True after a manual pause until the next start_all(). The scheduler
    consults this so a user pause isn't reversed on the next tick. Transient —
    lost on uvicorn reload, same as the other lifecycle flags."""
    return _user_paused


async def status() -> dict:
    chat_up, embed_up = await asyncio.gather(
        _port_alive(CHAT_BASE_URL),
        _port_alive(EMBEDDING_BASE_URL),
    )
    return {"chat_up": chat_up, "embed_up": embed_up}


async def _start_one(spec: _ServerSpec) -> None:
    if await _port_alive_tolerant(spec.base_url):
        # Already up. Record a usable PID so a later stop_all() can kill it.
        if spec.process is None:
            existing = _read_pid_file(spec.pid_file)
            if existing and _is_pid_alive(existing):
                logger.info("llama-server '%s' already up at %s (pid %d, adopted from file)",
                            spec.name, spec.base_url, existing)
            else:
                # Pid file missing or stale — discover the listening pid and persist.
                discovered = _pid_listening_on(spec.base_url)
                if discovered and _is_pid_alive(discovered):
                    spec.pid_file.write_text(str(discovered))
                    logger.info("llama-server '%s' already up at %s (pid %d, discovered via lsof)",
                                spec.name, spec.base_url, discovered)
                else:
                    logger.info("llama-server '%s' already up at %s (pid unknown)",
                                spec.name, spec.base_url)
        return

    # Port isn't answering, but if we still hold a live pid the server is most
    # likely busy/wedged rather than gone. Spawning now would put a second
    # llama-server on the same port — leave the existing one and let the next
    # tick re-check instead of stacking duplicates that waste memory.
    existing = spec.process.pid if spec.process else _read_pid_file(spec.pid_file)
    if existing and _is_pid_alive(existing):
        logger.warning(
            "llama-server '%s' pid %d alive but %s not responding; "
            "skipping launch to avoid a duplicate",
            spec.name, existing, spec.base_url,
        )
        return

    missing = [a for a in spec.launch[1:] if a.endswith(".gguf") and not os.path.exists(a)]
    if missing:
        logger.error("llama-server '%s' not launched: model file(s) missing: %s",
                     spec.name, ", ".join(missing))
        return

    spec.log_file.parent.mkdir(parents=True, exist_ok=True)
    log_fh = open(spec.log_file, "ab", buffering=0)
    logger.info("Launching llama-server '%s' → %s", spec.name, spec.log_file)
    spec.process = subprocess.Popen(
        spec.launch,
        stdout=log_fh,
        stderr=subprocess.STDOUT,
        start_new_session=True,  # detach so uvicorn --reload doesn't reap it
    )
    spec.pid_file.write_text(str(spec.process.pid))


async def _wait_ready(spec: _ServerSpec, timeout: float = 180.0) -> bool:
    loop = asyncio.get_event_loop()
    deadline = loop.time() + timeout
    while loop.time() < deadline:
        if await _port_alive(spec.base_url):
            logger.info("llama-server '%s' ready at %s", spec.name, spec.base_url)
            return True
        # A child we launched that has already exited (bad model path, port in
        # use, ...) will never answer — fail fast instead of burning the timeout.
        if spec.process is not None and spec.process.poll() is not None:
            logger.error(
                "llama-server '%s' exited with code %s before becoming ready; see %s",
                spec.name, spec.process.returncode, spec.log_file,
            )
            return False
        await asyncio.sleep(1.0)
    logger.error("llama-server '%s' did not become ready within %.0fs",
                 spec.name, timeout)
    return False


async def start_all(wait: bool = True, timeout: float = 180.0) -> bool:
    """Start both servers (no-op if already up). Returns True iff both are ready
    (or True immediately when wait=False)."""
    global _starting, _intentionally_paused, _user_paused
    async with _lifecycle_lock:
        _intentionally_paused = False
        _user_paused = False
        _starting = True
        try:
            await asyncio.gather(*(_start_one(s) for s in _servers.values()))
            if not wait:
                # Fire-and-forget: clear _starting in a background watcher so the
                # pill flips to "LLMs idle" when the port comes alive.
                asyncio.create_task(_clear_starting_when_ready(timeout))
                return True
            results = await asyncio.gather(
                *(_wait_ready(s, timeout) for s in _servers.values())
            )
            return all(results)
        finally:
            if wait:
                _starting = False


async def _clear_starting_when_ready(timeout: float) -> None:
    """Used by start_all(wait=False) — clear _starting once both ports respond."""
    global _starting
    try:
        results = await asyncio.gather(
            *(_wait_ready(s, timeout) for s in _servers.values())
        )
        if not all(results):
            logger.warning("ensure_running: not all llama-servers became ready")
    finally:
        _starting = False


async def ensure_running(timeout: float = 180.0) -> bool:
    """Idempotent: return immediately if both servers are up; otherwise start
    them and wait. Used by request handlers that need llama right now."""
    chat_up, embed_up = await asyncio.gather(
        _port_alive(CHAT_BASE_URL), _port_alive(EMBEDDING_BASE_URL)
    )
    if chat_up and embed_up:
        return True
    return await start_all(wait=True, timeout=timeout)


def get_lifecycle_state(chat_up: bool, embed_up: bool) -> str:
    """Return one of: running, starting, paused, down. Caller passes the live
    port-alive readings to avoid a second probe round-trip."""
    if chat_up and embed_up:
        return "running"
    if _starting:
        return "starting"
    if _intentionally_paused:
        return "paused"
    return "down"


def _server_exited(spec: _ServerSpec, pid: int) -> bool:
    """Whether the server process is gone. For a child we launched, Popen.poll()
    detects exit AND reaps it — os.kill(pid, 0) keeps reporting a not-yet-reaped
    zombie as alive, which made every shutdown burn the full timeout (llama
    exits on SIGTERM in ~0.1s but then lingered as a zombie). For an adopted pid
    (no handle, e.g. after a uvicorn reload) the OS reaps it, so the kill-probe
    is accurate there."""
    if spec.process is not None:
        return spec.process.poll() is not None
    return not _is_pid_alive(pid)


async def _stop_one(spec: _ServerSpec) -> None:
    pid = spec.process.pid if spec.process else _read_pid_file(spec.pid_file)
    if not pid or _server_exited(spec, pid):
        spec.process = None
        try:
            spec.pid_file.unlink()
        except OSError:
            pass
        return

    logger.info("Stopping llama-server '%s' (pid %d)", spec.name, pid)
    try:
        os.kill(pid, signal.SIGTERM)
    except OSError as e:
        logger.warning("SIGTERM to %d failed: %s", pid, e)

    # llama-server handles SIGTERM and exits within ~0.1s, so _server_exited
    # (via poll) normally breaks on the first check. The cap only matters for a
    # genuinely wedged server.
    for _ in range(40):  # safety cap ~10 s
        if _server_exited(spec, pid):
            break
        await asyncio.sleep(0.25)
    else:
        logger.warning("llama-server '%s' did not exit on SIGTERM, sending SIGKILL", spec.name)
        try:
            os.kill(pid, signal.SIGKILL)
        except OSError:
            pass
        if spec.process is not None:
            try:
                spec.process.wait(timeout=5)  # reap so it doesn't linger as a zombie
            except Exception:
                pass

    spec.process = None
    try:
        spec.pid_file.unlink()
    except OSError:
        pass


async def _reap_orphan_on_port(spec: _ServerSpec) -> None:
    """Kill any llama-server still listening on this server's port that _stop_one
    didn't account for. A server launched with start_new_session=True survives a
    uvicorn --reload as a detached orphan; if the new worker never adopted it
    (port was momentarily unresponsive at adoption time), _stop_one only ever
    knows the tracked pid and the orphan keeps the model weights resident through
    the whole downtime window."""
    pid = _pid_listening_on(spec.base_url)
    if not pid or not _is_pid_alive(pid):
        return
    logger.warning("llama-server '%s' orphan still on %s (pid %d); killing",
                   spec.name, spec.base_url, pid)
    try:
        os.kill(pid, signal.SIGTERM)
    except OSError as e:
        logger.warning("SIGTERM to orphan %d failed: %s", pid, e)
        return
    for _ in range(20):  # safety cap ~5 s
        if not _is_pid_alive(pid):
            return
        await asyncio.sleep(0.25)
    logger.warning("orphan llama-server '%s' pid %d ignored SIGTERM; SIGKILL", spec.name, pid)
    try:
        os.kill(pid, signal.SIGKILL)
    except OSError:
        pass


async def stop_all(user_initiated: bool = False) -> None:
    """Stop both servers. When user_initiated, also latch _user_paused so the
    scheduler leaves llama down until it is explicitly started again."""
    global _intentionally_paused, _user_paused
    async with _lifecycle_lock:
        if user_initiated:
            # Latch the intent before the kill, not after: a stubborn llama can
            # take a couple of minutes to die (SIGTERM → SIGKILL), and a
            # scheduler tick landing mid-shutdown must already see "user-paused"
            # so it won't queue a restart while a scheduled run is still active.
            _user_paused = True
        await asyncio.gather(*(_stop_one(s) for s in _servers.values()))
        # Belt-and-braces: _stop_one only kills the pid we tracked. Sweep the
        # ports too so a detached orphan (untracked after a --reload) can't keep
        # llama resident through downtime.
        await asyncio.gather(*(_reap_orphan_on_port(s) for s in _servers.values()))
        _intentionally_paused = True
