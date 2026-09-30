"""Configuration - all paths resolved relative to this file's location."""
import os
import sys
from pathlib import Path

# Disable ChromaDB telemetry before it is imported anywhere
os.environ["ANONYMIZED_TELEMETRY"] = "False"

if getattr(sys, "frozen", False):
    # PyInstaller EXE: data lives next to the EXE, bundled resources in _MEIPASS
    RAG_DIR = Path(sys.executable).resolve().parent
    BUNDLE_DIR = Path(sys._MEIPASS)
else:
    # Running from source
    RAG_DIR = Path(__file__).resolve().parent.parent
    BUNDLE_DIR = RAG_DIR

# Engineering root (parent of _0RAG)
ENGINEERING_ROOT = RAG_DIR.parent

# Data directories
DATA_DIR = RAG_DIR / "data"
CHROMA_DIR = DATA_DIR / "chroma"
CACHE_DIR = DATA_DIR / "cache"
LOG_DIR = DATA_DIR / "logs"
METADATA_DB = DATA_DIR / "metadata.db"

# SQLAlchemy database URL
DATABASE_URL = f"sqlite:///{METADATA_DB}"

# llama-server endpoints (launched in-process by backend.services.llama_supervisor)
CHAT_BASE_URL = "http://127.0.0.1:8001"
EMBEDDING_BASE_URL = "http://127.0.0.1:8002"
CHAT_URL = f"{CHAT_BASE_URL}/v1/chat/completions"
EMBEDDING_URL = f"{EMBEDDING_BASE_URL}/v1/embeddings"

# llama-server model files.
# Each path can be overridden with its own env var (absolute path). Otherwise
# they resolve under GEORAG_LLM_DIR (default: ~/LLMs), matching the README layout.
LLM_DIR = Path(os.environ.get("GEORAG_LLM_DIR", Path.home() / "LLMs"))
LLAMA_CHAT_MODEL = os.environ.get(
    "LLAMA_CHAT_MODEL",
    str(LLM_DIR / "lmstudio-community/Qwen3.5-9B-GGUF/Qwen3.5-9B-Q4_K_M.gguf"),
)
LLAMA_CHAT_MMPROJ = os.environ.get(
    "LLAMA_CHAT_MMPROJ",
    str(LLM_DIR / "lmstudio-community/Qwen3.5-9B-GGUF/mmproj-Qwen3.5-9B-BF16.gguf"),
)
LLAMA_EMBED_MODEL = os.environ.get(
    "LLAMA_EMBED_MODEL",
    str(LLM_DIR / "second-state/Nomic-embed-text-v1.5-Embedding-GGUF/nomic-embed-text-v1.5-Q8_0.gguf"),
)

# MLX model directory and runtime (Apple Silicon only). MLX models are served
# by mlx_lm.server, which lives in its own virtualenv: mlx-lm needs a newer
# tokenizers/transformers than the chromadb pinned in requirements.txt allows.
MLX_CHAT_MODEL = os.environ.get(
    "MLX_CHAT_MODEL",
    str(LLM_DIR / "lmstudio-community/Qwen3.8-27B-MLX-4bit"),
)
MLX_LM_SERVER = os.environ.get(
    "MLX_LM_SERVER",
    str(RAG_DIR / "venv-mlx" / "bin" / "mlx_lm.server"),
)

