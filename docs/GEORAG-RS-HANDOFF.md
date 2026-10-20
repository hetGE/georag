# GeoRAG, Rust edition: build handoff

Status: approved plan, revision C, 2026-10-19.
Visual companion: https://claude.ai/artifact/7XB4UEZPvHfr135PjCcDDN (diagrams and screen mocks; this file is the text of record).
Reference implementation: the Python v1 app at `hetGE/georag` (read only, never modified, never pushed to).

This document is written for a Claude Code session that starts a new repository from nothing. Read it completely before creating files. Where it says "decided", do not reopen the question; where it says "accepted", implement as written and raise a note if something turns out to be impossible. Section 15 is the build order; section 16 lists the conventions the session must follow.

---

## 0. The four priorities

Every tie, every "should we", is decided by these, in this order.

1. **Memory goes to the model.** No browser, no Python runtime, no second server process. On unified memory (Apple silicon) a browser's tabs are context the model cannot have; on RTX systems, layers that spill from VRAM land in the same RAM. Models stay resident while the app runs; nothing else competes for that memory.
2. **Truly portable.** The app, its models, its index and its settings are one folder, on the machine or on external storage. Nothing is installed, nothing is written outside the folder.
3. **Native speed.** Compiled Rust, llama.cpp in-process, embeddings batched through one call instead of HTTP, extraction in a CPU pool.
4. **Cross-platform by folder copy.** Same OS and GPU class: copy the folder. Different GPU class: swap the binary flavour, keep everything else. macOS, Linux and Windows from one codebase.

A feature that costs memory, portability, speed or a platform needs a written reason.

---

## 1. What the product is

A local assistant for geotechnical engineers. It indexes a document library (PDF, Word, Excel, PowerPoint, images, text, CAD by filename), classifies files into topic tags, and answers questions grounded in those files with page-level citations. Everything runs on the user's machine through llama.cpp. No cloud, no API keys.

### Kept from v1
- Topic tags: 34 defaults, heuristic rules from folder and file names, LLM fallback for unmatched files.
- Manual retrieval depth tiers (Quick, Optimal, Deep, Deeper, Ludicrous) sized to GPU memory.
- Conversations with citations (file, page or sheet or slide, chunk).
- Vision captions for images through the chat model's multimodal projector (mmproj).
- Backup and restore of the whole index (later phase).

### Changed
- Full-screen terminal UI from the first build, beside plain subcommands.
- **Auto depth** by default: every chunk above the relevance threshold is read, in as many passes as memory needs, then a final pass combines the answers. Manual tiers remain as an option.
- Models run in-process and stay resident from launch to quit.
- One SQLite file holds metadata, vectors, full text and chats.
- Hybrid retrieval: vector plus keyword (FTS5), fused.
- OCR output goes into the index; original PDFs are never rewritten.
- Several library roots instead of one `Engineering/` folder.
- v1 tags and file lists import on day one.

### Dropped
- The browser, Python, venvs, PyInstaller.
- Separate `llama-server` and `mlx_lm.server` processes, the supervisor, pid files, port adoption.
- Scheduled downtime and manual pause.
- Tesseract and ocrmypdf. ChromaDB. Browser cross-tab mirroring.

---

## 2. Architecture

### 2.1 Workspace

```
georag/                      Cargo workspace, binary name `georag`
├─ crates/
│  ├─ georag-core/           library: config, library, pipeline, tagging, chunking,
│  │                         retrieval, chat (auto depth), models, import, update, backup
│  ├─ georag-engine/         trait InferenceEngine + LlamaCppEngine (llama-cpp-2), backend probe, VRAM fit
│  ├─ georag-extract/        extractors: pdf, docx, xlsx, pptx, text/html/md/csv/rtf, image (vision), ocr
│  ├─ georag-store/          SQLite (rusqlite, bundled), sqlite-vec, FTS5, migrations, one writer task
│  ├─ georag-daemon/         axum on 127.0.0.1, SSE, job supervisor, discovery file
│  ├─ georag-tui/            ratatui + crossterm screens (Elm-style loop)
│  └─ georag-cli/            the `georag` binary: clap subcommands, launches TUI when no args
├─ xtask/                    release bundling, manifest generation, signing helpers
├─ .github/workflows/        build matrix, release
└─ docs/                     this file, ADRs
```

Dependency direction: `cli` and `tui` depend on `core` (and on `daemon` only as a client). `daemon` depends on `core`. `core` depends on `engine`, `extract`, `store`. `engine`, `extract`, `store` depend on nothing of ours. Front ends never call `engine` or `store` directly.

`unsafe` and FFI live only in `engine` (llama.cpp) and `extract` (pdfium). Everything else is safe Rust.

### 2.2 Threading model

- `tokio` multi-thread runtime for IO, jobs, HTTP, the TUI loop.
- `rayon` pool for CPU-heavy extraction and chunking (`spawn_blocking` bridges into it).
- The engine runs on **one dedicated OS thread** that owns all llama.cpp contexts. Requests arrive on a priority inbox (section 5.3). Nothing else touches the GPU.
- SQLite: one connection for writes on its own task (mpsc of closures), a small pool of read connections. WAL mode, `busy_timeout` 5 s.

### 2.3 Core traits (sketch)

```rust
// georag-engine
pub trait InferenceEngine: Send + Sync {
    fn backend(&self) -> BackendInfo;                       // Metal | Cuda | Vulkan | Cpu, device name, total/free memory
    fn load(&self, spec: LoadSpec) -> Result<()>;           // chat (+mmproj) and embed models; idempotent
    fn resize_ctx(&self, n_ctx: u32) -> Result<()>;         // KV only; weights untouched
    fn fits(&self, model: &GgufInfo, n_ctx: u32) -> FitVerdict; // weights + KV estimate vs free memory
    fn tokenize(&self, role: Role, text: &str) -> Result<Vec<Token>>;
    fn embed(&self, texts: &[String], prio: Lane) -> Result<Vec<Vec<f32>>>;  // batched, n_batch
    fn generate(&self, req: GenRequest, prio: Lane, sink: TokenSink, cancel: CancelToken) -> Result<GenResult>;
    fn caption(&self, image: &[u8], prompt: &str, prio: Lane) -> Result<String>; // mmproj / mtmd
    fn stats(&self) -> EngineStats;                          // resident models, n_ctx, memory used, tokens/s
}

// georag-extract
pub trait Extractor: Send + Sync {
    fn handles(&self, ext: &str) -> bool;
    fn extract(&self, path: &Path, ctx: &ExtractCtx) -> Result<Extracted>;  // text with page/sheet/slide markers
}

// georag-core::chat
pub enum RunControl { SkipPass, AcceptPass, StopAll }
pub enum ChatEvent { Plan{passes: Vec<PassPlan>, kept: usize, total_tokens: u32, fallback: bool},
                     PassStart{k: u16}, Token{k: Option<u16>, text: String}, PassEnd{k: u16, status: PassStatus},
                     FinalStart, Done{message_id: i64, sources: Vec<Source>, cut_off: bool}, Error(String) }
```

