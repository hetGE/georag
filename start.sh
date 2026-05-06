#!/usr/bin/env bash
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
VENV_DIR="$SCRIPT_DIR/venv"
DATA_DIR="$SCRIPT_DIR/data"

echo "==============================="
echo "  GeoRAG - Geotechnical RAG"
echo "==============================="
echo ""

# Check Python 3.11
if command -v python3.11 &> /dev/null; then
    PYTHON=python3.11
elif command -v python3 &> /dev/null; then
    PYTHON=python3
    PY_VERSION=$($PYTHON --version 2>&1 | awk '{print $2}')
    echo "Using Python $PY_VERSION (3.11 recommended)"
else
    echo "ERROR: Python 3 not found. Please install Python 3.11+"
    exit 1
fi

# Create venv if needed
if [ ! -d "$VENV_DIR" ]; then
    echo "Creating virtual environment..."
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

# Check llama-server (chat on :8001, embeddings on :8002)
echo ""
CHAT_OK=0
EMB_OK=0
curl -s --connect-timeout 2 http://127.0.0.1:8001/v1/models > /dev/null 2>&1 && CHAT_OK=1
curl -s --connect-timeout 2 http://127.0.0.1:8002/v1/models > /dev/null 2>&1 && EMB_OK=1

if [ "$CHAT_OK" = 1 ] && [ "$EMB_OK" = 1 ]; then
    echo "llama-server (chat :8001, embeddings :8002): Connected"
else
    [ "$CHAT_OK" = 0 ] && echo "WARNING: chat llama-server not detected at http://127.0.0.1:8001"
    [ "$EMB_OK" = 0 ] && echo "WARNING: embedding llama-server not detected at http://127.0.0.1:8002"
    echo "  Start the servers as described in the README ('Step 1: Set Up llama-server')."
fi

echo ""
echo "Starting GeoRAG server..."
echo "  URL: http://localhost:3000"
echo ""

cd "$SCRIPT_DIR"
exec "$VENV_DIR/bin/uvicorn" backend.app:app --host 0.0.0.0 --port 3000 --reload
