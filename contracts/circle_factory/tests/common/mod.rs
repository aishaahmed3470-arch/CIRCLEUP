//! Shared fixture for the factory registry integration tests.
//!
//! The factory unit tests run with a dummy circle WASM hash, so a real
//! `create_circle` cannot deploy.  These helpers drive the registry through
//! `registry::push` — the exact commit step `create_circle` runs after a
//! successful deploy — so the paging and ordering tests exercise production
//! code rather than a re-implementation.

#![allow(dead_code)]

use circle_factory::{registry, CircleFactory, CircleFactoryClient, DataKey};
use soroban_sdk::{testutils::Address as _, Address, BytesN, Env, Vec};

pub struct Registry<'a> {
    pub env: Env,
    pub id: Address,
    pub client: CircleFactoryClient<'a>,
}

/// An initialized factory with an empty registry.
pub fn setup() -> Registry<'static> {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register_contract(None, CircleFactory);
    let client = CircleFactoryClient::new(&env, &id);
    let wasm_hash: BytesN<32> = BytesN::from_array(&env, &[0u8; 32]);
    client.initialize(
        &Address::generate(&env),
        &wasm_hash,
        &Address::generate(&env),
        &Address::generate(&env),
    );
    Registry { env, id, client }
}

impl<'a> Registry<'a> {
    /// Append one fresh circle address through `registry::push`.
    pub fn append(&self) -> Address {
        let circle = Address::generate(&self.env);
        self.env.as_contract(&self.id, || {
            let mut circles = registry::load(&self.env);
            let count: u32 = self
                .env
                .storage()
                .instance()
                .get(&DataKey::CircleCount)
                .unwrap();
            registry::push(&self.env, &mut circles, count, circle.clone());
        });
        circle
    }

    /// Append `n` fresh circles one at a time; returns them in order.
    pub fn append_n(&self, n: u32) -> Vec<Address> {
        let mut out = Vec::new(&self.env);
        for _ in 0..n {
            out.push_back(self.append());
        }
        out
    }

    /// Write `n` fresh circles in a single storage write.
    ///
    /// Produces the same end state as `n` appends (list + matching counter)
    /// without the quadratic rewrite cost, for the high-water-mark tests.
    pub fn bulk_seed(&self, n: u32) -> Vec<Address> {
        let mut circles = Vec::new(&self.env);
        for _ in 0..n {
            circles.push_back(Address::generate(&self.env));
        }
        self.env.as_contract(&self.id, || {
            self.env.storage().instance().set(&DataKey::Circles, &circles);
            self.env.storage().instance().set(&DataKey::CircleCount, &n);
        });
        circles
    }

    /// Walk every page with `limit` and return the concatenated addresses.
    pub fn walk(&self, limit: u32) -> (Vec<Address>, u32) {
        let mut all = Vec::new(&self.env);
        let mut cursor = 0u32;
        let mut pages = 0u32;
        loop {
            let page = self.client.get_circles_paginated(&cursor, &limit);
            pages += 1;
            all.append(&page.circles);
            match page.next_cursor {
                Some(next) => {
                    assert!(next > cursor, "next_cursor must advance");
                    cursor = next;
                }
                None => break,
            }
        }
        (all, pages)
    }
}
