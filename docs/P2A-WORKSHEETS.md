# P2a: spreadsheets as worked examples, not literature

Status: proposal for review, revision B, 2026-10-28. Nothing in this
document is built yet. P2a sits between P2 (index and import) and P3
(service) in section 15 of the handoff. It changes how spreadsheet files are
digested, retrieved and shown.

Revision B, after review: the file kind and subkind are internal to the
engine. Nothing about them is exposed as a flag, a column, a setting or a
command. The engine decides from the question whether worksheets are
relevant and shows them only then.

## 1. Why

The handoff (section 6.3) treats a spreadsheet like a book: calamine dumps
every sheet as tab-separated rows, the chunker cuts the dump into 1000-token
pieces, and every piece is embedded and searched like a page of a textbook.
The first full index of the real library shows what that costs.

| Measure | Value |
|---|---|
| Spreadsheet files indexed | 1,202 of 15,763 (8 percent of files) |
| Chunks from spreadsheets | 197,788 of 435,242 (45 percent of chunks) |
| Median chunks per spreadsheet | 3 |
| Spreadsheets with more than 500 chunks | 23 |
| Largest single file | 50,927 chunks from one settlement workbook |
| Encrypted workbooks that failed | 20, nearly all calculation sheets |

Three problems follow.

1. **Wrong content.** A cell dump of a calculation sheet is a wall of numbers
   with the labels far from the values. Embedding it produces vectors that
   sit near every other wall of numbers, so a question about bearing capacity
   can pull a settlement sheet as a top hit and the model then quotes its
   intermediate values as if they were recommendations.
2. **Wrong role.** A spreadsheet in this library is not a source of knowledge.
   It is either a calculation method the user built or collected, which is
   most likely when the workbook is locked, or project data such as a
   production progress report, a test register or a tracker. The right answer
   to "how do I calculate bored pile capacity" is the literature plus "here
   are three worksheets in your library that do this calculation", not a
   paraphrase of the cells.
3. **Wrong cost.** Half the embedding time and 1.2 GB of the store went into
   chunks that will rarely be the right answer.

## 2. What changes, in one paragraph

Spreadsheets become a second kind of record inside the engine, the
**worksheet**. A worksheet is never chunked as prose. The pipeline builds one
compact **card** per file: what the workbook is called and where it lives,
its sheets and their size, the labels and headings it contains, how much of
it is formula, whether it is protected, and an internal classification as a
calculation, a report or a register. The card is embedded and searched in
its own lane. For every question the engine first predicts which kinds of
record the question is after. Literature is always retrieved and is the only
material the answer is written from. Worksheets are retrieved as well, and
when the prediction and the scores agree that the question wants a worked
calculation or a report example, the matching worksheets are attached under
the answer as examples. The user sees sources and, sometimes, examples. The
user never sees or sets a kind.

## 3. Data model

### 3.1 Migration 0002

```sql
ALTER TABLE files ADD COLUMN kind TEXT NOT NULL DEFAULT 'document';
ALTER TABLE files ADD COLUMN subkind TEXT;
CREATE INDEX files_kind ON files(kind);
UPDATE files SET kind = 'worksheet'
  WHERE lower(ext) IN ('xls','xlsx','xlsm','ods','csv','tsv');
UPDATE files SET state = 'stale'
  WHERE kind = 'worksheet' AND state IN ('indexed','failed');
```

`kind` is decided by extension at scan and import time and never by content.
Values on day one: `document` and `worksheet`. `drawing` and `image` are
reserved for P4 so the same switch can route them later.

`subkind` is filled by the worksheet stage: `calculation`, `report`,
`register` or `unknown`. It steers ranking and the wording of the examples
block. It is not a filter the user can set.

The last statement marks every worksheet stale, so the next `georag index`
rebuilds them as cards with no new flag and no user action beyond the
command they already know. `status` reports them as stale until then.

### 3.2 Chunks

No schema change. A worksheet's card is stored as its chunks (two to four of
them) with `locator = card`, so the store API, the record view and the
sources pane work unchanged. `chunk_vec` and `chunk_fts` fill as before.

Filtering by kind is a join on `files.kind` inside `Store::knn` and
`Store::fts`; `SearchFilter` gains a `kind: Option<FileKind>` field that
only core sets.

## 4. The worksheet stage (georag-extract, georag-core)

### 4.1 A separate path, the same queue

The supervisor keeps one `extract` job kind so queueing, resume, progress and
the watcher stay as they are. Inside the stage, `extract_file` looks at
`file.kind` and hands worksheets to a new `profile_worksheet` function. No
second supervisor loop is needed; a worksheet simply takes a different branch
and comes out as `extracted` with a card instead of a dump.

