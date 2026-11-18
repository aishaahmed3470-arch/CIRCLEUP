//! Registry storage-growth measurements and high-water-mark boundary tests.
//!
//! Backs the numbers in `docs/scalability/FACTORY_REGISTRY_GROWTH.md`: the
//! serialized registry size is measured here at increasing scales and checked
//! against the model in `src/scalability.rs`.

mod common;

use circle_factory::{
    registry,
    scalability::{
        estimated_hard_capacity, estimated_registry_bytes, BYTES_PER_REGISTRY_ENTRY,
        EMPTY_REGISTRY_BYTES, INSTANCE_OVERHEAD_BYTES, PROTOCOL_MAX_ENTRY_BYTES,
        PROTOCOL_MAX_TX_READ_BYTES, PROTOCOL_MAX_TX_WRITE_BYTES, REGISTRY_HIGH_WATER_MARK,
        REGISTRY_WARNING_THRESHOLD,
    },
    MAX_PAGE_SIZE,
};
use common::{setup, Registry};
use soroban_sdk::xdr::ToXdr;

/// Serialized XDR size of the registry as stored.
fn registry_bytes(r: &Registry) -> u32 {
    r.env.as_contract(&r.id, || registry::load(&r.env).to_xdr(&r.env).len())
}

#[test]
fn measured_entry_cost_matches_model() {
    let r = setup();
    assert_eq!(registry_bytes(&r), EMPTY_REGISTRY_BYTES);
    r.bulk_seed(100);
    assert_eq!(
        registry_bytes(&r),
        EMPTY_REGISTRY_BYTES + 100 * BYTES_PER_REGISTRY_ENTRY,
        "each contract address costs exactly BYTES_PER_REGISTRY_ENTRY"
    );
}

#[test]
fn measured_size_matches_model_at_increasing_scales() {
    for n in [0u32, 1, 100, 250, 500, REGISTRY_WARNING_THRESHOLD, REGISTRY_HIGH_WATER_MARK] {
        let r = setup();
        r.bulk_seed(n);
        assert_eq!(registry_bytes(&r), estimated_registry_bytes(n), "registry of {n} circles");
    }
}

#[test]
fn high_water_mark_fits_protocol_limits_with_headroom() {
    let r = setup();
    r.bulk_seed(REGISTRY_HIGH_WATER_MARK);
    let instance_estimate = registry_bytes(&r) + INSTANCE_OVERHEAD_BYTES;

    assert!(instance_estimate < PROTOCOL_MAX_ENTRY_BYTES);
    assert!(instance_estimate < PROTOCOL_MAX_TX_READ_BYTES);
    assert!(instance_estimate < PROTOCOL_MAX_TX_WRITE_BYTES);
    // At least a third of the entry limit stays free at the high-water mark.
    assert!(
        instance_estimate * 3 <= PROTOCOL_MAX_ENTRY_BYTES * 2,
        "high-water mark uses {instance_estimate} of {PROTOCOL_MAX_ENTRY_BYTES} bytes"
    );
}

#[test]
fn thresholds_are_ordered_below_hard_capacity() {
    assert!(REGISTRY_WARNING_THRESHOLD < REGISTRY_HIGH_WATER_MARK);
    assert!(REGISTRY_HIGH_WATER_MARK < estimated_hard_capacity());
    assert!(
        estimated_registry_bytes(estimated_hard_capacity()) + INSTANCE_OVERHEAD_BYTES
            <= PROTOCOL_MAX_ENTRY_BYTES
    );
    assert!(
        estimated_registry_bytes(estimated_hard_capacity() + 1) + INSTANCE_OVERHEAD_BYTES
            > PROTOCOL_MAX_ENTRY_BYTES
    );
}

#[test]
fn registry_integrity_at_high_water_mark() {
    let r = setup();
    let mut expected = r.bulk_seed(REGISTRY_HIGH_WATER_MARK - 1);
    // The final circle goes through the production append path.
    expected.push_back(r.append());

    assert_eq!(r.client.get_circle_count(), REGISTRY_HIGH_WATER_MARK);
    assert_eq!(r.client.get_circles(), expected);

    let (walked, pages) = r.walk(MAX_PAGE_SIZE);
    assert_eq!(walked, expected, "paged walk returns every circle once, in creation order");
    assert_eq!(pages, (REGISTRY_HIGH_WATER_MARK + MAX_PAGE_SIZE - 1) / MAX_PAGE_SIZE);

    let last = r
        .client
        .get_circles_paginated(&(REGISTRY_HIGH_WATER_MARK - 1), &MAX_PAGE_SIZE);
    assert_eq!(last.circles.len(), 1);
    assert_eq!(last.circles.get(0), expected.get(REGISTRY_HIGH_WATER_MARK - 1));
    assert_eq!(last.next_cursor, None);
    assert_eq!(last.total, REGISTRY_HIGH_WATER_MARK);
}

#[test]
fn appends_past_high_water_mark_keep_cursor_semantics() {
    // The high-water mark is an operational threshold, not an on-chain cap:
    // crossing it must not change ordering or cursor behaviour.
    let r = setup();
    let seeded = r.bulk_seed(REGISTRY_HIGH_WATER_MARK);
    let before = r.client.get_circles_paginated(&0, &MAX_PAGE_SIZE);

    let added = r.append_n(3);

    assert_eq!(r.client.get_circle_count(), REGISTRY_HIGH_WATER_MARK + 3);
    assert_eq!(r.client.get_circles_paginated(&0, &MAX_PAGE_SIZE), {
        let mut p = before.clone();
        p.total = REGISTRY_HIGH_WATER_MARK + 3;
        p
    });
    let tail = r.client.get_circles_paginated(&REGISTRY_HIGH_WATER_MARK, &MAX_PAGE_SIZE);
    assert_eq!(tail.circles, added);
    assert_eq!(tail.next_cursor, None);
    assert_eq!(
        r.client.get_circles().slice(0..REGISTRY_HIGH_WATER_MARK),
        seeded,
        "existing entries are untouched"
    );
}