---

## 3. Home folder and portability (decided)

```
GeoRAG/                        app home = folder containing the executable
├─ georag(.exe)                override: --home DIR, or GEORAG_HOME
├─ engine/                     ggml backend libs · pdfium shared lib · ocrs models · cudart (CUDA flavour)
├─ config/
│  ├─ settings.toml            library roots, chat mode, auto thresholds, backend override, update checks
│  ├─ models.toml              model registry: managed | linked, role, sha256, in-use flags
│  └─ tags.toml                tag definitions and heuristic patterns (34 defaults, editable)
├─ models/<org>/<repo>/        managed GGUF downloads
├─ data/
│  ├─ georag.db                files · tags · chunks (+ token counts) · vectors · FTS · chats · jobs
│  ├─ daemon.json              port, token, pid while `georag serve` runs
│  ├─ updates/                 downloaded bundles, staging, .old after an update
│  ├─ logs/                    rotating logs
│  └─ cache/                   rendered pages, thumbnails
└─ exports/                    .georag backup archives (P5)
```

Rules:
- Home resolution order: `--home`, `GEORAG_HOME`, the executable's directory. `georag init` creates the layout.
- Nothing is written outside home. No registry keys, no `~/.config`, no temp state that matters. `std::env::temp_dir` only for files deleted before return.
- Document paths are stored relative to their library root. Roots can be re-pointed (`georag library move NAME NEWPATH`) when a drive letter or mount changes.
- A read-only home folder: refuse to index or update with a clear message. Never escalate privileges.
- Library roots live wherever the user keeps documents. The old v1 `_0RAG/` folder is excluded from scanning if it sits inside a root.

### 3.1 settings.toml (defaults)

```toml
[app]
version_checked_at = ""            # last update check, RFC3339
[engine]
backend = "auto"                   # auto | metal | cuda | vulkan | cpu
gpu_layers = -1                    # -1 = all
kv_cache_type = "q8_0"
[chat]
mode = "auto"                      # auto | manual
manual_tier = "optimal"
history_turns = 4
thinking = false
[auto]
min_relevance = 0.65
max_passes = 6
min_candidates = 16
synthesis = true
candidate_k = 400
[index]
chunk_tokens = 1000
chunk_overlap = 200
extract_parallelism = 0            # 0 = number of physical cores
embed_batch = 64
watch_debounce_ms = 2000
[update]
auto_check = true
channel = "stable"                 # stable | nightly
[tui]
color = "auto"                     # auto | truecolor | 256 | 16
mouse = true
```

### 3.2 models.toml

```toml
[[model]]
id = "qwen3.5-9b-q4_k_m"
role = "chat"
kind = "managed"
path = "models/lmstudio-community/Qwen3.5-9B-GGUF/Qwen3.5-9B-Q4_K_M.gguf"
sha256 = "…"
n_ctx_train = 262144
size_bytes = 6200000000
in_use = true

[[model]]
id = "mmproj-qwen3.5-bf16"
role = "vision"
kind = "managed"
path = "models/lmstudio-community/Qwen3.5-9B-GGUF/mmproj-Qwen3.5-9B-BF16.gguf"
pairs_with = "qwen3.5-9b-q4_k_m"

[[model]]
id = "nomic-embed-text-v1.5"
role = "embed"
kind = "managed"
path = "models/nomic-ai/nomic-embed-text-v1.5-GGUF/nomic-embed-text-v1.5.Q8_0.gguf"
dim = 768
in_use = true

[[model]]
id = "qwen3-27b-q4_k_m"
role = "chat"
kind = "linked"
path = "/Volumes/LLMs/Qwen3-27B-Q4_K_M.gguf"
```

The curated catalogue (`core::models::catalog`) is a compiled-in list of leading GGUF chat, vision and embedding models with HF repo, filename, size, licence and notes; the user can also paste any `org/repo[:file]`. Changing the embedding model invalidates the vector table (section 4.2, `meta.embed_model`), so the TUI warns and offers a re-embed.

---

## 4. Store (georag-store)

SQLite via `rusqlite` with the `bundled` feature. Extensions: `sqlite-vec` (vector search) loaded from the crate, FTS5 from the bundled build. Schema version in `meta`; migrations are numbered SQL files applied in order.

### 4.1 Schema

