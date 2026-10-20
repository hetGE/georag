# ADR 0001: one HTTP client for model pulls and updates, not hf-hub

Date: 2026-10-20. Status: accepted. Deviates from handoff section 13 ("Downloads: hf-hub with resume").

## Decision

Model pulls use the same blocking `reqwest` client as the updater, with HTTP
Range resume of `.part` files, pause through a cancel flag, speed and ETA from
the byte counter, and sha256 taken from the Hugging Face tree API
(`/api/models/{repo}/tree/main`, field `lfs.oid`).

## Why

`hf-hub` 1.0 is async-first (its `blocking` feature wraps a tokio runtime),
pulls in a second HTTP stack beside the updater's, and does not resume a
partially written file across restarts; its Range support is a per-call
parameter. Pulls must survive leaving the Models screen and restarting the app
(handoff section 9.2), which needs resume from the `.part` size. One client,
one proxy configuration (`HTTPS_PROXY`), one code path for progress.

## Priority served

Priority 3 (native speed: one stack, no runtime hop) and priority 2
(portability: nothing cached outside the home folder; hf-hub keeps its own
cache directory by default).
