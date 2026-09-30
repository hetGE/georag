#!/usr/bin/env bash
set -e

# Model file paths are defined in backend/config.py; the FastAPI app launches
# llama-server itself via backend.services.llama_supervisor so the scheduler
# can stop/start them during downtime windows.
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
VENV_DIR="$SCRIPT_DIR/venv"

# Optional per-machine overrides (untracked): e.g. GEORAG_LLM_DIR, LLAMA_EMBED_MODEL.
if [ -f "$SCRIPT_DIR/.env" ]; then
    set -a; . "$SCRIPT_DIR/.env"; set +a
fi

# Override any path with its own env var (absolute); otherwise they resolve
# under GEORAG_LLM_DIR (default: ~/Apps/LLMs).
LLM_DIR="${GEORAG_LLM_DIR:-$HOME/Apps/LLMs}"
CHAT_MODEL="${LLAMA_CHAT_MODEL:-$LLM_DIR/lmstudio-community/Qwen3.5-9B-GGUF/Qwen3.5-9B-Q4_K_M.gguf}"
CHAT_MMPROJ="${LLAMA_CHAT_MMPROJ:-$LLM_DIR/lmstudio-community/Qwen3.5-9B-GGUF/mmproj-Qwen3.5-9B-BF16.gguf}"
EMBED_MODEL="${LLAMA_EMBED_MODEL:-$LLM_DIR/second-state/Nomic-embed-text-v1.5-Embedding-GGUF/nomic-embed-text-v1.5-Q8_0.gguf}"
# Export so backend/config.py (which launches llama-server) sees the same paths
export GEORAG_LLM_DIR="$LLM_DIR" LLAMA_CHAT_MODEL="$CHAT_MODEL" LLAMA_CHAT_MMPROJ="$CHAT_MMPROJ" LLAMA_EMBED_MODEL="$EMBED_MODEL"
DATA_DIR="$SCRIPT_DIR/data"

echo "==============================="
echo "  GeoRAG - Geotechnical RAG"
echo "==============================="
echo ""

# Check Python 3.13
if command -v python3.13 &> /dev/null; then
    PYTHON=python3.13
    PY_VERSION=$($PYTHON --version 2>&1 | awk '{print $2}')
    echo "Using Python $PY_VERSION"
elif command -v python3.12 &> /dev/null; then
    PYTHON=python3.12
    PY_VERSION=$($PYTHON --version 2>&1 | awk '{print $2}')
    echo "Using Python $PY_VERSION (3.13 recommended)"
elif command -v python3.11 &> /dev/null; then
    PYTHON=python3.11
    PY_VERSION=$($PYTHON --version 2>&1 | awk '{print $2}')
    echo "Using Python $PY_VERSION (3.13 recommended)"
elif command -v python3 &> /dev/null; then
    PYTHON=python3
    PY_VERSION=$($PYTHON --version 2>&1 | awk '{print $2}')
    echo "WARNING: Using Python $PY_VERSION (3.13 recommended)"
else
    echo "ERROR: Python 3 not found. Please install Python 3.13+"
    exit 1
fi

# Create venv if needed
if [ ! -d "$VENV_DIR" ]; then
    echo "Creating virtual environment with $PYTHON..."
    $PYTHON -m venv "$VENV_DIR"
    echo "Installing dependencies..."
    "$VENV_DIR/bin/pip" install --upgrade pip -q
    "$VENV_DIR/bin/pip" install -r "$SCRIPT_DIR/requirements.txt" -q
    echo "Dependencies installed."
else
    echo "Virtual environment found."
fi

# Create data directories
mkdir -p "$DATA_DIR"/{chroma,cache,logs}

# Verify llama-server binary and model files (fail fast before importing the app)
if ! command -v llama-server &> /dev/null; then
    echo "ERROR: llama-server not found in PATH. Install llama.cpp first."
    exit 1
fi
for f in "$CHAT_MODEL" "$CHAT_MMPROJ" "$EMBED_MODEL"; do
    if [ ! -f "$f" ]; then
        echo "ERROR: Model file not found: $f"
        exit 1
    fi
done

echo ""
echo "Starting GeoRAG server (llama-servers launched by the app)..."
echo "  URL: http://localhost:3000"
echo ""

cd "$SCRIPT_DIR"
exec "$VENV_DIR/bin/uvicorn" backend.app:app --host 0.0.0.0 --port 3000 --reload
