//! Registry storage-growth model.
//!
//! Design note: `docs/scalability/FACTORY_REGISTRY_GROWTH.md`.
//!
//! The registry (`DataKey::Circles`) lives in **instance** storage, and all
//! instance keys share a single ledger entry (the contract instance).  So:
//!
//! * every factory invocation reads the whole registry, whichever getter it
//!   calls (pagination bounds the *return value*, not the read);
//! * every `create_circle` rewrites the whole registry;
//! * the registry can never exceed the network's maximum contract-data entry
//!   size, and once the instance entry hits that size `create_circle` fails
//!   for everyone.
//!
//! The constants below separate network-configurable protocol limits from the
//! thresholds this project chooses to operate within.  Tests in
//! `tests/scalability_tests.rs` measure the real XDR size against them.

// ── Soroban protocol limits (network settings, not chosen by this project) ──
//
// These are the most conservative values the Stellar network has used for
// each setting.  They are network-configurable and have been raised over
// time; check the live values with `stellar network settings` (or the
// `ConfigSetting` ledger entries) before relying on more headroom.

/// `contract_data_entry_size_bytes`: maximum size of one contract-data ledger
/// entry, including the contract instance that holds all instance storage.
pub const PROTOCOL_MAX_ENTRY_BYTES: u32 = 65_536;

/// `tx_max_read_bytes`: maximum ledger bytes one transaction may read.
pub const PROTOCOL_MAX_TX_READ_BYTES: u32 = 200_000;

/// `tx_max_write_bytes`: maximum ledger bytes one transaction may write.
pub const PROTOCOL_MAX_TX_WRITE_BYTES: u32 = 132_096;

// ── Measured encoding cost ──────────────────────────────────────────────────

/// XDR bytes added to the registry per circle: a contract `ScVal::Address`
/// is a 4-byte `ScVal` tag, a 4-byte `ScAddress` tag, and a 32-byte hash.
pub const BYTES_PER_REGISTRY_ENTRY: u32 = 40;

/// XDR bytes of an empty registry vector: `ScVal` tag, `Option` flag, length.
pub const EMPTY_REGISTRY_BYTES: u32 = 12;

/// Generous allowance for every other instance key (admin, wasm hash,
/// reputation and token addresses, counter) plus the instance entry framing.
pub const INSTANCE_OVERHEAD_BYTES: u32 = 1_024;

// ── Project-selected operational thresholds ─────────────────────────────────

/// Registry size at which operators should start planning the migration in
/// the design note.  ~30 KB of registry, under half the entry limit.
pub const REGISTRY_WARNING_THRESHOLD: u32 = 750;

/// Highest registry size this layout is supported at.  ~40 KB of registry,
/// leaving ~37 % of the entry limit as headroom for the migration transaction
/// and any future instance keys.  Exercised by the boundary tests.
pub const REGISTRY_HIGH_WATER_MARK: u32 = 1_000;

/// Estimated serialized size of a registry holding `count` circles.
pub const fn estimated_registry_bytes(count: u32) -> u32 {
    EMPTY_REGISTRY_BYTES + count * BYTES_PER_REGISTRY_ENTRY
}

/// Estimated registry size at which the instance entry would reach
/// [`PROTOCOL_MAX_ENTRY_BYTES`] and `create_circle` would start failing.
pub const fn estimated_hard_capacity() -> u32 {
    (PROTOCOL_MAX_ENTRY_BYTES - INSTANCE_OVERHEAD_BYTES - EMPTY_REGISTRY_BYTES)
        / BYTES_PER_REGISTRY_ENTRY
}

// The operational thresholds must stay ordered and below the hard capacity.
const _: () = assert!(REGISTRY_WARNING_THRESHOLD < REGISTRY_HIGH_WATER_MARK);
const _: () = assert!(REGISTRY_HIGH_WATER_MARK < estimated_hard_capacity());
const _: () = assert!(
    estimated_registry_bytes(REGISTRY_HIGH_WATER_MARK) + INSTANCE_OVERHEAD_BYTES
        < PROTOCOL_MAX_TX_WRITE_BYTES
);
