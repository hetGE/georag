#!/usr/bin/env bash
# Kill GeoRAG processes: the app server (uvicorn :3000), the llama-server
# subprocesses (chat :8001, embed :8002), and the overnight watchdog.
#
# Matches by both command line AND listening port, because the server is often
# launched with a relative path (e.g. `venv/bin/uvicorn ...`) whose command line
# does NOT contain the project path. No sudo: these are all owned by you.

APP_PORT=3000
LLAMA_PORTS="8001 8002"

# --- collect PIDs (deduped) -------------------------------------------------
pids=""

add() { for p in $1; do [ -n "$p" ] && pids="$pids $p"; done; }

# By command line.
add "$(pgrep -f 'uvicorn backend.app:app')"
add "$(pgrep -f 'llama-server')"
add "$(pgrep -f 'overnight-watch')"
# Catch-all: anything running out of the project dir.
add "$(ps aux | grep '_0RAG/' | grep -v grep | awk '{print $2}')"
# By listening port (catches relative-path launches).
for port in $APP_PORT $LLAMA_PORTS; do
    add "$(lsof -ti tcp:$port -sTCP:LISTEN 2>/dev/null)"
done

# Dedupe, drop this script's own pid.
self=$$
pids=$(printf '%s\n' $pids | sort -un | grep -v "^${self}$")

if [ -z "$pids" ]; then
    echo "No GeoRAG processes found."
    exit 0
fi

echo "Found GeoRAG processes:"
for p in $pids; do
    ps -o pid=,command= -p "$p" 2>/dev/null | cut -c1-120
done
echo ""

# --- terminate: SIGTERM first, SIGKILL the survivors ------------------------
echo "Sending SIGTERM..."
for p in $pids; do kill "$p" 2>/dev/null; done

sleep 2

survivors=""
for p in $pids; do
    if kill -0 "$p" 2>/dev/null; then survivors="$survivors $p"; fi
done

if [ -n "$survivors" ]; then
    echo "Force-killing survivors:$survivors"
    for p in $survivors; do kill -9 "$p" 2>/dev/null; done
fi

echo "Done."