```sql
CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
-- schema_version, embed_model, embed_dim, created_at, app_version

CREATE TABLE library_roots(
  id INTEGER PRIMARY KEY, name TEXT UNIQUE NOT NULL, path TEXT NOT NULL, added_at TEXT NOT NULL);

CREATE TABLE files(
  id INTEGER PRIMARY KEY,
  root_id INTEGER NOT NULL REFERENCES library_roots(id) ON DELETE CASCADE,
  rel_path TEXT NOT NULL, filename TEXT NOT NULL, ext TEXT,
  size_bytes INTEGER, mtime REAL, hash TEXT,                    -- blake3 of content
  state TEXT NOT NULL CHECK(state IN ('new','imported','extracted','tagged','indexed','failed','stale','skipped')),
  error TEXT, preview TEXT, chunk_count INTEGER DEFAULT 0, token_count INTEGER DEFAULT 0,
  indexed_at TEXT, v1_note TEXT,
  UNIQUE(root_id, rel_path));
CREATE INDEX files_state ON files(state); CREATE INDEX files_ext ON files(ext);

CREATE TABLE tags(
  id INTEGER PRIMARY KEY, name TEXT UNIQUE NOT NULL, display_name TEXT NOT NULL,
  description TEXT, color TEXT);

CREATE TABLE file_tags(
  file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
  source TEXT NOT NULL CHECK(source IN ('manual','rule','llm')),
  confidence REAL, imported INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL,
  PRIMARY KEY(file_id, tag_id));

CREATE TABLE chunks(
  id INTEGER PRIMARY KEY,
  file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  idx INTEGER NOT NULL, text TEXT NOT NULL, tokens INTEGER NOT NULL,
  page INTEGER, locator TEXT,                                   -- "p.12" | "sheet:Results" | "slide 4"
  UNIQUE(file_id, idx));

CREATE VIRTUAL TABLE chunk_vec USING vec0(chunk_id INTEGER PRIMARY KEY, embedding float[768]);
CREATE VIRTUAL TABLE chunk_fts USING fts5(text, content='chunks', content_rowid='id', tokenize='unicode61');

CREATE TABLE conversations(
  id INTEGER PRIMARY KEY, title TEXT, tags TEXT NOT NULL DEFAULT '[]',   -- JSON array of tag names
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL, deleted_at TEXT);

CREATE TABLE messages(
  id INTEGER PRIMARY KEY, conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
  role TEXT NOT NULL CHECK(role IN ('user','assistant')), content TEXT NOT NULL,
  sources TEXT NOT NULL DEFAULT '[]',                           -- JSON [{file_id, rel_path, locator, chunk_id, score}]
  mode TEXT, cut_off INTEGER DEFAULT 0, created_at TEXT NOT NULL);

CREATE TABLE passes(
  id INTEGER PRIMARY KEY, message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
  idx INTEGER NOT NULL,                                         -- 1..N, 0 = final
  status TEXT NOT NULL CHECK(status IN ('done','skipped','stopped','accepted')),
  chunk_ids TEXT NOT NULL, answer TEXT, started_at TEXT, finished_at TEXT);

CREATE TABLE jobs(
  id INTEGER PRIMARY KEY, kind TEXT NOT NULL,                   -- scan|extract|ocr|tag|chunk|embed|caption|pull|import
  file_id INTEGER REFERENCES files(id) ON DELETE CASCADE, payload TEXT,
  state TEXT NOT NULL CHECK(state IN ('queued','running','done','failed','cancelled')),
  attempts INTEGER DEFAULT 0, error TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);

CREATE TABLE activity(id INTEGER PRIMARY KEY, at TEXT NOT NULL, level TEXT NOT NULL, text TEXT NOT NULL);
```

Notes:
- `chunk_vec` dimension is created from `meta.embed_dim`; a different embedding model means drop and recreate, then re-embed all files.
- Tag filtering in search is a join: `chunk_id IN (SELECT c.id FROM chunks c JOIN file_tags ft ON ft.file_id=c.file_id JOIN tags t ON t.id=ft.tag_id WHERE t.name IN (...))`. sqlite-vec supports `WHERE chunk_id IN (...)` pre-filters; use them.
- Chunk IDs are stable across reprocessing of other files; reprocessing a file deletes and reinserts its chunks (vec and fts rows via triggers or explicit calls).

### 4.2 Store API surface (core uses only this)

`roots`, `files` (upsert_from_scan, set_state, query with filters and facet counts, record), `tags` (crud, seed from tags.toml), `file_tags`, `chunks` (replace_for_file, by_ids with tokens), `search` (knn(vec, filter, k), fts(query, filter, k), cosine_for(chunk_ids, vec)), `conversations`, `messages` (+passes), `jobs` (enqueue, claim, update, list, cancel), `activity`, `meta`.

---

## 5. Engine (georag-engine)

### 5.1 Decided
- `llama-cpp-2` crate (utilityai), llama.cpp in-process. Cargo features `metal`, `cuda`, `vulkan` map to the sys crate's build flags. CPU is always available.
- Apple GPU through **ggml Metal**. MLX is a future option behind `InferenceEngine` once `mlx-rs` matures; note this in the README. Nothing in the plan depends on MLX.
- Models are **resident**: load at launch (or first use in one-shot mode), unload at quit. Never on a timer. A depth change resizes the KV context only. A model switch is the only time weights change.

### 5.2 Backend selection at start-up
1. `settings.engine.backend` or `--backend` if not `auto`.
2. Apple silicon: Metal.
3. CUDA device and runtime present (this binary is the CUDA flavour): CUDA.
4. Vulkan device present (this binary is the Vulkan flavour): Vulkan.
5. CPU, with a warning in the status bar.

`georag doctor` prints: flavour, backend chosen and why, device name, total and free GPU memory, the context ladder that fits for the in-use chat model, a 50-token generation timing, and the cosine score distribution for a sample question (section 7.5).

### 5.3 Priority lanes
One inbox with three lanes; the engine thread always takes the highest non-empty lane:
- **P0 interactive**: chat generation, question embedding, pass synthesis.
- **P1 vision and tagging**: image captions, LLM tag fallback.
- **P2 bulk**: embedding batches.

A P2 batch is bounded (≤ `index.embed_batch` texts) so a P0 request waits at most one batch. Generation is not pre-empted mid-token; a cancel token is checked between tokens.

### 5.4 Memory fit and the context ladder
From the GGUF header: architecture, `n_ctx_train`, layer count, head dims, weight bytes. KV bytes per token are computed for the configured cache type. `fits(model, n_ctx)` compares weights + KV(n_ctx) + a fixed overhead (1 GB) against free memory reported by the backend (Metal: recommended working set; CUDA: `cudaMemGetInfo`; Vulkan: device heap budget). The ladder is 32 768, 131 072, 262 144 capped at `n_ctx_train`; `max_ctx_that_fits()` returns the largest rung that fits. Manual tiers map to rungs (section 7.6); auto mode uses `max_ctx_that_fits()`.

### 5.5 Tokenisation and embeddings
- All token counting (chunks, budgets, history) uses the engine's tokenizer for the relevant model. No estimates.
- Embeddings: nomic-style task prefixes (`search_document:` at index time, `search_query:` at question time) applied by the embed adapter when the model id matches; configurable per model in the catalogue.
- Vectors are L2-normalised so cosine is a dot product.

