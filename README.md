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
- [Step 1: Set Up llama-server](#step-1-set-up-llama-server)
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

**Key principle:** Your data never leaves your computer. The AI models run locally through [llama.cpp](https://github.com/ggml-org/llama.cpp)'s `llama-server` (open source, free). There are no subscriptions, no cloud uploads, and no API costs.

## What Can It Do?

| Capability | What It Means |
|-----------|---------------|
| **Read your documents** | Scans your `Engineering/` folder and extracts text from PDFs, Word, Excel, PowerPoint, images, and more |
| **OCR scanned PDFs** | Automatically OCR scanned/image-based PDFs that have no embedded text, making them permanently searchable |
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
2. **`llama-server`** (from [llama.cpp](https://github.com/ggml-org/llama.cpp)), two instances run locally, one for chat (port 8001) and one for embeddings (port 8002)

---

# Part 2: Getting Started

## What You Need

| Requirement | Minimum | Recommended |
|-------------|---------|-------------|
| **Computer** | 16 GB RAM | 32 GB+ RAM |
| **Disk space** | 10 GB free | 50 GB+ (AI models are large) |
| **Operating system** | macOS 14+, Windows 10+, or Ubuntu 20.04+ | macOS with Apple Silicon (M1/M2/M3/M4) |
| **Python** | 3.10 | 3.11 or newer |
| **Tesseract OCR** | 4.x+ | Latest (`brew install tesseract` on macOS) |
| **llama-server** | Recent build of [llama.cpp](https://github.com/ggml-org/llama.cpp), on your `PATH` | Apple Silicon Metal build or CUDA build matching your GPU |
| **GPU / VRAM** | 12 GB VRAM (or Apple Silicon unified memory) | 16 GB+ — Apple Silicon (M1/M2/M3/M4) or NVIDIA GPU |

> **Why 12 GB?** GeoRAG always launches its chat model with a fixed 131,072-token (128K) context window, so every Retrieval Depth level — including Ludicrous — works out of the box without any reconfiguration. The Qwen 3.5 9B weights, the vision projector, and the 128K KV cache (quantized to q8_0) together need roughly 12 GB of GPU/unified memory; the embedding model adds a bit more. Machines with less will fall back to slow CPU inference or fail to load the model.

## Step 1: Set Up llama-server

GeoRAG launches and manages **two `llama-server` processes** (from [llama.cpp](https://github.com/ggml-org/llama.cpp)) itself — you don't start them by hand:

| Role | Port | Default model | Alias |
|------|------|---------------|-------|
| Chat (with vision) | 8001 | Qwen 3.5 9B + mmproj sidecar | `qwen3.5-9B` |
| Embeddings | 8002 | Nomic embed text v1.5 | `nomic-embed-text-v1.5` |

The FastAPI backend (`backend/services/llama_supervisor.py`) spawns both processes on startup, tracks their PIDs, and can stop/restart them — for a manual pause from the UI, or automatically during a scheduled downtime window. All you need to do is install `llama-server` and point GeoRAG at your model files.

### Install llama.cpp

Build or install from the upstream project: <https://github.com/ggml-org/llama.cpp>. On macOS the simplest route is `brew install llama.cpp`. After install, `llama-server --version` should print a version, and `llama-server` must be on your `PATH` (the app shells out to the `llama-server` binary by name).

### Download the GGUF model files

Place GGUFs anywhere; the examples below assume `~/LLMs/`. You need **three** files:

```
~/LLMs/lmstudio-community/Qwen3.5-9B-GGUF/Qwen3.5-9B-Q4_K_M.gguf
~/LLMs/lmstudio-community/Qwen3.5-9B-GGUF/mmproj-Qwen3.5-9B-BF16.gguf
~/LLMs/second-state/Nomic-embed-text-v1.5-Embedding-GGUF/nomic-embed-text-v1.5-Q8_0.gguf
```

The `mmproj-*.gguf` sidecar is the multimodal projector for the chat model. Without it, llama-server cannot caption images and `backend/services/extractors/image_extractor.py` will fail on PNG/JPG documents.

Both Qwen GGUFs ship together in `lmstudio-community/Qwen3.5-9B-GGUF`; the embedding GGUF lives in `second-state/Nomic-embed-text-v1.5-Embedding-GGUF`.

Point GeoRAG at your model file paths by editing `LLAMA_CHAT_MODEL`, `LLAMA_CHAT_MMPROJ`, and `LLAMA_EMBED_MODEL` in `backend/config.py`.

### Reference: exact launch arguments

You don't need to run these yourself — this is what `llama_supervisor.py` launches automatically, shown here for reference (e.g. if you want to reproduce the exact behavior outside the app, or tune parameters in `backend/config.py`).

<details>
<summary>Chat server (port 8001)</summary>

```bash
llama-server \
  --model ~/LLMs/lmstudio-community/Qwen3.5-9B-GGUF/Qwen3.5-9B-Q4_K_M.gguf \
  --mmproj ~/LLMs/lmstudio-community/Qwen3.5-9B-GGUF/mmproj-Qwen3.5-9B-BF16.gguf \
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
  --chat-template-kwargs '{"preserve_thinking": true}'
```

`--chat-template-kwargs '{"preserve_thinking": true}'` keeps Qwen 3.5's `<think>...</think>` reasoning blocks in the streamed reply. The chat UI auto-collapses them after the model finishes thinking, with a click-to-expand toggle. `-c 131072` is fixed regardless of the Retrieval Depth setting — see the note above.

</details>

<details>
<summary>Embedding server (port 8002)</summary>

```bash
llama-server \
  --model ~/LLMs/second-state/Nomic-embed-text-v1.5-Embedding-GGUF/nomic-embed-text-v1.5-Q8_0.gguf \
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
  -ngl 99
```

</details>

The `--alias` values **must match** `CHAT_MODEL` and `EMBEDDING_MODEL` in `backend/config.py`. If you change one, change the other.

> GeoRAG starts both servers automatically when the app launches. Watch the status pill in the top bar: it reads "Starting LLMs" while the models load, "LLMs paused" if manually paused (click to resume), and shows a warning if a server fails to start — check `data/logs/llama-chat.log` / `data/logs/llama-embed.log` for details. Both processes are also stopped and restarted automatically during any scheduled downtime window you configure.

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
4. Check that both llama-server instances (ports 8001 and 8002) are reachable
5. Start the web server

You'll see:
```
===============================
  GeoRAG - Geotechnical RAG
===============================

llama-server (chat :8001, embeddings :8002): Connected
Starting GeoRAG server...
  URL: http://localhost:3000
```

Open **http://localhost:3000** in your browser.

### The Manual Way

If you prefer to control each step:

```bash
# Install Tesseract (required for OCR of scanned PDFs)
brew install tesseract             # macOS
# or: sudo apt install tesseract-ocr  # Ubuntu/Debian

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

3. **OCR** (if needed): After processing, some scanned/image-based PDFs may fail because they have no embedded text. Click "OCR Failed Files (N)" in the Library Panel. GeoRAG will use Tesseract to add a text layer to each PDF, overwriting the original so it becomes permanently searchable. After OCR completes, click "Process New Files" to run the normal pipeline on the now-readable PDFs.

4. **Chat**: Once processing finishes, go to the Chat page, select one or more topic tags, and start asking questions.

> Processing speed depends on your hardware and the number of files. The embedding llama-server (port 8002) is the bottleneck. You can stop and resume processing at any time; already processed files won't be redone.

## Using the Chat

- **Select topics first**: On the welcome screen, click one or more topic tags (e.g., "Piling", "Deep Excavation"). This tells GeoRAG which document collections to search.
- **Ask anything**: Type your question and press Send. The AI will find relevant document sections and write an answer.
- **Check the sources**: Below each answer, you'll see which documents were cited, with page numbers and relevance scores. Click a source to open the original file.
- **Adjust depth**: Click the gear icon next to the tag bar to control how many document chunks the AI considers:

| Depth Level | Speed | Detail | Good For |
|-------------|-------|--------|----------|
| Quick and less demanding | Fastest | Brief answers | Simple factual lookups |
| Optimal and balanced | Balanced | Good detail | Most questions (default) |
| Deep and demanding | Slower | Thorough | Multi-document analysis |
| Deeper and very demanding | Slow | Very detailed | Complex technical questions |
| Ludicrous | Slowest | Maximum context | When you need everything |

- **Conversations are saved**: Use the sidebar on the left to switch between past conversations.
- **Trash and restore**: Delete a conversation and it goes to trash. You can restore it or permanently delete it.

## Managing Your Documents

Go to the **Documents** page (`/documents`) to:

- **Search**: Find files by name
- **Filter**: Narrow by file type (PDF, Word, Excel, etc.), processing status, or topic tag
- **Tag files**: Click a file to manage its tags, or select multiple files for batch tagging. The stats bar and tag filter counts update live after every tag change.
- **Manage tags**: Click "Manage Tags" to create, edit, or delete topic tags. You can change a tag's display name, color, and description. The internal slug is immutable once created. Deleting a tag removes it from all files and deletes its vector collection. Tag creation and deletion are disabled while processing is running.
- **Open files**: Click the filename to open it directly in your system's default application
- **See stats**: The top bar shows total file counts and processing status, refreshed automatically when tags are assigned or removed

## Stopping the Server

```bash
# Option 1: Press Ctrl+C in the terminal where start.sh is running

# Option 2: Run the kill script
./kill.sh
```

## File Types That Work

| Type | Formats | What Gets Extracted |
|------|---------|--------------------|
| **Documents** | PDF | Full text with page boundaries. Scanned/image-based PDFs can be OCR'd via the Library Panel to add a searchable text layer. |
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

### OCR button is disabled or missing
- Make sure Tesseract is installed: `brew install tesseract` (macOS) or `apt install tesseract-ocr` (Linux)
- Make sure `ocrmypdf` is installed: `pip install ocrmypdf`
- The button only appears when there are failed PDF files. Process your library first; scanned PDFs that fail text extraction will show up as candidates for OCR.

### OCR fails on some files
- Some PDFs may be corrupted or password-protected; OCR will skip those and continue to the next file
- Check the error list in the Library Panel for details on individual failures
- For non-English documents, add the language code to `OCR_LANGUAGES` in `backend/config.py` (e.g., `["eng", "tur"]` for English + Turkish). You may also need to install the Tesseract language pack (e.g., `brew install tesseract-lang`).

### "Local LLM Servers Not Available" / "llama-server not detected"
GeoRAG auto-launches both llama-server instances on startup; this means one or both failed to come up. Check:
- `llama-server` is installed and on your `PATH` (`llama-server --version`)
- The model files exist at the paths configured in `backend/config.py` (`LLAMA_CHAT_MODEL`, `LLAMA_CHAT_MMPROJ`, `LLAMA_EMBED_MODEL`)
- `data/logs/llama-chat.log` and `data/logs/llama-embed.log` for the actual startup error

Once running, verify with:
```bash
curl http://127.0.0.1:8001/v1/models   # chat
curl http://127.0.0.1:8002/v1/models   # embeddings
```

### Chat gives empty or broken responses
- Check `data/logs/llama-chat.log` for errors from the chat llama-server (port 8001)
- Make sure `--alias qwen3.5-9B` matches `CHAT_MODEL` in `backend/config.py` — if you changed the model, update both `LLAMA_CHAT_MODEL`/`--alias` and `CHAT_MODEL` to match

### Processing fails on embeddings
- Make sure the embedding llama-server (port 8002) is running with `--embeddings` and `--alias nomic-embed-text-v1.5`
- The chat server alone is not sufficient; both processes must be up

### Image extraction fails
- The chat llama-server must be started with `--mmproj <path-to-mmproj.gguf>`. Without it, llama-server cannot accept images and `image_extractor.py` will mark image files as failed.

### Processing is really slow
The local llama-servers are doing all the AI work. To speed things up:
- Pass `-ngl 99` (or as many layers as fit) to push the model onto your GPU
- Use a smaller/faster chat GGUF (e.g. a smaller Qwen quantization)
- Close other apps competing for GPU/RAM

### Files don't show up after scanning
- Files must be inside the `Engineering/` parent directory (not inside `_0RAG/` itself)
- The file extension must be supported (see the table above)
- Folders named `.git`, `__pycache__`, `node_modules`, and `venv` are skipped

### Port 3000 is already in use
```bash
lsof -i :3000        # See what's using it
./kill.sh            # Or just kill all GeoRAG processes
```

### Virtual environment broken after Python reinstall
If you reinstall or upgrade Python, the existing `venv/` will still reference the old Python binary and you'll see errors like `cannot execute: required file not found`. Delete the venv and let `start.sh` recreate it:
```bash
rm -rf venv/
./start.sh
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
+--------------------------------------+   +---------------------------+
|  llama-server (chat, :8001)          |   | Data Storage              |
|  +--------------------------------+  |   |                           |
|  | /v1/chat/completions           |  |   | +---------------------+   |
|  | (qwen3.5-9B + mmproj vision)   |  |   | | ChromaDB            |   |
|  +--------------------------------+  |   | | (vectors)           |   |
+--------------------------------------+   | +---------------------+   |
+--------------------------------------+   | | SQLite              |   |
|  llama-server (embeddings, :8002)    |   | | (metadata)          |   |
|  +--------------------------------+  |   | +---------------------+   |
|  | /v1/embeddings                 |  |   +---------------------------+
|  | (nomic-embed-text-v1.5)        |  |
|  +--------------------------------+  |
+--------------------------------------+
```

**Three tier layout:**
- **Browser**: Vanilla JS frontend served as static files. Chat page, documents page, and library panel. Tabs sync via BroadcastChannel API.
- **FastAPI Backend** (port 3000): Python async server handling API requests, orchestrating the RAG pipeline, and streaming responses via SSE.
- **Two `llama-server` instances** (ports 8001 / 8002): Local LLM runtimes exposing OpenAI-compatible APIs. The chat server runs the chat model plus its multimodal projector for image captioning; the embedding server runs the embedding model.

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
    → Embedding Client: batch embed via embedding llama-server /v1/embeddings
    → Vector Store: upsert into ChromaDB per tag collections
    → SQLite: update status=processed, save chunk count + text preview
```

### Chat Query

```
User message + selected tags
    → Embed query via embedding llama-server /v1/embeddings
    → Query each selected tag's ChromaDB collection (top K per tag)
    → Deduplicate across tags, rank by cosine similarity
    → Assemble system prompt with top context chunks
    → Append last 4 conversation turns (8 messages)
    → Stream completion from chat llama-server /v1/chat/completions
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
| Images | llama-server vision (`--mmproj`) | Base64 encoded image sent to chat model for description (max 10 MB) |
| Text/HTML/RTF/CSV/MD | Built in | Direct read with HTML tag stripping; UTF 8 to latin 1 fallback |
| DWG/DXF | N/A | Filename indexed only, no content extraction |

Failed extractions mark the file as `status=failed` and log the error. For PDFs, this typically means the file is a scanned image with no embedded text layer. These can be recovered via the OCR pipeline (see below).

### Phase 2b: OCR Recovery (`services/ocr_processor.py`)

An optional step triggered from the Library Panel after initial processing. Targets only PDFs with `status=failed`:

1. Queries all files where `scan_status=failed` and `extension=pdf`
2. For each file, runs `ocrmypdf` (Tesseract) in a background thread via `asyncio.to_thread()`
3. OCR writes to a temporary file, then `shutil.move()` replaces the original (no corruption on failure)
4. Uses `skip_text=True` to handle mixed PDFs (some pages already have text)
5. On success: resets `scan_status=new`, clears `extracted_text_preview` and `chunk_count`
6. On failure: file stays `failed`, error logged, pipeline continues to next file

After OCR completes, the user clicks "Process New Files" to run the normal pipeline on the now-readable PDFs.

**Mutual exclusion:** OCR, processing, and tag exploration all block each other. Only one can run at a time.

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

Chunks are batched (up to 128 per request) and sent to the embedding llama-server's `/v1/embeddings` endpoint using the `nomic-embed-text-v1.5` model. Three concurrent embedding requests are allowed (semaphore controlled). Each chunk becomes a 768 dimensional float vector.

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
7. The full prompt is streamed to the chat llama-server via `/v1/chat/completions` with `stream=true`
8. Tokens are forwarded to the client as SSE events (`event: token`, `data: {"token": "..."}`)
9. On completion, a `done` event sends `{conversation_id, sources}` and the message is persisted

**Stream mirroring:** Other browser tabs can connect to `GET /api/chat/stream-mirror` to receive the same token stream in real time via async queues.

**Cancellation:** `POST /api/chat/stop` sets a cancellation flag that the streaming loop checks between tokens.

### Retrieval Depth Presets

| Level | Top K Per Tag | Max Context Chunks |
|-------|--------------|-------------------|
| Quick and less demanding | 5 | 8 |
| Optimal and balanced | 10 | 16 |
| Deep and demanding | 20 | 32 |
| Deeper and very demanding | 50 | 80 |
| Ludicrous | 100 | 200 |

All levels are served by the same chat llama-server, which always launches with a fixed 131,072-token context window — no restart or reconfiguration is needed when switching depths (see [What You Need](#what-you-need)).

## Concurrency Model

| Resource | Limit | Mechanism | Rationale |
|----------|-------|-----------|-----------|
| File extraction | 10 parallel | `asyncio.gather` batch | I/O bound, benefits from parallelism |
| LLM tagging | 1 | `asyncio.Semaphore(1)` | llama-server single-slot serialization |
| Embedding requests | 3 | `asyncio.Semaphore(3)` | Balance throughput vs. embedding llama-server capacity |
| Database writes | 1 | `asyncio.Lock` | SQLite transaction safety |
| OCR processing | 1 file | Sequential | CPU-intensive Tesseract, avoids thrashing |
| Pipeline exclusion | 1 pipeline | Mutual exclusion checks | Processing, OCR, and exploration block each other |
| Embedding batch size | 128 texts | Config constant | llama-server request size limit |
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
| httpx | 0.28.1 | Async HTTP client (llama-server communication) |
| pdfplumber | 0.11.4 | PDF text extraction |
| PyMuPDF | 1.25.1 | PDF support |
| python-docx | 1.1.2 | Word document parsing |
| openpyxl | 3.1.5 | Excel parsing |
| python-pptx | 1.0.2 | PowerPoint parsing |
| langchain-text-splitters | 0.3.4 | Recursive character text splitting |
| sse-starlette | 2.2.1 | Server Sent Events |
| ocrmypdf | latest | OCR scanned PDFs via Tesseract (requires system `tesseract` binary) |
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
| llama-server (chat) | 8001 | Local chat model + vision (OpenAI compatible API) |
| llama-server (embeddings) | 8002 | Local embedding model (OpenAI compatible API) |
| Tesseract OCR | N/A | System binary for OCR (used by `ocrmypdf`) |

## Project Structure

```
_0RAG/
├── start.sh                        # Startup: venv, deps, llama-server check, server
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
│   │   ├── processing.py           # Scan, start/stop pipeline, onboarding
│   │   ├── explore.py              # Tag exploration: discover new categories
│   │   └── ocr.py                  # OCR start/stop/status for failed PDFs
│   │
│   └── services/
│       ├── document_processor.py   # Pipeline orchestrator (batch + concurrency)
│       ├── ocr_processor.py        # OCR scanned PDFs via ocrmypdf/Tesseract
│       ├── scanner.py              # Filesystem walk + DB synchronization
│       ├── chunker.py              # RecursiveCharacterTextSplitter wrapper
│       ├── embedding_client.py     # Async batch embeddings via llama-server
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
| `GET` | `/api/tags` | All tags with live file counts. |
| `POST` | `/api/tags` | Create tag. Body: `{name, display_name, description?, color?}`. |
| `PUT` | `/api/tags/{id}` | Update tag. Body: `{display_name?, description?, color?}`. Slug is immutable. |
| `DELETE` | `/api/tags/{name}` | Delete tag, all file associations, and its vector collection. |

### Processing Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/processing/scan` | Scan filesystem, sync DB. Returns `{files_found, files_new, files_removed}`. |
| `POST` | `/api/processing/start` | Start pipeline. Body: `{tag_names?, file_ids?, reprocess?}`. |
| `POST` | `/api/processing/stop` | Gracefully stop processing. |
| `GET` | `/api/processing/status` | Pipeline state: `{is_running, total_files, processed_files, failed_files, skipped_files, current_file, errors}`. |
| `GET` | `/api/processing/onboarding-status` | Full onboarding state with phase, progress, extension breakdown, and OCR status. |
| `POST` | `/api/processing/onboarding-dismiss` | Dismiss onboarding wizard. |
| `DELETE` | `/api/processing/onboarding-dismiss` | Reset onboarding visibility. |

### OCR Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/ocr/start` | Start OCR on all failed PDF files. Mutually exclusive with processing and exploration. |
| `POST` | `/api/ocr/stop` | Gracefully stop OCR after current file. |
| `GET` | `/api/ocr/status` | OCR state: `{is_running, total_files, processed_files, ocr_success, ocr_failed, current_file, errors}`. |

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

### llama-server Connection

| Variable | Default | Description |
|----------|---------|-------------|
| `CHAT_BASE_URL` | `http://127.0.0.1:8001` | Chat llama-server root |
| `EMBEDDING_BASE_URL` | `http://127.0.0.1:8002` | Embedding llama-server root |
| `CHAT_URL` | `{CHAT_BASE_URL}/v1/chat/completions` | Chat completion endpoint |
| `EMBEDDING_URL` | `{EMBEDDING_BASE_URL}/v1/embeddings` | Embedding endpoint |
| `CHAT_MODEL` | `qwen3.5-9B` | Chat model name (must match `--alias` on the chat llama-server) |
| `EMBEDDING_MODEL` | `nomic-embed-text-v1.5` | Embedding model name (must match `--alias` on the embedding llama-server) |
| `EMBEDDING_DIM` | `768` | Vector dimensionality |
| `LLM_PARALLEL_SLOTS` | `1` | Concurrent inference slots (must match the chat server's `-np`) |

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

### OCR

| Variable | Default | Description |
|----------|---------|-------------|
| `OCR_LANGUAGES` | `["eng"]` | Tesseract language codes for OCR. Add codes for other languages (e.g., `["eng", "tur"]`). Requires corresponding Tesseract language packs. |

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
| `panel.js` | 670 | Library panel lifecycle (scan, process, OCR, explore, complete), processing status polling (2s interval), OCR start/stop, reprocess all confirmation |
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
4. curl llama-server at 127.0.0.1:8001 and 127.0.0.1:8002 (warn if either unreachable)
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
