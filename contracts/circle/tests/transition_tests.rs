//! Table-driven checks of `circle::transition::TRANSITION_MATRIX`.
//!
//! Every (condition, mutation) cell is applied to a freshly built fixture:
//!
//! * `Allowed(next)` — the call succeeds, the circle ends in `next`, and the
//!   change is observable in storage or balances.
//! * `Rejected(err)` — the call fails with exactly `err`, and status, round,
//!   counters, token balances, collateral, default and contribution records,
//!   reputation scores, and emitted events are all unchanged.
//!
//! The pause overlay is then swept across every other base condition, and the
//! contract's `pub fn` list is checked against the matrix so a new entry-point
//! cannot be added without classifying it.

use circle::transition::{
    expected, paused_overlay, Condition, Lifecycle, Mutation, Outcome, Phase, CONDITION_COUNT,
    MUTATION_COUNT,
};
use circle::{CircleContract, CircleContractClient, CircleStatus, DataKey, RoundState};
use reputation::{ReputationContract, ReputationContractClient};
use soroban_sdk::{
    testutils::{Address as _, Events},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, Error, InvokeError,
};

const ROUND_AMOUNT: i128 = 100_000_000;
const ROUND_DEADLINE: u32 = 1_000;
const MEMBERS: u32 = 4;

// ── Fixture ───────────────────────────────────────────────────────────────────

struct Fx<'a> {
    env: Env,
    circle: CircleContractClient<'a>,
    circle_id: Address,
    token: TokenClient<'a>,
    token_id: Address,
    rep: ReputationContractClient<'a>,
    rep_id: Address,
    admin: Address,
    members: soroban_sdk::Vec<Address>,
}

impl Fx<'static> {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let sac = env.register_stellar_asset_contract_v2(Address::generate(&env));
        let token_id = sac.address();
        let token = TokenClient::new(&env, &token_id);
        let asset = StellarAssetClient::new(&env, &token_id);

        let mut members = soroban_sdk::Vec::new(&env);
        for _ in 0..MEMBERS {
            let m = Address::generate(&env);
            asset.mint(&m, &(ROUND_AMOUNT * (1 + MEMBERS as i128)));
            members.push_back(m);
        }

        let circle_id = env.register_contract(None, CircleContract);
        let circle = CircleContractClient::new(&env, &circle_id);

        let rep_id = env.register_contract(None, ReputationContract);
        let rep = ReputationContractClient::new(&env, &rep_id);
        let rep_admin = Address::generate(&env);
        rep.initialize(&rep_admin);
        rep.add_authorized_caller(&rep_admin, &circle_id);

        let admin = Address::generate(&env);
        circle.initialize(&admin, &members, &ROUND_AMOUNT, &token_id, &rep_id, &ROUND_DEADLINE);

        Fx { env, circle, circle_id, token, token_id, rep, rep_id, admin, members }
    }

    /// A fixture in the state the matrix row describes.
    fn in_condition(condition: Condition) -> Self {
        let fx = Fx::new();
        match condition {
            Condition::Pending => fx.circle.join(&fx.member(0)),
            Condition::Active => fx.join_all(),
            Condition::Completed => {
                fx.join_all();
                fx.complete_all_rounds();
            }
            Condition::Cancelled => {
                fx.circle.join(&fx.member(0));
                fx.circle.cancel(&fx.member(0));
            }
            Condition::Paused => {
                fx.join_all();
                fx.circle.pause(&fx.admin);
            }
            Condition::Closed => {
                fx.join_all();
                fx.complete_all_rounds();
                fx.circle.close(&fx.member(0));
            }
        }
        assert_eq!(fx.lifecycle(), condition.lifecycle(), "fixture for {condition:?}");
        fx
    }

    fn member(&self, i: u32) -> Address {
        self.members.get(i).unwrap()
    }

    fn join_all(&self) {
        for m in self.members.iter() {
            self.circle.join(&m);
        }
    }

    fn complete_all_rounds(&self) {
        for _ in 0..MEMBERS {
            for m in self.members.iter() {
                self.circle.contribute(&m);
            }
            self.circle.payout();
        }
    }

    fn lifecycle(&self) -> Lifecycle {
        Lifecycle {
            phase: Phase::from(&self.circle.get_status()),
            paused: self.circle.is_paused(),
            closed: self.circle.is_closed(),
        }
    }

    /// Apply `mutation` with its canonical caller (see `TRANSITION_MATRIX`).
    fn apply(&self, mutation: Mutation) -> Result<(), Error> {
        let c = &self.circle;
        match mutation {
            Mutation::Initialize => norm(c.try_initialize(
                &self.admin,
                &self.members,
                &ROUND_AMOUNT,
                &self.token_id,
                &self.rep_id,
                &ROUND_DEADLINE,
            )),
            Mutation::Join => norm(c.try_join(&self.member(MEMBERS - 1))),
            Mutation::Cancel => norm(c.try_cancel(&self.member(0))),
            Mutation::Contribute => norm(c.try_contribute(&self.member(0))),
            Mutation::Payout => norm(c.try_payout()),
            Mutation::MarkDefault => norm(c.try_mark_default(&self.member(0))),
            Mutation::SettleRound => norm(c.try_settle_round()),
            Mutation::Close => norm(c.try_close(&self.member(0))),
            Mutation::Pause => norm(c.try_pause(&self.admin)),
            Mutation::Resume => norm(c.try_resume(&self.admin)),
        }
    }

    /// Everything a rejected mutation must leave untouched.
    fn snapshot(&self) -> Snapshot {
        let mut s = self.env.as_contract(&self.circle_id, || {
            let inst = self.env.storage().instance();
            let pers = self.env.storage().persistent();
            let round: Option<RoundState> = inst.get(&DataKey::CurrentRound);
            let mut s = Snapshot {
                status: inst.get(&DataKey::Status),
                paused: inst.get(&DataKey::Paused).unwrap_or(false),
                closed: inst.has(&DataKey::Closed),
                rounds_completed: inst.get(&DataKey::RoundsCompleted).unwrap(),
                round: round.map(|r| {
                    (r.round_index, r.recipient, r.contributions_received, r.deadline_ledger, r.paid_out)
                }),
                ..Snapshot::default()
            };
            for m in self.members.iter() {
                s.collateral.push(pers.get(&DataKey::Collateral(m.clone())));
                s.defaults.push(pers.get(&DataKey::Defaults(m.clone())));
                for r in 0..MEMBERS {
                    s.contributed.push(pers.has(&DataKey::Contributed(m.clone(), r)));
                    s.defaulted.push(pers.has(&DataKey::Defaulted(m.clone(), r)));
                }
            }
            s
        });
        s.circle_balance = self.token.balance(&self.circle_id);
        for m in self.members.iter() {
            s.balances.push(self.token.balance(&m));
            s.reputation.push(self.rep.score(&m));
        }
        s
    }

    /// Run `f` and return how many events the circle published during it.
    fn circle_events_during(&self, f: impl FnOnce() -> Result<(), Error>) -> (Result<(), Error>, usize) {
        let before = self.env.events().all().len();
        let result = f();
        let emitted = self
            .env
            .events()
            .all()
            .iter()
            .skip(before as usize)
            .filter(|(contract, _, _)| contract == &self.circle_id)
            .count();
        (result, emitted)
    }
}