---

## 6. Ingestion pipeline (georag-core::pipeline, georag-extract)

### 6.1 Stages and jobs
`scan → extract → (ocr) → tag → chunk → embed → index`. Every stage is a `jobs` row per file; the supervisor runs them with bounded parallelism per kind (extract: CPU pool; tag: 1 on P1; embed: 1 on P2; ocr: 1 CPU). Jobs are idempotent and resumable: a crash mid-job leaves the file in its previous state and the job `queued`.

File state machine: `new → extracted → tagged → indexed`; `new → failed` (no text) `→ extracted` after OCR; `indexed → stale` when the scanner sees a changed hash or mtime `→ new` on reprocess; `imported` behaves like `new` but skips the tag stage.

### 6.2 Scan
Walk each root with `walkdir`, skipping `.git`, `node_modules`, `venv`, `_0RAG`, hidden files. Upsert by `(root_id, rel_path)`; compute blake3 only when size or mtime changed. Files missing from disk are marked `skipped` with a note, not deleted, until `georag library prune`.

The **watcher** (`notify`, 2 s debounce) feeds changed paths into scan while the daemon runs.

### 6.3 Extractors
| Type | Crate | Output |
|---|---|---|
| pdf | `pdfium-render` with pdfium shipped in `engine/` | text per page with `[Page N]` markers; page images for OCR and vision |
| docx, pptx | `zip` + `quick-xml` on `word/document.xml`, `ppt/slides/slideN.xml` | paragraphs and table cells; `[Slide N]` markers |
| xlsx, xls, ods, csv | `calamine`, `csv` | `[Sheet: name]` markers, rows as tab-separated text |
| txt, md, html, rtf | built-in; `html2text` for HTML | plain text |
| png, jpg, webp, tiff | engine `caption()` via mmproj, P1 lane | one description paragraph, max 10 MB input |
| dwg, dxf | none | filename and folder indexed only |

A PDF with no text layer (fewer than 20 characters per page on average) goes to `failed` with reason `no_text`; `georag ocr` or the Library screen's reprocess queues OCR: render pages with pdfium at 200 dpi, run `ocrs` (pure Rust, models in `engine/`), store the text as the extraction. The original file is never written.

### 6.4 Tagging
Port v1's rules from `backend/services/tagger.py` and the defaults from `backend/config.py::DEFAULT_TAGS` into `config/tags.toml`:
```toml
[[tag]]
name = "piling"; display_name = "Piling"; color = "#8a5a2b"
patterns = ["pile", "bored pile", "cfa", "driven pile", "pile cap"]
```
Rule pass: patterns matched against folder path and filename, confidence 0.7, source `rule`. LLM fallback: files with no rule match, batched up to 10, first 1500 characters each, JSON answer, confidence 0.8, source `llm`, lane P1. Manual tags from the UI are `manual`, confidence 1.0, never overwritten. Imported tags keep their source and set `imported = 1`; the tag stage skips files that have any tag unless `--retag`.

### 6.5 Chunking
`text-splitter` with a sizer backed by the engine tokenizer: 1000 tokens, 200 overlap, separators paragraph, line, sentence, word. Page/sheet/slide markers set `page` and `locator` on each chunk. Token count stored per chunk.

### 6.6 Embed and index
Batches of `index.embed_batch` chunk texts through `engine.embed(P2)`; insert into `chunks`, `chunk_vec`, `chunk_fts` in one write transaction per file; set `indexed`, `chunk_count`, `token_count`, `preview` (first 300 characters).

---

## 7. Retrieval and chat (georag-core::retrieval, ::chat)

### 7.1 Hybrid retrieval
```
q      = engine.embed([search_query: question])[0]
vec    = store.knn(q, tag_filter, k = auto.candidate_k)          // cosine
kw     = store.fts(question, tag_filter, k = auto.candidate_k)   // bm25
cands  = union by chunk_id; for kw-only hits fetch embedding and compute cosine
score  = cosine (stored on every candidate; this is "relevance")
rank   = reciprocal rank fusion of vec and kw orderings (k = 60), used for ordering ties and for manual mode
```

### 7.2 Context budget
```
n_ctx         = auto: engine.max_ctx_that_fits()   manual: tier.n_ctx (refuse if it does not fit)
reply_reserve = n_ctx / 2
prompt_budget = n_ctx - reply_reserve
chunk_budget  = prompt_budget - tokens(system) - tokens(history_kept) - tokens(question) - 256
```
History: last `chat.history_turns` turns, newest first, kept while they fit in `prompt_budget / 4`; older ones dropped first. Stored `<think>` blocks are stripped from history.

### 7.3 Single pass (manual mode, or auto when everything fits)
System prompt with the chunks (each prefixed `[S<n>] <rel_path> <locator>`), history, question. Stream with `thinking = false` unless configured. `finish_reason == length` sets `cut_off` and appends a visible note. Sources returned are the chunks cited or, if the model cites nothing, the top chunks used.

### 7.4 Auto depth (decided, default)

```
fn plan_auto(question, tags, cfg) -> Plan {
    cands = hybrid_retrieve(question, tags)
    kept  = cands.filter(|c| c.score >= cfg.min_relevance)
    fallback = kept.is_empty()
    if fallback { kept = cands.by_rrf().take(cfg.min_candidates) }
    budget = chunk_budget()                                   // section 7.2, at max_ctx_that_fits
    if kept.sum(tokens) <= budget { return Plan::Single(kept) }

    // group by document, document relevance = max chunk score, chunks in file order
    docs = group_by_file(kept).sort_by(relevance desc)
    passes = []
    for doc in docs {
        pieces = if doc.tokens <= budget { [doc] } else { split_in_page_order(doc, budget) }  // only exception to "never split"
        for piece in pieces {
            match passes.iter_mut().find(|p| p.tokens + piece.tokens <= budget) {   // first fit
                Some(p) => p.push(piece),
                None if passes.len() < cfg.max_passes => passes.push(Pass::new(piece)),
                None => { dropped.push(piece); }
            }
        }
    }
    Plan::Multi { passes, dropped, fallback }
}
```

