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

    # Check LM Studio connectivity
    try:
        import urllib.request

        urllib.request.urlopen("http://127.0.0.1:1234/v1/models", timeout=2)
        print("LM Studio: Connected")
    except Exception:
        print("WARNING: LM Studio not detected at http://127.0.0.1:1234")
        print("         Make sure LM Studio is running before using chat.")

    print()
    print("Starting GeoRAG server...")
    print("  URL: http://localhost:3000")
    print()

    import uvicorn
    from backend.app import app

    uvicorn.run(app, host="0.0.0.0", port=3000)


if __name__ == "__main__":
    main()