#[derive(Debug, Default, PartialEq)]
struct Snapshot {
    status: Option<CircleStatus>,
    paused: bool,
    closed: bool,
    rounds_completed: u32,
    round: Option<(u32, Address, u32, u64, bool)>,
    circle_balance: i128,
    balances: Vec<i128>,
    collateral: Vec<Option<i128>>,
    defaults: Vec<Option<u32>>,
    contributed: Vec<bool>,
    defaulted: Vec<bool>,
    reputation: Vec<u32>,
}

/// Collapse a generated `try_*` result into `Result<(), Error>`.
///
/// A contract error outside the entry-point's declared error enum (e.g.
/// `ContractError::CirclePaused` from `close`) arrives as
/// `InvokeError::Contract(code)`; it is mapped to the same `Error` value.
fn norm<T, C, E: Into<Error>>(r: Result<Result<T, C>, Result<E, InvokeError>>) -> Result<(), Error> {
    match r {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(_)) => panic!("return value failed to decode"),
        Err(Ok(e)) => Err(e.into()),
        Err(Err(InvokeError::Contract(code))) => Err(Error::from_contract_error(code)),
        Err(Err(InvokeError::Abort)) => panic!("invocation aborted without a contract error"),
    }
}

/// Apply one mutation and assert the outcome the matrix (or overlay) expects.
fn check_cell(fx: &Fx, label: &str, mutation: Mutation, outcome: Outcome) {
    let before = fx.snapshot();
    let (result, events) = fx.circle_events_during(|| fx.apply(mutation));
    match outcome {
        Outcome::Allowed(next) => {
            assert_eq!(result, Ok(()), "{label}: {mutation:?} must succeed");
            assert_eq!(fx.lifecycle(), next, "{label}: {mutation:?} resulting state");
            assert_ne!(fx.snapshot(), before, "{label}: {mutation:?} must change observable state");
        }
        Outcome::Rejected(rejection) => {
            assert_eq!(
                result,
                Err(Error::from_contract_error(rejection.code())),
                "{label}: {mutation:?} must fail with {rejection:?}"
            );
            assert_eq!(fx.snapshot(), before, "{label}: rejected {mutation:?} changed state");
            assert_eq!(events, 0, "{label}: rejected {mutation:?} emitted events");
        }
    }
}