Execution (`run_auto`):
1. Emit `Plan` (pass count, kept chunk count, total tokens, dropped documents, fallback flag).
2. For each pass k: build the prompt with the pass system text *"You are reading sources {i} to {j} of {n} for this question. Answer only from these sources. End with one line listing what these sources do not cover."*, the pass's chunks, the history, the question. Stream tokens tagged with k. Watch `RunControl`:
   - `SkipPass`: cancel generation, mark `skipped`, continue.
   - `AcceptPass`: cancel nothing; when the pass ends mark `accepted`, stop the run, this answer is the final message content.
   - `StopAll`: cancel, mark `stopped`, stop the loop; passes done so far remain. The UI offers "combine what is done" which runs step 4 over them.
3. If a pass answer was cut off, note it and continue.
4. Final pass (if `auto.synthesis` and no accept): system text *"Combine the following partial answers into one answer to the question. Reconcile disagreements, say where sources differ, keep every citation tag unchanged, and list sources that were skipped or not read."* Input is the question plus each done pass's answer labelled `Pass k (sources i to j)`. If that exceeds `prompt_budget`, combine in groups of passes, then combine the group results.
5. Persist: the assistant `message` holds the final answer and merged sources; `passes` rows hold every pass with status, chunk ids and answer. The CLI prints passes as they finish; `--json` returns `{passes: [...], final: ..., plan: ...}`.

Citation tags are `[S<n>]` globally numbered across the whole run so the final pass can keep them; the UI maps them to file and locator.

### 7.5 Threshold calibration
`georag doctor --scores "sample question"` prints the cosine distribution of the top 200 candidates (deciles), so `auto.min_relevance` can be checked when the embedding model changes. 0.65 is the default for nomic-embed-text v1.5.

### 7.6 Manual tiers (carried over)
| Tier | top k per tag | max chunks | n_ctx |
|---|---|---|---|
| quick | 5 | 8 | 32 768 |
| optimal | 10 | 16 | 32 768 |
| deep | 20 | 32 | 131 072 |
| deeper | 50 | 80 | 131 072 |
| ludicrous | 100 | 200 | 262 144 |

`--depth TIER` or `chat.mode = "manual"` selects a tier; a tier whose context does not fit is refused with the numbers.

---

## 8. Daemon and IPC (georag-daemon)

- `georag serve` starts it; the TUI starts it on demand when none is running (and leaves it running on quit unless `--no-daemon`). Subcommands use it when `data/daemon.json` names a live process (pid alive, `/v1/status` answers with the token), else run core in-process.
- Bind `127.0.0.1` on a free port; write `data/daemon.json` `{port, token, pid, started_at, version}` with mode 0600. Every request needs `Authorization: Bearer <token>`.
- Events to clients over SSE; one firehose plus per-chat streams.

Endpoints (JSON unless SSE):
```
GET  /v1/status                      snapshot: engine, models, counts, jobs, version, update notice
GET  /v1/events                      SSE: job, activity, engine stats, watcher, update
POST /v1/chat                        SSE: ChatEvent stream; body {conversation_id?, message, tags, mode?, tier?}
POST /v1/chat/{run_id}/control       {action: skip|accept|stop}
GET  /v1/conversations  GET/DELETE /v1/conversations/{id}  POST /v1/conversations/{id}/restore
GET  /v1/files?q&state&ext&tag&page  with facet counts    GET /v1/files/{id}   GET /v1/files/{id}/chunks
POST /v1/files/{id}/tags/{tag}  DELETE same  POST /v1/files/reprocess {ids}  POST /v1/files/{id}/open
GET  /v1/tags  POST  PUT /v1/tags/{id}  DELETE /v1/tags/{name}   POST /v1/tags/explore
GET  /v1/library/roots  POST  DELETE /v1/library/roots/{id}  POST /v1/library/scan  POST /v1/library/index
GET  /v1/jobs  POST /v1/jobs/{id}/cancel
GET  /v1/models  POST /v1/models/pull  POST /v1/models/link  POST /v1/models/use  DELETE /v1/models/{id}  POST /v1/models/{id}/verify
GET  /v1/models/catalog?q
POST /v1/import/v1/preview  POST /v1/import/v1/run          SSE progress on /v1/events
GET  /v1/update/check  POST /v1/update/download  POST /v1/update/apply  POST /v1/update/rollback
GET  /v1/settings  PUT /v1/settings
POST /v1/shutdown {drain: true}
```

---

## 9. Terminal UI (georag-tui), day one

Stack: `ratatui`, `crossterm`, `tui-textarea` (input), `unicode-width`. Elm-style loop: `tokio::select!` over terminal events, core events and a 250 ms tick → `update(&mut state, msg) -> Vec<Cmd>` → `view(&state, frame)`. Redraw at most 30 fps; token streams are coalesced per tick. State changes only in `update`.

### 9.1 Chrome on every screen
- Row 1 status bar: `georag <version>` · home path · engine state (loading / ready / generating N t/s) · backend and memory used/total · right-aligned update notice when one is available.
- Row 2 tab row: `1 Chat  2 Library  3 Models  4 Status  5 Settings` (Home and wizards show the row without a highlight).
- Last row key bar: the keys of the focused pane.
- Global keys: `1-5` jump, `tab`/`shift-tab` cycle, `j k ↑ ↓ pgup pgdn g G` move, `/ n N` filter and find, `?` help overlay, `esc` close overlay or stop the current run, `q` back then quit (confirm if a pass or job is running), `ctrl-c` quit now (daemon keeps running), `y` copy (OSC 52, falls back to writing `data/clipboard.txt` and saying so), `e` export conversation to markdown in `exports/`.
- Overlays: help (scrollable key list), confirm (y/n), picker (multi-select with filter: tags, models), text input (validated), toast (5 s, also logged to activity), progress modal for import and update.
- Resize: panes collapse right to left below 110 columns; one pane at a time below 80; never clip text, wrap or scroll it. Colour: truecolor when reported, else 256, else 16; meaning is never carried by colour alone. Mouse optional (click tabs and rows, wheel scroll).

