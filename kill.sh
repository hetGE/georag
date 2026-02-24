#!/usr/bin/env bash
# Kill GeoRAG processes

PIDS=$(ps aux | grep "_0RAG/" | grep -v grep | awk '{print $2}')

if [ -z "$PIDS" ]; then
    echo "No GeoRAG processes found."
    exit 0
fi

echo "Found GeoRAG processes:"
ps aux | grep "_0RAG/" | grep -v grep
echo ""

for PID in $PIDS; do
    echo "Killing PID $PID..."
    sudo kill "$PID"
done

echo "Done."
