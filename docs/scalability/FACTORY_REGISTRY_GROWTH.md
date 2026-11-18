# Factory registry storage growth

**Status:** accepted · **Scope:** `contracts/circle_factory` · **Model:** `src/scalability.rs` · **Evidence:** `tests/scalability_tests.rs`

## Summary

The factory keeps every circle address in one `Vec<Address>` under
`DataKey::Circles` in **instance** storage. Instance storage is a single ledger
entry, so the registry's size is bounded by the network's maximum
contract-data entry size, and every factory call pays to read all of it.

With the most conservative entry limit the network has used (64 KiB), the
registry tops out at roughly **1,600 circles**. Past that point `create_circle`
fails for every caller. We support this layout up to a **high-water mark of
1,000 circles** and start migration planning at **750**. Pagination
(`get_circles_paginated`) bounds what clients receive; it does not reduce what
the contract reads.

## Current storage layout

| Key | Storage | Type | Size | Written by |
|---|---|---|---|---|
| `Admin`, `ReputationContract`, `UsdcToken` | instance | `Address` | ~40 B each | `initialize` |
| `CircleWasmHash` | instance | `BytesN<32>` | ~40 B | `initialize` |
| `CircleCount` | instance | `u32` | ~8 B | `initialize`, `create_circle` |
| `Circles` | instance | `Vec<Address>` | 12 B + 40 B × n | `initialize`, `create_circle` |

All instance keys live inside the contract-instance ledger entry. Three
consequences follow:

1. **Every invocation reads the whole registry.** Soroban loads the instance
   entry for any call to the contract, including `get_circle_count` and
   `get_circles_paginated`.
2. **Every `create_circle` rewrites the whole registry.** `registry::push`
   appends one address and writes the vector back, so write bytes grow by
   40 B per existing circle.
3. **The entry size cap is a hard ceiling.** Once the instance entry would
   exceed the limit, the write in `create_circle` fails and no new circle can
   be created.

## Measured encoding cost

These figures are asserted by `tests/scalability_tests.rs`, which serializes
the stored registry with `to_xdr` at each size:

- Empty registry: **12 bytes** (`ScVal` tag, `Option` flag, length).
- Per circle: **40 bytes** (`ScVal::Address` tag 4 B, `ScAddress::Contract`
  tag 4 B, contract hash 32 B).

| Circles | Registry bytes | + 1 KiB instance overhead | Share of 64 KiB entry |
|---:|---:|---:|---:|
| 0 | 12 | 1,036 | 1.6 % |
| 100 | 4,012 | 5,036 | 7.7 % |
| 250 | 10,012 | 11,036 | 16.8 % |
| 500 | 20,012 | 21,036 | 32.1 % |
| **750** (warning) | 30,012 | 31,036 | 47.4 % |
| **1,000** (high-water mark) | 40,012 | 41,036 | 62.6 % |
| ~1,612 (estimated hard capacity) | 64,492 | 65,516 | ~100 % |

The 1 KiB overhead (`INSTANCE_OVERHEAD_BYTES`) is a deliberately generous
allowance for the other instance keys and entry framing; the real figure is
smaller.

**Not measured here:** CPU instructions and fees. The tests run in the Soroban
test host, which does not reproduce network fees. To measure them on testnet at
a given scale, seed a factory and run:

```bash
stellar contract invoke --id $FACTORY --network testnet --cost -- get_circles_paginated --cursor 0 --limit 50
stellar contract invoke --id $FACTORY --network testnet --cost --source $CREATOR -- create_circle ...
```

and record `read_bytes`, `write_bytes`, `instructions` and the resource fee
alongside the registry size. Read and write bytes should track the table
above.

## Protocol limits vs. project thresholds

**Protocol limits** are network settings. Validators vote on them and they
have been raised over time. `src/scalability.rs` uses the most conservative
value each setting has had. Check the live values before relying on more
headroom: `stellar network settings`, or the `ConfigSetting` ledger entries.