### 9.2 Screens
**Home** (bare `georag`): left menu `1 Chat, 2 Library, 3 Models, 4 Status, 5 Settings, i Import from v1, u Update, ? Help, q Quit`; right "at a glance" (same data as `status --json`: roots, files by state, models and memory, daemon, jobs) and recent conversations. First run (no root or no in-use chat/embed model): the right pane is a checklist, each item opens the right screen: home folder ✓, pull models, add a library root, import from v1 (optional).

**Chat**: left conversations list (`n` new, `x` delete to trash, `/` find, `c` toggle pane); main: mode line `tags … [t] · depth auto/manual … [d] · model … [m]`, transcript (markdown-lite: headings, bold, lists, code blocks; citations `[S3]` rendered as `[file locator]` in the accent colour), **pass rail** when a multi-pass run is active or shown (`1 ✓ done · 2 ▶ 62 % · 3 ○ · 4 ○ · final ○`, plus `N sources ≥ 0.65 · tokens`), each pass's answer under its own header as it streams, the final answer last; input box (`enter` send, `alt-enter` newline, `ctrl-j` on legacy Windows console); right sources pane (`S` toggle; per pass: score, file, locator; `enter` view chunk text, `o` open file in the OS, `1-4` show a pass's sources). Keys during a run: `esc` stop all, `s` skip pass, `a` accept pass. After `esc`, a confirm offers "combine what is done".

**Library**: left facets (filter text, state counts, tag counts, type counts; `f` toggle, `enter` apply); main table (select, state glyph ● indexed ◐ indexing ○ new ✕ failed, path, type, tags, chunks, modified; sort with `o`); detail strip for the highlighted file (state, chunks, tags with source and confidence, preview); `enter` opens the record view (chunk list, last error, OCR offer). Keys: `space` select, `t` tag (picker), `T` untag, `r` reprocess, `o` open in OS, `a` add root (text input), `R` rescan, `/` filter.

**Models**: top "installed" table (role, id, size, n_ctx_train, where: managed or linked path, state: in use / loaded / fits at N ctx / does not fit); bottom catalogue with `/` search or paste `org/repo[:file]`, fit column from the GGUF header and current free memory, download rows with gauge, speed, ETA. Keys: `enter` pull, `p` pause download, `u` use, `l` link a path, `r` set role, `d` remove (confirm), `v` verify sha256. Pulls run in the background, survive leaving the screen, resume after restart.

**Status**: GPU panel (model, n_ctx, resident since, memory gauge, embed model, tokens/s sparkline), pipeline panel (per-stage progress bars and counts, failed count with reason), jobs list (`x` cancel, `enter` detail), activity log. All from `/v1/status` and `/v1/events`.

**Settings**: form list of `settings.toml` keys with inline edit and validation, library roots add/remove, tag file path, "re-check update now", "run doctor". Writes TOML; the engine reloads only when the backend key changes (confirm first).

**Update**: current and latest version, release notes (scrollable), download gauge, verify result, `enter` apply (confirm; drains the daemon), `r` rollback. Section 11.

**Import from v1** (wizard, `i` from Home, also from Settings): steps locate → map root → preview (dry run, exact counts) → options (chats, retag, dry run) → run with progress → summary with "start indexing now". Section 10.

### 9.3 Watch-outs to design for
Markdown is styled text only (no images, no complex tables). Legacy Windows console: 16 colours, no alt-enter. Coalesce tokens to the tick. Below 80 × 24 show one pane. Use `unicode-width` for all column maths.

---

## 10. Import from v1 (decided, day one)

Input: the v1 `_0RAG` folder, its `data/metadata.db`, or a `.georag` backup archive (zip with `manifest.json`, `files.jsonl`, `tags.json`, `file_tags.jsonl`; `*.npz` vectors ignored). Never writes to the v1 folder.

v1 schema (SQLAlchemy, `backend/models/schemas.py`): `files(relative_path, filename, extension, size_bytes, modified_time, parent_directory, content_hash, scan_status, extracted_text_preview, chunk_count, auto_tagged, auto_tag_confidence)`, `tags(name, display_name, description, color)`, `file_tags(file_id, tag_id, source ∈ {auto, manual, folder_hint}, confidence)`, `conversations(title, selected_tags, deleted_at)`, `messages(role, content, sources JSON)`, `app_settings` (downtime; ignored).

Mapping:
| v1 | v2 | rule |
|---|---|---|
| root | `library_roots` | default: parent of `_0RAG/`; `--root` overrides; `_0RAG/` excluded |
| `files.relative_path` | `files.rel_path` | re-based on the mapped root; files missing on disk are imported as `skipped` with `v1_note = "missing at import"` and listed in the log |
| `files.scan_status` | `files.state = 'imported'` | all re-indexed; `failed` files get `v1_note = "no text in v1"` so OCR is offered first |
| `files.content_hash` | `files.hash` (if sha256; else null) | the first scan recomputes blake3 and flags changed files as `stale` |
| `tags.*` | `tags.toml` + `tags` | built-ins matched by name; custom tags appended to `tags.toml` with their colour and description |
| `file_tags.source` | `manual → manual`, `folder_hint → rule`, `auto → llm` | confidence kept; `imported = 1` |
| conversations, messages | same | only with `--with-chats`; soft-deleted skipped; citations kept as JSON and shown as text until the file is indexed, then re-linked by `rel_path + locator` |
| chroma vectors | nothing | not compatible; re-embedded |

Algorithm: open read-only → validate schema → compute preview (counts above, including on-disk check) → with confirmation: insert root, tags, files, file_tags, (chats) in one transaction → enqueue `extract` jobs for all imported files → summary. `--dry-run` stops after the preview. Idempotent: re-running merges by `(root, rel_path)` and `(file, tag)`.

CLI: `georag import v1 PATH [--root DIR] [--with-chats] [--retag] [--dry-run] [--json]`.

---

## 11. Updater and distribution

### 11.1 Release layout (GitHub Releases on the distribution repo, tag `vX.Y.Z`)
```
georag-X.Y.Z-aarch64-apple-darwin.tar.gz
georag-X.Y.Z-x86_64-pc-windows-msvc-cuda.zip
georag-X.Y.Z-x86_64-pc-windows-msvc-vulkan.zip
georag-X.Y.Z-x86_64-unknown-linux-gnu-cuda.tar.gz
georag-X.Y.Z-x86_64-unknown-linux-gnu-vulkan.tar.gz
SHA256SUMS            SHA256SUMS.minisig
manifest.json         manifest.json.minisig
notes.md
```
Each archive holds `georag(.exe)`, `engine/` (ggml backend libs, pdfium, ocrs models, cudart for CUDA flavours) and `config/*.toml.default`. Flavour string (`target triple` + `-cuda|-vulkan` where relevant) is compiled into the binary.

