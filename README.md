<p align="center">
    <a href="https://www.hetge.com">
      <img src="frontend/static/images/hetge.svg" alt="hetGE" height="60">
    </a>
  </p>
  
<h1 align="center">geoRAG</h1>

<p align="center">
  <strong>Geotechnical Engineering RAG Assistant</strong><br>
</p>

<p align="center">
  A locally hosted AI assistant that reads your entire engineering document library and answers questions grounded in your own files. PDFs, Word docs, spreadsheets, presentations, images, CAD files; all indexed, all searchable, all private. Everything runs on your machine. No cloud. No API keys. No data leaves your network.
</p>

---

## Table of Contents

### [Part 1: Executive Summary](#part-1-executive-summary)
- [What Is This?](#what-is-this)
- [What Can It Do?](#what-can-it-do)
- [How Does It Work (in 30 Seconds)?](#how-does-it-work-in-30-seconds)

### [Part 2: Getting Started](#part-2-getting-started)
- [What You Need](#what-you-need)
- [Step 1: Set Up LM Studio](#step-1-set-up-lm-studio)
- [Step 2: Start GeoRAG](#step-2-start-georag)
- [Step 3: First-Time Walkthrough](#step-3-first-time-walkthrough)
- [Using the Chat](#using-the-chat)
- [Managing Your Documents](#managing-your-documents)
- [Stopping the Server](#stopping-the-server)
- [File Types That Work](#file-types-that-work)
- [Built-In Topic Tags](#built-in-topic-tags)
- [Common Issues & Fixes](#common-issues--fixes)

### [Part 3: Technical Reference](#part-3-technical-reference)
- [System Architecture](#system-architecture)
- [Data Flow](#data-flow)
- [Processing Pipeline Deep Dive](#processing-pipeline-deep-dive)
- [Chat & Retrieval Internals](#chat--retrieval-internals)
- [Concurrency Model](#concurrency-model)
- [Tech Stack](#tech-stack)
- [Project Structure](#project-structure)
- [API Reference](#api-reference)
- [Configuration Reference](#configuration-reference)
- [Database Schema](#database-schema)
- [Frontend Architecture](#frontend-architecture)
- [Scripts Reference](#scripts-reference)
- [License](#license)

---

# Part 1: Executive Summary

## What Is This?

GeoRAG is a **local AI assistant for geotechnical engineers**. It reads your engineering documents (reports, calculations, specifications, test results, drawings) and lets you ask questions in natural language. The AI answers using the actual content of your files, citing exactly which documents and pages it drew from.

It's like having a colleague who has read every document in your library and can instantly recall any detail.

**Key principle:** Your data never leaves your computer. The AI models run locally through [LM Studio](https://lmstudio.ai/), a free desktop application. There are no subscriptions, no cloud uploads, and no API costs.

## What Can It Do?

| Capability | What It Means |
|-----------|---------------|
| **Read your documents** | Scans your `Engineering/` folder and extracts text from PDFs, Word, Excel, PowerPoint, images, and more |
| **Organize by topic** | Automatically classifies documents into geotechnical categories (piling, tunneling, ground improvement, etc.) |
| **Answer questions** | Ask anything about your documents and get answers with source citations |
| **Stream responses** | See the AI's answer appear word by word in real time |
| **Remember conversations** | All chat history is saved and can be revisited later |
| **Work across tabs** | Open multiple browser tabs and they stay in sync |
| **Open source files** | Click any cited document to open it directly on your machine |

## How Does It Work (in 30 Seconds)?

```
Your Engineering/ folder
        ↓
  GeoRAG scans and reads every supported file
        ↓
  Each document is split into small pieces and converted to numbers (embeddings)
        ↓
  These are stored in a searchable database on your machine
        ↓
  When you ask a question, GeoRAG finds the most relevant pieces
        ↓
  The AI reads those pieces and writes an answer, citing its sources
```

Two pieces of software make this work:
1. **GeoRAG** (this project), the web app that processes documents and runs the interface
2. **LM Studio** (free, separate download), runs the AI models locally on your hardware

---

# Part 2: Getting Started

## What You Need

| Requirement | Minimum | Recommended |
|-------------|---------|-------------|
| **Computer** | 16 GB RAM | 32 GB+ RAM |
| **Disk space** | 10 GB free | 50 GB+ (AI models are large) |
| **Operating system** | macOS 14+, Windows 10+, or Ubuntu 20.04+ | macOS with Apple Silicon (M1/M2/M3/M4) |
| **Python** | 3.10 | 3.11 or newer |
| **LM Studio** | Version 0.3+ | Latest version ([download here](https://lmstudio.ai/download)) |
| **GPU** | Not strictly required | Apple Silicon or NVIDIA GPU (much faster) |

## Step 1: Set Up LM Studio

LM Studio is a free app that runs AI models on your computer. GeoRAG talks to it behind the scenes.

### Install It

Download from [lmstudio.ai/download](https://lmstudio.ai/download):
- **Mac**: Open the `.dmg` and drag to Applications
- **Windows**: Run the `.exe` installer
- **Linux**: Download the AppImage and make it executable

### Download Two AI Models

Open LM Studio and go to the **Discover** tab (the search/magnifying glass icon). You need two models:

**1. The "brain", a chat model that writes answers:**

Search for and download one of these (pick based on your hardware):

| Model | Download Size | RAM Needed | Best For |
|-------|-------------|------------|----------|
| Qwen 3 VL 30B | ~18 GB | 24 GB+ | Best quality + can read images |
| Qwen 3 8B | ~5 GB | 10 GB+ | Great quality, works on most machines |
| Gemma 3 4B | ~3 GB | 8 GB+ | Good balance of speed and quality |
| Llama 3.2 3B | ~2 GB | 6 GB+ | Fastest, works on modest hardware |

> For geotechnical work, bigger models (8B+) understand technical content better. If your machine can handle it, go bigger.

**2. The "librarian", an embedding model that finds relevant documents:**

Search for `nomic-embed-text` and download:
```
nomic-ai/nomic-embed-text-v1.5-GGUF
```
This is small (~260 MB) and runs on any machine.

### Start the Local Server

1. **Load both models**: click each one and hit Load (or press `Cmd+L` / `Ctrl+L`)
2. Go to the **Developer** tab (the `</>` icon)
3. Make sure the server shows **Started** on port **1234**
4. To verify, open your browser to `http://127.0.0.1:1234/v1/models`; you should see your loaded models listed

### Recommended Settings

| Setting | Set It To | Why |
|---------|----------|-----|
| Context Length | 4096 to 8192 | Gives enough room for document context + conversation |
| GPU Offload | Max layers | Much faster answers |
| Temperature | 0.1 to 0.3 | Keeps answers factual and grounded |
| Max Concurrent Predictions | 2 to 3 | Lets embedding and chat run at the same time |

> **LM Studio must be running before you start GeoRAG.** The startup script will warn you if it can't connect.

## Step 2: Start GeoRAG

### The Easy Way (Recommended)

```bash
# Navigate to the project
cd /path/to/Engineering/_0RAG

# Make the scripts runnable (first time only)
chmod +x start.sh kill.sh

# Start everything
./start.sh
```

That's it. The script will:
1. Create a Python virtual environment (first run only)
2. Install all dependencies (first run only)
3. Set up data directories
4. Check that LM Studio is reachable
5. Start the web server

You'll see:
```
===============================
  GeoRAG - Geotechnical RAG
===============================

LM Studio: Connected
Starting GeoRAG server...
  URL: http://localhost:3000
```

Open **http://localhost:3000** in your browser.

### The Manual Way

If you prefer to control each step:

```bash
# Create and activate a Python environment
python3.11 -m venv venv
source venv/bin/activate          # macOS/Linux
# or: venv\Scripts\activate       # Windows

# Install dependencies
pip install --upgrade pip
pip install -r requirements.txt

# Create data folders
mkdir -p data/{chroma,cache,logs}

# Start the server
uvicorn backend.app:app --host 0.0.0.0 --port 3000 --reload
```

## Step 3: First-Time Walkthrough

When you first open GeoRAG, the **Library Panel** on the right side walks you through setup:

1. **Scan**: Click the scan button. GeoRAG discovers all supported files in your `Engineering/` folder. You'll see a count of files found, broken down by type.

2. **Process**: Click "Start Processing". GeoRAG will:
   - Extract text from every document
   - Auto-classify documents into topic categories
   - Split text into searchable chunks
   - Generate embeddings for similarity search
   - This takes time; the progress bar shows where you are, and you can see which file is being processed at any moment.

3. **Chat**: Once processing finishes, go to the Chat page, select one or more topic tags, and start asking questions.

> Processing speed depends on your hardware and the number of files. LM Studio (specifically the embedding model) is the bottleneck. You can stop and resume processing at any time; already processed files won't be redone.

## Using the Chat

- **Select topics first**: On the welcome screen, click one or more topic tags (e.g., "Piling", "Deep Excavation"). This tells GeoRAG which document collections to search.
- **Ask anything**: Type your question and press Send. The AI will find relevant document sections and write an answer.
- **Check the sources**: Below each answer, you'll see which documents were cited, with page numbers and relevance scores. Click a source to open the original file.
- **Adjust depth**: Click the gear icon next to the tag bar to control how many document chunks the AI considers:

| Depth Level | Speed | Detail | Good For |
|-------------|-------|--------|----------|
| Quick | Fastest | Brief answers | Simple factual lookups |
| Standard | Balanced | Good detail | Most questions (default) |
| Deep | Slower | Thorough | Multi-document analysis |
| Extreme | Slow | Very detailed | Complex technical questions |
| Ludicrous | Slowest | Maximum context | When you need everything |

- **Conversations are saved**: Use the sidebar on the left to switch between past conversations.
- **Trash and restore**: Delete a conversation and it goes to trash. You can restore it or permanently delete it.

## Managing Your Documents

Go to the **Documents** page (`/documents`) to:

- **Search**: Find files by name
- **Filter**: Narrow by file type (PDF, Word, Excel, etc.), processing status, or topic tag
- **Tag files**: Click a file to manage its tags, or select multiple files for batch tagging
- **Open files**: Click the filename to open it directly in your system's default application
- **See stats**: The top bar shows total file counts and processing status

## Stopping the Server

```bash
# Option 1: Press Ctrl+C in the terminal where start.sh is running

# Option 2: Run the kill script
./kill.sh
```

## File Types That Work

| Type | Formats | What Gets Extracted |
|------|---------|--------------------|
| **Documents** | PDF | Full text with page boundaries |
| | Word (.doc, .docx) | Paragraphs and tables |
| | Excel (.xls, .xlsx, .xlsm) | All sheets with sheet names |
| | PowerPoint (.ppt, .pptx) | All slides with text and tables |
| **Images** | PNG, JPG, GIF, BMP, TIFF | AI generated description of image content (max 10 MB) |
| **Text files** | TXT, Markdown, CSV, HTML, RTF | Direct text content |
| **CAD** | DWG, DXF | Indexed by filename only (no content extraction yet) |
| **Video** | MP4, AVI, MOV, MKV | Discovered but not content extracted |

## Built-In Topic Tags

GeoRAG comes with 11 geotechnical categories. Documents are automatically classified into these during processing:

| Tag | Color |
|-----|-------|
| Piling | Red |
| Diaphragm Wall | Blue |
| Ground Improvement | Green |
| Ground Anchors | Purple |
| Deep Excavation | Orange |
| Tunneling | Teal |
| Earthquake | Dark Orange |
| Lab Testing | Dark Gray |
| In-Situ Testing | Green |
| Finite Element Analysis | Purple |

You can also create your own custom tags through the Documents page or the API.

## Common Issues & Fixes

### "LM Studio not detected"
LM Studio isn't running or the server isn't started. Open LM Studio, go to the Developer tab, and make sure the server is running on port 1234.

### Chat gives empty or broken responses
- Make sure a **chat model** is loaded in LM Studio (not just the embedding model)
- The model name in `backend/config.py` must match what LM Studio shows
- Try increasing the context length to 4096+

### Processing fails on embeddings
- Make sure the **nomic-embed-text-v1.5** model is loaded in LM Studio
- Both models (chat + embedding) need to be loaded simultaneously

### Processing is really slow
LM Studio is the bottleneck; it's doing all the AI work locally. To speed things up:
- Turn on GPU offloading in LM Studio (offload as many layers as possible)
- Increase "Max Concurrent Predictions" when loading models
- Close other apps competing for GPU/RAM
- Consider using a smaller/faster chat model

### Files don't show up after scanning
- Files must be inside the `Engineering/` parent directory (not inside `_0RAG/` itself)
- The file extension must be supported (see the table above)
- Folders named `.git`, `__pycache__`, `node_modules`, and `venv` are skipped

### Port 3000 is already in use
```bash
lsof -i :3000        # See what's using it
./kill.sh            # Or just kill all GeoRAG processes
```

### Nuclear option: start fresh
```bash
rm -rf data/         # Deletes all processed data (your source documents are safe)
./start.sh           # Re-creates everything from scratch
```

---

# Part 3: Technical Reference

## System Architecture

```
+------------------------------------------------------------+
|                        Browser                             |
|                                                            |
|  +-----------+  +-------------+  +----------------+        |
|  | Chat Page |  | Documents   |  | Library Panel  |        |
|  | (chat.js) |  | (docs.js)   |  | (panel.js)     |        |
|  +-----+-----+  +------+------+  +-------+--------+        |
|        |                |                 |                |
|        | BroadcastChannel (cross-tab)     |                |
+--------+----------------+-----------------+----------------+
         |                |                 |
         | SSE/HTTP       | HTTP            | HTTP (poll)
         v                v                 v
+------------------------------------------------------------+
|                 FastAPI Backend (:3000)                    |
|                                                            |
|  +--------+  +---------+  +----------+  +--------+         |
|  | Chat   |  | Docs    |  |Processing|  | Tags   |         |
|  | Router |  | Router  |  | Router   |  | Router |         |
|  +---+----+  +----+----+  +----+-----+  +---+----+         |
|      |            |             |            |             |
|  +----------------------------------------------+          |
|  |              Services Layer                  |          |
|  |                                              |          |
|  | +----------+ +---------+ +--------------+    |          |
|  | | LLM      | |Embedding| | Document     |    |          |
|  | | Client   | | Client  | | Processor    |    |          |
|  | | (chat +  | |         | |(orchestrator)|    |          |
|  | |  vision) | |         | +------+-------+    |          |
|  | +----+-----+ +----+----+  +-----+------+     |          |
|  |      |             |       | Scanner    |    |          |
|  |      |             |       | Chunker    |    |          |
|  |      |             |       | Tagger     |    |          |
|  |      |             |       | Extractors |    |          |
|  |      |             |       +------------+    |          |
|  +----------------------------------------------+          |
|          |             |                                   |
+----------+-------------+-----------------------------------+
           |             |
           v             v
+----------------------------+   +---------------------------+
| LM Studio (:1234)          |   | Data Storage              |
|                            |   |                           |
| +------------------------+ |   | +---------------------+   |
| | /v1/chat/completions   | |   | | ChromaDB            |   |
| | (chat model)           | |   | | (vectors)           |   |
| +------------------------+ |   | +---------------------+   |
| | /v1/embeddings         | |   | | SQLite              |   |
| | (nomic-embed-text-     | |   | | (metadata)          |   |
| |  v1.5)                 | |   | +---------------------+   |
| +------------------------+ |   +---------------------------+
+----------------------------+
```

**Three tier layout:**
- **Browser**: Vanilla JS frontend served as static files. Chat page, documents page, and library panel. Tabs sync via BroadcastChannel API.
- **FastAPI Backend** (port 3000): Python async server handling API requests, orchestrating the RAG pipeline, and streaming responses via SSE.
- **LM Studio** (port 1234): Local LLM runtime exposing an OpenAI compatible API for chat completions, embeddings, and vision.

**Storage:**
- **ChromaDB**: On disk vector database. One collection per topic tag, cosine distance metric, 768 dim vectors.
- **SQLite**: File metadata, tag associations, conversations, messages, and source citations.

## Data Flow

### Document Ingestion

```
Engineering/ directory
    → Scanner: walks filesystem, registers files in SQLite (status=new)
    → Extractors: format specific text extraction (10 files in parallel)
    → Tagger: heuristic keyword matching, then LLM fallback for unmatched files
    → Chunker: RecursiveCharacterTextSplitter (1000 tokens, 200 overlap)
    → Embedding Client: batch embed via LM Studio /v1/embeddings
    → Vector Store: upsert into ChromaDB per tag collections
    → SQLite: update status=processed, save chunk count + text preview
```

### Chat Query

```
User message + selected tags
    → Embed query via LM Studio /v1/embeddings
    → Query each selected tag's ChromaDB collection (top K per tag)
    → Deduplicate across tags, rank by cosine similarity
    → Assemble system prompt with top context chunks
    → Append last 4 conversation turns (8 messages)
    → Stream completion from LM Studio /v1/chat/completions
    → Deliver tokens to browser via SSE
    → Save message + source citations to SQLite
```

## Processing Pipeline Deep Dive

When you click "Start Processing", the system runs a six phase pipeline with managed concurrency:

### Phase 1: File Scanning (`services/scanner.py`)

Walks `Engineering/` recursively. Skips `_0RAG/`, `.git/`, `__pycache__/`, `node_modules/`, `venv/`. Registers each discovered file in SQLite with `status=new`. Detects files removed from disk and cleans them from both SQLite and ChromaDB.

### Phase 2: Text Extraction (`services/extractors/*.py`)

10 files extracted in parallel. Each format has a dedicated extractor:

| Format | Library | Strategy |
|--------|---------|----------|
| PDF | pdfplumber | Per page extraction with `[Page N]` markers |
| DOCX | python-docx | Paragraph text + table cell contents |
| XLSX | openpyxl | All sheets with `[Sheet: name]` markers |
| PPTX | python-pptx | Per slide text with `[Slide N]` markers + tables |
| Images | LM Studio vision | Base64 encoded image sent to vision model for description (max 10 MB) |
| Text/HTML/RTF/CSV/MD | Built in | Direct read with HTML tag stripping; UTF 8 to latin 1 fallback |
| DWG/DXF | N/A | Filename indexed only, no content extraction |

Failed extractions mark the file as `status=failed` and log the error.

### Phase 3: Auto Tagging (`services/tagger.py`)

Two pass classification:

1. **Heuristic pass**: 94 keyword patterns matched against folder paths and filenames. Maps patterns like "pile", "bored pile", "diaphragm", "grouting", "plaxis" to tag categories. Confidence: 0.7. Instant, no LLM call.

2. **LLM fallback**: Files with no heuristic match are batched (up to 10) and sent to the chat model with the first 1500 characters of text. The LLM returns a JSON array of tag classifications. Confidence: 0.8. Falls back to individual calls if batch parsing fails.

Tags are saved to the `FileTag` junction table with `source=auto` or `source=folder_hint`.

### Phase 4: Chunking (`services/chunker.py`)

Text is split using `RecursiveCharacterTextSplitter` from langchain:
- **Chunk size:** 1000 tokens
- **Chunk overlap:** 200 tokens
- **Separator priority:** `\n\n` then `\n` then `. ` then ` ` then `""`

Page/slide/sheet metadata is extracted via regex from markers embedded during extraction and attached to each chunk's metadata.

### Phase 5: Embedding (`services/embedding_client.py`)

Chunks are batched (up to 128 per request) and sent to LM Studio's `/v1/embeddings` endpoint using the `nomic-embed-text-v1.5` model. Three concurrent embedding requests are allowed (semaphore controlled). Each chunk becomes a 768 dimensional float vector.

### Phase 6: Vector Storage (`services/vector_store.py`)

Chunks are upserted into ChromaDB:
- **One collection per tag**, named `tag_{tag_name}`, cosine distance metric
- **Chunk IDs**: `file_path::chunk_N` (enables idempotent reprocessing via upsert)
- **Stored per chunk:** embedding vector, chunk text, metadata (`file_path`, `filename`, `file_type`, `parent_dir`, `page`, `chunk_index`)
- **Batch size:** 5000 chunks per upsert call
- Files with multiple tags are stored in multiple collections

## Chat & Retrieval Internals

When a user sends a message via `POST /api/chat`:

1. The user's query is embedded using the same Nomic model
2. For each selected tag, the corresponding ChromaDB collection is queried for the `top_k_per_tag` most similar chunks (default: 5)
3. Results from all tags are merged and deduplicated by chunk ID
4. The top `max_context_chunks` (default: 8) are selected by relevance score
5. A system prompt is built containing the context chunks with source metadata
6. The last `CONVERSATION_HISTORY_TURNS` turns (default: 4 turns = 8 messages) are appended
7. The full prompt is streamed to LM Studio via `/v1/chat/completions` with `stream=true`
8. Tokens are forwarded to the client as SSE events (`event: token`, `data: {"token": "..."}`)
9. On completion, a `done` event sends `{conversation_id, sources}` and the message is persisted

**Stream mirroring:** Other browser tabs can connect to `GET /api/chat/stream-mirror` to receive the same token stream in real time via async queues.

**Cancellation:** `POST /api/chat/stop` sets a cancellation flag that the streaming loop checks between tokens.

### Retrieval Depth Presets

| Level | Top K Per Tag | Max Context Chunks |
|-------|--------------|-------------------|
| Quick | 3 | 4 |
| Standard | 5 | 8 |
| Deep | 8 | 12 |
| Extreme | 12 | 20 |
| Ludicrous | 15 | 25 |

## Concurrency Model

| Resource | Limit | Mechanism | Rationale |
|----------|-------|-----------|-----------|
| File extraction | 10 parallel | `asyncio.gather` batch | I/O bound, benefits from parallelism |
| LLM tagging | 1 | `asyncio.Semaphore(1)` | LM Studio single model inference bottleneck |
| Embedding requests | 3 | `asyncio.Semaphore(3)` | Balance throughput vs. LM Studio capacity |
| Database writes | 1 | `asyncio.Lock` | SQLite transaction safety |
| Embedding batch size | 128 texts | Config constant | LM Studio request size limit |
| Processing batch | 10 files | Config constant | Memory bounded pipeline stage |

## Tech Stack

### Backend Dependencies

| Package | Version | Role |
|---------|---------|------|
| fastapi | 0.115.6 | Async web framework with OpenAPI |
| uvicorn[standard] | 0.34.0 | ASGI server |
| sqlalchemy | 2.0.36 | ORM and database toolkit |
| aiosqlite | 0.20.0 | Async SQLite driver |
| chromadb | 0.5.23 | Vector database |
| httpx | 0.28.1 | Async HTTP client (LM Studio communication) |
| pdfplumber | 0.11.4 | PDF text extraction |
| PyMuPDF | 1.25.1 | PDF support |
| python-docx | 1.1.2 | Word document parsing |
| openpyxl | 3.1.5 | Excel parsing |
| python-pptx | 1.0.2 | PowerPoint parsing |
| langchain-text-splitters | 0.3.4 | Recursive character text splitting |
| sse-starlette | 2.2.1 | Server Sent Events |
| jinja2 | 3.1.4 | HTML templating |
| pydantic | (via FastAPI) | Data validation |

### Frontend Stack

| Technology | Role |
|-----------|------|
| Vanilla JavaScript | No framework; direct DOM manipulation, module pattern controllers |
| PicoCSS | Minimal classless CSS framework |
| Marked.js | Markdown to HTML rendering for assistant responses |
| Inter (Google Fonts) | UI typography |
| EventSource API | SSE streaming for real time chat |
| BroadcastChannel API | Cross tab state synchronization |
| History API | Client side SPA routing |
| localStorage | Persists retrieval depth setting and panel state |

### External Infrastructure

| Component | Port | Role |
|-----------|------|------|
| GeoRAG (Uvicorn) | 3000 | Web server + API |
| LM Studio | 1234 | Local LLM inference (OpenAI compatible API) |

## Project Structure

```
_0RAG/
├── start.sh                        # Startup: venv, deps, LM Studio check, server
├── kill.sh                         # Find and kill all GeoRAG processes
├── requirements.txt                # Pinned Python dependencies
├── .gitignore                      # Excludes venv/, data/, __pycache__/
│
├── backend/
│   ├── app.py                      # FastAPI app entry point, lifespan init
│   ├── config.py                   # All paths, model names, tuning params
│   │
│   ├── models/
│   │   ├── database.py             # SQLAlchemy engine + session factory
│   │   ├── schemas.py              # ORM: File, Tag, FileTag, Conversation, Message
│   │   └── pydantic_models.py      # Request/response validation schemas
│   │
│   ├── routers/
│   │   ├── chat.py                 # POST /api/chat (SSE), stream mirror, stop
│   │   ├── conversations.py        # CRUD, soft delete, trash, restore
│   │   ├── documents.py            # Search, filter, paginate, tag, open
│   │   ├── tags.py                 # Tag CRUD
│   │   └── processing.py           # Scan, start/stop pipeline, onboarding
│   │
│   └── services/
│       ├── document_processor.py   # Pipeline orchestrator (batch + concurrency)
│       ├── scanner.py              # Filesystem walk + DB synchronization
│       ├── chunker.py              # RecursiveCharacterTextSplitter wrapper
│       ├── embedding_client.py     # Async batch embeddings via LM Studio
│       ├── vector_store.py         # ChromaDB per tag collection management
│       ├── llm_client.py           # Chat completions + vision (streaming)
│       ├── tagger.py               # 94 pattern heuristic + LLM classification
│       └── extractors/
│           ├── pdf_extractor.py    # pdfplumber with [Page N] markers
│           ├── docx_extractor.py   # python-docx text + tables
│           ├── xlsx_extractor.py   # openpyxl multi sheet
│           ├── pptx_extractor.py   # python-pptx slides + tables
│           ├── image_extractor.py  # Vision model via base64
│           └── text_extractor.py   # TXT, HTML, RTF, CSV, MD
│
├── frontend/
│   ├── templates/
│   │   ├── base.html               # Shared nav, library panel, script tags
│   │   ├── spa.html                # SPA shell (chat + documents in one page)
│   │   ├── index.html              # Server rendered chat page
│   │   └── documents.html          # Server rendered documents page
│   │
│   └── static/
│       ├── css/
│       │   ├── app.css             # Brand colors, layout, components, animations
│       │   └── pico.min.css        # PicoCSS framework
│       ├── js/
│       │   ├── chat.js             # Chat: SSE streaming, tags, conversations
│       │   ├── documents.js        # Documents: search, filter, batch tag
│       │   ├── panel.js            # Library: scan, process, progress polling
│       │   ├── router.js           # SPA routing via History API
│       │   ├── utils.js            # HTTP helpers, tag badges, sync channel
│       │   └── marked.min.js       # Markdown parser (vendored)
│       └── images/                 # Logo, icons, favicons
│
└── data/                           # Runtime data (gitignored)
    ├── metadata.db                 # SQLite: files, tags, conversations
    ├── chroma/                     # ChromaDB: vector collections
    ├── cache/                      # Application cache
    ├── logs/                       # Application logs
    └── onboarding_dismissed        # Flag file
```

## API Reference

FastAPI auto generates interactive docs at `/docs` (Swagger) and `/redoc`.

### Chat Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/chat` | Stream a chat response via SSE. Body: `{message, conversation_id?, tag_names?, top_k_per_tag?, max_context_chunks?}`. Events: `token` then `done` then `error`. |
| `GET` | `/api/chat/streaming` | Check streaming status. Returns `{streaming: bool, conversation_id: int or null}`. |
| `POST` | `/api/chat/stop` | Cancel the active stream. |
| `GET` | `/api/chat/stream-mirror` | SSE mirror of active stream for cross tab sync. |

### Conversation Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/conversations` | List active conversations (newest first, limit 50). |
| `GET` | `/api/conversations/{id}` | Full conversation with messages and sources. |
| `DELETE` | `/api/conversations/{id}` | Soft delete (move to trash). |
| `DELETE` | `/api/conversations/{id}/permanent` | Hard delete conversation + messages. |
| `POST` | `/api/conversations/{id}/restore` | Restore from trash. |
| `GET` | `/api/conversations/trash` | List trashed conversations. |

### Document Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/documents` | Paginated list. Params: `search`, `extension`, `tag` (`__none__` for untagged), `status`, `page`, `per_page` (max 200). |
| `GET` | `/api/documents/stats` | Counts by status and extension. |
| `POST` | `/api/documents/{id}/open` | Open file in system default app. |
| `POST` | `/api/documents/open-by-path` | Open by relative path. Body: `{file_path}`. |
| `POST` | `/api/documents/{id}/tags/{tag}` | Add tag to file. |
| `DELETE` | `/api/documents/{id}/tags/{tag}` | Remove tag from file. |
| `POST` | `/api/documents/batch/tags/{tag}` | Batch add tag. Body: `{file_ids: []}`. |
| `DELETE` | `/api/documents/batch/tags/{tag}` | Batch remove tag. Body: `{file_ids: []}`. |

### Tag Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/tags` | All tags with file counts. |
| `POST` | `/api/tags` | Create tag. Body: `{name, display_name, description?, color?}`. |
| `DELETE` | `/api/tags/{name}` | Delete tag + all associations. |

### Processing Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/processing/scan` | Scan filesystem, sync DB. Returns `{files_found, files_new, files_removed}`. |
| `POST` | `/api/processing/start` | Start pipeline. Body: `{tag_names?, file_ids?, reprocess?}`. |
| `POST` | `/api/processing/stop` | Gracefully stop processing. |
| `GET` | `/api/processing/status` | Pipeline state: `{is_running, total_files, processed_files, failed_files, skipped_files, current_file, errors}`. |
| `GET` | `/api/processing/onboarding-status` | Full onboarding state with phase, progress, and extension breakdown. |
| `POST` | `/api/processing/onboarding-dismiss` | Dismiss onboarding wizard. |
| `DELETE` | `/api/processing/onboarding-dismiss` | Reset onboarding visibility. |

## Configuration Reference

All values live in `backend/config.py`.

### Paths

| Variable | Default | Description |
|----------|---------|-------------|
| `ENGINEERING_ROOT` | Parent of `_0RAG/` | Root directory scanned for documents |
| `DATA_DIR` | `_0RAG/data/` | Runtime data root |
| `CHROMA_DIR` | `data/chroma/` | ChromaDB on disk storage |
| `METADATA_DB` | `data/metadata.db` | SQLite database path |
| `CACHE_DIR` | `data/cache/` | Application cache |
| `LOG_DIR` | `data/logs/` | Application logs |

### LM Studio Connection

| Variable | Default | Description |
|----------|---------|-------------|
| `LM_STUDIO_BASE_URL` | `http://127.0.0.1:1234` | LM Studio API root |
| `EMBEDDING_URL` | `{base}/v1/embeddings` | Embedding endpoint |
| `CHAT_URL` | `{base}/v1/chat/completions` | Chat completion endpoint |
| `EMBEDDING_MODEL` | `text-embedding-nomic-embed-text-v1.5` | Embedding model name |
| `EMBEDDING_DIM` | `768` | Vector dimensionality |
| `CHAT_MODEL` | `qwen3-vl-30b` | Chat/vision model name |

### Processing Tuning

| Variable | Default | Description |
|----------|---------|-------------|
| `CHUNK_SIZE` | `1000` | Tokens per chunk |
| `CHUNK_OVERLAP` | `200` | Overlap between adjacent chunks |
| `EMBEDDING_BATCH_SIZE` | `128` | Texts per embedding API call |
| `BATCH_SIZE` | `10` | Files processed concurrently per batch |
| `TOP_K_PER_TAG` | `5` | Chunks retrieved per tag during chat |
| `MAX_CONTEXT_CHUNKS` | `8` | Max chunks assembled into the LLM prompt |
| `CONVERSATION_HISTORY_TURNS` | `4` | Past turns (8 messages) included in prompt |

### File Scanning

| Variable | Value |
|----------|-------|
| `SKIP_DIRS` | `_0RAG`, `.git`, `__pycache__`, `node_modules`, `.DS_Store`, `venv` |
| `SUPPORTED_EXTENSIONS` | 28 formats: `pdf`, `doc`, `docx`, `xls`, `xlsx`, `xlsm`, `ppt`, `pptx`, `png`, `jpg`, `jpeg`, `gif`, `bmp`, `tiff`, `txt`, `html`, `htm`, `rtf`, `csv`, `md`, `dwg`, `dxf`, `mp4`, `avi`, `mov`, `mkv` |

## Database Schema

### Entity Relationship Diagram

```
┌───────────────┐       ┌──────────────┐       ┌───────────────┐
│     File      │──1:M──│   FileTag    │──M:1──│     Tag       │
│               │       │              │       │               │
│ id (PK)       │       │ id (PK)      │       │ id (PK)       │
│ relative_path │       │ file_id (FK) │       │ name (UNIQUE) │
│ filename      │       │ tag_id (FK)  │       │ display_name  │
│ extension     │       │ source       │       │ description   │
│ size_bytes    │       │ confidence   │       │ color         │
│ modified_time │       │ created_at   │       │ file_count    │
│ parent_dir    │       └──────────────┘       └───────────────┘
│ content_hash  │
│ scan_status   │   ┌───────────────┐       ┌──────────────┐
│ text_preview  │   │ Conversation  │──1:M──│   Message    │
│ chunk_count   │   │               │       │              │
│ processed_at  │   │ id (PK)       │       │ id (PK)      │
│ auto_tagged   │   │ title         │       │ conv_id (FK) │
│ auto_tag_conf │   │ selected_tags │       │ role         │
└───────────────┘   │ created_at    │       │ content      │
                    │ updated_at    │       │ sources (JSON│
                    │ deleted_at    │       │ created_at   │
                    └───────────────┘       └──────────────┘
```

### Key Field Values

| Field | Values | Meaning |
|-------|--------|---------|
| `File.scan_status` | `new`, `processed`, `failed`, `skipped` | Processing lifecycle |
| `FileTag.source` | `auto`, `manual`, `folder_hint` | How the tag was assigned |
| `FileTag.confidence` | 0.0 to 1.0 | Classification confidence (0.7 heuristic, 0.8 LLM) |
| `Conversation.deleted_at` | `NULL` or timestamp | `NULL` = active; timestamp = in trash |
| `Message.role` | `user`, `assistant`, `system` | Message author |
| `Message.sources` | JSON array | `[{filename, file_path, page, chunk_index, relevance_score}]` |

### Migrations

On startup, `init_db()` creates all tables if missing and runs a migration to add `deleted_at` to the conversations table (for soft delete support). `seed_tags()` inserts the 11 default geotechnical tags if the tag table is empty.

## Frontend Architecture

### Technology Choices

The frontend is intentionally **framework free**. Vanilla JavaScript with module pattern controllers (IIFEs) and PicoCSS for styling. No build step, no transpilation, no node_modules. Static files served directly by FastAPI.

### Controllers

| File | Lines | Responsibility |
|------|-------|----------------|
| `chat.js` | 871 | Welcome state, tag selection, message submission, SSE streaming, conversation sidebar, trash management, depth settings, cross tab mirroring |
| `documents.js` | 385 | File table rendering, search (debounced 300ms), type/status/tag filters, pagination, single and batch tagging, tag modal |
| `panel.js` | 381 | Library panel lifecycle (scan, process, complete), processing status polling (2s interval), reprocess all confirmation |
| `router.js` | 55 | SPA routing via History API, page toggle, `spa:pageshow` custom event for lazy init |
| `utils.js` | 65 | `apiGet`/`apiPost`/`apiDelete` wrappers, `escapeHtml`, tag badge factory, `syncChannel` (BroadcastChannel) |

### Cross Tab Communication

Two BroadcastChannel instances coordinate state across browser tabs:
- **`app-sync`**: Non streaming events (conversation created/deleted, processing started/stopped)
- **`chat-streaming`**: Real time streaming coordination (stream started, token, stream ended)

When Tab A starts streaming, Tab B detects this via polling `GET /api/chat/streaming`, then connects to `GET /api/chat/stream-mirror` to receive the same token stream.

### Styling

- **Brand color:** `#14bf98` (teal/mint)
- **CSS variables:** `--brand-color`, `--sidebar-width` (280px), `--chat-max-width` (900px), `--panel-width` (340px)
- **Responsive:** Sidebar hides at 768px or narrower viewport width
- **Animations:** Tag pill pop in (0.3s), panel slide out (0.3s), loading dots spinner
- **Theme:** Light mode only (PicoCSS dark mode ready but not wired up)

### Routing

Two modes supported:
- **SPA mode** (`spa.html`): Single HTML page with both chat and documents sections toggled by client side router using `history.pushState`
- **Server rendered mode**: Separate `index.html` and `documents.html` pages with full page reloads

The SPA router fires a custom `spa:pageshow` event, which triggers lazy initialization of the documents page on first visit.

## Scripts Reference

### `start.sh`

```
1. Detect Python 3.11 (fallback to python3)
2. Create venv/ if missing, then pip install requirements.txt
3. mkdir -p data/{chroma,cache,logs}
4. curl LM Studio at 127.0.0.1:1234 (warn if unreachable)
5. exec uvicorn backend.app:app --host 0.0.0.0 --port 3000 --reload
```

### `kill.sh`

```
1. ps aux | grep "_0RAG/" to collect PIDs
2. sudo kill each PID
```

---

## License

This project is licensed under the **GNU Affero General Public License v3.0 (AGPLv3)**.

You are free to use, modify, and distribute this software. If you run a modified version of GeoRAG as a network service, you must make your source code available to users of that service under the same license.

See [LICENSE](LICENSE) for the full text.