# ── Chat models ──────────────────────────────────────────────────────────
# The chat model is chosen in the Retrieval Depth dialog (or preset with
# GEORAG_CHAT_MODEL) and served on CHAT_BASE_URL by one of two backends:
#
#   llama  llama-server with a GGUF model. The context window (-c) is allocated
#          when the model loads, so each memory tier has its own context size
#          and moving up a tier reloads the model. llama.cpp reports whether it
#          fits in GPU memory (see llama_supervisor.ensure_chat_ctx).
#   mlx    mlx_lm.server with an MLX model. Nothing is preallocated: memory is
#          the weights plus a KV cache that grows with the tokens in use, so a
#          tier's context is a budget, checked against the Mac's GPU working
#          set from the figures below before a chat starts. Text only (no image
#          description).
#
# "tiers" maps each Retrieval Depth tier to its context window and to the
# memory it assumes: dedicated VRAM for llama, total unified memory of the Mac
# for mlx. The assumed figure is a label; the real check is the backend's.
#
# Measured for Qwen3.5-9B Q4_K_M with a q8_0 KV cache (llama.cpp's own GPU
# projection; add ~1.8 GB for the vision projector and the embedding model):
#   32,768 tokens -> 5.9 GB    131,072 -> 7.5 GB    262,144 -> 10.0 GB
CHAT_MODELS = {
    "qwen3.5-9b": {
        "label": "Qwen 3.5 9B",
        "detail": "GGUF Q4_K_M on llama.cpp. Fast, runs from 8 GB VRAM, describes images.",
        "backend": "llama",
        "path": LLAMA_CHAT_MODEL,
        "mmproj": LLAMA_CHAT_MMPROJ,
        "request_model": "qwen3.5-9B",       # the --alias llama-server is launched with
        "vision": True,
        "memory_label": "GB VRAM",
        "memory_phrase": "{} GB of VRAM",
        "tiers": {
            "base": {"ctx": 32768, "memory_gb": 8},
            "mid": {"ctx": 131072, "memory_gb": 16},
            "top": {"ctx": 262144, "memory_gb": 24},   # the model's full trained context
        },
    },
    "qwen3.8-27b-mlx": {
        "label": "Qwen 3.8 27B",
        "detail": "MLX 4-bit, Apple Silicon only. Larger model: slower, needs a 32 GB+ Mac, text only.",
        "backend": "mlx",
        "path": MLX_CHAT_MODEL,
        "request_model": "default_model",    # mlx_lm.server's name for the model it was started with
        "vision": False,
        "memory_label": "GB Mac",
        "memory_phrase": "a {} GB Mac",
        # Measured on an M4 Max with mlx-lm 0.31: the KV cache costs 190-265 KB
        # per token of context (about 16x llama.cpp's q8_0 cache for the 9B),
        # and up to ~1 GB more is in flight while a new prompt replaces the
        # previous one. Weights are read from disk (15 GB).
        "kv_bytes_per_token": 270_000,
        "overhead_bytes": 1536 * 1024 * 1024,
        # The contexts are sized by that cost.
        "tiers": {
            "base": {"ctx": 24576, "memory_gb": 32},
            "mid": {"ctx": 40960, "memory_gb": 36},
            "top": {"ctx": 73728, "memory_gb": 48},
        },
        # Not a reply budget but a crash guard: mlx_lm.server 0.31 died with a
        # Metal "Resource limit exceeded" about 10,000 tokens into a reply,
        # losing the whole reply and hanging the request (see llm_client),
        # while 8,192 tokens completes. Set to None to let it run unguarded.
        "reply_guard_tokens": 8192,
    },
}
DEFAULT_CHAT_MODEL = os.environ.get("GEORAG_CHAT_MODEL", "qwen3.5-9b")
if DEFAULT_CHAT_MODEL not in CHAT_MODELS:
    DEFAULT_CHAT_MODEL = "qwen3.5-9b"

# ── Retrieval Depth levels ───────────────────────────────────────────────
# Each level sets how much is retrieved (top_k per tag, max_chunks in the
# prompt) and which memory tier it needs.
#
# There is no limit on the reply: the model answers for as long as it wants,
# and stops on its own or when its context window is full.
CHAT_DEPTH_LEVELS = [
    {"key": "quick", "label": "Quick and less demanding", "tier": "base",
     "top_k": 5, "max_chunks": 8},
    {"key": "optimal", "label": "Optimal and balanced", "tier": "base",
     "top_k": 10, "max_chunks": 16},
    {"key": "deep", "label": "Deep and demanding", "tier": "mid",
     "top_k": 20, "max_chunks": 32},
    {"key": "deeper", "label": "Deeper and very demanding", "tier": "mid",
     "top_k": 50, "max_chunks": 80},
    {"key": "ludicrous", "label": "Ludicrous", "tier": "top",
     "top_k": 100, "max_chunks": 200},
]
DEFAULT_CHAT_DEPTH = "optimal"

