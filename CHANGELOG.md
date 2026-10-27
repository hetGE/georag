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
