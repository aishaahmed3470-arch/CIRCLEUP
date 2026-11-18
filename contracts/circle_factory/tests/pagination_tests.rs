//! `get_circles_paginated` page boundaries, plus the registry ordering and
//! cursor guarantees specified in `src/registry.rs`.

mod common;

use circle_factory::{
    registry, CircleFactory, CircleFactoryClient, CirclePage, DataKey, FactoryError, MAX_PAGE_SIZE,
};
use common::setup;
use soroban_sdk::{testutils::Address as _, Address, Env, Vec};

// ── Page boundaries ───────────────────────────────────────────────────────────

#[test]
fn empty_registry_returns_empty_final_page() {
    let r = setup();
    let page = r.client.get_circles_paginated(&0, &10);
    assert_eq!(
        page,
        CirclePage { circles: Vec::new(&r.env), next_cursor: None, total: 0 }
    );
}

#[test]
fn uninitialized_factory_returns_empty_final_page() {
    let env = Env::default();
    let id = env.register_contract(None, CircleFactory);
    let client = CircleFactoryClient::new(&env, &id);
    let page = client.get_circles_paginated(&0, &10);
    assert!(page.circles.is_empty());
    assert_eq!(page.next_cursor, None);
    assert_eq!(page.total, 0);
}

#[test]
fn first_page() {
    let r = setup();
    let all = r.append_n(7);
    let page = r.client.get_circles_paginated(&0, &3);
    assert_eq!(page.circles, all.slice(0..3));
    assert_eq!(page.next_cursor, Some(3));
    assert_eq!(page.total, 7);
}

#[test]
fn middle_page() {
    let r = setup();
    let all = r.append_n(7);
    let page = r.client.get_circles_paginated(&3, &3);
    assert_eq!(page.circles, all.slice(3..6));
    assert_eq!(page.next_cursor, Some(6));
    assert_eq!(page.total, 7);
}

#[test]
fn final_partial_page() {
    let r = setup();
    let all = r.append_n(7);
    let page = r.client.get_circles_paginated(&6, &3);
    assert_eq!(page.circles, all.slice(6..7));
    assert_eq!(page.next_cursor, None);
    assert_eq!(page.total, 7);
}

#[test]
fn final_page_that_exactly_fills_limit_has_no_next_cursor() {
    let r = setup();
    let all = r.append_n(6);
    let page = r.client.get_circles_paginated(&3, &3);
    assert_eq!(page.circles, all.slice(3..6));
    assert_eq!(page.next_cursor, None, "no empty trailing page is required");
}

#[test]
fn cursor_equal_to_total_returns_empty_final_page() {
    let r = setup();
    r.append_n(4);
    let page = r.client.get_circles_paginated(&4, &10);
    assert!(page.circles.is_empty());
    assert_eq!(page.next_cursor, None);
    assert_eq!(page.total, 4);
}

#[test]
fn cursor_beyond_total_is_rejected() {
    let r = setup();
    r.append_n(4);
    assert_eq!(
        r.client.try_get_circles_paginated(&5, &10),
        Err(Ok(FactoryError::CursorOutOfRange))
    );
    assert_eq!(
        r.client.try_get_circles_paginated(&u32::MAX, &10),
        Err(Ok(FactoryError::CursorOutOfRange))
    );
}

#[test]
fn cursor_beyond_empty_registry_is_rejected() {
    let r = setup();
    assert_eq!(
        r.client.try_get_circles_paginated(&1, &1),
        Err(Ok(FactoryError::CursorOutOfRange))
    );
}

#[test]
fn zero_limit_is_rejected() {
    let r = setup();
    r.append_n(2);
    assert_eq!(
        r.client.try_get_circles_paginated(&0, &0),
        Err(Ok(FactoryError::InvalidPageLimit))
    );
}

#[test]
fn oversized_limit_is_clamped_to_max_page_size() {
    let r = setup();
    let all = r.bulk_seed(MAX_PAGE_SIZE + 10);
    let page = r.client.get_circles_paginated(&0, &u32::MAX);
    assert_eq!(page.circles.len(), MAX_PAGE_SIZE);
    assert_eq!(page.circles, all.slice(0..MAX_PAGE_SIZE));
    assert_eq!(page.next_cursor, Some(MAX_PAGE_SIZE));
}

#[test]
fn huge_limit_near_end_does_not_overflow() {
    let r = setup();
    let all = r.append_n(3);
    let page = r.client.get_circles_paginated(&2, &u32::MAX);
    assert_eq!(page.circles, all.slice(2..3));
    assert_eq!(page.next_cursor, None);
}

#[test]
fn rejected_requests_do_not_change_registry() {
    let r = setup();
    let all = r.append_n(3);
    let _ = r.client.try_get_circles_paginated(&9, &1);
    let _ = r.client.try_get_circles_paginated(&0, &0);
    assert_eq!(r.client.get_circles(), all);
    assert_eq!(r.client.get_circle_count(), 3);
}

// ── Post-append behaviour ─────────────────────────────────────────────────────

#[test]
fn final_page_can_be_resumed_from_total_after_append() {
    let r = setup();
    let first = r.append_n(5);
    let last = r.client.get_circles_paginated(&3, &10);
    assert_eq!(last.next_cursor, None);
    assert_eq!(last.total, 5);

    let new = r.append_n(2);

    let resumed = r.client.get_circles_paginated(&last.total, &10);
    assert_eq!(resumed.circles, new, "resume returns exactly the new circles");
    assert_eq!(resumed.next_cursor, None);
    assert_eq!(resumed.total, 7);

    // Pages read before the append are unchanged.
    assert_eq!(r.client.get_circles_paginated(&0, &3).circles, first.slice(0..3));
}

