# sqview Rust architecture and cleanup audit

Audit date: 2026-09-06 (Europe/Helsinki). Base revision: `0eab019e5f76fd47cc12ce0fc16978e7c684a2dc`, including the existing uncommitted changes to `README.md`, `src/app.rs`, `src/ui/popup/help.rs`, and `src/ui/popup/text_editor.rs`.

Requested methods: Rustitect, Codebase Kitchen Cleanup, Super Rust Engineer, and Crusty. The baseline below records the pre-fix state; the findings were fixed for v0.2.6 and their regressions were migrated into the normal Rust test suite.

## Executive summary

The main risk is the write boundary: it does not reliably establish which row and value the user intended to change. There are **13 prioritized findings**, supported by source inspection and **18 failing assertions of intended behavior**. The failures are audit discoveries, separate from the existing suite of 197 passing tests.

1. **Data loss:** a declared `rowid` column can make one-row deletion remove multiple rows. Cached cells can also be paired with a different row's current identity.
2. **Write lifecycle:** repeated submission inserts twice; single-row deletion stores no usable undo data; undo bypasses the session read-only policy in some branches.
3. **Value integrity:** invalid numeric editor input commits as `NULL`; selecting a BLOB from the value picker returns its display label as text.
4. **Boundary safety:** persisted filter paths permit traversal and collide between databases. SQL identifiers are not consistently escaped.
5. **Asynchronous state:** old reads overwrite newer query state, failures leave loading gates stuck, and external refresh misses structural changes.
6. **Scale and verification:** exports materialize entire tables, synchronous queries block the UI loop, and a test helper silently discards messages.

The existing single-crate design is appropriate. Keep rusqlite/r2d2 and LIMIT/OFFSET, as required by `DECISIONS.md`. Strengthen identity, task ownership, and value conversion before reorganizing files.

## Architecture map

```text
main: CLI, startup, terminal lifetime, filesystem watcher, Tokio event loop
  └─ App: session state + input routing + jobs + SQL + writes/undo + clipboard
       ├─ db: pool, schema introspection, row queries, transactions
       ├─ filter: persisted rules + SQL compilation + filesystem state
       ├─ export: repeats query setup + manual CSV/JSON/SQL serialization
       │    └─ grid::SortSpec                 [presentation-owned query type]
       └─ grid / ui: navigation, editors, rendering
            └─ db::Column / SqlValue

config / app_dirs → theme / symbols and filesystem locations
background jobs → unbounded Message channel → App::update
```

Cargo contains one Rust 2021 binary, `sqview` 0.2.5, 24 direct dependencies and 224 locked packages including the root package. There are no project feature flags, workspace subcrates, declared MSRV, or repository toolchain pin. CI and releases target Linux with stable Rust. No application `unsafe` or shared-state `Mutex`/`RwLock` was found. `Arc<DbPool>` expresses legitimate shared access; replacing it with a giant locked application state would make ownership worse.

`App` has several independent reasons to change: SQL semantics, persistence, database observation, request lifecycle, undo, keyboard/mouse policy, and clipboard formatting. This is the reason to separate responsibilities; its roughly 4,100 lines before the test module are supporting evidence, not the finding by themselves. The large grid renderer is substantially more cohesive and does not need an arbitrary file split.

## Baseline and evidence

Host: `x86_64-unknown-linux-gnu`, `rustc 1.94.1 (e408947bf 2026-03-25)`.

| Verification actually run | Result |
|---|---|
| `cargo metadata --no-deps --format-version 1` | One binary, no project features |
| `cargo tree -d`, `cargo tree -e features --depth 1` | Reviewed direct activation and duplicate-version parents |
| `cargo check --workspace --all-targets` | Pass |
| `cargo test --workspace --all-targets` | **197 passed** |
| `cargo fmt --all -- --check` | Pass |
| `cargo clippy -- -D warnings` | Pass; this matches the existing CI lint scope |
| `cargo clippy --workspace --all-targets -- -D warnings` | Fail: three diagnostics in test-target code; see A13 |
| `python3 docs/audits/2026-09-06-reproduce.py` | **18 assertions fail**, exit 101, each exposing an existing defect |
| Dependency-pruning experiment, `cargo check --offline --all-targets` in a temporary copy | Pass; see cleanup inventory |