# Whether the chat model reasons ("thinks") before it answers. Off: answers are
# grounded in the retrieved text, so the reasoning pass mostly adds waiting. On
# a 50-source question Qwen 3.8 27B reasoned for ~7,700 tokens (over 9 minutes)
# and Qwen 3.5 9B for ~3,700 before writing anything. When True the reasoning
# is unlimited and shown in the chat as a collapsible block.
CHAT_THINKING = False


def chat_depth_levels(model_key: str) -> list[dict]:
    """The Retrieval Depth levels as they apply to one chat model: each level
    with the context window and assumed memory of its tier on that model."""
    model = CHAT_MODELS[model_key]
    levels = []
    for level in CHAT_DEPTH_LEVELS:
        tier = model["tiers"][level["tier"]]
        levels.append({
            **level,
            "ctx": tier["ctx"],
            "vram_gb": tier["memory_gb"],
            "memory_label": model["memory_label"],
            "memory_text": model["memory_phrase"].format(tier["memory_gb"]),
        })
    return levels


def chat_depth_level(model_key: str, depth_key: str | None) -> dict:
    levels = {level["key"]: level for level in chat_depth_levels(model_key)}
    return levels.get(depth_key) or levels[DEFAULT_CHAT_DEPTH]


# Chat server launch argv (used by backend.services.llama_supervisor)
def chat_launch(model_key: str, ctx: int) -> list[str]:
    """Command line that serves a chat model on port 8001. `ctx` is the context
    window; only llama-server takes it (mlx_lm.server grows its cache on demand)."""
    model = CHAT_MODELS[model_key]
    if model["backend"] == "mlx":
        return [
            MLX_LM_SERVER,
            "--model", model["path"],
            "--host", "127.0.0.1",
            "--port", "8001",
            "--temp", "0.6",
            "--top-p", "0.95",
            "--top-k", "20",
            # Used only for a request that names no max_tokens; chat requests
            # always do (see chat.py).
            "--max-tokens", str(model["reply_guard_tokens"] or model["tiers"]["top"]["ctx"]),
            # Every request carries different retrieved chunks, so old prompts
            # are rarely reusable; keeping the default 10 KV caches around
            # would only hold gigabytes of memory.
            "--prompt-cache-size", "1",
            # Smaller prompt-processing steps: same speed on a long prompt,
            # but ~3 GB less transient memory than the default 2048.
            "--prefill-step-size", "512",
        ]
    return [
        "llama-server",
        "--model", model["path"],
        "--mmproj", model["mmproj"],
        "--port", "8001",
        "--alias", model["request_model"],
        "-c", str(ctx),
        # No cap on reply length: generation ends when the model stops or
        # the context window is full.
        "-n", "-1",
        "--no-context-shift",
        "--temp", "0.6",
        "--top-p", "0.95",
        "--top-k", "20",
        "--repeat-penalty", "1.00",
        "--presence-penalty", "0.00",
        "--fit", "on",
        "-fa", "on",
        "-ctk", "q8_0",
        "-ctv", "q8_0",
        "--chat-template-kwargs", '{"preserve_thinking": true}',
    ]


LLAMA_EMBED_LAUNCH = [
    "llama-server",
    "--model", LLAMA_EMBED_MODEL,
    "--port", "8002",
    "--alias", "nomic-embed-text-v1.5",
    "--embeddings",
    "--pooling", "mean",
    "-c", "8192",
    "-b", "8192",
    "-ub", "8192",
    "--rope-scaling", "yarn",
    "--rope-freq-scale", "0.75",
    "-fa", "on",
    "-ngl", "99",
]

