# ADR 0002: one unsafe block in georag-store to register sqlite-vec

Date: 2026-10-20. Status: accepted. Deviates from handoff sections 2.1 and 16 ("unsafe and FFI live only in engine and extract").

## Decision

`georag-store` contains exactly one `unsafe` block: the call to
`sqlite3_auto_extension` that registers `sqlite3_vec_init` before the first
connection opens. The block carries a comment stating the invariant (the
function has the loadable-extension entry signature, and registration happens
once per process through `std::sync::Once`). The workspace lint is
`deny(unsafe_code)`; every crate other than engine, extract and store adds
`#![forbid(unsafe_code)]` so the allowance cannot spread.

## Why

The vector table is a SQLite extension and the only way to load one into a
bundled `rusqlite` build is the C auto-extension hook. Moving that call to
the engine crate would make the store depend on the engine, inverting the
dependency direction in section 2.1, which is the larger deviation.

Also noted: the newest `sqlite-vec` crate on crates.io (0.1.10-alpha.4) does
not compile because its C amalgamation includes a file the package omits;
the store pins 0.1.9, the latest stable release.

## Priority served

Priority 3: exact KNN inside the one database file, no second index store.