`manifest.json`:
```json
{ "version": "2.1.0", "channel": "stable", "published": "2026-11-02T10:00:00Z",
  "min_version_for_update": "2.0.0", "notes": "notes.md",
  "assets": { "aarch64-apple-darwin": {"file": "georag-2.1.0-aarch64-apple-darwin.tar.gz", "sha256": "…", "size": 123456789 },
              "x86_64-pc-windows-msvc-cuda": { … } } }
```

### 11.2 Flow (`georag update`, or `u` in the TUI)
1. Fetch `https://github.com/<owner>/<repo>/releases/latest/download/manifest.json` (+ `.minisig`); nightly channel uses the `nightly` pre-release tag. No API calls, no rate limits. Honour `HTTPS_PROXY`.
2. Verify the manifest signature with the minisign public key compiled into the binary. Compare semver with the built-in version; `--check` stops here.
3. Select the asset for this binary's flavour. Never infer from hardware.
4. Download to `data/updates/` with Range resume and progress events.
5. Verify sha256 against the manifest (and `SHA256SUMS` + its signature).
6. Stage: extract to `data/updates/staging/`.
7. Drain: ask the daemon (if any) to finish the current file and exit; the TUI shows progress.
8. Swap: `georag → georag.old`, `engine → engine.old`, move staging into place (`self-replace` crate for the running executable on Windows).
9. Relaunch the new binary with `--self-check` (doctor subset). Success deletes `.old`; failure restores `.old` and reports.
`--file BUNDLE` enters at step 5 for offline sites. `--rollback` swaps `.old` back. Auto-check at most once per 24 h, only if `update.auto_check`, never auto-install; the only unasked UI is the status-bar notice.

### 11.3 CI
GitHub Actions matrix: `macos-14` (Metal), `windows-latest` × {CUDA toolkit, Vulkan SDK}, `ubuntu-22.04` × {CUDA, Vulkan}. Steps: build with the feature, bundle `engine/`, run `georag doctor --ci` (CPU path), package, sha256, sign with minisign (secret key in CI secrets), upload. `cargo-dist` may generate most of this; keep the manifest format above regardless. Key rotation: a release signed by the old key that ships the new public key.

---

## 12. CLI surface

```
georag                                   full-screen UI
georag init [--home DIR]                 create the home layout
georag doctor [--scores "question"] [--ci]
georag status [--json]
georag config set KEY VALUE | get KEY

georag models list|catalog [--json]
georag models pull ORG/REPO [FILE] [--role chat|embed|vision]
georag models link PATH --role ROLE
georag models use chat|embed|vision ID
georag models remove ID | verify ID

georag library add PATH [--name NAME] | list | move NAME PATH | remove NAME | prune
georag scan
georag index [--watch] [--tags a,b] [--reprocess] [--ids …]
georag ocr [--retry-failed]
georag docs list [--q --state --ext --tag --json] | show ID | tag ID TAG | untag ID TAG | open ID
georag tags list|add|edit|remove|explore

georag chat [-t tags] [--mode auto|manual] [--depth TIER] [-c CONVERSATION_ID]
georag ask "question" [-t tags] [--json] [--mode] [--depth] [--min-relevance F] [--max-passes N] [--no-synthesis]
georag search "query" [-t tags] [--k N] [--json]
georag conversations list|show ID|rm ID|restore ID|export ID

georag serve [--port N] | stop
georag update [--check] [--channel stable|nightly] [--file BUNDLE] [--rollback] [--yes]
georag import v1 PATH [--root DIR] [--with-chats] [--retag] [--dry-run] [--json]
georag backup export|import FILE          (P5)
georag logs [-f] [--level]
```
Every command works without a daemon and routes through the daemon when one is running. `--json` on every read command.

---

## 13. Decisions

| Area | Choice | Status |
|---|---|---|
| Inference | `llama-cpp-2`, in-process, features metal / cuda / vulkan | decided |
| Apple GPU | ggml Metal; MLX a future engine once `mlx-rs` matures (README note) | decided |
| Resident models | load at launch, keep until quit, resize context only | decided |
| Depth | auto multi-pass default, manual tiers option | decided |
| Front end | ratatui TUI from day one, plus subcommands; native windowed UI in P6 | decided |
| v1 import | metadata.db or .georag; tags and file list; re-embed | decided |
| Build matrix | macOS arm64 Metal; Windows x64 CUDA and Vulkan; Linux x64 CUDA and Vulkan; static features | accepted |
| Vectors | sqlite-vec in georag.db, exact KNN, tag filter by join; LanceDB/usearch if > ~1 M chunks | accepted |
| Keyword search | FTS5 on chunk text, RRF with vectors | accepted |
| Database | rusqlite bundled, WAL, one writer task | accepted |
| PDF | pdfium-render, pdfium shipped in engine/ | accepted |
| OCR | ocrs, pure Rust, models in engine/; originals untouched | accepted |
| Office | calamine (xlsx), zip + quick-xml (docx, pptx) | accepted |
| Chunking | text-splitter with the engine tokenizer; token count stored per chunk | accepted |
| Downloads | hf-hub with resume; sha256 from LFS metadata | accepted |
| IPC | localhost HTTP + SSE (axum), bearer token in data/daemon.json | accepted |
| Concurrency | tokio for IO and jobs; rayon for extraction; engine on its own thread | accepted |
| Config | TOML in config/, app-relative home | accepted |
| Watching | notify with 2 s debounce | accepted |
| Updater | manifest on GitHub Releases, own client, self-replace, minisign | accepted |

---

## 14. Crates to use (verify current versions and feature names on crates.io before pinning)

