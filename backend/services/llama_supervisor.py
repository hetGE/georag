"""Manage the model-server subprocesses for chat (:8001) and embedding (:8002).

The embedding server is always llama-server. The chat server is llama-server
or mlx_lm.server, depending on the chat model selected (config.CHAT_MODELS).

The FastAPI app owns these processes so the scheduler can stop/start them
during a downtime window. Idempotent: start_all() is a no-op if a server is
already responding on its port (so it survives uvicorn --reload).
"""
import asyncio
import json
import logging
import os
import platform
import re
import shutil
import signal
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable, Optional

import httpx

from backend.config import (
    CHAT_BASE_URL,
    CHAT_MODELS,
    DEFAULT_CHAT_MODEL,
    EMBEDDING_BASE_URL,
    LLAMA_EMBED_LAUNCH,
    LOG_DIR,
    DATA_DIR,
    MLX_LM_SERVER,
    chat_launch,
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


# The chat model in use and the context window its server is launched with.
# The context follows the Retrieval Depth level in use (see ensure_chat_ctx).
# Both are persisted so a restart comes back up the same way. The llama-chat.*
# file names are historical: they belong to whichever chat backend is active.
_CHAT_MODEL_FILE = DATA_DIR / "chat-model"
_CHAT_CTX_FILE = DATA_DIR / "llama-chat.ctx"


def chat_model_unavailable(key: str) -> Optional[str]:
    """Why a chat model cannot be served on this machine, or None if it can."""
    model = CHAT_MODELS[key]
    if model["backend"] == "mlx":
        if not (sys.platform == "darwin" and platform.machine() == "arm64"):
            return "MLX models need an Apple Silicon Mac."
        if not os.path.isfile(os.path.join(model["path"], "config.json")):
            return f"Model folder not found: {model['path']}"
        if not os.path.isfile(MLX_LM_SERVER):
            return (f"The MLX runtime is not installed (no {MLX_LM_SERVER}). "
                    "See 'Optional: the MLX chat model' in the README.")
        return None
    missing = [p for p in (model["path"], model["mmproj"]) if not os.path.isfile(p)]
    if missing:
        return "Model file not found: " + ", ".join(missing)
    if shutil.which("llama-server") is None:
        return "llama-server is not installed."
    return None


def _load_chat_model() -> str:
    try:
        key = _CHAT_MODEL_FILE.read_text().strip()
    except OSError:
        return DEFAULT_CHAT_MODEL
    if key not in CHAT_MODELS or chat_model_unavailable(key):
        return DEFAULT_CHAT_MODEL
    return key


_chat_model_key: str = _load_chat_model()


def active_chat_model_key() -> str:
    return _chat_model_key


def active_chat_model() -> dict:
    return CHAT_MODELS[_chat_model_key]


def _base_ctx() -> int:
    return active_chat_model()["tiers"]["base"]["ctx"]


def _load_chat_ctx() -> int:
    try:
        ctx = int(_CHAT_CTX_FILE.read_text().strip())
    except (OSError, ValueError):
        return _base_ctx()
    tier_sizes = [tier["ctx"] for tier in active_chat_model()["tiers"].values()]
    return ctx if ctx in tier_sizes else _base_ctx()


_chat_ctx_target: int = _load_chat_ctx()


_servers: dict[str, _ServerSpec] = {
    "chat": _ServerSpec(
        name="chat",
        launch=chat_launch(_chat_model_key, _chat_ctx_target),
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


# ── Chat context window (Retrieval Depth) ────────────────────────────────

# Serialises ensure_chat_ctx so a depth change and a chat request can't both
# restart the chat server.
_ctx_lock = asyncio.Lock()

_fit_cache: Optional[tuple[int, Optional[dict]]] = None  # (chat server pid, report)

_FIT_PROJECTED_RE = re.compile(
    rb"projected to use (\d+) MiB of device memory vs\. (\d+) MiB of free device memory")
_FIT_SHORT_RE = re.compile(
    rb"cannot meet free memory target of \d+ MiB, need to reduce device memory by (\d+) MiB")
_FIT_OFFLOADED_RE = re.compile(rb"offloaded (\d+)/(\d+) layers to GPU")


def chat_fit_report() -> Optional[dict]:
    """What llama.cpp's --fit pass said about GPU memory on the most recent
    chat-server launch, read from its log.

    With --fit on and an explicit -c, llama.cpp never refuses a context size
    that is too big for the GPU: it keeps the context and moves model layers to
    the CPU instead, which runs but is far slower. It does log that it could not
    meet its memory target, and that line is the reliable signal that the
    context did not fit. Returns None when the log has no fit report (no GPU
    device, or a server this app did not launch). llama backend only.
    """
    global _fit_cache
    spec = _servers["chat"]
    pid = spec.process.pid if spec.process else _read_pid_file(spec.pid_file)
    if pid and _fit_cache and _fit_cache[0] == pid:
        return _fit_cache[1]
    try:
        log = spec.log_file.read_bytes()
    except OSError:
        return None
    report = None
    start = log.rfind(b"common_params_fit_impl: projected to use")
    section = log[start:] if start >= 0 else b""
    projected = _FIT_PROJECTED_RE.search(section)
    if projected:
        short = _FIT_SHORT_RE.search(section)
        offloaded = _FIT_OFFLOADED_RE.search(section)
        report = {
            "fits": short is None,
            "projected_mib": int(projected.group(1)),
            "free_mib": int(projected.group(2)),
            "short_mib": int(short.group(1)) if short else 0,
            "gpu_layers": int(offloaded.group(1)) if offloaded else None,
            "total_layers": int(offloaded.group(2)) if offloaded else None,
        }
    if pid:
        _fit_cache = (pid, report)  # the log only grows; one read per process
    return report


def _process_footprint_mib(spec: _ServerSpec) -> Optional[int]:
    """Physical memory footprint of a server process, GPU allocations included
    (macOS `footprint`; unlike RSS it counts Metal memory)."""
    pid = spec.process.pid if spec.process else _read_pid_file(spec.pid_file)
    if not pid or not _is_pid_alive(pid):
        pid = _pid_listening_on(spec.base_url)
    if not pid:
        return None
    try:
        out = subprocess.run(["footprint", str(pid)], capture_output=True, text=True, timeout=15)
    except (OSError, subprocess.SubprocessError):
        return None
    m = re.search(r"phys_footprint: ([\d.]+) (KB|MB|GB)", out.stdout)
    if not m:
        return None
    return round(float(m.group(1)) * {"KB": 1 / 1024, "MB": 1, "GB": 1024}[m.group(2)])


_BUFFER_SIZE_RE = re.compile(rb"buffer size\s*=\s*([\d.]+) MiB")
_PROJECTOR_SIZE_RE = re.compile(rb"load_hparams: model size:\s*([\d.]+) MiB")
_PROMPT_CACHE_RE = re.compile(rb"cache state: \d+ prompts, ([\d.]+) MiB")


def loaded_memory_mib(name: str) -> Optional[int]:
    """Memory held by a server, summed from what llama.cpp logged when it last
    loaded: every model / KV-cache / compute buffer (GPU and CPU), the vision
    projector's weights, and the current size of the prompt cache. The process
    RSS is no substitute: GPU-mapped weights do not show up in it. Returns None
    when the log has no load section.

    mlx_lm.server logs no such figures and allocates as it goes, so for the MLX
    chat model the process's physical footprint is measured instead."""
    if name == "chat" and active_chat_model()["backend"] == "mlx":
        return _process_footprint_mib(_servers[name])
    try:
        log = _servers[name].log_file.read_bytes()
    except OSError:
        return None
    start = log.rfind(b"main: loading model")
    if start < 0:
        return None
    section = log[start:]
    buffers = [float(m) for m in _BUFFER_SIZE_RE.findall(section)]
    if not buffers:
        return None
    total = sum(buffers) + sum(float(m) for m in _PROJECTOR_SIZE_RE.findall(section))
    prompt_cache = _PROMPT_CACHE_RE.findall(section)
    if prompt_cache:
        total += float(prompt_cache[-1])
    return round(total)


async def chat_ctx() -> Optional[int]:
    """Context window of the running chat server, or None if it is not up (or
    still loading its model). mlx_lm.server has no fixed window; for it this is
    the context budget of the Retrieval Depth level last applied."""
    try:
        async with httpx.AsyncClient(timeout=4.0) as client:
            if active_chat_model()["backend"] == "mlx":
                r = await client.get(f"{CHAT_BASE_URL}/health")
                return _chat_ctx_target if r.status_code == 200 else None
            r = await client.get(f"{CHAT_BASE_URL}/props")
            if r.status_code != 200:
                return None
            return int(r.json()["default_generation_settings"]["n_ctx"])
    except Exception:
        return None


def set_chat_ctx_target(ctx: int) -> None:
    """Context window to use the next time the chat server is launched."""
    global _chat_ctx_target
    _chat_ctx_target = ctx
    _servers["chat"].launch = chat_launch(_chat_model_key, ctx)
    try:
        _CHAT_CTX_FILE.write_text(str(ctx))
    except OSError as e:
        logger.warning("Could not persist chat context size: %s", e)


def _set_chat_model(key: str, ctx: int) -> None:
    global _chat_model_key
    _chat_model_key = key
    try:
        _CHAT_MODEL_FILE.write_text(key)
    except OSError as e:
        logger.warning("Could not persist chat model choice: %s", e)
    set_chat_ctx_target(ctx)


# ── MLX memory check ─────────────────────────────────────────────────────

_mlx_device_info_cache: Optional[dict] = None


def _mlx_device_info() -> Optional[dict]:
    """mlx.core.device_info() of this Mac, asked of the MLX runtime's own
    interpreter (mlx is not installed in the app's environment)."""
    global _mlx_device_info_cache
    if _mlx_device_info_cache is None:
        python = Path(MLX_LM_SERVER).parent / "python"
        try:
            out = subprocess.run(
                [str(python), "-c", "import json, mlx.core as mx; print(json.dumps(mx.device_info()))"],
                capture_output=True, text=True, timeout=30,
            )
            _mlx_device_info_cache = json.loads(out.stdout.strip().splitlines()[-1])
        except (OSError, subprocess.SubprocessError, ValueError, IndexError):
            return None
    return _mlx_device_info_cache


def mlx_memory_estimate(model: dict, ctx: int) -> int:
    """Bytes the MLX chat server needs with `ctx` tokens in its KV cache: the
    weights, the working memory of a forward pass, and the cache itself."""
    weights = sum(f.stat().st_size for f in Path(model["path"]).glob("*.safetensors"))
    return weights + model["overhead_bytes"] + model["kv_bytes_per_token"] * ctx


def _mlx_memory_check(model: dict, ctx: int) -> tuple[bool, Optional[str]]:
    """Whether the MLX model with a `ctx`-token context budget fits in the
    memory macOS lets the GPU use. mlx_lm.server allocates on demand, so there
    is no load-time verdict to read as with llama.cpp: a context that is too big
    would only show up mid-answer, as swapping or an out-of-memory failure. The
    estimate is therefore checked up front."""
    info = _mlx_device_info()
    if not info:
        return True, None   # cannot tell; let MLX decide
    need = mlx_memory_estimate(model, ctx)
    available = info["max_recommended_working_set_size"]
    embed_mib = loaded_memory_mib("embed")
    if embed_mib:
        available -= embed_mib * 1024 * 1024
    if need <= available:
        return True, None
    gb = 1024 ** 3
    return False, (f"{model['label']} needs about {need / gb:.1f} GB with a {ctx:,}-token "
                   f"context, and this Mac ({info['memory_size'] / gb:.0f} GB) makes "
                   f"{available / gb:.1f} GB available to the GPU.")


async def _wait_loaded(spec: _ServerSpec, timeout: float = 180.0) -> bool:
    """Wait until the server has finished loading its model. Stricter than
    _wait_ready: /v1/models already answers while the model is still loading,
    /health only returns 200 once it can serve requests."""
    loop = asyncio.get_event_loop()
    deadline = loop.time() + timeout
    async with httpx.AsyncClient(timeout=2.0) as client:
        while loop.time() < deadline:
            try:
                if (await client.get(f"{spec.base_url}/health")).status_code == 200:
                    return True
            except Exception:
                pass
            if spec.process is not None and spec.process.poll() is not None:
                logger.error("llama-server '%s' exited with code %s while loading; see %s",
                             spec.name, spec.process.returncode, spec.log_file)
                return False
            await asyncio.sleep(0.5)
    logger.error("llama-server '%s' did not finish loading within %.0fs", spec.name, timeout)
    return False


async def _restart_chat(ctx: int) -> bool:
    """Relaunch the chat server with a different context window. Returns True
    once the model is loaded."""
    global _starting
    spec = _servers["chat"]
    async with _lifecycle_lock:
        _starting = True
        try:
            logger.info("Restarting the chat server (%s) with a %d-token context",
                        active_chat_model()["label"], ctx)
            await _stop_one(spec)
            await _reap_orphan_on_port(spec)
            spec.launch = chat_launch(_chat_model_key, ctx)
            await _start_one(spec)
            return await _wait_loaded(spec)
        finally:
            _starting = False


async def restart_chat_server() -> bool:
    """Relaunch the chat server as it is configured now (same model, same
    context). For a server that is up but no longer serving requests."""
    ok = await _restart_chat(_chat_ctx_target)
    if not ok:
        logger.error("Chat server did not come back after a restart; see %s",
                     _servers["chat"].log_file)
    return ok


def _chat_ctx_acceptable(ctx: Optional[int], required: int, fit: Optional[dict]) -> bool:
    if ctx is None or ctx < required:
        return False
    # The base context is the documented minimum and is allowed to run with
    # layers on the CPU (small GPUs, CPU-only machines). Anything larger has to
    # fit in GPU memory entirely.
    return ctx <= _base_ctx() or not (fit and fit["fits"] is False)


def _fit_failure_message(ctx: int, fit: Optional[dict]) -> str:
    if fit and fit["fits"] is False:
        available = fit["projected_mib"] - fit["short_mib"]
        return (f"llama.cpp could not fit a {ctx:,}-token context in GPU memory: it needs "
                f"about {fit['projected_mib'] / 1024:.1f} GB and only {available / 1024:.1f} GB "
                "is available.")
    return (f"llama.cpp could not load the chat model with a {ctx:,}-token context "
            f"(most likely out of memory; see {_servers['chat'].log_file}).")


async def ensure_chat_ctx(
    required: int,
    busy_reason: Optional[Callable[[], Optional[str]]] = None,
) -> tuple[bool, Optional[str]]:
    """Make sure the running chat server has a context window of at least
    `required` tokens that fits in memory, restarting it with a larger one if
    needed. Returns (True, None), or (False, reason) after putting the server
    back on a context size that works.

    `busy_reason` returns a description of any LLM work in progress; the server
    is never restarted underneath it.
    """
    spec = _servers["chat"]
    async with _ctx_lock:
        if active_chat_model()["backend"] == "mlx":
            # Nothing to reload: the context is a budget, checked by estimate.
            ok, message = _mlx_memory_check(active_chat_model(), required)
            if ok:
                set_chat_ctx_target(required)
            return ok, message

        ctx = await chat_ctx()
        if ctx is None:
            if not await _wait_loaded(spec, timeout=120.0):
                return False, "The chat LLM server is not running."
            ctx = await chat_ctx()
        fit = chat_fit_report()
        if _chat_ctx_acceptable(ctx, required, fit):
            set_chat_ctx_target(required)
            return True, None

        reason = busy_reason() if busy_reason else None
        if reason:
            if ctx is not None and ctx >= required and required <= _base_ctx():
                # Big enough, just not fully on the GPU: usable, and not worth
                # interrupting the running work to shrink it.
                return True, None
            return False, (f"This needs the chat model reloaded with a {required:,}-token "
                           f"context, which cannot be done while {reason}. Try again when "
                           "it has finished.")

        previous = ctx
        loaded = await _restart_chat(required)
        fit = chat_fit_report() if loaded else None
        if loaded and _chat_ctx_acceptable(required, required, fit):
            set_chat_ctx_target(required)
            logger.info("Chat server now running with a %d-token context", required)
            return True, None

        message = _fit_failure_message(required, fit)
        logger.warning("Chat context of %d tokens refused: %s", required, message)
        if required <= _base_ctx():
            return False, message
        fallback = previous if previous and previous < required else _base_ctx()
        set_chat_ctx_target(fallback)
        if not await _restart_chat(fallback):
            message += " The chat LLM server also failed to restart with its previous settings."
        return False, message


def chat_ctx_fit(ctx: int) -> tuple[Optional[bool], Optional[str]]:
    """Whether the active model can run a context of `ctx` tokens here, when
    that can be told without trying: (True, None), (False, reason), or
    (None, None) for llama.cpp, whose verdict only exists once it has loaded."""
    model = active_chat_model()
    if model["backend"] != "mlx":
        return None, None
    return _mlx_memory_check(model, ctx)


async def switch_chat_model(
    key: str,
    busy_reason: Optional[Callable[[], Optional[str]]] = None,
) -> tuple[bool, Optional[str]]:
    """Serve a different chat model: stop the chat server and start the new
    model's backend at its base context. Returns (True, None), or (False, reason)
    with the previous model running again if the new one cannot be loaded."""
    async with _ctx_lock:
        if key == _chat_model_key:
            return True, None
        reason = chat_model_unavailable(key)
        if reason:
            return False, reason
        model = CHAT_MODELS[key]
        base = model["tiers"]["base"]["ctx"]
        if model["backend"] == "mlx":
            ok, message = _mlx_memory_check(model, base)
            if not ok:
                return False, message
        busy = busy_reason() if busy_reason else None
        if busy:
            return False, f"The chat model cannot be changed while {busy}. Try again when it has finished."

        previous_key, previous_ctx = _chat_model_key, _chat_ctx_target
        running = await _port_alive(CHAT_BASE_URL)
        _set_chat_model(key, base)
        if not running:
            return True, None   # models are paused: the choice applies at the next start

        logger.info("Switching chat model: %s -> %s", previous_key, key)
        loaded = await _restart_chat(base)
        if loaded:
            return True, None
        message = (f"{model['label']} failed to load (most likely out of memory; "
                   f"see {_servers['chat'].log_file}).")
        logger.warning("Chat model switch to %s failed; restoring %s", key, previous_key)
        _set_chat_model(previous_key, previous_ctx)
        if not await _restart_chat(previous_ctx):
            message += " The previous chat model also failed to restart."
        return False, message


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
