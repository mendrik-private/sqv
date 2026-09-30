# Architecture Decisions

## LIMIT/OFFSET vs Keyset Pagination

**Decision**: Use `LIMIT`/`OFFSET` for pagination.

**Rationale**: Keyset pagination requires a stable, ordered key for the "last seen" value. SQLite's `rowid` could serve this purpose, but it's not always monotonic (after deletes and re-inserts). Additionally, random access (jumping to row N or jumping to a letter in the alphabet rail) is natural with LIMIT/OFFSET but awkward with keyset. The performance of OFFSET on SQLite is acceptable for tables up to ~10M rows; beyond that, we would need a more sophisticated approach.

## r2d2 + rusqlite vs sqlx

**Decision**: Use `r2d2` connection pool with `rusqlite` instead of `sqlx`.

**Rationale**:
- `rusqlite` gives direct access to `ValueRef` for zero-copy type inspection, critical for efficient cell rendering.
- Custom SQLite functions (REGEXP) require `rusqlite`'s `create_scalar_function`, not available in `sqlx`.
- `rusqlite` supports `unchecked_transaction()` for fine-grained transaction control in write operations.
- `sqlx` would introduce async overhead for a local file database where latency is near-zero.
- `r2d2` provides a simple synchronous pool that works naturally with `tokio::task::spawn_blocking`.

## Column Sizing Algorithm

**Decision**: Size each column independently from a trimmed mean of sampled cell widths, and scroll horizontally when the columns do not fit.

**Rationale**:
- Widths come from the first rows loaded for a table and stay stable while scrolling, so columns do not jump as the window moves.
- Dropping the widest and narrowest samples keeps one long value from dominating a column; widths are capped at 40 cells.
- Every column is at least wide enough for its header name, its key or link markers, and a floor of 6 cells.
- Unicode display width is measured with `unicode-width` to handle CJK and emoji correctly.

## No Animation Except the Loading Stripe

**Decision**: Only one animation (the loading stripe in the gutter when fetching rows).

**Rationale**:
- Animations in terminal apps are distracting for data-focused work.
- The loading stripe is the minimal signal needed to indicate background activity without being intrusive.
- The 30Hz render loop (33ms tick) provides sufficient responsiveness for smooth cursor navigation without wasting CPU on decorative animations.
- Slide-in/slide-out animations for toasts and popups would add complexity (interpolation state, timer management) for minimal UX benefit in a terminal.
