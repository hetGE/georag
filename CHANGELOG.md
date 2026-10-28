# Changelog

## 2.0.0-dev

Rust edition, phases P0 to P2.

P0: workspace, home folder, settings, logging, backend probe, `doctor`,
`status`, the terminal UI chrome with the first-run checklist, release
bundling and the CI matrix.

P1: model registry, catalogue and resumable pulls; the resident llama.cpp
engine with the context ladder; hybrid retrieval over sqlite-vec and FTS5;
single-pass and auto-depth chat with skip, accept and stop, synthesis and
persisted passes; manual tiers; the Chat and Models screens; the updater
with staging, self-check and rollback.

P2: library roots, scanning with the v1 skip rules, extractors for pdf,
docx, pptx, xlsx, csv, text, html and rtf, token-sized chunking with page,
sheet and slide locators, rule and LLM tagging from tags.toml, embedding
and indexing as resumable jobs, a folder watcher, the Library and Settings
screens, and the v1 import (folder, metadata.db or .georag archive) with
its wizard.

Hardened against a real v1 library of 16,000 files and 47 GB: `status`
reports models, roots, file counts and jobs; scanning hashes in parallel
and writes a batch per transaction; the extract loop counts the queue
instead of loading it; legacy Office files and media are recorded as
"unsupported file type" rather than missing or failed; a parser panic on
one file is contained as a failure for that file; `import v1 --json`
emits only JSON; answers may cite ranges such as `[S4-S7]`; the GGUF
header reader compiles on MSVC.