# Embedding settings (model name must match the --alias passed to llama-server)
EMBEDDING_MODEL = "nomic-embed-text-v1.5"
EMBEDDING_DIM = 768
EMBEDDING_BATCH_SIZE = 128

# Parallel inference slots (must match llama-server -np; default 1)
LLM_PARALLEL_SLOTS = 1

# Chunking settings
CHUNK_SIZE = 1000
CHUNK_OVERLAP = 200

# RAG settings. TOP_K_PER_TAG is the vector-search default; chat takes its
# retrieval sizes from CHAT_DEPTH_LEVELS.
TOP_K_PER_TAG = 5
CONVERSATION_HISTORY_TURNS = 4

# Scanning
SKIP_DIRS = {"_0RAG", ".git", "__pycache__", "node_modules", ".DS_Store", "venv"}
JUNK_FILENAMES = {".DS_Store", "Thumbs.db", "desktop.ini", "ehthumbs.db", "ehthumbs_vista.db"}
JUNK_FILENAME_PREFIXES = {"._"}
SUPPORTED_EXTENSIONS = {
    "pdf", "doc", "docx", "xls", "xlsx", "xlsm",
    "ppt", "pptx", "png", "jpg", "jpeg", "gif", "bmp", "tiff",
    "txt", "html", "htm", "rtf", "csv", "md",
    "dwg", "dxf",
    "mp4", "avi", "mov", "mkv",
}

# OCR settings (requires system prerequisite: brew install tesseract)
OCR_LANGUAGES = ["eng"]