#[test]
fn page_that_was_final_gains_next_cursor_after_append() {
    let r = setup();
    r.append_n(4);
    assert_eq!(r.client.get_circles_paginated(&2, &2).next_cursor, None);
    r.append();
    let page = r.client.get_circles_paginated(&2, &2);
    assert_eq!(page.next_cursor, Some(4));
    assert_eq!(page.total, 5);
}

#[test]
fn walk_covers_registry_for_every_page_size() {
    let r = setup();
    let all = r.append_n(11);
    for limit in 1..=12u32 {
        let (walked, pages) = r.walk(limit);
        assert_eq!(walked, all, "limit {limit}: walk must return every circle once, in order");
        assert_eq!(pages, (11 + limit - 1) / limit, "limit {limit}: page count");
    }
}

// ── Backwards compatibility of get_circles ────────────────────────────────────

#[test]
fn get_circles_is_unchanged_and_matches_paged_walk() {
    let r = setup();
    let all = r.append_n(9);
    assert_eq!(r.client.get_circles(), all);
    assert_eq!(r.client.get_circle_count(), 9);
    assert_eq!(r.walk(4).0, r.client.get_circles());
}

// ── Registry ordering & cursor specification ─────────────────────────────────

#[test]
fn registry_order_is_creation_order_not_address_order() {
    let r = setup();
    let created = r.append_n(8);
    assert_eq!(r.client.get_circles(), created);
    for i in 0..created.len() {
        let page = r.client.get_circles_paginated(&i, &1);
        assert_eq!(page.circles.get(0), created.get(i), "index {i} is the {i}th create");
    }
}

#[test]
fn push_returns_the_index_the_circle_is_stored_at() {
    let r = setup();
    r.append_n(3);
    let circle = Address::generate(&r.env);
    let index = r.env.as_contract(&r.id, || {
        let mut circles = registry::load(&r.env);
        registry::push(&r.env, &mut circles, 3, circle.clone())
    });
    assert_eq!(index, 3);
    assert_eq!(r.client.get_circles_paginated(&3, &1).circles.get(0), Some(circle));
    assert_eq!(r.client.get_circle_count(), 4);
}

#[test]
#[should_panic(expected = "registry invariant violated")]
fn push_rejects_counter_that_disagrees_with_list() {
    let r = setup();
    r.append_n(2);
    r.env.as_contract(&r.id, || {
        let mut circles = registry::load(&r.env);
        registry::push(&r.env, &mut circles, 5, Address::generate(&r.env));
    });
}

#[test]
fn repeated_reads_are_stable() {
    let r = setup();
    r.append_n(10);
    let a = r.client.get_circles_paginated(&4, &3);
    let b = r.client.get_circles_paginated(&4, &3);
    let c = r.client.get_circles_paginated(&4, &3);
    assert_eq!(a, b);
    assert_eq!(b, c);
}

#[test]
fn existing_prefix_is_immutable_across_appends() {
    let r = setup();
    let mut seen = Vec::new(&r.env);
    for _ in 0..6 {
        let before = r.client.get_circles();
        assert_eq!(before, seen, "prefix read earlier must not change");
        let added = r.append();
        let after = r.client.get_circles();
        assert_eq!(after.slice(0..before.len()), before);
        assert_eq!(after.get(before.len()), Some(added.clone()));
        seen.push_back(added);
    }
}

#[test]
fn concurrent_clients_agree_while_registry_grows() {
    let r = setup();
    r.append_n(3);
    // Two independent clients for the same factory, interleaving page reads
    // with new creates.
    let a = CircleFactoryClient::new(&r.env, &r.id);
    let b = CircleFactoryClient::new(&r.env, &r.id);

    let mut seen_a = Vec::new(&r.env);
    let mut cursor_a = 0u32;
    let mut seen_b = Vec::new(&r.env);
    let mut cursor_b = 0u32;

    for _ in 0..6 {
        let pa = a.get_circles_paginated(&cursor_a, &2);
        seen_a.append(&pa.circles);
        cursor_a = pa.next_cursor.unwrap_or(pa.total);

        r.append();

        let pb = b.get_circles_paginated(&cursor_b, &3);
        seen_b.append(&pb.circles);
        cursor_b = pb.next_cursor.unwrap_or(pb.total);
    }
    // Drain both to the current end.
    for (client, seen, cursor) in [(&a, &mut seen_a, &mut cursor_a), (&b, &mut seen_b, &mut cursor_b)] {
        loop {
            let p = client.get_circles_paginated(cursor, &5);
            seen.append(&p.circles);
            *cursor = p.next_cursor.unwrap_or(p.total);
            if p.next_cursor.is_none() {
                break;
            }
        }
    }

    let registry = r.client.get_circles();
    assert_eq!(seen_a, registry, "client A saw every circle exactly once, in order");
    assert_eq!(seen_b, registry, "client B saw every circle exactly once, in order");
}

#[test]
fn counter_always_equals_list_length() {
    let r = setup();
    for n in 1..=5u32 {
        r.append();
        let count: u32 = r.env.as_contract(&r.id, || {
            r.env.storage().instance().get(&DataKey::CircleCount).unwrap()
        });
        assert_eq!(count, n);
        assert_eq!(r.client.get_circles().len(), n);
        assert_eq!(r.client.get_circles_paginated(&0, &1).total, n);
    }
}