`llama-cpp-2` (+ `llama-cpp-sys-2` features), `rusqlite` (bundled), `sqlite-vec`, `tokio`, `rayon`, `axum`, `tower-http`, `reqwest` (rustls), `hf-hub`, `serde`, `serde_json`, `toml`, `clap` (derive), `ratatui`, `crossterm`, `tui-textarea`, `unicode-width`, `notify`, `notify-debouncer-full`, `walkdir`, `blake3`, `sha2`, `pdfium-render`, `ocrs`, `calamine`, `zip`, `quick-xml`, `html2text`, `text-splitter`, `self-replace`, `minisign-verify`, `semver`, `tracing`, `tracing-subscriber`, `tracing-appender`, `thiserror`, `anyhow`, `indicatif` (for non-TUI progress), `arboard` optional, `directories` not used (priority 2).

Check early (P0): that `llama-cpp-2` exposes multimodal (mtmd) for captions and the backend memory queries; that `sqlite-vec` loads under `rusqlite` bundled on all three OSes; that `ocrs` model files can live in `engine/`.

---

## 15. Build order and acceptance

P0 to P2 together are the **day-1 build**: the first release the user switches to from v1.

### P0: scaffold and shell
- Workspace, crates, `edition = "2024"`, clippy and fmt clean in CI.
- `georag init`, home resolution, `settings.toml` defaults, logging to `data/logs`.
- `georag-engine` with backend probe, `doctor`, CPU path working on all three OSes; Metal, CUDA, Vulkan feature builds in the CI matrix producing bundles (unsigned is fine until the key exists).
- TUI chrome: status bar, tab row, key bar, help overlay, Home with the first-run checklist (items may open placeholder screens).
- Accept: `georag doctor` on a Mac (Metal), a Windows CUDA box and a Linux Vulkan box reports the backend and memory; `georag` opens the TUI and quits cleanly; CI uploads five bundles.

### P1: models and chat
- `models.toml`, catalogue, `pull` (resume, sha256, progress), `link`, `use`, `remove`, `verify`.
- Resident engine: load chat (+mmproj) and embed at start, context ladder, `fits`, resize.
- Single-pass chat with hybrid retrieval stubbed on an empty index (answers from history only) and full once P2 lands; streaming; conversations persisted.
- Auto depth: planner, pass runner, controls, synthesis, persistence; manual tiers.
- TUI Chat (pass rail, sources pane, conversations) and Models screens.
- `update` (check, download, verify, apply, rollback) against a test release.
- Accept: on a 16 GB Mac, a question over a seeded index with 40 k tokens of relevant chunks runs 4 passes and a final pass; `s`, `a`, `esc` behave as specified; the saved message shows every pass; `--json` emits the plan and passes; `update --check` reports the test release.

### P2: index and import
- Library roots, scan, watcher (in-process for now), extract for pdf, docx, xlsx, pptx, text, html; chunking with stored token counts; embed; index; FTS.
- Tagging: rules from `tags.toml`, LLM fallback.
- `search`, `docs`, TUI Library and Settings screens.
- `import v1`: preview, run, wizard.
- Accept: import of a real v1 `_0RAG` folder reproduces its tag assignments and file list; the first index run skips the tag stage for imported files; a question cites the right pages; a file saved into a root is indexed within a minute while `georag` is open.

### P3: service
Daemon (`serve`, discovery, token, endpoints, SSE), subcommands routed through it, resumable jobs, TUI Status. Accept: close the TUI, run `georag ask`, no model reload; `status --json` matches the Status screen.

### P4: vision, OCR, tag explore
Image captions, `ocr`, tag explore (propose new tags from clusters, as v1), record view OCR offer.

### P5: backup
`.georag` export and import for v2 (schema dump, not vectors unless `--with-vectors`).

### P6: native UI
Planned separately once P0 to P3 are stable; Tauri or Slint; embeds core or attaches to the daemon.

---

## 16. Conventions for the coding session

- Rust stable, edition 2024, `cargo fmt`, `cargo clippy -D warnings`, `cargo test` in CI on all three OSes (CPU path).
- Errors: `thiserror` enums in library crates, `anyhow` at the binary edge. Every user-facing error says what happened and what to do.
- Logging: `tracing`; file logs in `data/logs/georag.log` (daily rotation, 14 files); `RUST_LOG` honoured.
- No `unsafe` outside `georag-engine` and `georag-extract`, and each block has a comment stating the invariant.
- Tests: unit tests per crate; `georag-store` tests on an in-memory database; planner tests with synthetic chunk tables (fit, split, drop, fallback); importer tests against a fixture `metadata.db` and a fixture `.georag`; TUI `update()` tests with scripted messages; an integration test that runs the CPU path end to end on a tiny fixture library.
- User-facing text (TUI, CLI, README) in Australian English. No em dashes anywhere, including comments and docs; use commas, colons or semicolons.
- README must include: the four priorities, the "why not the browser app" reasons, the Metal now / MLX later note, the portability rules, and the update and import commands.
- Commit per logical step with a message that names the crate; no model names or session links in commits.
- Never read from or write to the v1 repository except as a read-only reference for `DEFAULT_TAGS`, the tagger patterns and the prompts in `backend/routers/chat.py` and `backend/services/llm_client.py`.
- Keep ADRs in `docs/adr/NNNN-title.md` for any deviation from this document, with the priority it serves.

---

## 17. Risks and open checks

| Risk | Check | Fallback |
|---|---|---|
| `llama-cpp-2` multimodal (captions) incomplete | P0 spike | captions via a small vision GGUF through the raw sys crate, or defer captions to P4 |
| CUDA bundle size (cudart, cublas ~400 to 600 MB) | P0 CI | ship Vulkan as the default Windows/Linux download, CUDA as opt-in |
| `sqlite-vec` loading on Windows with bundled rusqlite | P0 | build sqlite-vec from source in `build.rs`, or brute-force cosine over stored blobs (fine to ~300 k chunks) |
| `ocrs` accuracy on engineering scans | P4 | add a tesseract bindings feature, off by default |
| Cosine threshold calibration per embedding model | `doctor --scores` | per-model default in the catalogue |
| Legacy Windows console limits | P0 | document Windows Terminal as recommended; 16-colour theme |
| GitHub release asset size limits (2 GB per file) | fine | split CUDA bundle into binary + engine archives if ever needed |
| minisign key custody | before first signed release | key in CI secrets only; rotation procedure in section 11.3 |