# Default tags
DEFAULT_TAGS = [
    {"name": "piling", "display_name": "Piling", "color": "#e74c3c",
     "description": "Pile foundations, driven piles, bored piles, pile load tests"},
    {"name": "diaphragm_wall", "display_name": "Diaphragm Wall", "color": "#3498db",
     "description": "Diaphragm walls, slurry walls, barrettes"},
    {"name": "ground_improvement", "display_name": "Ground Improvement", "color": "#2ecc71",
     "description": "Soil improvement, compaction, grouting, soil mixing"},
    {"name": "ground_anchors", "display_name": "Ground Anchors", "color": "#00ffe1",
     "description": "Ground anchors, soil nails, rock bolts, tiebacks"},
    {"name": "deep_excavation", "display_name": "Deep Excavation", "color": "#f31212",
     "description": "Deep excavations, shorign design, braced cuts, sheet piling, cofferdam"},
    {"name": "tunneling", "display_name": "Tunneling", "color": "#1abc9c",
     "description": "Tunnel design, TBM, NATM, cut and cover"},
    {"name": "earthquake", "display_name": "Earthquake Engineering", "color": "#e67e22",
     "description": "Seismic design, liquefaction, dynamic analysis"},
    {"name": "lab_testing", "display_name": "Lab Testing", "color": "#34495e",
     "description": "Laboratory soil testing, triaxial, consolidation, direct shear"},
    {"name": "insitu_testing", "display_name": "In-Situ Testing", "color": "#16a085",
     "description": "SPT, CPT, pressuremeter, vane shear, plate load test"},
    {"name": "finite_element_analysis", "display_name": "FEA / Numerical", "color": "#8e44ad",
     "description": "Finite element analysis, PLAXIS, numerical modelling"},
    {"name": "grouting", "display_name": "Grouting", "color": "#cdab04",
     "description": "Grouting methods and specs"},
    {"name": "soil_nail", "display_name": "Soil Nails", "color": "#099bae",
     "description": "Soil Nailing methods and specs"},
    {"name": "concrete", "display_name": "Concrete", "color": "#6c757d",
     "description": "Concrete material relevant specs and documents"},
    {"name": "soilmechanics", "display_name": "Soil Mechanics", "color": "#ff0569",
     "description": "General Books and Documents on Soil Mechanics"},
    {"name": "sustainability", "display_name": "Sustainability", "color": "#00ff4c",
     "description": "Sustainable design principle and materials"},
    {"name": "energypiles", "display_name": "Energy Piles", "color": "#6c757d",
     "description": "Geothermal heating and cooling for buildings"},
    {"name": "geosynthetics", "display_name": "Geosynthetics", "color": "#27292b",
     "description": "Documentation on geotextiles, geogrids, geomembranes, and geosynthetic liners"},
    {"name": "slope", "display_name": "Slope stability", "color": "#ff8800",
     "description": "General Books and Documents on Slope stability and landslide"},
    {"name": "foundations", "display_name": "Shallow Foundations", "color": "#2adfc1",
     "description": "Settlement and Bearing Capacity of Shallow Foundations"},
    {"name": "project_documents", "display_name": "Project Documents", "color": "#c0392c",
     "description": "Project-related documents including administrative procedures, reports, and guidelines."},
    {"name": "geotechnical_research", "display_name": "Geotechnical Research", "color": "#2980b9",
     "description": "Research papers and technical reports on geotechnical topics such as soil behavior, geochemistry, and rock mechanics."},
    {"name": "project_management", "display_name": "Project Management", "color": "#f1c40f",
     "description": "Documents related to project administration, tendering, and contract management in civil engineering projects."},
    {"name": "civil", "display_name": "Civil Engineering", "color": "#cf9eff",
     "description": "Civil structures design and construction including roads, bridges, dams, airports, and water systems."},
    {"name": "mesh_free_methods", "display_name": "Mesh Free Methods", "color": "#2980b9",
     "description": "Documents related to mesh free methods in engineering, including element free Galerkin (EFG), meshless local Petrov\u2013Galerkin (MLPG), and other meshless techniques for solving partial differential equations."},
    {"name": "transportation_geotechnics", "display_name": "Transportation Geotechnics", "color": "#f1c40f",
     "description": "Research and applications related to geotechnical engineering in transportation infrastructure, including roads, railways, airports, and pavements."},
    {"name": "panel_concrete_forms", "display_name": "Panel Concrete Documentation", "color": "#d35400",
     "description": "Excel-based documentation for concrete panel production, including mix design, curing, and quality control."},
    {"name": "conference_proceedings", "display_name": "Conference Proceedings", "color": "#7f8c8d",
     "description": "Collections of technical papers and presentations from engineering conferences, particularly in geotechnical and civil engineering."},
    {"name": "dwg_drawing_templates", "display_name": "DWG Drawing Templates", "color": "#2980b9",
     "description": "AutoCAD DWG files containing architectural or engineering drawings, such as elevations, sections, and corners, used in construction documentation and quantity surveying."},
    {"name": "zemin_gerilme_analysis", "display_name": "Soil Stress Analysis", "color": "#d35400",
     "description": "Technical documents and calculations focused on soil stress distribution and analysis, particularly in the context of historical structures and foundation design."},
    {"name": "flow_net_analysis", "display_name": "Flow Net Analysis", "color": "#7f8c8d",
     "description": "Documents and data related to flow net analysis in geotechnical engineering, including graphical representations, calculations, and modeling of groundwater flow through soil."},
    {"name": "shafts", "display_name": "Circular Shafts", "color": "#fb00ff",
     "description": "Asymmetrical and symmetrical multiple shafts or cells"},
    {"name": "drill_form", "display_name": "Drilling Forms", "color": "#bbff00",
     "description": "Excel-based forms used for recording any drilling or excavation activity for piles, anchors, diaphragm walls etc"},
    {"name": "speaker_profiles", "display_name": "Speaker Profiles", "color": "#d35400",
     "description": "Documents and images related to profiles of speakers at engineering conferences and events, including biographical and professional information."},
    {"name": "bim_visualizations", "display_name": "BIM Visualizations", "color": "#1e8449",
     "description": "Videos and presentations demonstrating Building Information Modeling (BIM) workflows, including visualization of construction processes and data integration."},
]