The [reproduction script](2026-09-06-reproduce.py) now runs the permanent regression suite with isolated config and data paths. The [captured results](2026-09-06-regressions.txt) preserve the original failing baseline and append the post-fix verification.

## Resolution in v0.2.6

All 13 findings were addressed. Row reads now carry a safe hidden rowid alongside displayed values; tables without rowids browse by ordered primary keys and disable unsafe mutations. Read requests have generations and terminal failures, write submission is gated, deletion captures undo data atomically, and undo history changes only after completion. Identifier quoting and filter validation are centralized at their boundaries. Filter storage uses path-derived database identities and encoded table names. Export streams atomically through the CSV and JSON libraries. Schema comparison includes columns and capabilities, the watcher observes the database directory for main/WAL/SHM changes, and the initial grid load no longer performs per-column distinct scans or random table scans. CI and release validation now lint and test all targets.

The audit reviewed application flows, database boundaries, filter persistence/compilation, export, grid state, popup conversion, startup, Cargo and CI. It did not conduct interactive terminal testing, benchmark release latency, run packaging/cross-platform builds, or perform a dependency advisory audit. Dependency-level unsafe code was outside scope. No performance improvement is claimed.

## Findings

### A01 [CRITICAL] A declared `rowid` can make a single-row delete remove multiple rows