Tagging and embedding run unchanged after that. The only difference the
later stages see is that a worksheet yields a handful of chunks.

### 4.2 The card

`georag_extract::sheets::profile_workbook(path) -> WorksheetProfile`, built
from calamine's cell range and formula range per sheet, plus a direct read of
the xlsx zip for the two things calamine does not expose.

| Field | How | Why it matters |
|---|---|---|
| name, folder path | from the file row | the folder is often the project |
| sheets | name, rows, columns, filled cells | a 40-sheet workbook is a model, a 1-sheet one is a table |
| labels | every text cell, in reading order, up to 400 | labels carry the meaning: "Pile diameter", "Cu", "Settlement at centre" |
| numeric summary | per sheet: count of numeric cells, and for the first 12 columns under a text header the min and max | lets a reader see magnitudes without the whole table |
| formulas | count per sheet, share of filled cells, up to 20 distinct formulas with their left-hand label | the signature of a calculation method |
| defined names | calamine `defined_names()` | engineers name their inputs |
| protection | xlsx: `<sheetProtection>` or `<workbookProtection>` in the XML; xls: the PROTECT record if calamine exposes it, else skipped | the user's own rule: locked usually means a method |
| charts | xlsx: count of `xl/charts/chart*.xml` entries | plots mean results, not raw data |
| dates | min and max of date cells | project period |
| units seen | tokens such as kN, kPa, MPa, m, mm, deg found in labels | cheap signal for geotechnical content |

The card is rendered as plain text under a `[Card]` marker, labels first,
then the summary lines, capped at 1500 tokens. Numbers are never dumped row
by row. The caps are constants in core, not settings.

Rendering example, abbreviated:

```
[Card]
Worksheet: Kazık Oturma Hesabı (Haram Center).xls
Folder: _Bauer/_Specific to UAE/from Keith
Kind: calculation (protected, 61 percent formulas, 2 charts)
Sheets: Input (38 x 9), Settlement (412 x 14), Plot (12 x 3)
Labels: Pile diameter, Pile length, Cu, Es, Poisson ratio, Load, Settlement
  at pile head, Elastic shortening, ...
Formulas: 288, for example Settlement at pile head = Q*(1-v^2)*I/(Es*D)
Named ranges: D_pile, L_pile, Q_work
Units: kN, kPa, MPa, m, mm
```

### 4.3 Classification

`subkind` is decided by rules first, in this order, and by the LLM only when
the rules say `unknown`.

1. `calculation` when protection is on, or formulas are more than 15 percent
   of filled cells, or the name or folder matches a calculation pattern
   (hesap, calc, design, capacity, settlement, check, analysis, sheet names
   such as Input or Output).
2. `register` when one sheet dominates, dates or identifiers fill a leading
   column, formulas are under 5 percent, and the name matches tracker, log,
   list, register, schedule, contact, org chart.
3. `report` when the name or folder matches progress, report, daily, weekly,
   monthly, production, summary, quantity, BoQ, and formulas are moderate.
4. `unknown` otherwise; the existing LLM tag batch takes the card as its
   preview and is asked for both topic tags and the subkind in the same JSON
   answer, so no extra model call is spent.

The patterns are constants in `georag-core/src/worksheets.rs`. They are not
in `tags.toml`, because tags are the user's vocabulary and kinds are the
engine's.

### 4.4 Encrypted workbooks

The 20 files that fail with "Workbook is password protected" are encrypted,
not merely sheet-protected, and no library can read them without the
password. P2a stops treating them as failures. They get a name-only card
(file name, folder, size, modified date, "encrypted"), `subkind` from the
name rules with `calculation` as the default, and state `indexed` with the
note "encrypted, catalogued by name". They still surface as examples; the
user knows the password.

### 4.5 CSV and TSV

Treated as worksheets with `register` as the default subkind. A CSV is almost
always exported data, never a method. The card lists the header row and the
row count.

## 5. Retrieval and chat

### 5.1 Predicting what the question is after

Before retrieval, `chat::intent::predict(question) -> Intent` decides which
kinds of record the question wants. It is deterministic and costs nothing.

| Signal | Effect |
|---|---|
| verbs and nouns of doing: calculate, compute, estimate, design, check, size, how do I, worked example, spreadsheet, worksheet, template, hesap | `calculation` intent |
| report words: progress, production, daily, weekly, monthly report, summary, quantities, register, tracker, log, schedule, list of | `report` intent |
| neither | `literature` only |
| both | both |

The intent also carries a weight so that a question with "how do I
calculate" opens the example gate wider than a question that merely mentions
a spreadsheet in passing. Document retrieval runs for every intent; a
question is never answered from cards alone unless the literature lane comes
back empty.

