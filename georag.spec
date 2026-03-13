# -*- mode: python ; coding: utf-8 -*-
"""PyInstaller spec for GeoRAG standalone Windows EXE.

Build with:  pyinstaller georag.spec
Output:      dist/georag/georag.exe
"""

from PyInstaller.utils.hooks import collect_all, collect_submodules

# ---------------------------------------------------------------------------
# Collect all of chromadb (native libs, data files, submodules)
# ---------------------------------------------------------------------------
chromadb_datas, chromadb_binaries, chromadb_hiddenimports = collect_all("chromadb")

# Also collect hnswlib native extension
hnswlib_datas, hnswlib_binaries, hnswlib_hiddenimports = collect_all("hnswlib")

# Collect pdfplumber (includes pdfminer data)
pdfplumber_datas, pdfplumber_binaries, pdfplumber_hiddenimports = collect_all("pdfplumber")

# ---------------------------------------------------------------------------
# Hidden imports that PyInstaller cannot trace from static analysis
# ---------------------------------------------------------------------------
hidden_imports = [
    # --- uvicorn internals (dynamically loaded) ---
    *collect_submodules("uvicorn"),
    # --- starlette ---
    *collect_submodules("starlette"),
    # --- anyio (async backend) ---
    "anyio",
    "anyio._backends",
    "anyio._backends._asyncio",
    # --- SSE streaming ---
    "sse_starlette",
    "sse_starlette.sse",
    # --- SQLAlchemy dialect ---
    "sqlalchemy.dialects.sqlite",
    "sqlalchemy.dialects.sqlite.aiosqlite",
    "aiosqlite",
    # --- httpx / httpcore ---
    *collect_submodules("httpx"),
    *collect_submodules("httpcore"),
    # --- Document extractors ---
    "pdfplumber",
    "fitz",
    "fitz.fitz",
    "docx",
    "openpyxl",
    "pptx",
    # --- Text splitters ---
    "langchain_text_splitters",
    *collect_submodules("langchain_text_splitters"),
    # --- Jinja2 / multipart ---
    "jinja2",
    "jinja2.ext",
    "multipart",
    # --- chromadb collected above ---
    *chromadb_hiddenimports,
    *hnswlib_hiddenimports,
    *pdfplumber_hiddenimports,
    # --- email/encodings (sometimes needed) ---
    "email.mime.text",
    "encodings",
]

# ---------------------------------------------------------------------------
# Data files to bundle (frontend assets)
# ---------------------------------------------------------------------------
extra_datas = [
    ("frontend/templates", "frontend/templates"),
    ("frontend/static", "frontend/static"),
    *chromadb_datas,
    *hnswlib_datas,
    *pdfplumber_datas,
]

# ---------------------------------------------------------------------------
# Extra binaries
# ---------------------------------------------------------------------------
extra_binaries = [
    *chromadb_binaries,
    *hnswlib_binaries,
    *pdfplumber_binaries,
]

# ---------------------------------------------------------------------------
# Analysis
# ---------------------------------------------------------------------------
a = Analysis(
    ["run_server.py"],
    pathex=["."],
    binaries=extra_binaries,
    datas=extra_datas,
    hiddenimports=hidden_imports,
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[
        # Exclude packages we don't need to reduce EXE size
        "tkinter",
        "matplotlib",
        "numpy.distutils",
        "setuptools",
        "wheel",
        "pip",
        "pytest",
    ],
    noarchive=False,
)

pyz = PYZ(a.pure)

exe = EXE(
    pyz,
    a.scripts,
    [],
    exclude_binaries=True,
    name="georag",
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=True,
    console=True,       # Show console window for server logs
    icon=None,           # Add icon path here if desired: icon="frontend/static/images/favicon.ico"
)

# ---------------------------------------------------------------------------
# Collect into a directory (onedir mode — more reliable than onefile)
# The output will be:  dist/georag/georag.exe  (plus supporting DLLs)
# Zip the dist/georag/ folder for distribution.
# ---------------------------------------------------------------------------
coll = COLLECT(
    exe,
    a.binaries,
    a.datas,
    strip=False,
    upx=True,
    upx_exclude=[],
    name="georag",
)