**Location:** [row lookup](../../src/db/mod.rs#L319), [deletion](../../src/db/write.rs#L89), [schema types](../../src/db/schema.rs#L10). Crusty: `PRB-0003`.

**Evidence:** `CREATE TABLE t(rowid INTEGER, value TEXT)` with `(7,'a'), (7,'b')` is valid SQLite. The lookup returns the declared value `7`; `delete_row` executes `WHERE rowid = 7`, accepts any affected count, and commits both deletions. The regression leaves **zero rows instead of one**. SQLite explicitly documents that declared columns shadow its special rowid names. [SQLite rowid rules](https://www.sqlite.org/rowidtable.html).

**Why it matters:** this is directly reproduced data loss, not a speculative injection scenario. Multi-selection deletion has the same identity assumption. Cell updates at least roll back when their affected count is not exactly one; deletion lacks that protection.

**Recommended boundary and structure:** `db/schema.rs` must establish a usable identity, represented in `db/rows.rs` as `RowKey` and attached to each fetched record. Hide the chosen SQLite identifier behind `db/query.rs`; callers must not select an identity spelling themselves.

**Refactoring and verification:** first reject destructive operations without a verified unique identity and enforce one affected row for a single-row mutation. Then support an unshadowed hidden alias or a suitable declared key. Test declared `rowid`, `_rowid_`, `oid`, duplicate/NULL values, and tables with no safe editable identity. Never infer uniqueness from the column name.

### A02 [HIGH] Cached cell values and later offset lookups can refer to different rows

**Location:** [focused cell context](../../src/app.rs#L3523), [offset lookup](../../src/app.rs#L3904), [window storage](../../src/grid/virtual_scroll.rs#L3), [selection deletion](../../src/db/write.rs#L126). Crusty: `PRB-0002`.

**Evidence:** the window stores only values. After caching `user-0`, deleting database row 0 externally, and opening that cached cell, `focused_cell_context` returns **rowid 1 with value `user-0`**. It also substitutes `NULL` when a requested cached cell is absent. Selected-row confirmations retain offsets and resolve them later against whatever rows then occupy those positions.

**Why it matters:** the user can edit or delete a row they never selected. Sort stability within one query does not establish identity across two queries or external writes.

**Recommended boundary and structure:** fetch `RowRecord { key, values }` atomically in `db/rows.rs`. `grid::VirtualWindow` consumes those records. `app/editing.rs` accepts a captured key and original value, never a freshly reinterpreted offset.

**Refactoring and verification:** carry keys through cached rows, search hits, selection and confirmations; reject editing unloaded cells. Add a compare-before-write conflict check using the captured original value where appropriate. Preserve OFFSET for navigation. Re-run the cached-cell regression and add intervening insert/delete/sort/filter cases for single and multi-row operations.

### A03 [HIGH] Background reads lack request identity and a terminal failure transition

**Location:** [WindowReady](../../src/app.rs#L409), [CycleSort](../../src/app.rs#L1058), [read jobs](../../src/app.rs#L3681), [window job](../../src/app.rs#L3750), [FindReady](../../src/app.rs#L1989). Crusty: `PRB-0004`.

**Evidence:** sorting during an in-flight fetch sets `needs_fetch`, but the old `WindowReady` accepts rows by table name alone and clears both loading and the pending refetch. A deterministic message-order test reproduces this. Filter requests can also overlap. `FindReady` carries no table or request identity. Several workers send a response only on `Ok(Ok(...))`, discarding database failures and join failures.

**Why it matters:** the displayed sort/filter can disagree with the rows. An invalid regex or failed database read can leave the grid loading forever. A stale completion can populate a newer popup.

**Recommended boundary and structure:** `app/read_jobs.rs` owns monotonic request IDs, the active query generation, job handles, and `Result` completions. Keep state private behind start/complete/invalidate operations.

**Refactoring and verification:** every job must produce success or failure; accept completion only for its owner and generation; clear loading on both outcomes; retain pending invalidation when an old job completes. Bound admission and coalesce obsolete reads. Dropping a task handle alone is not a policy for stopping synchronous SQLite work. Test reordered completions, invalid regex, connection failure, closing/reopening a popup, and quit during a long query.

### A04 [HIGH] Writes can be submitted twice and undo is not a reliable transaction lifecycle

**Location:** [insert submission](../../src/app.rs#L1329), [delete submission](../../src/app.rs#L1516), [undo recording](../../src/app.rs#L1617), [undo execution](../../src/app.rs#L1691), [restore helper](../../src/db/write.rs#L232). Crusty: `PRB-0005`.

**Evidence:** submitting one staged insert twice before processing completion creates rowids **1 and 2**. The form has no submitting state. Single-row deletion explicitly drops the cloned columns and records `UndoOp::Delete` with an empty payload, so restoration is impossible. Undo pops history and reports success before the worker finishes. Undo of an insert deletes data even with the session read-only toggle enabled; this is reproduced on a writable connection.

**Why it matters:** duplicate writes, unusable undo, lost history after failure, and a bypassed user protection. The CLI's actual read-only file connection still enforces SQLite's open mode; the demonstrated bypass concerns the runtime toggle on a writable connection. `INSERT OR REPLACE` in the currently unusable restore path could overwrite a conflicting row if restoration were enabled without fixing conflict semantics.

**Recommended boundary and structure:** `app/editing.rs` owns `Editing/Submitting` state and a single write policy; `app/history.rs` owns typed undo entries and pending undo; `db/write.rs` owns atomic mutations and their outcomes.

**Refactoring and verification:** lock admission before spawning, capture deleted data in the same transaction as deletion, and return it in `WriteOutcome`. Apply the read-only policy to every mutation, including undo. Remove an undo entry only after confirmed success; refresh only after completion. Restore with conflict detection, not replacement. Decide and expose bulk-delete undo behavior explicitly. Test duplicate submit, failed undo, delete/restore round trip, read-only undo and completion after changing tabs.

### A05 [HIGH] Filter persistence permits path traversal and aliases distinct databases

**Location:** [filter paths and persistence](../../src/filter/mod.rs#L7). Crusty: `PRB-0006`.

**Evidence:** the storage key is the database file stem plus a raw table name. `/tmp/one/data.db` and `/tmp/two/data.db` produce the same filter path. A table named `/tmp/escaped` produces `/tmp/escaped.toml`, outside the state directory. `save_filter` then creates parents and writes that destination. The audit tested path construction; it did not overwrite any real external file.

**Why it matters:** browsing an untrusted database and changing a filter can overwrite a writable `.toml` destination selected by its schema. Ordinary same-named databases also share or overwrite each other's filters. Save/load errors are currently ignored at application call sites.

**Recommended boundary and structure:** private `filter/store.rs` owns a stable database identity and filename-safe table key. Schema names remain data, never path components.

**Refactoring and verification:** derive keys from the canonical database location with an explicit in-memory policy, encode table names reversibly or with a stable collision-resistant key, and publish files atomically. Migrate legitimate old state only at this boundary. Test absolute paths, `..`, separators, duplicate basenames and interrupted writes. Distinguish missing state from corrupt or unreadable state.

### A06 [HIGH] Editor conversions discard invalid values and SQLite storage types

**Location:** [commit conversion](../../src/app.rs#L840), [editor validation](../../src/ui/popup/text_editor.rs#L242), [distinct values](../../src/db/mod.rs#L369), [picker conversion](../../src/ui/popup/value_picker.rs#L58). Crusty: `PRB-0007`.

**Evidence:** entering `30x` in an INTEGER editor sets `valid = false`, yet CommitEdit ignores that state and `as_sql_value` turns the parse failure into `NULL`. The actual database value changes from 30 to NULL. A BLOB `X'AB'` becomes the distinct label `<blob 1 bytes>` and selecting it produces `SqlValue::Text`, not the original BLOB. Direct BLOB editing also starts from an empty string.

**Why it matters:** validation is presentation-only and formatted text is mistaken for domain data. SQLite's dynamic storage values cannot be reconstructed reliably from declared column type and a label.

**Recommended boundary and structure:** `db/value.rs` owns lossless database-value conversion. Editors expose fallible `edited_value()` results. Picker options retain `SqlValue` separately from display/search text. Share parsing policy with insertion where the semantics match; keep explicit `NULL` distinct from empty text and invalid input.

**Refactoring and verification:** move validation to the commit boundary, retain typed picker values, and reject or provide a lossless editor for unsupported values. Re-run numeric and BLOB regressions. Add empty text/NULL, numeric overflow, mixed storage types, malformed JSON, non-finite real values and no-change round trips.

### A07 [MEDIUM] SQL identifier quoting is duplicated and incomplete

**Location:** [schema SQL](../../src/db/mod.rs#L117), [write SQL](../../src/db/write.rs#L14), [filter SQL](../../src/filter/predicate.rs#L31), [export SQL](../../src/export/mod.rs#L144), [FK SQL](../../src/app.rs#L642). Crusty: `PRB-0008`.

**Evidence:** table `a"b` loads as a schema name but fails `PRAGMA table_info("a"b")`. The same wrapping without escaping occurs in SELECT, UPDATE, DELETE, filtering and export. Values are generally parameterized, which is good, but that does not quote identifiers.

**Recommended boundary and structure:** one private identifier encoder in `db/query.rs`, shared by every SQL construction path; move application-side FK and navigation SQL there. Use parameterized table-valued PRAGMA queries where appropriate.

**Refactoring and verification:** migrate all identifier interpolations in one bounded change; eliminate duplicate order/WHERE builders in `db/write.rs`. Test table and column names containing quotes, spaces, keywords and Unicode across introspection, browsing, filtering, editing, deletion and export. The proven failure is valid-schema rejection; this audit does not claim a separately reproduced arbitrary-SQL exploit.

### A08 [MEDIUM] Schema metadata cannot represent supported SQLite key and column semantics

**Location:** [schema model](../../src/db/schema.rs#L10), [column introspection](../../src/db/mod.rs#L117), [foreign keys](../../src/db/mod.rs#L133), [ordering](../../src/db/mod.rs#L220). Crusty: `PRB-0009`.

**Evidence:** `WITHOUT ROWID` tables fail ordinary row fetching because every order includes `rowid`. For `FOREIGN KEY(x,y) REFERENCES p` where `p` has primary key `(a,b)`, both child fields resolve to `a`; the second should resolve to `b`. `Column.is_pk` drops key ordinal, and `ForeignKey` drops constraint grouping and sequence. `table_info` also omits generated columns. [SQLite introspection documentation](https://www.sqlite.org/pragma.html#pragma_table_xinfo).

**Recommended boundary and structure:** `db/schema.rs` owns ordered key columns, grouped foreign keys and column/table capabilities; expose read/write capabilities rather than making the UI guess.

**Refactoring and verification:** use complete introspection, preserve FK `(id, seq)` and PK order, and model generated/non-writable columns. Build browsing independently of editability. A safe first stage disables unsupported mutations with an explicit reason. Test composite implicit/explicit FKs, generated columns and WITHOUT ROWID browsing. Views are listed but not openable through the current sidebar/app path; define that capability deliberately rather than treating a view name as an editable table.

### A09 [MEDIUM] External refresh overlooks changed columns and WAL commits

**Location:** [watcher](../../src/main.rs#L220), [FileChanged](../../src/app.rs#L1867), [schema comparison](../../src/app.rs#L1906). Crusty: `PRB-0010`.

**Evidence:** after `ALTER TABLE users ADD COLUMN extra TEXT`, passing a freshly loaded schema to ExternalRefresh leaves the application at **four columns instead of five** because it compares only names and object kinds. The watcher observes only the main file. SQLite WAL commits append to a separate WAL file until checkpointing. [SQLite WAL documentation](https://www.sqlite.org/wal.html). The latter is a source-supported gap, not an OS-event reproduction in this audit.

**Why it matters:** stale columns, relationships and row data persist. The 500 ms own-write suppression can discard unrelated external writes. A failed schema probe never clears `file_check_in_flight`, as discussed in A03.

**Recommended boundary and structure:** `db/observe.rs` owns invalidation signals; `app/session.rs` applies schema/data generations and rebuilds affected table state.

**Refactoring and verification:** track database changes using a connection-aware version probe or correctly scoped directory/WAL observation, compare full relevant schema or schema version, and debounce into a pending invalidation instead of dropping events. If using `PRAGMA data_version`, compare values on the same long-lived connection. Test WAL-only commits, ALTER TABLE, file replacement, failed probes, and an external write immediately after an own write.

### A10 [MEDIUM] Export uses inconsistent serialization and unbounded materialization

**Location:** [export functions](../../src/export/mod.rs#L11), [CSV escaping](../../src/export/mod.rs#L55), [JSON keys](../../src/export/mod.rs#L99), [destination selection](../../src/app.rs#L2064). Crusty: `PRB-0011`.

**Evidence:** a single field `a\rb` is exported as two CSV records because CR is not escaped. A newline in a column name produces invalid JSON because keys escape only quotes. Both round-trip regressions fail. All three exporters fetch with `i64::MAX` and hold all rows, including BLOBs, before writing. Repeated jobs use the same per-format filename and `File::create` truncates existing output.

**Recommended boundary and structure:** `export/mod.rs` owns a shared row stream plus format-specific writers; `db/query.rs` owns the selection. Exports should consume query types owned by the database/query boundary, not `grid::SortSpec`.

**Refactoring and verification:** use the already installed CSV and JSON serializers, share query selection with browsing, stream through a buffered writer, and atomically publish completed output. Serialize concurrent exports per destination or allocate distinct destinations. Define BLOB and non-finite JSON behavior explicitly. Test round trips, writer failure, existing destination preservation, concurrent export and large BLOB datasets. Target O(one row + output buffer) memory instead of O(entire result payload).

### A11 [MEDIUM] Synchronous UI work and repeated scans undermine virtual scrolling

**Location:** [distinct lookup](../../src/app.rs#L730), [copying](../../src/app.rs#L2156), [initial fetch](../../src/app.rs#L3681), [window fetch](../../src/app.rs#L3769), [row lookup](../../src/app.rs#L3904), [bulk deletion](../../src/db/write.rs#L154). Crusty: `PRB-0012`.

**Evidence:** event handling synchronously acquires pooled connections and runs distinct, count, identity, offset and selection queries. Copying selected rows performs one OFFSET query per selected offset, on the event-loop thread. Each window fetch recounts the filtered result; initial loading scans distinct values for every column and performs `ORDER BY RANDOM()` sampling. Bulk deletion similarly resolves every offset independently.

**Why it matters:** a small result limit does not bound scan/sort cost. For k increasing offsets in an unindexed view of N rows, repeated scans can approach O(kN), and selecting most rows approaches O(N²). Background execution alone does not reduce that cost. These are source/complexity findings; no measured latency claim is made.

**Recommended boundary and structure:** run database operations through the bounded read/write owners, cache counts by query/data generation, and let batch selection operate on captured keys or a single transaction-scoped traversal.

**Refactoring and verification:** preserve OFFSET navigation, remove repeated per-row resolution, defer advisory color/width work, and cache regex compilation per prepared-query context. Benchmark release-mode opening, scrolling, copy, delete and export with 100k/1m rows, wide text and BLOBs. Inspect query plans and separately measure UI latency, query time and memory. Do not replace navigation with keyset pagination without revisiting the explicit design decision.

### A12 [MEDIUM] Filter parsing does not establish a valid predicate

**Location:** [REGEXP function](../../src/db/mod.rs#L25), [predicate compiler](../../src/filter/predicate.rs#L17), [formula fallback](../../src/filter/predicate.rs#L230), [popup rule creation](../../src/ui/popup/filter.rs#L251). Crusty: `PRB-0013`.

**Evidence:** filtering a column containing NULL and `abc` with regex `abc` fails the entire query because the UDF requires `String` for each cell. Invalid regexes are admitted by the popup. Independently serializable `FilterOp` and `FilterValue` permit incompatible combinations that the SQL compiler maps to `NULL` or `1=1`; invalid formulas also become `1=1`. Formula is a persisted-model path, not an operator currently offered by the popup.

**Recommended boundary and structure:** `filter/rule.rs` owns validated rule variants; `filter/store.rs` converts legacy serialized data once; SQL compilation is fallible until all supported rules establish their invariants. REGEXP's NULL semantics belong to the database adapter.

**Refactoring and verification:** validate regexes at rule creation/load, define NULL handling, reject invalid formulas and mismatched serialized values, and surface errors without broadening the query. Preserve real persisted-format compatibility at the loader. Test regex NULL/invalid syntax, malformed stored rules, missing columns, and literal wildcard behavior distinct from LIKE.

### A13 [MEDIUM] The test dispatcher drops messages and CI excludes test-target linting

**Location:** [test helper](../../src/app.rs#L4340), [float fixture](../../src/grid/layout.rs#L189), [test module placement](../../src/ui/statusbar.rs#L294), [CI lint command](../../.github/workflows/ci.yml#L28). Crusty: `PRB-0014`.

**Evidence:** after processing one message, `drain_messages` calls `try_recv()` to check emptiness and discards that second message. Queueing Resize then Quit leaves `should_quit == false`. Existing tests pass despite the broken event-driving helper. All-target Clippy reports `while_let_loop` here, `approx_constant` for `3.14` in a layout fixture, and `items_after_test_module` in statusbar.

**Recommended boundary and structure:** test drivers must process each queued event exactly once. Keep database/task lifecycle tests beside their owners after extraction; use actual SQLite and controlled message ordering rather than mocks of every component.

**Refactoring and verification:** fix the drain helper first and re-run existing tests before trusting them during refactoring. Extend CI's existing warnings-denied policy to all targets after fixing these diagnostics. Migrate the relevant audit assertions into normal regression tests as fixes land. Tests of key-to-message mapping alone do not validate persistence, completion ordering or cancellation.

## Cleanup inventory

| Candidate | Evidence / disposition |
|---|---|
| Direct `thiserror`, `tracing`, `tracing-subscriber`, `lru` dependencies | No application uses found; all-target check passes after removing them in a temporary copy. `lru` still exists transitively through Ratatui. No binary-size saving was measured. |
| `csv` as a production dependency | Currently used only by export tests. Prefer using its writer to fix A10; otherwise move it to dev-dependencies. Both are concrete choices, not simultaneous recommendations. |
| rusqlite `chrono`/`blob`, chrono `serde`, Tokio `fs`/`signal` features | No corresponding application usage; all-target check passes with these activations removed in the same temporary experiment. This is compiler evidence, not cross-platform validation. |
| `src/db/query.rs` | One-line “Phase 2” stub. Replace it with the canonical query owner in A07, then remove the historical stub comment. Do not keep a parallel unused module. |
| `fetch_row_by_rowid` / delete restore scaffolding | No caller for the former; the restore branch cannot obtain a nonempty production delete payload. Complete the documented undo capability atomically or remove the unreachable implementation and inaccurate promise. |
| `Message::DistinctCountReady` | Unproduced message with a no-op handler, covered by blanket dead-code suppression; removable after a final live caller sweep. |
| SQL order/WHERE construction and value decoding | Repeated between db, db/write, app's FK loader, filters and exporters. Consolidate by semantic owner, not into a generic utilities module. |
| `#[allow(dead_code)]` on whole types/enums | Conceals obsolete message variants and fields. Narrow/remove after caller and compiler verification; do not delete all schema metadata just because the current renderer ignores it. |
| `sqv` → `sqview` config migration | **Keep:** README explicitly promises copying the legacy configuration. The migration is a real external contract. Consolidate it at app_dirs/config; do not delete merely because it says legacy. |
| Duplicate `bitflags`, `hashbrown`, `unicode-width` versions | Traceable to separate upstream semver requirements. Do not force version overrides to flatten `cargo tree -d`. |

The temporary dependency experiment removed the four unused direct crates, moved CSV to dev-dependencies and disabled the feature activations above. `cargo check --offline --all-targets` exited 0. The real manifest and lockfile were unchanged.

Crusty's static cleanup candidates included actually called helpers such as `clamp_grid_viewport` and `ensure_current_config_file`, plus test modules. These are false positives for deletion. Live source and compiler results take precedence.

## Intended ownership and API

Use one crate and incremental module extraction. The following names describe proposed responsibilities, not files already created:

```text
src/
├─ main.rs                 CLI, terminal guard and event-loop bootstrap
├─ app/
│  ├─ mod.rs               App facade and composition
│  ├─ session.rs           active table, tabs, query and schema generations
│  ├─ input.rs             key/mouse translation to application commands
│  ├─ read_jobs.rs         read admission, cancellation, request IDs, results
│  ├─ editing.rs           editor/submission state and write policy
│  ├─ history.rs           undo entries and completion-driven history
│  └─ clipboard.rs         clipboard output and reporting
├─ db/
│  ├─ mod.rs               narrow facade; connection/pool setup
│  ├─ schema.rs            introspection and table/column/key capabilities
│  ├─ query.rs             ViewQuery, ordering, predicates, identifier quoting
│  ├─ rows.rs              RowKey, RowRecord, window/selection/FK reads
│  ├─ value.rs             lossless conversion of SQLite values
│  ├─ write.rs             atomic mutations, conflicts and WriteOutcome
│  └─ observe.rs           external invalidation signals
├─ filter/
│  ├─ mod.rs               narrow facade
│  ├─ rule.rs              validated rules
│  ├─ predicate.rs         query compilation at the SQLite boundary
│  └─ store.rs             persistence identity, migration, atomic writes
├─ export/mod.rs           stream + explicit format writers
├─ grid/                   view/navigation state and rendering
└─ ui/                     presentation and editor interactions
```

The first extraction should establish the query/row/value APIs. Split App only as each owner becomes real; do not move the entire giant match into another giant file.

| Owner | Expose within the crate | Keep private / prevent bypass |
|---|---|---|
| db schema/rows | `TableCapabilities`, validated `RowKey`, `RowRecord`, row/selection queries | SQL identifier choice, raw row decoding, identity construction |
| db query | `ViewQuery`, `SortOrder`, validated selection inputs | String concatenation, placeholder numbering, ordering duplication |
| db write | mutations returning `WriteOutcome` or contextual conflict/failure | transaction lifecycle and backup capture; no raw offset writes |
| app jobs/editing/history | start/complete/invalidate operations | request generation, handles, loading flags, history mutation |
| filter | validated rules and fallible load/save | filenames, serialized compatibility DTOs and permissive fallbacks |
| grid/ui | navigation commands and render projections | ownership of database requests, SQL or undo transactions |

Target direction: `main → app → db/filter/export`; `ui/grid → presentation state + shared row/value/query models`; `export → db query/value`. Remove export's dependency on grid-owned sort types and move direct rusqlite orchestration out of input handlers. Module roots should re-export deliberate APIs with `pub(crate)` or narrower visibility; private submodules should remain filesystem details.

`anyhow` is reasonable in this binary for reportable failures. Crusty's five “opaque public library errors” findings do not establish a published library contract here. Introduce matchable errors where the application must distinguish conflict, invalid edit, missing row, cancellation or I/O failure; avoid replacing every `anyhow::Result` mechanically. Preserve source errors instead of repeatedly formatting them into strings in the write layer.

Do not introduce a trait for the single SQLite implementation, a crate per module, a generic job framework or a global `utils` owner. Shared concepts worth extracting are row identity, query identity, mutation outcomes, lossless value conversion and persisted filter identity.

## Incremental implementation sequence

Each stage should compile and retain the original user changes. Port relevant audit tests into the normal suite rather than making the historical harness an architectural dependency.

| Stage | Concrete change | Acceptance evidence |
|---|---|---|
| 1 | Repair event test driver; add reproductions for A01/A02/A04/A06 | Original tests still pass; new regressions fail for the documented reasons before fixes |
| 2 | Establish identity/capabilities, safe identifier quoting and typed records; enforce mutation counts | Rowid-shadow, WITHOUT ROWID policy, quoted-name and cached-cell tests; `cargo test db::` plus focused app tests |
| 3 | Centralize fallible edit conversion, write admission, read-only policy and completion-owned undo | Invalid numeric/BLOB, duplicate insert, read-only undo, failure/restore/conflict tests |
| 4 | Add read request generations and explicit success/failure/cancellation outcomes | Old WindowReady, delayed popup completion, invalid regex, pool failure and shutdown tests |
| 5 | Repair filter storage/validation and schema observation | Path containment/collision, stored-format migration, FK, ALTER TABLE and WAL integration tests |
| 6 | Stream export with standard serializers; batch selected-row work and invalidate counts deliberately | CSV/JSON/SQL round trips, failed/concurrent output tests, release benchmarks with fixed datasets |
| 7 | Extract the now-established App owners; remove duplicate SQL, dead paths and unused Cargo declarations | Full check/test/fmt/Clippy; live search for removed names and old callers; inspect final dependency tree |

Final standard gates:

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
```

There are no current project features, so default/no-default/all-features do not describe distinct application configurations. Add a real matrix only when features or supported targets are introduced. Benchmark commands and datasets must be recorded when performance work begins; this audit does not supply invented measurements.

## Crusty adoption and interpretation

Crusty was consulted before the substantive audit. Its first response had no guidance because the index had never been published. An explicit workspace refresh created generation 1 with 872 indexed nodes and 2,274 edges. A second consultation returned repository governance including CI, contributing guidance and `DECISIONS.md`.

The durable architecture audit is `arch_6b4363727257`. The live architecture map and cleanup candidates were cross-checked against source. The automatic architecture report found five medium error-boundary candidates; it did not identify the reproduced data-integrity bugs above. Its advisory findings are evidence leads, not the final review.

Findings A01–A13 were recorded as agent audit problems `PRB-0002` through `PRB-0014` (individual mapping appears above). `PRB-0001` is an obsolete duplicate superseded by A01's `PRB-0003`. Crusty also creates proposed quality records for recorded problems. No proposed constraint was activated and no finding was promoted into authorized implementation work.

Document preparation used context `ctx_900098e57135`. Crusty validation completed with no new or worsened architectural findings; its five existing advisory findings remained. Commands were run explicitly as listed in the baseline rather than re-run by the validator. Its tracked-file list includes the pre-existing user edits and does not enumerate the new untracked audit artifacts; those artifacts were checked separately for syntax, links and consistency.

The database state lives in `.rust-repo-intelligence/`, which was already untracked at the start. Source changes remain governed by the user's instruction to consult Crusty, then prepare before edits and validate after edits. An index marked stale because of a dirty worktree must not override live source/compiler evidence.
