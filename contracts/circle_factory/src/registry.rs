//! Append-only circle registry: ordering invariants, cursor model, paging.
//!
//! # Ordering
//!
//! The registry is ordered strictly by **creation order** — the order in which
//! `create_circle` calls committed — not by address.  Entry `i` is the circle
//! whose `factory/circle_created` event carried `circle_index == i`.
//!
//! # Append-only immutability
//!
//! The only write path is [`push`], called once at the end of a fully
//! successful `create_circle`.  It appends one address at index
//! `CircleCount` and bumps `CircleCount` in the same invocation.  Nothing
//! removes, reorders, or overwrites an existing entry, so for every ledger
//! `L1 <= L2` and every `i < count(L1)`:
//!
//! ```text
//! circles(L1)[i] == circles(L2)[i]
//! ```
//!
//! # Cursor model
//!
//! Because the storage representation is an index-addressed, append-only
//! vector, the cursor is the zero-based **index** of the first entry to
//! return.  An index cursor never goes stale: the entry it points at can
//! never move, so two clients paging at different times always agree on the
//! prefix they have both seen.
//!
//! * Start a walk with `cursor = 0`.
//! * Continue with `cursor = page.next_cursor` while it is `Some`.
//! * `next_cursor == None` means "no more entries **as of this read**".  The
//!   registry may grow later; to pick up new circles, resume from
//!   `cursor = page.total` (equal to `cursor + page.circles.len()` on the
//!   final page).  Previously returned entries never need re-fetching.
//! * `cursor == total` is a valid request and returns an empty final page.
//! * `cursor > total` can never come from a previous page, so it is rejected
//!   with [`FactoryError::CursorOutOfRange`] instead of silently returning an
//!   empty page that would hide a client bug.
//! * `limit == 0` is rejected with [`FactoryError::InvalidPageLimit`]; it
//!   would return a page whose `next_cursor` equals the request cursor and
//!   loop forever.  `limit > MAX_PAGE_SIZE` is clamped to `MAX_PAGE_SIZE`.
//!
//! Any future storage migration (see `scalability.rs`) must keep the global
//! creation index as the cursor so existing clients keep working.

use soroban_sdk::{contracttype, Address, Env, Vec};

use crate::{DataKey, FactoryError};

/// Largest number of addresses a single page returns.
///
/// Project-selected, not a protocol limit: 50 contract addresses serialize to
/// ~2 KB, well under the Soroban return-value/event size limit, and keep the
/// per-call decode cost flat for clients.
pub const MAX_PAGE_SIZE: u32 = 50;

/// One bounded slice of the registry, returned by `get_circles_paginated`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct CirclePage {
    /// Circle addresses at indices `[cursor, cursor + circles.len())`, in
    /// creation order.
    pub circles: Vec<Address>,
    /// Cursor for the next page, or `None` when this page reaches the end of
    /// the registry as of this read.
    pub next_cursor: Option<u32>,
    /// Registry length (`CircleCount`) at the time of this read.
    pub total: u32,
}

/// Load the full registry vector, or an empty one before `initialize`.
pub fn load(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&DataKey::Circles)
        .unwrap_or(Vec::new(env))
}

/// Append `circle` at index `count` and persist both registry keys together.
///
/// `circles` and `count` must be the values read at the start of the current
/// invocation.  Returns the index the circle was stored at.  Panics if the
/// stored count and list length disagree afterwards, which would break the
/// cursor model above.
pub fn push(env: &Env, circles: &mut Vec<Address>, count: u32, circle: Address) -> u32 {
    if circles.len() != count {
        panic!("factory: registry invariant violated: count != circles.len()");
    }
    let next = count
        .checked_add(1)
        .unwrap_or_else(|| panic!("factory: CircleCount overflow"));
    circles.push_back(circle);
    env.storage().instance().set(&DataKey::Circles, &*circles);
    env.storage().instance().set(&DataKey::CircleCount, &next);
    count
}

/// Read one page starting at `cursor`.  See the module docs for semantics.
pub fn page(env: &Env, cursor: u32, limit: u32) -> Result<CirclePage, FactoryError> {
    if limit == 0 {
        return Err(FactoryError::InvalidPageLimit);
    }
    let circles = load(env);
    let total = circles.len();
    if cursor > total {
        return Err(FactoryError::CursorOutOfRange);
    }
    let size = if limit > MAX_PAGE_SIZE { MAX_PAGE_SIZE } else { limit };
    let end = core::cmp::min(cursor.saturating_add(size), total);
    Ok(CirclePage {
        circles: circles.slice(cursor..end),
        next_cursor: if end < total { Some(end) } else { None },
        total,
    })
}