// ── Matrix structure ──────────────────────────────────────────────────────────

#[test]
fn indices_are_dense_and_match_all_arrays() {
    assert_eq!(Condition::ALL.len(), CONDITION_COUNT);
    assert_eq!(Mutation::ALL.len(), MUTATION_COUNT);
    for (i, c) in Condition::ALL.iter().enumerate() {
        assert_eq!(c.index(), i, "{c:?}");
    }
    for (i, m) in Mutation::ALL.iter().enumerate() {
        assert_eq!(m.index(), i, "{m:?}");
    }
}

#[test]
fn paused_row_is_the_overlay_applied_to_active() {
    for m in Mutation::ALL {
        assert_eq!(
            expected(Condition::Paused, m),
            paused_overlay(Condition::Active.lifecycle(), m),
            "{m:?}"
        );
    }
}

#[test]
fn closed_overlay_only_reachable_from_terminal_phases() {
    for c in Condition::ALL {
        for m in Mutation::ALL {
            if let Outcome::Allowed(next) = expected(c, m) {
                if next.closed {
                    assert!(
                        matches!(next.phase, Phase::Completed | Phase::Cancelled),
                        "{c:?} × {m:?} closes a non-terminal circle"
                    );
                }
                if c.lifecycle().closed {
                    assert!(next.closed, "{c:?} × {m:?} reopens a closed circle");
                }
            }
        }
    }
}

/// Every `pub fn` on `CircleContract` must be either a `Mutation` or a known
/// read-only view, so a new entry-point forces a matrix update.
#[test]
fn every_contract_entry_point_is_classified() {
    const VIEWS: &[&str] = &[
        "is_closed",
        "is_paused",
        "get_admin",
        "get_usdc_token",
        "get_protocol_params",
        "get_config",
        "get_status",
        "get_current_round",
        "get_pot_amount",
        "get_collateral",
        "get_defaults",
        "has_contributed",
    ];
    let src = include_str!("../src/lib.rs");
    let body = src
        .split("impl CircleContract {")
        .nth(1)
        .expect("contractimpl block");
    let body = body.split("\n}").next().unwrap();
    let entry_points: Vec<&str> = body
        .lines()
        .filter_map(|l| l.strip_prefix("    pub fn "))
        .map(|l| l.split('(').next().unwrap())
        .collect();

    for name in &entry_points {
        let is_mutation = Mutation::ALL.iter().any(|m| m.name() == *name);
        let is_view = VIEWS.contains(name);
        assert!(
            is_mutation ^ is_view,
            "`{name}` must be listed exactly once: as a transition::Mutation or as a view"
        );
    }
    for m in Mutation::ALL {
        assert!(entry_points.contains(&m.name()), "{m:?} has no entry-point");
    }
}

// ── Matrix vs. contract ───────────────────────────────────────────────────────

#[test]
fn every_matrix_cell_matches_contract_behaviour() {
    for c in Condition::ALL {
        for m in Mutation::ALL {
            let fx = Fx::in_condition(c);
            check_cell(&fx, &format!("{c:?}"), m, expected(c, m));
        }
    }
}

#[test]
fn pause_overlay_holds_for_every_base_condition() {
    for base in Condition::ALL {
        if base == Condition::Paused {
            continue;
        }
        for m in Mutation::ALL {
            let fx = Fx::in_condition(base);
            fx.circle.pause(&fx.admin);
            check_cell(&fx, &format!("{base:?}+paused"), m, paused_overlay(base.lifecycle(), m));
        }
    }
}

#[test]
fn pause_then_resume_restores_exact_state() {
    for base in Condition::ALL {
        if base == Condition::Paused {
            continue;
        }
        let fx = Fx::in_condition(base);
        let before = fx.snapshot();
        fx.circle.pause(&fx.admin);
        fx.circle.resume(&fx.admin);
        assert_eq!(fx.snapshot(), before, "{base:?}");
    }
}

#[test]
fn rejections_are_repeatable() {
    // A rejected call must not "use up" anything: repeating it gives the
    // same error and still changes nothing.
    for c in Condition::ALL {
        let fx = Fx::in_condition(c);
        for m in Mutation::ALL {
            if let Outcome::Rejected(_) = expected(c, m) {
                check_cell(&fx, &format!("{c:?} (1st)"), m, expected(c, m));
                check_cell(&fx, &format!("{c:?} (2nd)"), m, expected(c, m));
            }
        }
    }
}