| Setting | Constant | Value used | Binding? |
|---|---|---:|---|
| `contract_data_entry_size_bytes` | `PROTOCOL_MAX_ENTRY_BYTES` | 65,536 | **Yes.** This is the first limit the registry reaches |
| `tx_max_read_bytes` | `PROTOCOL_MAX_TX_READ_BYTES` | 200,000 | No. The entry cap is reached first |
| `tx_max_write_bytes` | `PROTOCOL_MAX_TX_WRITE_BYTES` | 132,096 | No. The entry cap is reached first |

**Project thresholds** are choices we make, not network limits:

| Threshold | Constant | Value | Why |
|---|---|---:|---|
| Page size | `MAX_PAGE_SIZE` | 50 | Keeps each response about 2 KB, well under the return-value limit |
| Warning | `REGISTRY_WARNING_THRESHOLD` | 750 | Under half the entry limit; leaves time to ship a migration |
| High-water mark | `REGISTRY_HIGH_WATER_MARK` | 1,000 | Leaves about 37 % of the entry free for the migration transaction and any future instance keys |

Compile-time assertions in `src/scalability.rs` keep the thresholds in order
and below the estimated hard capacity. They are not enforced on-chain:
`create_circle` does not refuse to run at 1,000 circles. Rejecting creates at
an arbitrary count would be a product decision, and the hard failure is still
about 600 circles away.

## Warning signals

| Signal | Source | Action |
|---|---|---|
| `get_circle_count() >= 750` | Any RPC read, or the indexer's count of `factory/circle_created` events | Schedule the migration below |
| `get_circle_count() >= 1,000` | Same | Freeze new features that add instance keys; ship the migration |
| `create_circle` resource fee or write bytes rising faster than linearly | `--cost` / `simulateTransaction` | Check for unexpected instance growth |
| Instance entry size within 10 % of the live entry limit | `getLedgerEntries` on the contract instance key | Emergency: migrate before the next creates |

## Migration options

Whatever we choose must keep two things stable: the global creation index as
the cursor, and the `CirclePage` shape. Then `get_circles_paginated` callers
see no change.

### A. Indexed persistent entries (recommended)

Freeze `Circles` at its current length `F`. Write each new circle to its own
persistent key, `CircleAt(index)`, for `index >= F`. `CircleCount` stays the
global length. `registry::page` reads `Circles[cursor..]` while `cursor < F`
and `CircleAt(i)` keys above that.

- Each create writes one small entry instead of rewriting the registry. Write
  cost becomes O(1).
- A page past `F` reads at most `MAX_PAGE_SIZE` entries, well inside the
  per-transaction entry-count limit.
- Every call still reads the frozen instance vector, so it remains a fixed
  cost. A follow-up can move the frozen prefix into persistent chunks.
- New cost: each persistent entry needs TTL management (rent). `create_circle`
  should extend the TTL of the entry it writes.
- `get_circles` cannot return a list longer than one entry allows. At
  migration time it should be bounded or deprecated under
  `DEPRECATION_POLICY.md`.

### B. Fixed-size persistent chunks

Store `Chunk(k)` vectors of, say, 256 addresses each. Cursor `i` maps to
`(i / 256, i % 256)`. Each entry is under 11 KB, and a page touches one or two
chunks. This uses fewer ledger entries than option A, but each create
rewrites a whole chunk.

### C. Factory epochs

Deploy a new factory once the current one reaches the high-water mark, and
link the epochs with a predecessor pointer. Cursors become `(epoch, index)`,
which changes the cursor type and breaks clients. Use this only if a
contract upgrade is not available.

### D. Event-sourced discovery

Treat the indexer, which consumes `factory/circle_created` events, as the
canonical list and keep only `CircleCount` on-chain. This is the lowest
on-chain cost, but on-chain enumeration is lost. Not recommended as the only
path.

**Recommendation:** use option A, triggered at the warning threshold. It keeps
cursor semantics unchanged and turns the per-create cost from O(n) into O(1).

## Related risk

The factory never extends its instance TTL. Instance rent grows with the
registry, and if nobody extends the TTL the instance can be archived; it must
then be restored before any create. Add instance TTL extension to the
operational runbook, or to `create_circle`, as part of the migration.