The prediction is written to the saved message's `mode` field for later
inspection, and the Status screen in P3 can count how often each intent
fires. Neither is shown in the chat.

### 5.2 Two lanes

`hybrid_retrieve` gains a `kind` argument. The chat runner calls it twice per
question.

1. **Literature**: `kind = document`, `candidate_k` as today. This is the
   only input to the planner, the passes and the `[S<n>]` citations.
   Removing worksheet chunks from this lane is itself a quality fix: 45
   percent of the index can no longer crowd out a textbook page.
2. **Worksheets**: `kind = worksheet`, `k = 8`, always run because it is one
   KNN over about 4,000 card chunks and takes milliseconds.

### 5.3 The gate

Worksheets are attached to the answer only when both of these hold.

- The predicted intent includes `calculation` or `report`, or no intent was
  predicted but a card scores above the best literature chunk, which means
  the question was really about something the user has a sheet for.
- The card's cosine is above `auto.min_relevance` plus a margin that the
  intent weight lowers: a strong "how do I calculate" accepts cards at the
  plain threshold, a weak signal needs a clear margin.

Cards are ranked by cosine, with the subkind that matches the intent moved
ahead: calculation cards first for a calculation question, report and
register cards first for a report question. At most five are attached.

When the gate is closed, the question is answered exactly as today and no
worksheet is mentioned. Nothing about the attempt is visible.

### 5.4 How examples reach the user

Examples are attached deterministically, after the answer, as a block the
CLI prints and the TUI shows under its own heading in the sources pane:

```
examples in your library:
  [E1] _Bauer/_Specific to UAE/from Keith/Kazık Oturma Hesabı (Haram Center).xls
       pile settlement calculation, protected, 3 sheets, 288 formulas
  [E2] _Worksheets and Programs/.../Settle Calcs_Schmertmann.xls
       settlement calculation, 5 sheets, 1,204 formulas
  [E3] Tekinvest_DD_Rapor/.../Pile Production Progress W32.xlsx
       production report, 1 sheet, 2019-08-05 to 2019-08-11
```

The one-line description is built from the card: the subkind in words, the
protection flag, the sheet count, the formula count or the date span. The
word "calculation" or "report" is a description of the file, the same way
"p.132" describes a page; it is not a category the user can query.

The cards are also sent to the model, tagged `[E<n>]` rather than `[S<n>]`,
with one added sentence in the system prompt: "Worksheets tagged [E<n>] are
examples from the user's own library. You may point to one as a worked
example of a method or a report; never quote their values as facts." The
cost is at most five cards of about 300 tokens each, taken from the chunk
budget before the literature is planned. `cited_tags` learns the `E` prefix
so a cited example is marked as such in the saved message.

If the question asks for a calculation or a report and the literature lane
is empty, the answer reads "No literature covers this; these worksheets do",
which the existing "say so plainly" instruction already produces.

### 5.5 Commands and screens

No new command, flag or setting. The visible changes are:

- `ask`, `chat` and the TUI Chat screen print the examples block when the
  gate opens.
- `search` returns literature chunks and cards alike, ordered by score; a
  card's snippet is its first lines, so a worksheet hit reads as a worksheet.
- The Library record view shows the card as the file's text, because the card
  is the file's text.
- `index` after the migration rebuilds the worksheets because they are stale.

## 6. Re-index of the existing library

Migration 0002 marks the 1,224 worksheets stale. The next `georag index`
deletes their 197,788 chunks and builds about 4,000 cards. Expected on the
M4 Max: a few minutes of extraction (the large workbooks still have to be
parsed, but nothing is tokenised row by row) and under five minutes of
embedding. The store shrinks by roughly 1.2 GB after `VACUUM`, which
`library prune` runs.

Documents are untouched; no second full index.

## 7. Files touched

