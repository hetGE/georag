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

# llama-server launch argv (used by backend.services.llama_supervisor)
LLAMA_CHAT_LAUNCH = [
    "llama-server",
    "--model", LLAMA_CHAT_MODEL,
    "--mmproj", LLAMA_CHAT_MMPROJ,
    "--port", "8001",
    "--alias", "qwen3.5-9B",
    "-c", "131072",
    "-n", "32768",
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

# Chat model (must match the --alias passed to llama-server)
CHAT_MODEL = "qwen3.5-9B"

# Parallel inference slots (must match llama-server -np; default 1)
LLM_PARALLEL_SLOTS = 1

# Chunking settings
CHUNK_SIZE = 1000
CHUNK_OVERLAP = 200

# RAG settings
TOP_K_PER_TAG = 5
MAX_CONTEXT_CHUNKS = 8
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

# Wiki settings
WIKI_COLLECTION_NAME = "wiki"
WIKI_INGEST_MAX_TOKENS = 8192   # Headroom for Qwen <think>...</think> + JSON output
WIKI_INGEST_MAX_SOURCE_CHARS = 24_000   # ~6K tokens of source material per LLM call
WIKI_INGEST_MAX_INDEX_CHARS = 6_000     # Cap on wiki index injected into system prompt
# Rebuild the wiki index page (LLM context) once per this many newly
# created/updated pages during a build, instead of after every page. The
# index/backlink scans are O(all pages); per-file rebuilds dominated build time.
# Only ~6K chars of the index reach the prompt anyway, so periodic freshness is
# plenty. Backlinks are rebuilt only once at the end.
WIKI_INDEX_REBUILD_EVERY = 25
WIKI_QUERY_MAX_CONTEXT_PAGES = 5
WIKI_CHAT_THRESHOLD = 0.7  # Min similarity score to use wiki instead of RAG
WIKI_CHAT_TOP_K = 5  # Max wiki pages to retrieve for chat

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
