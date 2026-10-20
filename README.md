# GeoRAG

A local assistant for geotechnical engineers. GeoRAG indexes a document
library (PDF, Word, Excel, PowerPoint, images, text, CAD by filename),
classifies files into topic tags, and answers questions grounded in those
files with page-level citations. Everything runs on your machine through
llama.cpp: no cloud, no API keys, no browser, no Python.

This is the Rust edition (v2). The Python v1 app remains as a read-only
reference. The build plan is `docs/GEORAG-RS-HANDOFF.md`.

## The four priorities

Every design question is settled by these, in this order.

1. **Memory goes to the model.** No browser, no Python runtime, no second
   server process. Models stay resident while the app runs; nothing else
   competes for that memory.
2. **Truly portable.** The app, its models, its index and its settings are
   one folder, on the machine or on external storage. Nothing is installed,
   nothing is written outside the folder.
3. **Native speed.** Compiled Rust, llama.cpp in-process, embeddings batched
   through one call, extraction in a CPU pool.
4. **Cross-platform by folder copy.** Same OS and GPU class: copy the folder.
   Different GPU class: swap the binary flavour, keep everything else.

## Why not the browser app

v1 ran a Python backend, a separate `llama-server` and a browser front end.
On unified memory (Apple silicon) the browser's tabs are context the model
cannot have, and on RTX systems layers that spill from VRAM land in the same
RAM the rest of the stack is using. Three processes also meant a supervisor,
pid files, port adoption and an installer. v2 is one binary with a
full-screen terminal UI and plain subcommands.

## Apple GPU: Metal now, MLX later

Apple silicon runs through ggml Metal inside llama.cpp. MLX is a future
option behind the `InferenceEngine` trait once `mlx-rs` matures; nothing in
the plan depends on it.

## Portability rules

- The home folder is the folder containing the executable. Override with
  `--home DIR` or `GEORAG_HOME`. `georag init` creates the layout.
- Nothing is written outside the home folder: no registry keys, no
  `~/.config`, no temp state that matters.
- Document paths are stored relative to their library root, so roots can be
  re-pointed when a drive letter or mount changes.
- A read-only home folder refuses to index or update with a clear message.
  GeoRAG never escalates privileges.

```
GeoRAG/
├─ georag(.exe)
├─ engine/        ggml backend libs, pdfium, OCR models
├─ config/        settings.toml, models.toml, tags.toml
├─ models/        managed GGUF downloads
├─ data/          georag.db, logs, cache, update staging
└─ exports/       backup archives and exported conversations
```

## Commands

```
georag                 full-screen UI
georag init            create the home layout
georag doctor          build flavour, backend, device memory, model fit
georag update          check, download, verify and apply a release
georag import v1 PATH  import tags and the file list from a v1 _0RAG folder
```

The full command surface is in section 12 of the handoff document. Commands
land phase by phase; `georag --help` lists what this build has.

## Building

Rust stable (1.85 or later), CMake and a C++ toolchain. llama.cpp is compiled
from source by the `llama-cpp-sys-2` crate.

```
cargo build --release                    # CPU, plus Metal on macOS
cargo build --release --features cuda    # CUDA flavour (Windows, Linux)
cargo build --release --features vulkan  # Vulkan flavour (Windows, Linux)
```

Run against a scratch home while developing:

```
cargo run -- --home .dev-home init
cargo run -- --home .dev-home doctor
```
