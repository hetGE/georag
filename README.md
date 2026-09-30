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
- [Scheduled Downtime](#scheduled-downtime)
- [Backup and Restore](#backup-and-restore)
- [Stopping the Server](#stopping-the-server)
- [File Types That Work](#file-types-that-work)
- [Built-In Topic Tags](#built-in-topic-tags)
- [Common Issues & Fixes](#common-issues--fixes)

### [Part 3: Technical Reference](#part-3-technical-reference)
- [System Architecture](#system-architecture)
- [Data Flow](#data-flow)
- [Processing Pipeline Deep Dive](#processing-pipeline-deep-dive)
- [Chat & Retrieval Internals](#chat--retrieval-internals)
- [Scheduler & Downtime](#scheduler--downtime)
- [llama-server Supervisor](#llama-server-supervisor)
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
| **Semantic document search** | On the Documents page, search file contents by meaning (embedding similarity), not just filename |
| **Work across tabs** | Open multiple browser tabs and they stay in sync |
| **Open source files** | Click any cited document to open it directly on your machine |
| **Schedule downtime** | Define a daily window in which the local LLMs shut down to free memory, then restart automatically when the window ends |
| **Back up and restore** | Export your RAG data to a portable `.georag` archive and import it on another machine |

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
| **Disk space** | 8 GB free | 20 GB+ |
| **Operating system** | macOS 14+, Windows 10+, or Ubuntu 20.04+ | macOS with Apple Silicon (M1/M2/M3/M4) |
| **Python** | 3.10 | 3.11 to 3.13 |
| **Tesseract OCR** | 4.x+ | Latest (`brew install tesseract` on macOS) |
| **llama-server** | Recent build of [llama.cpp](https://github.com/ggml-org/llama.cpp), on your `PATH` | Apple Silicon Metal build or CUDA build matching your GPU |
| **GPU / VRAM** | 8 GB VRAM, or any 16 GB Apple Silicon Mac (unified memory). Runs the Quick and Optimal Retrieval Depth levels with the default Qwen 3.5 9B model | 16 GB VRAM for Deep and Deeper, 24 GB for Ludicrous (see [Retrieval Depth Presets](#retrieval-depth-presets)) |
| **Optional larger model** | Not needed | Qwen 3.8 27B (MLX): an Apple Silicon Mac with 32 GB for Quick and Optimal, 36 GB for Deep and Deeper, 48 GB for Ludicrous (see [Optional: the MLX chat model](#optional-the-mlx-chat-model)) |

> **Memory footprint (default model).** The default chat model is Qwen 3.5 9B (a 4-bit `Q4_K_M` quant), and the three GGUF model files are only around 7 GB on disk. What decides GPU memory beyond the weights is the **context window** the chat model is loaded with, because it has to hold the retrieved chunks, the conversation history and the reply. GeoRAG therefore loads a different context size per VRAM tier, chosen by the Retrieval Depth level in use:
>
> | Assumed VRAM | Retrieval Depth levels | Context window | Chat model | Total with vision projector + embedding model |
> |---|---|---|---|---|
> | 8 GB | Quick, Optimal | 32,768 tokens | ~5.9 GB | ~7.7 GB |
> | 16 GB | Deep, Deeper | 131,072 tokens | ~7.5 GB | ~9.3 GB |
> | 24 GB | Ludicrous | 262,144 tokens (the model's maximum) | ~10.0 GB | ~11.8 GB |
>
> The memory figures are llama.cpp's own projections for this model with a q8_0 KV cache, measured on Apple Silicon; the vision projector adds about 1.1 GB and the embedding model about 0.7 GB. These are GPU-memory figures; each server also holds roughly 0.5 to 1 GB of ordinary RAM for CPU-side buffers, so the "frees about" total in the pause dialog, which counts everything actually allocated, is higher (13.6 GB with Ludicrous loaded). The 8 GB tier is a tight fit, while the 16 GB and 24 GB tiers leave generous headroom for the desktop, other applications and llama.cpp's 1 GB safety margin. The tier is an assumption, not a gate: when you pick a level, GeoRAG reloads the model with that context and asks llama.cpp whether it actually fits. If it does not, the level is refused with an error before any chat starts, and the previous context is restored. The base tier is the exception: it always runs, and on machines smaller than the minimum llama.cpp (`--fit on`) moves some layers to the CPU, which is slower.
> **Memory footprint (Qwen 3.8 27B, MLX).** The optional larger model behaves differently. Its weights alone take 15 GB, nothing is preallocated, and the MLX runtime's context cache costs about 0.25 GB per 1,000 tokens (roughly 16 times llama.cpp's quantized cache for the 9B). Its tiers therefore use much smaller context budgets:
>
> | Assumed Mac (unified memory) | Retrieval Depth levels | Context budget | Estimated need with a full context |
> |---|---|---|---|
> | 32 GB | Quick, Optimal | 24,576 tokens | ~22.6 GB |
> | 36 GB | Deep, Deeper | 40,960 tokens | ~26.8 GB |
> | 48 GB | Ludicrous | 73,728 tokens | ~35.0 GB |
>
> Measured on a 36 GB M4 Max with mlx-lm 0.31: 15.0 GB loaded, 18.0 GB with a 12,000-token prompt, 21.0 GB with 24,000 tokens. macOS lets the GPU use about 28 GB on that machine, so Quick through Deeper run and Ludicrous is refused. The need is an estimate checked before a chat starts (there is no load-time verdict to read as with llama.cpp); typical use stays well below it because the context is rarely full.

## Step 1: Set Up llama-server

GeoRAG launches and manages **two `llama-server` processes** (from [llama.cpp](https://github.com/ggml-org/llama.cpp)) itself; you don't start them by hand:

| Role | Port | Default model | Alias |
|------|------|---------------|-------|
| Chat (with vision) | 8001 | Qwen 3.5 9B + mmproj sidecar (or, when selected, Qwen 3.8 27B served by `mlx_lm.server`) | `qwen3.5-9B` |
| Embeddings | 8002 | Nomic embed text v1.5 | `nomic-embed-text-v1.5` |

The FastAPI backend (`backend/services/llama_supervisor.py`) spawns both processes on startup, tracks their PIDs, and can stop/restart them, whether for a manual pause from the UI or automatically during a scheduled downtime window. All you need to do is install `llama-server` and point GeoRAG at your model files.

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

You don't need to run these yourself; this is what `llama_supervisor.py` launches automatically, shown here for reference (e.g. if you want to reproduce the exact behavior outside the app, or tune parameters in `backend/config.py`).

<details>
<summary>Chat server (port 8001)</summary>

```bash
llama-server \
  --model ~/LLMs/lmstudio-community/Qwen3.5-9B-GGUF/Qwen3.5-9B-Q4_K_M.gguf \
  --mmproj ~/LLMs/lmstudio-community/Qwen3.5-9B-GGUF/mmproj-Qwen3.5-9B-BF16.gguf \
  --port 8001 \
  --alias qwen3.5-9B \
  -c 32768 \
  -n -1 \
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

Qwen can reason ("think") before it answers, but GeoRAG turns that off for chat (`CHAT_THINKING = False` in `backend/config.py`, sent with every chat request as `chat_template_kwargs: {"enable_thinking": false}`): the answer is grounded in the retrieved text, and the reasoning pass mostly added waiting. With `CHAT_THINKING = True` the reasoning is streamed separately from the answer (`reasoning_content`), forwarded wrapped in `<think>...</think>`, and shown in the chat as a block that auto-collapses once the model finishes thinking, with a click-to-expand toggle. The reply is not limited either way: `-n -1` lets the model generate until it stops by itself or the context window is full. `-c` is shown here for the base tier; the app sets it to 32768, 131072 or 262144 according to the Retrieval Depth level in use (see the note above).

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

The `--alias` values **must match** the chat model's `request_model` in `CHAT_MODELS` and `EMBEDDING_MODEL` in `backend/config.py`. If you change one, change the other.

> GeoRAG starts both servers automatically when the app launches. Watch the status pill in the top bar: it reads "Starting LLMs" while the models load, "LLMs paused" if manually paused (click to resume), and shows a warning if a server fails to start; check `data/logs/llama-chat.log` / `data/logs/llama-embed.log` for details. Both processes are also stopped and restarted automatically during any scheduled downtime window you configure.

### Optional: the MLX chat model

On an Apple Silicon Mac, GeoRAG can serve a second, larger chat model, **Qwen 3.8 27B (MLX 4-bit)**, in place of the 9B. You choose between them in the **Model and Retrieval Depth** dialog (the gear icon in the chat), and the choice is remembered across restarts. Only one chat model is loaded at a time: switching stops one server and starts the other on the same port.

MLX models cannot be loaded by llama.cpp, so this model is served by `mlx_lm.server` from the [mlx-lm](https://github.com/ml-explore/mlx-lm) package. That runtime lives in its own virtualenv, `venv-mlx/`, because it needs a newer `tokenizers` than the `chromadb` pinned in `requirements.txt` allows.

Setup:

1. Put the model folder at `~/LLMs/lmstudio-community/Qwen3.8-27B-MLX-4bit` (about 15 GB), or point `MLX_CHAT_MODEL` at it in `.env`.
2. Run `./start.sh`. When it finds the model folder on an Apple Silicon Mac it creates `venv-mlx/` and installs `requirements-mlx.txt` (one time, about 430 MB). To do it by hand:

```bash
python3 -m venv venv-mlx
venv-mlx/bin/pip install -r requirements-mlx.txt
```

3. Open the dialog and pick **Qwen 3.8 27B**. If the model folder or the runtime is missing, or the Mac does not have the memory, the card says why and the 9B stays active.

What to expect compared with the 9B (measured on a 36 GB M4 Max):

| | Qwen 3.5 9B (llama.cpp) | Qwen 3.8 27B (MLX) |
|---|---|---|
| Reading the retrieved context | about 465 tokens/s | about 130 to 175 tokens/s (a 24,000-token prompt takes 3 minutes before the first word) |
| Writing the answer | about 35 tokens/s | 21 tokens/s on a short prompt, 12 to 14 at 24,000 tokens |
| Memory | 8 to 14 GB depending on depth | 15 GB plus about 0.25 GB per 1,000 tokens in use |
| Image description during library processing | Yes | No: `mlx_lm.server` is text only, so images indexed while it is selected get no description |
| Reply length | Unlimited: the model answers until it stops or its context window is full | The same, except for a crash guard at 8,192 tokens (the MLX server fails on much longer replies) |
| Memory check | llama.cpp's own fit report at load | Estimate against the Mac's GPU working set, before each chat |

Process your library with the 9B selected if it contains images, then switch to the 27B for chat if you want its answers.

If the MLX server stops producing output part-way through a reply (it does not report its own failures; the request would otherwise hang), GeoRAG gives up after 180 seconds of silence, shows an error in the chat and restarts the model.

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
4. Fail fast if the `llama-server` binary or any configured model file is missing
5. Start the web server, which launches and manages both llama-server instances itself

You'll see:
```
===============================
  GeoRAG - Geotechnical RAG
===============================

Using Python 3.13.x
Virtual environment found.

Starting GeoRAG server (llama-servers launched by the app)...
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
- **Choose the model and depth**: Click the gear icon next to the tag bar. The dialog has two parts. **Chat model** switches between Qwen 3.5 9B and, on an Apple Silicon Mac with it installed, Qwen 3.8 27B (see [Optional: the MLX chat model](#optional-the-mlx-chat-model)). **Retrieval depth** controls how many document chunks the AI considers:

| Depth Level | Speed | Detail | Good For | Assumes (9B) | Assumes (27B MLX) |
|-------------|-------|--------|----------|--------------|-------------------|
| Quick and less demanding | Fastest | Brief answers | Simple factual lookups | 8 GB VRAM | 32 GB Mac |
| Optimal and balanced | Balanced | Good detail | Most questions (default) | 8 GB VRAM | 32 GB Mac |
| Deep and demanding | Slower | Thorough | Multi-document analysis | 16 GB VRAM | 36 GB Mac |
| Deeper and very demanding | Slow | Very detailed | Complex technical questions | 16 GB VRAM | 36 GB Mac |
| Ludicrous | Slowest | Maximum context | When you need everything | 24 GB VRAM | 48 GB Mac |

  Deeper levels need a larger context window and so more memory. Picking a model, or a level in a higher tier, reloads the chat model if needed (a few seconds) and checks that it fits; if your machine does not have the memory, the dialog shows an error and stays on the previous choice. When you switch model, your depth level is kept if the new model can run it here; otherwise the dialog falls back to Optimal and tells you why. The exact limits per level are listed under [Retrieval Depth Presets](#retrieval-depth-presets).

- **Conversations are saved**: Use the sidebar on the left to switch between past conversations.
- **Trash and restore**: Delete a conversation and it goes to trash. You can restore it or permanently delete it.

## Managing Your Documents

Go to the **Documents** page (`/documents`) to:

- **Search**: Find files by name, or tick **Semantic Search** and press Enter to search file *contents* by meaning (the query is embedded and matched against the vector store), so you find relevant documents even when the filename doesn't mention the term.
- **Filter**: Narrow by file type (PDF, Word, Excel, etc.), processing status, or topic tag
- **Reprocess or skip**: Select files and click **Mark as New** to queue them for reprocessing, or **Mark as Skipped** to exclude them from processing.
- **Tag files**: Click a file to manage its tags, or select multiple files for batch tagging. The stats bar and tag filter counts update live after every tag change.
- **Manage tags**: Click "Manage Tags" to create, edit, or delete topic tags. You can change a tag's display name, color, and description. The internal slug is immutable once created. Deleting a tag removes it from all files and deletes its vector collection. Tag creation and deletion are disabled while processing is running.
- **Open files**: Click the filename to open it directly in your system's default application. Use **Open Folder** from the Manage File dialog to reveal it in your file manager.
- **See stats**: The top bar shows total file counts and processing status, refreshed automatically when tags are assigned or removed

## Scheduled Downtime

The local LLM models hold GPU/unified memory while the app runs. If you want your machine free during working hours, define a daily **downtime window** in the system settings (the gear/status area in the top bar).

- During the window, **both llama-servers are shut down** to free GPU/unified memory.
- When the window ends, the servers are restarted automatically.
- If you restart the app in the middle of the window, the servers stay down until the window closes.
- Click **End downtime now** to override the current window immediately (the saved schedule still applies tomorrow).
- The window can wrap past midnight (e.g. `22:00` to `06:00`).

The status pill in the top bar reflects all of this: `Starting LLMs`, `LLMs idle`, `LLMs paused`, `Scheduled pause`, and so on. You can also manually pause the LLMs (to free memory) by clicking the pill; GeoRAG refuses if any LLM work is currently active. The pause confirmation lists what is loaded (the chat model with its current context window, and the embedding model) and how much memory pausing will free, read from llama.cpp's load logs.

## Backup and Restore

From the system menu you can export your data to a single portable `.georag` archive and import it on another machine.

- **Export**: Exports include the **RAG** data (document metadata, tags, vector collections). The export runs in the background; download the archive when it completes.
- **Import**: Upload a `.georag` archive. It is validated first, then imported in the background. Older archives that contain wiki data are still accepted; the wiki parts are ignored.
- Export and import are mutually exclusive with each other and with document processing; GeoRAG returns a busy error rather than corrupting state.

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

GeoRAG seeds **34 built-in geotechnical categories** on first launch (defined in `DEFAULT_TAGS` in `backend/config.py`). Documents are automatically classified into these during processing. The core categories include:

| Tag | Focus |
|-----|-------|
| Piling | Pile foundations, driven/bored piles, load tests |
| Diaphragm Wall | Diaphragm/slurry walls, barrettes |
| Ground Improvement | Compaction, grouting, soil mixing |
| Ground Anchors | Anchors, soil nails, rock bolts, tiebacks |
| Deep Excavation | Shoring, braced cuts, sheet piling, cofferdams |
| Tunneling | Tunnel design, TBM, NATM, cut and cover |
| Earthquake Engineering | Seismic design, liquefaction, dynamic analysis |
| Lab Testing | Triaxial, consolidation, direct shear |
| In-Situ Testing | SPT, CPT, pressuremeter, plate load |
| FEA / Numerical | PLAXIS, finite element / numerical modelling |

...plus categories such as Grouting, Soil Nails, Slope Stability, Shallow Foundations, Soil Mechanics, Geosynthetics, Energy Piles, Transportation Geotechnics, Project Documents, Conference Proceedings, DWG Drawing Templates, and more. See `DEFAULT_TAGS` for the full list.

You can also create your own custom tags through the Documents page (**Manage Tags**) or the API, and use **Explore New Tags** to have the LLM discover new categories in your library.

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
- Make sure `--alias qwen3.5-9B` matches `CHAT_MODEL` in `backend/config.py`; if you changed the model, update both `LLAMA_CHAT_MODEL`/`--alias` and `CHAT_MODEL` to match

### Processing fails on embeddings
- Make sure the embedding llama-server (port 8002) is running with `--embeddings` and `--alias nomic-embed-text-v1.5`
- The chat server alone is not sufficient; both processes must be up

### Image extraction fails
- The chat llama-server must be started with `--mmproj <path-to-mmproj.gguf>`. Without it, llama-server cannot accept images and `image_extractor.py` will mark image files as failed.
- Image description is not available while the MLX chat model (Qwen 3.8 27B) is selected: `mlx_lm.server` is text only. Select Qwen 3.5 9B before processing a library that contains images.

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
- **Browser**: Vanilla JS frontend served as static files. Chat page, documents page, library panel, and a system status pill. Tabs sync via BroadcastChannel API.
- **FastAPI Backend** (port 3000): Python async server handling API requests, orchestrating the RAG pipeline, and streaming responses via SSE.
- **Two `llama-server` instances** (ports 8001 / 8002): Local LLM runtimes exposing OpenAI-compatible APIs. The chat server runs the chat model plus its multimodal projector for image captioning; the embedding server runs the embedding model.

**Storage:**
- **ChromaDB**: On disk vector database. One collection per topic tag, cosine distance metric, 768 dim vectors.
- **SQLite**: File metadata, tag associations, conversations, messages, source citations, and the single-row app settings (downtime schedule). Runs in WAL mode.

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
    → Assemble system prompt with the top document chunks
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

1. The request's Retrieval Depth level is resolved for the chat model in use and the chat server is checked against it (`llama_supervisor.ensure_chat_ctx`): with llama.cpp the model is reloaded first if the level needs a larger context, with MLX the level's memory estimate is checked. If it does not fit in memory the request ends with an `error` event before anything else happens
2. The user's query is embedded using the same Nomic model
3. For each selected tag, the corresponding ChromaDB collection is queried for the level's `top_k` most similar chunks
4. Results from all tags are merged and deduplicated by chunk ID
5. The level's top `max_chunks` are selected by relevance score
6. A system prompt is built containing the context chunks with source metadata
7. The last `CONVERSATION_HISTORY_TURNS` turns (default: 4 turns = 8 messages) are appended, newest first, for as long as they fit: half of the level's context window is kept free for the reply, and older messages that would push the prompt past the other half are dropped
8. The full prompt is streamed to the chat llama-server via `/v1/chat/completions` with `stream=true`, thinking switched off (unless `CHAT_THINKING` is on) and no `max_tokens`, so the model answers for as long as it wants (the MLX server is the exception: it is given what is left of the level's context budget, capped by its crash guard)
9. Tokens are forwarded to the client as SSE events (`event: token`, `data: {"token": "..."}`). With `CHAT_THINKING` on, reasoning tokens arrive first, wrapped in `<think>...</think>`, followed by the answer
10. If the reply ends because the context window filled up, or on MLX at the crash guard (`finish_reason: "length"`), a visible "response cut off" note is appended so a truncated reply is never mistaken for a complete one
11. On completion, a `done` event sends `{conversation_id, sources}` and the message is persisted

Replies stored while thinking was on keep their `<think>` block for display, but it is stripped from the conversation history sent back to the model on later turns.

**Stream mirroring:** Other browser tabs can connect to `GET /api/chat/stream-mirror` to receive the same token stream in real time via async queues.

**Cancellation:** `POST /api/chat/stop` sets a cancellation flag that the streaming loop checks between tokens.

### Retrieval Depth Presets

The levels are defined in `backend/config.py` as `CHAT_DEPTH_LEVELS`, each naming a memory tier (`base`, `mid`, `top`). What a tier means, its context window and the memory it assumes, is defined per chat model in `CHAT_MODELS`. The chat UI loads the result for the active model from `GET /api/system/chat-depths`.

Retrieval is the same for both models:

| Level | Tier | Top K Per Tag | Max Context Chunks |
|-------|------|--------------|-------------------|
| Quick and less demanding | base | 5 | 8 |
| Optimal and balanced | base | 10 | 16 |
| Deep and demanding | mid | 20 | 32 |
| Deeper and very demanding | mid | 50 | 80 |
| Ludicrous | top | 100 | 200 |

| Level | Context Window, Qwen 3.5 9B | Assumed VRAM | Context Budget, Qwen 3.8 27B (MLX) | Assumed Mac |
|-------|-----------------------------|--------------|------------------------------------|-------------|
| Quick and less demanding | 32,768 | 8 GB | 24,576 | 32 GB |
| Optimal and balanced | 32,768 | 8 GB | 24,576 | 32 GB |
| Deep and demanding | 131,072 | 16 GB | 40,960 | 36 GB |
| Deeper and very demanding | 131,072 | 16 GB | 40,960 | 36 GB |
| Ludicrous | 262,144 | 24 GB | 73,728 | 48 GB |

- **No thinking, no reply limit.** The model answers directly (`CHAT_THINKING = False`) and for as long as it wants. A reply ends when the model stops by itself, when you press Stop, or when the context window is full, in which case it ends with a visible "response cut off" note and you can send "continue". The context window is shared by the prompt and the reply, so half of it is kept free for the reply when the prompt is assembled (earlier messages are dropped first).
- **Why thinking is off.** Measured on a 36 GB M4 Max: for a two-sentence question the 9B reasoned for 3,537 tokens (76 seconds) before answering, against 0.6 seconds to the first word with thinking off; on a 50-source question the 27B reasoned for about 7,700 tokens (over 9 minutes) and reached its crash guard without writing an answer. Set `CHAT_THINKING = True` to bring reasoning back; it is then unlimited.
- **One exception on MLX.** `mlx_lm.server` 0.31 fails with a Metal "Resource limit exceeded" error roughly 10,000 tokens into a reply, losing the reply and leaving the request hanging. The 27B therefore has a crash guard, not a budget: a reply stops at 8,192 tokens (`reply_guard_tokens` in `CHAT_MODELS`; set it to `None` to remove it). If the server does fail, GeoRAG reports it after 180 seconds of silence and restarts the model.
- **Context Window / Budget** is what costs memory; see [What You Need](#what-you-need) for the measured figures. For the 9B it is the `-c` llama-server is loaded with. For the 27B nothing is preallocated, so it is a budget: the prompt and reply together are kept inside it, and the memory it would take when full is what gets checked. The 27B's budgets are far smaller because its context cache costs about 16 times more memory per token.

**Switching levels (llama.cpp).** Selecting a level calls `POST /api/system/chat-depth`. If the loaded context is already large enough nothing is restarted. Otherwise the chat server is relaunched with the level's context (a few seconds) and llama.cpp's `--fit` report is read from `data/logs/llama-chat.log`: llama.cpp never refuses an oversized context outright, it logs that it cannot meet its memory target and moves model layers to the CPU, and that log line is what GeoRAG treats as "does not fit". The level is then refused with llama.cpp's numbers, and the server is put back on its previous context. A process that fails to load at all is handled the same way. The context only grows while the app is running (going back to a lower level does not reload the model), and the server is never restarted while other LLM work (library processing, OCR, tag exploration, a streaming chat) is in progress.

**Switching levels (MLX).** `mlx_lm.server` allocates on demand, so there is nothing to reload and no load-time verdict to read: a context that is too big would only show up mid-answer, as swapping or an out-of-memory failure. Instead the level's need is estimated (`llama_supervisor.mlx_memory_estimate`: the weights on disk, plus `kv_bytes_per_token` for every token of the context budget, plus `overhead_bytes`) and compared with the memory macOS lets the GPU use (`max_recommended_working_set_size`, as reported by MLX, less what the embedding server holds). A level that does not fit is refused with those numbers.

**Switching models.** `POST /api/system/chat-model` stops the chat server and starts the other model's backend at its base tier. If the new model cannot be served on this machine (not an Apple Silicon Mac, runtime or model folder missing, not enough memory, or it fails to load) the previous model is restored and the reason returned. The depth level sent with the request is carried over when the new model can run it; otherwise the response falls back to the default level and says why.

The same checks run at the start of every chat request, so a level remembered by the browser but no longer runnable is caught before the chat starts. The model in use is stored in `data/chat-model` and the context of the level last selected in `data/llama-chat.ctx`; both are used the next time the chat server starts.

## Scheduler & Downtime

`backend/services/scheduler.py` runs a periodic reconcile loop (every 30s) that drives the daily downtime window. Settings live in a single-row `app_settings` table (id=1).

- **Downtime math**: a window is `downtime_start`/`downtime_end` (local `HH:MM`, default 06:30 to 09:30), enabled by `schedule_enabled`. Windows may wrap past midnight.
- **Entering downtime**: both llama-servers are stopped (including a sweep for orphaned servers left by a `uvicorn --reload`).
- **Exiting downtime**: when the window ends, both llama-servers are restarted.
- **Overrides**: `POST /api/system/end-downtime` forces uptime until the current window's end. A **manual pause** latches a user-paused flag that the scheduler honours so it won't undo the pause on the next tick.
- The reconcile loop is idempotent, so it self-heals if any single transition didn't fully take.

## llama-server Supervisor

`backend/services/llama_supervisor.py` owns the two model-server subprocesses so the app (and scheduler) can stop/start them: the embedding `llama-server`, and the chat server, which is `llama-server` or `mlx_lm.server` depending on the chat model selected. The `llama-chat.*` pid, log and context files belong to whichever chat backend is active.

- On app startup (`lifespan` in `backend/app.py`) both servers are launched unless the app started inside a downtime window. They are **not** stopped on app shutdown, so a fresh instance (or a `--reload`) can adopt them via their pid files and keep the warm KV cache.
- Startup is **idempotent**: if a port already answers, the running process is adopted (pid discovered via the pid file or `lsof`) rather than launching a duplicate.
- Stops send `SIGTERM` (escalating to `SIGKILL`), detect exit via `Popen.poll()` so a stop finishes in ~1s instead of waiting on a zombie, and reap orphaned servers still listening on the port.
- Lifecycle state (`running`, `starting`, `paused`, `down`) feeds the top-bar status pill; `GET /api/system/status` composes it with the current busy operation into a single human-readable label + severity.

## Concurrency Model

| Resource | Limit | Mechanism | Rationale |
|----------|-------|-----------|-----------|
| File extraction | 10 parallel | `asyncio.gather` batch | I/O bound, benefits from parallelism |
| LLM tagging | 1 | `asyncio.Semaphore(1)` | llama-server single-slot serialization |
| Embedding requests | 3 | `asyncio.Semaphore(3)` | Balance throughput vs. embedding llama-server capacity |
| Database writes | 1 | `asyncio.Lock` | SQLite transaction safety |
| OCR processing | 1 file | Sequential | CPU-intensive Tesseract, avoids thrashing |
| Pipeline exclusion | 1 pipeline | Mutual exclusion checks | Processing, OCR, exploration, and backup/restore block each other |
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
| KaTeX | LaTeX math rendering in chat responses (loaded from CDN) |
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
| mlx_lm.server (chat, optional) | 8001 | Replaces the chat llama-server while the MLX model is selected; text only, Apple Silicon only |
| llama-server (embeddings) | 8002 | Local embedding model (OpenAI compatible API) |
| Tesseract OCR | N/A | System binary for OCR (used by `ocrmypdf`) |

## Project Structure

```
_0RAG/
├── start.sh                        # macOS/Linux startup: venv, deps, server
├── start.ps1                       # Windows PowerShell startup script
├── kill.sh                         # Find and kill all GeoRAG processes (by port + cmdline)
├── run_server.py                   # Standalone entry point (used by the PyInstaller EXE)
├── georag.spec                     # PyInstaller spec for the Windows EXE build
├── requirements.txt                # Pinned Python dependencies
├── requirements-mlx.txt            # MLX runtime for the optional MLX chat model (installed into venv-mlx/)
├── .gitignore                      # Excludes venv/, venv-mlx/, data/, __pycache__/
│
├── backend/
│   ├── app.py                      # FastAPI app entry point, lifespan (llama + scheduler)
│   ├── config.py                   # All paths, model files, launch argv, tuning params
│   │
│   ├── models/
│   │   ├── database.py             # SQLAlchemy engine (WAL), session factory, migrations
│   │   ├── schemas.py              # ORM: File, Tag, FileTag, Conversation, Message,
│   │   │                           #      AppSettings
│   │   └── pydantic_models.py      # Request/response validation schemas
│   │
│   ├── routers/
│   │   ├── chat.py                 # POST /api/chat (SSE), stream mirror, stop
│   │   ├── conversations.py        # CRUD, soft delete, trash, restore
│   │   ├── documents.py            # Search (name + semantic), filter, paginate, tag, open
│   │   ├── tags.py                 # Tag CRUD
│   │   ├── processing.py           # Scan, start/stop pipeline, onboarding
│   │   ├── explore.py              # Tag exploration: discover new categories
│   │   ├── ocr.py                  # OCR start/stop/status for failed PDFs
│   │   ├── backup.py               # Export/import .georag archives
│   │   ├── system.py               # Settings, status pill, downtime, llama pause/start
│   │   └── health.py               # llama-server health probe
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
│       ├── scheduler.py            # Downtime window reconcile loop
│       ├── llama_supervisor.py     # Owns the two llama-server subprocesses
│       ├── backup_service.py       # .georag export/import (exporter, importer)
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
│   │   ├── base.html               # Shared nav, status pill, library panel, about/settings modals
│   │   ├── spa.html                # SPA shell (chat + documents in one page)
│   │   ├── index.html              # Server rendered chat page
│   │   └── documents.html          # Server rendered documents page
│   │
│   └── static/
│       ├── css/
│       │   ├── app.css             # Brand colors, layout, components, animations
│       │   └── pico.min.css        # PicoCSS framework
│       ├── js/
│       │   ├── chat.js             # Chat: SSE streaming, tags, conversations, depth
│       │   ├── documents.js        # Documents: search (name + semantic), filter, batch tag
│       │   ├── panel.js            # Library: scan, process, OCR, explore, progress polling
│       │   ├── system_status.js    # Status pill, downtime settings, llama pause/start
│       │   ├── router.js           # SPA routing via History API
│       │   ├── utils.js            # HTTP helpers, tag badges, sync channel
│       │   └── marked.min.js       # Markdown parser (vendored)
│       └── images/                 # Logo, icons, favicons
│
└── data/                           # Runtime data (gitignored)
    ├── metadata.db                 # SQLite: files, tags, conversations, settings
    ├── chroma/                     # ChromaDB: vector collections (per tag)
    ├── cache/                      # Application cache
    ├── exports/                    # Backup .georag archives + uploads
    ├── logs/                       # app.log, llama-chat.log, llama-embed.log
    ├── chat-model                  # Key of the chat model in use (config.CHAT_MODELS)
    ├── llama-chat.pid              # PID of the chat server (for adopt/stop)
    ├── llama-chat.ctx              # Context window (or budget) of the chat server
    ├── llama-embed.pid             # PID of the embedding llama-server
    └── onboarding_dismissed        # Flag file
```

## API Reference

FastAPI auto generates interactive docs at `/docs` (Swagger) and `/redoc`.

### Chat Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/chat` | Stream a chat response via SSE. Body: `{message, conversation_id?, tag_names?, depth?, top_k_per_tag?, max_context_chunks?}`; `depth` is a Retrieval Depth level key (default `optimal`), and the last two override that level's retrieval sizes. Events: `token` then `done`, or `error`. |
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

### System Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/system/settings` | Current schedule settings (downtime window, flags). |
| `PUT` | `/api/system/settings` | Update settings. Body: `{schedule_enabled?, downtime_start?, downtime_end?}` (HH:MM). |
| `GET` | `/api/system/status` | Composite status: llama state, downtime, next boundary, and the status-pill `{state_label, state_severity}`. |
| `POST` | `/api/system/end-downtime` | Force uptime until the current window's end. |
| `GET` | `/api/system/chat-depths` | Chat models and Retrieval Depth levels: `{default, model, models, levels, server_ctx}`. `models` lists each chat model with `available` and, if not, `unavailable_reason`; `levels` are resolved for the active model with each level's limits, context window and assumed memory. |
| `POST` | `/api/system/chat-depth` | Prepare the chat server for a level. Body: `{depth}`. Reloads the model with a larger context if needed (llama.cpp) or checks the memory estimate (MLX) and returns `{ok, checked}`, or `{ok: false, error}` if it does not fit in memory. |
| `POST` | `/api/system/chat-model` | Switch the chat model. Body: `{model, depth?}`. Returns `{ok, model, depth, notice, levels}`, where `depth` is the level to use (the requested one, or the default with a `notice` if the new model cannot run it here), or `{ok: false, error}` with the previous model restored. |
| `GET` | `/api/system/llama/memory` | What is loaded and the memory it holds: `{chat_up, embed_up, chat_model, chat_backend, chat_ctx, chat_mib, embed_mib, total_mib}`. llama-server sizes are summed from its load logs; the MLX server's is its measured process footprint. Shown in the pause dialog. |
| `POST` | `/api/system/llama/pause` | Manually stop both llama-servers to free memory (409 if LLM work is active). |
| `POST` | `/api/system/llama/start` | Manually start both llama-servers (returns immediately). |

### Backup Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/backup/export` | Start a background export. Params: `include_rag` (default true). |
| `GET` | `/api/backup/export/status` | Poll export progress. |
| `GET` | `/api/backup/export/download` | Download the completed `.georag` archive. |
| `POST` | `/api/backup/export/stop` | Cancel a running export. |
| `POST` | `/api/backup/import/validate` | Upload and validate a `.georag` archive without importing. |
| `POST` | `/api/backup/import` | Upload and import a `.georag` archive (background). |
| `GET` | `/api/backup/import/status` | Poll import progress. |
| `POST` | `/api/backup/import/stop` | Cancel a running import. |

### Health Endpoint

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/health/llm` | Probe both llama-servers; returns an actionable checklist of any problems (unreachable, wrong/missing model alias, etc.). |

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
| `EMBEDDING_MODEL` | `nomic-embed-text-v1.5` | Embedding model name (must match `--alias` on the embedding llama-server) |
| `EMBEDDING_DIM` | `768` | Vector dimensionality |
| `LLM_PARALLEL_SLOTS` | `1` | Concurrent inference slots (must match the chat server's `-np`) |
| `CHAT_MODELS` | (2 models) | The selectable chat models: backend (`llama` or `mlx`), path, the name sent in requests, whether it describes images, and its memory tiers (context and assumed memory per tier), plus the memory-estimate constants and the crash guard for MLX |
| `DEFAULT_CHAT_MODEL` | `qwen3.5-9b` | Chat model used until one is chosen in the dialog. Env: `GEORAG_CHAT_MODEL` |
| `LLAMA_CHAT_MODEL` | (GGUF path) | Chat model GGUF file the supervisor launches |
| `LLAMA_CHAT_MMPROJ` | (GGUF path) | Multimodal projector GGUF for image captioning |
| `LLAMA_EMBED_MODEL` | (GGUF path) | Embedding model GGUF file |
| `MLX_CHAT_MODEL` | (model folder) | MLX chat model directory. Env: `MLX_CHAT_MODEL` |
| `MLX_LM_SERVER` | `venv-mlx/bin/mlx_lm.server` | The MLX runtime's server executable. Env: `MLX_LM_SERVER` |
| `chat_launch(model, ctx)` / `LLAMA_EMBED_LAUNCH` | (argv lists) | Exact command lines the supervisor spawns; the chat one takes the model key and the context window size |

### Processing Tuning

| Variable | Default | Description |
|----------|---------|-------------|
| `CHUNK_SIZE` | `1000` | Tokens per chunk |
| `CHUNK_OVERLAP` | `200` | Overlap between adjacent chunks |
| `EMBEDDING_BATCH_SIZE` | `128` | Texts per embedding API call |
| `BATCH_SIZE` | `10` | Files processed concurrently per batch |
| `TOP_K_PER_TAG` | `5` | Default chunks retrieved per tag by a vector search (chat uses its depth level's `top_k`) |
| `CONVERSATION_HISTORY_TURNS` | `4` | Past turns (8 messages) included in prompt |
| `CHAT_THINKING` | `False` | Whether the chat model reasons before answering. Off by default; when on, the reasoning is unlimited and shown as a collapsible block |
| `CHAT_DEPTH_LEVELS` | (5 levels) | Retrieval Depth levels: `tier`, `top_k` and `max_chunks` for each; see [Retrieval Depth Presets](#retrieval-depth-presets) |
| `chat_depth_levels(model)` | (function) | The levels as they apply to one model: context and assumed memory of each level's tier |
| `DEFAULT_CHAT_DEPTH` | `optimal` | Level used when a chat request names none |

> The daily downtime window (`schedule_enabled`, `downtime_start`, `downtime_end`) is **not** in `config.py`; they live in the single-row `app_settings` table and are edited from the UI (defaults: disabled, 06:30 to 09:30).

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

### Settings Table

```
┌──────────────────────────────┐
│   AppSettings (single row)   │
│                              │
│ id = 1                       │
│ schedule_enabled             │
│ downtime_start / _end (HH:MM)│
│ auto_shutdown_on_manual_pause│
│ updated_at                   │
└──────────────────────────────┘
```

- **`AppSettings`** is a single row (id=1) holding the downtime schedule and the manual-pause behavior.
- Legacy `wiki_pages` / `wiki_log` tables and the `wiki` Chroma collection may still exist in older databases but are unused.

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

On startup, `init_db()` creates all tables if missing and runs lightweight `ALTER TABLE` migrations against existing databases:

- add `deleted_at` to `conversations` (soft delete)

It also seeds the `app_settings` row (id=1) if missing. `seed_tags()` inserts the 34 default geotechnical tags (`DEFAULT_TAGS`) if the tag table is empty. SQLite runs in WAL mode with a `busy_timeout` so background writes don't lock out API reads.

## Frontend Architecture

### Technology Choices

The frontend is intentionally **framework free**. Vanilla JavaScript with module pattern controllers (IIFEs) and PicoCSS for styling. No build step, no transpilation, no node_modules. Static files served directly by FastAPI.

### Controllers

| File | Lines | Responsibility |
|------|-------|----------------|
| `chat.js` | ~1170 | Welcome state, tag selection, message submission, SSE streaming, KaTeX rendering, conversation sidebar, trash management, depth settings, cross tab mirroring |
| `documents.js` | ~690 | File table rendering, name + semantic search, type/status/tag filters, pagination, single and batch tagging, mark-as-new/skipped, tag modal, Manage Tags |
| `panel.js` | ~915 | Library panel lifecycle (scan, process, OCR, explore, complete), processing status polling (2s interval), OCR start/stop, reprocess all confirmation |
| `system_status.js` | ~290 | Status pill rendering + polling, downtime schedule settings, manual llama pause/start, upcoming-event indicator |
| `router.js` | 55 | SPA routing via History API, page toggle (chat/documents), `spa:pageshow` custom event for lazy init |
| `utils.js` | ~125 | `apiGet`/`apiPost`/`apiDelete` wrappers, `escapeHtml`, tag badge factory, `syncChannel` (BroadcastChannel) |

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

### `start.sh` (macOS / Linux)

```
1. Detect Python 3.13 (fallback to 3.12 → 3.11 → python3)
2. Create venv/ if missing, then pip install requirements.txt
   (on Apple Silicon, if the MLX model folder exists: also create venv-mlx/ and install requirements-mlx.txt)
3. mkdir -p data/{chroma,cache,logs}
4. Verify the llama-server binary and all three model files exist (fail fast)
5. exec uvicorn backend.app:app --host 0.0.0.0 --port 3000 --reload
   (the app itself launches/manages the two llama-servers)
```

### `start.ps1` (Windows) — experimental

PowerShell equivalent of `start.sh`: sets up the venv, installs dependencies, creates data directories, and starts the server.

> **Windows support is experimental and unsupported.** The Windows startup script and the packaged EXE (below) have not been tested and may not work. GeoRAG is developed and run on macOS (Apple Silicon); treat the Windows path as a work in progress. Use `start.sh` on macOS/Linux for a supported setup.

### `run_server.py` and `georag.spec` (Windows EXE) — experimental

Standalone entry point used by the PyInstaller EXE. `run_server.py` sets the working directory next to the executable, creates data directories, prints the banner, and runs the app; `georag.spec` builds it into a Windows `.exe` via `pyinstaller georag.spec`.

> **Experimental / needs testing.** The bundled EXE build is not yet supported. Do not rely on it for production use.

### `kill.sh`

```
1. Find app + llama-server PIDs by port (3000/8001/8002) and command line
2. kill each PID (no sudo)
```

---

## License

This project is licensed under the **GNU Affero General Public License v3.0 (AGPLv3)**.

You are free to use, modify, and distribute this software. If you run a modified version of GeoRAG as a network service, you must make your source code available to users of that service under the same license.

See [LICENSE](LICENSE) for the full text.
