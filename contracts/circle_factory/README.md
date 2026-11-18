# circle_factory

Deploys circle contract instances and keeps the registry of every circle it
has created. Method reference: [`docs/CONTRACT_REFERENCE.md`](../../docs/CONTRACT_REFERENCE.md#circlefactory-contract).

## Registry specification

### Ordering

The registry is ordered strictly by **creation order** — the order in which
`create_circle` calls committed. It is not sorted by address. Entry `i` is the
circle whose `factory/circle_created` event carried `circle_index == i`, so an
indexer can rebuild the same order from events alone.

### Append-only immutability

`create_circle` is the only write path. After deploy, `initialize` and
reputation registration all succeed, it calls `registry::push`, which appends
one address at index `CircleCount` and increments `CircleCount` in the same
invocation. A failed create writes nothing. No entry-point removes, reorders or
replaces an entry, so for any two ledgers `L1 <= L2`:

```text
for every i < count(L1):  circles(L1)[i] == circles(L2)[i]
count(L1) <= count(L2)
CircleCount == circles.len()   (always)
```

### Cursor model

The registry is stored as an index-addressed vector, so the cursor is the
zero-based **index** of the first entry to return. An index cursor cannot go
stale: the entry it names never moves.

| Request | Result |
|---|---|
| `cursor < total` | Up to `min(limit, 50)` entries; `next_cursor = Some(end)` if more remain, else `None` |
| `cursor == total` | Empty page, `next_cursor = None` |
| `cursor > total` | `FactoryError::CursorOutOfRange` (111) — no earlier page can produce this |
| `limit == 0` | `FactoryError::InvalidPageLimit` (110) |
| `limit > 50` | Clamped to `MAX_PAGE_SIZE` (50) |

**Detecting remaining pages:** keep reading while `next_cursor` is `Some`.
`None` means the end of the registry *as of that read*. To pick up circles
created later, store `total` from the final page and resume from it — the
response contains exactly the new circles, and nothing read earlier needs to
be fetched again.

**Multi-client consistency:** two clients paging the same factory, at any
page sizes and with creates landing between their reads, each see every circle
exactly once and in the same order. `tests/pagination_tests.rs` checks this
(`concurrent_clients_agree_while_registry_grows`).

### Compatibility

`get_circles` is unchanged and still returns the full list. Its response grows
with the registry, so new consumers should use `get_circles_paginated`.
Bounding or removing `get_circles` is a breaking change and will follow
[`docs/DEPRECATION_POLICY.md`](../../docs/DEPRECATION_POLICY.md).

Any future storage migration must keep the global creation index as the
cursor, so `get_circles_paginated` callers are unaffected. See
[`docs/scalability/FACTORY_REGISTRY_GROWTH.md`](../../docs/scalability/FACTORY_REGISTRY_GROWTH.md).

## Tests

| File | Covers |
|---|---|
| `src/lib.rs` (unit) | Input validation, salts, init guards, factory ↔ reputation trust boundary |
| `tests/pagination_tests.rs` | First / middle / final / empty / out-of-range pages, clamping, post-append resume, ordering and cursor stability |
| `tests/scalability_tests.rs` | Measured registry size vs. the growth model; integrity at the high-water mark |
