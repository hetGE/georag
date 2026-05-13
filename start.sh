#!/usr/bin/env bash
set -e

# Model file paths (edit these to point to your local GGUF files)
CHAT_MODEL="/Users/bora/Desktop/LLMs/lmstudio-community/Qwen3.5-9B-GGUF/Qwen3.5-9B-Q4_K_M.gguf"
CHAT_MMPROJ="/Users/bora/Desktop/LLMs/lmstudio-community/Qwen3.5-9B-GGUF/mmproj-Qwen3.5-9B-BF16.gguf"
EMBED_MODEL="/Users/bora/Desktop/LLMs/second-state/Nomic-embed-text-v1.5-Embedding-GGUF/nomic-embed-text-v1.5-Q8_0.gguf"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
VENV_DIR="$SCRIPT_DIR/venv"
DATA_DIR="$SCRIPT_DIR/data"
LOG_DIR="$SCRIPT_DIR/data/logs"

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

# Verify llama-server binary
if ! command -v llama-server &> /dev/null; then
    echo "ERROR: llama-server not found in PATH. Install llama.cpp first."
    exit 1
fi

# Verify model files exist
for f in "$CHAT_MODEL" "$CHAT_MMPROJ" "$EMBED_MODEL"; do
    if [ ! -f "$f" ]; then
        echo "ERROR: Model file not found: $f"
        exit 1
    fi
done

CHAT_PID=""
EMB_PID=""

cleanup() {
    [ -n "$CHAT_PID" ] && kill "$CHAT_PID" 2>/dev/null || true
    [ -n "$EMB_PID" ] && kill "$EMB_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

wait_for_server() {
    local url="$1"
    local name="$2"
    local tries=120
    while [ $tries -gt 0 ]; do
        if curl -s --connect-timeout 1 "$url" > /dev/null 2>&1; then
            echo "  $name ready."
            return 0
        fi
        sleep 1
        tries=$((tries - 1))
    done
    echo "ERROR: $name did not become ready in time."
    return 1
}

echo ""

# Start chat llama-server on :8001 if not already running
if curl -s --connect-timeout 2 http://127.0.0.1:8001/v1/models > /dev/null 2>&1; then
    echo "Chat llama-server already running on :8001"
else
    echo "Starting chat llama-server on :8001..."
    llama-server \
        --model "$CHAT_MODEL" \
        --mmproj "$CHAT_MMPROJ" \
        --port 8001 \
        --alias qwen3.5-9B \
        -c 131072 \
        -n 32768 \
        --no-context-shift \
        --temp 0.6 \
        --top-p 0.95 \
        --top-k 20 \
        --repeat-penalty 1.00 \
        --presence-penalty 0.00 \
        --fit on \
        -fa on \
        -ctk q8_0 \
        -ctv q8_0 \
        --chat-template-kwargs '{"preserve_thinking": true}' \
        > "$LOG_DIR/llama-chat.log" 2>&1 &
    CHAT_PID=$!
    echo "  PID $CHAT_PID, log: $LOG_DIR/llama-chat.log"
fi

# Start embedding llama-server on :8002 if not already running
if curl -s --connect-timeout 2 http://127.0.0.1:8002/v1/models > /dev/null 2>&1; then
    echo "Embedding llama-server already running on :8002"
else
    echo "Starting embedding llama-server on :8002..."
    llama-server \
        --model "$EMBED_MODEL" \
        --port 8002 \
        --alias nomic-embed-text-v1.5 \
        --embeddings \
        --pooling mean \
        -c 8192 \
        -b 8192 \
        -ub 8192 \
        --rope-scaling yarn \
        --rope-freq-scale 0.75 \
        -fa on \
        -ngl 99 \
        > "$LOG_DIR/llama-embed.log" 2>&1 &
    EMB_PID=$!
    echo "  PID $EMB_PID, log: $LOG_DIR/llama-embed.log"
fi

# Wait for both servers to be ready
wait_for_server "http://127.0.0.1:8001/v1/models" "chat llama-server"
wait_for_server "http://127.0.0.1:8002/v1/models" "embedding llama-server"

echo ""
echo "Starting GeoRAG server..."
echo "  URL: http://localhost:3000"
echo ""

cd "$SCRIPT_DIR"
"$VENV_DIR/bin/uvicorn" backend.app:app --host 0.0.0.0 --port 3000 --reload