| Crate or doc | Change |
|---|---|
| `georag-store/migrations/0002_file_kind.sql` | kind, subkind, index, backfill, worksheets marked stale |
| `georag-store/src/files.rs` | `FileKind`, `kind` and `subkind` on `File`, `files_set_subkind` |
| `georag-store/src/search.rs` | `SearchFilter.kind`, join on files in `knn` and `fts` |
| `georag-extract/src/sheets.rs` | `profile_workbook`, `WorksheetProfile`, card renderer, xlsx zip probe for protection and charts; `extract_workbook` stays for tests |
| `georag-extract/src/lib.rs` | `Extracted.card: bool` so the chunker knows not to split prose-wise |
| `georag-core/src/pipeline/chunker.rs` | `[Card]` marker; a card splits at line boundaries only, never mid-line |
| `georag-core/src/pipeline/stages.rs` | branch on `file.kind`; `profile_worksheet`; encrypted path |
| `georag-core/src/pipeline/scan.rs`, `import_v1.rs` | set `kind` from extension |
| `georag-core/src/worksheets.rs` (new) | classification rules and patterns, card description line |
| `georag-core/src/tags.rs` | subkind in the LLM JSON answer |
| `georag-core/src/chat/intent.rs` (new) | intent prediction |
| `georag-core/src/retrieval.rs` | `kind` argument |
| `georag-core/src/chat/runner.rs`, `prompts.rs`, `planner.rs` | worksheet lane, the gate, `[E<n>]`, budget reservation, system sentence |
| `georag-cli/src/main.rs` | examples block in `ask` and `chat` |
| `georag-tui` | examples heading in the Chat sources pane |
| `docs/GEORAG-RS-HANDOFF.md` | 6.3 row for spreadsheets rewritten; new 6.7 Worksheets; new 7.7 Intent and examples; section 15 gains P2a with acceptance |
| `docs/adr/0003-worksheets-are-examples.md` | the deviation from handoff 6.3 and why |
| `README.md` | a "Spreadsheets are examples" paragraph |
| `CHANGELOG.md` | P2a entry |

Also folded into P2a, since the embed loop is reopened anyway: the per-file
info log line for embed, and the queue count query in place of loading every
queued job after each file.

## 8. Tests

- `sheets`: a fixture xlsx written in the test as a zip of hand-made XML with
  two sheets, formulas, a protected sheet and a chart part; assert the
  profile's counts, labels, formula sample and protection flag. A CSV fixture
  for the register path.
- `worksheets`: classification table tests, one row per rule, including the
  encrypted name-only case; description lines.
- `intent`: a table of questions and the expected intent and weight,
  including Turkish calculation words and questions that mention a
  spreadsheet without wanting one.
- `chunker`: a card never splits mid-line and never exceeds the cap.
- `store`: `knn` and `fts` with `kind` return only that kind; migration 0002
  backfills from `ext` and marks worksheets stale.
- `chat::runner`: the gate stays closed for a pure literature question even
  when a card scores moderately; opens for a calculation question; opens
  without intent when a card beats the best chunk.
- `chat::prompts`: `cited_tags` with `[E2]` and mixed `[S1][E1]`.

## 9. Acceptance

1. `georag index` after the migration on the real library finishes the
   worksheets in under 15 minutes and the chunk count falls from 435,242 to
   about 240,000.
2. "How do I calculate the settlement of a bored pile in clay" returns
   literature sources under `sources:` and three to five worksheets under
   `examples:`, with a Bauer pile settlement calculation among them.
3. "What ultimate skin friction values are recommended for bored piles in
   stiff clay" returns the same literature answer as today and no examples.
4. "Show me a pile production progress report" lists report worksheets from
   the Tekinvest and Bauer folders as examples, and the answer says the
   literature does not cover it.
5. The 20 encrypted workbooks are `indexed` with the encrypted note and can
   appear as examples by name.
6. No worksheet chunk appears in any `[S<n>]` source.
7. `georag --help`, `settings.toml` and the Settings screen are unchanged.
8. Clippy, fmt and the test suite are clean on the three CI targets.

## 10. Commits, in order

1. store: file kind and subkind, migration 0002, kind filter in search
2. extract: worksheet profiles and cards
3. core: worksheet stage, classification rules, kind on scan and import
4. core: intent prediction, worksheet lane and the examples gate in chat
5. cli, tui: examples block under answers
6. docs: handoff 6.7 and 7.7, ADR 0003, README, changelog

Each step builds and tests on its own, so the plan can be stopped after any
of them with the library still usable.

## 11. Questions for review

1. Should the cards go to the model at all, or only be listed under the
   answer? Sending them lets the model say "worksheet [E2] does this
   calculation" in context; listing only is simpler and can never leak a
   number. The proposal sends them, with the guard sentence.
2. Is the gate right? The proposal opens it on predicted intent or on a card
   outscoring every literature chunk. A stricter rule would be intent only.
3. Should the numeric summary (min and max per labelled column) be on the
   card? The proposal keeps a small summary because "settlement 12 to 48 mm"
   helps the engine match a question about magnitudes.
4. `.xls` files that calamine cannot parse (the one overflow crash, the
   undetected format): name-only card like the encrypted ones, or leave as
   failed? The proposal catalogues them by name.
5. Should the intent words include Turkish throughout (hesap, rapor,
   ilerleme, metraj), given the library? The proposal says yes.
