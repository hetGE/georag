"""GeoRAG standalone server entry point (used by PyInstaller EXE)."""
import sys
import os


def main():
    # When running as a PyInstaller EXE, set CWD to the EXE's directory
    if getattr(sys, "frozen", False):
        os.chdir(os.path.dirname(sys.executable))

    # Create data directories
    for sub in ("data", "data/chroma", "data/cache", "data/logs"):
        os.makedirs(sub, exist_ok=True)

    print("=" * 35)
    print("  GeoRAG - Geotechnical RAG")
    print("=" * 35)
    print()

    # Check llama-server connectivity (chat on :8001, embeddings on :8002)
    import urllib.request

    def _probe(url: str) -> bool:
        try:
            urllib.request.urlopen(url, timeout=2)
            return True
        except Exception:
            return False

    chat_ok = _probe("http://127.0.0.1:8001/v1/models")
    emb_ok = _probe("http://127.0.0.1:8002/v1/models")

    if chat_ok and emb_ok:
        print("llama-server (chat :8001, embeddings :8002): Connected")
    else:
        if not chat_ok:
            print("WARNING: chat llama-server not detected at http://127.0.0.1:8001")
        if not emb_ok:
            print("WARNING: embedding llama-server not detected at http://127.0.0.1:8002")
        print("         Start the servers as described in the README ('Step 1: Set Up llama-server').")

    print()
    print("Starting GeoRAG server...")
    print("  URL: http://localhost:3000")
    print()

    import uvicorn
    from backend.app import app

    uvicorn.run(app, host="0.0.0.0", port=3000)


if __name__ == "__main__":
    main()
