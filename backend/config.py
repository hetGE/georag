"""Configuration - all paths resolved relative to this file's location."""
from pathlib import Path

# _0RAG directory
RAG_DIR = Path(__file__).resolve().parent.parent

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

# LM Studio endpoints
LM_STUDIO_BASE_URL = "http://127.0.0.1:1234"
EMBEDDING_URL = f"{LM_STUDIO_BASE_URL}/v1/embeddings"
CHAT_URL = f"{LM_STUDIO_BASE_URL}/v1/chat/completions"

# Embedding settings
EMBEDDING_MODEL = "text-embedding-nomic-embed-text-v1.5"
EMBEDDING_DIM = 768
EMBEDDING_BATCH_SIZE = 128

# Chat model
CHAT_MODEL = "qwen3-vl-30b"

# Chunking settings
CHUNK_SIZE = 1000
CHUNK_OVERLAP = 200

# RAG settings
TOP_K_PER_TAG = 5
MAX_CONTEXT_CHUNKS = 8
CONVERSATION_HISTORY_TURNS = 4

# Scanning
SKIP_DIRS = {"_0RAG", ".git", "__pycache__", "node_modules", ".DS_Store", "venv"}
SUPPORTED_EXTENSIONS = {
    "pdf", "doc", "docx", "xls", "xlsx", "xlsm",
    "ppt", "pptx", "png", "jpg", "jpeg", "gif", "bmp", "tiff",
    "txt", "html", "htm", "rtf", "csv", "md",
    "dwg", "dxf",
    "mp4", "avi", "mov", "mkv",
}

# Default tags
DEFAULT_TAGS = [
    {"name": "piling", "display_name": "Piling", "color": "#e74c3c",
     "description": "Pile foundations, driven piles, bored piles, pile load tests"},
    {"name": "diaphragm_wall", "display_name": "Diaphragm Wall", "color": "#3498db",
     "description": "Diaphragm walls, slurry walls, barrettes"},
    {"name": "ground_improvement", "display_name": "Ground Improvement", "color": "#2ecc71",
     "description": "Soil improvement, compaction, grouting, soil mixing"},
    {"name": "ground_anchors", "display_name": "Ground Anchors", "color": "#9b59b6",
     "description": "Ground anchors, soil nails, rock bolts, tiebacks"},
    {"name": "deep_excavation", "display_name": "Deep Excavation", "color": "#f39c12",
     "description": "Deep excavations, braced cuts, sheet piling, cofferdam"},
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
]
