//! Lifecycle transition matrix for every externally callable mutation.
//!
//! This module is the specification; `tests/transition_tests.rs` drives the
//! real contract through every cell and checks it agrees.
//!
//! # State model
//!
//! The observable lifecycle is a [`Lifecycle`]: the stored [`CircleStatus`]
//! plus two independent flags.
//!
//! | Component | Storage | Values |
//! |---|---|---|
//! | base status | `DataKey::Status` | `Pending → Active → Completed`, or `Pending → Cancelled` |
//! | pause overlay | `DataKey::Paused` | set by `pause`, cleared by `resume`, in any status |
//! | closed overlay | `DataKey::Closed` | set once by `close`, only from `Completed` / `Cancelled`; never cleared |
//!
//! # Pause overlay rule
//!
//! Pause does not change the base status.  While paused:
//!
//! * every fund-moving mutation (`join`, `cancel`, `contribute`, `payout`,
//!   `mark_default`, `settle_round`, `close`) is rejected with
//!   `ContractError::CirclePaused`, **before** any status check — so a
//!   paused circle reports `CirclePaused` even where the unpaused circle
//!   would report `WrongStatus` or `AlreadyClosed`;
//! * `pause` is rejected with `PauseError::AlreadyPaused`;
//! * `resume` clears the flag and restores the exact prior state;
//! * `initialize` is still rejected with `InitError::AlreadyInitialized`.
//!
//! [`paused_overlay`] states this rule for any base state.
//!
//! # Terminal states
//!
//! `Completed` and `Cancelled` accept only `close`, `pause`, and `resume`
//! (`close` releases collateral and sets the closed overlay).  `Closed` is
//! absorbing: `close` returns `CloseError::AlreadyClosed`, every other
//! fund-moving mutation `WrongStatus`.  `pause` / `resume` still work on a
//! closed circle; they move no funds.
//!
//! # Keeping the matrix complete
//!
//! [`TRANSITION_MATRIX`] is a `[[Outcome; MUTATION_COUNT]; CONDITION_COUNT]`
//! grid, and [`Condition::index`], [`Mutation::index`] and
//! `Phase::from(&CircleStatus)` are exhaustive `match`es.  Adding a status,
//! condition or mutation therefore fails to compile until the grid gains the
//! new row or column.  `tests/transition_tests.rs` additionally fails if a
//! `pub fn` on `CircleContract` is neither a [`Mutation`] nor a listed view.

use crate::{CircleStatus, CloseError, ContractError, InitError, PauseError};

/// `Copy` mirror of [`CircleStatus`] usable in `const` tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Pending,
    Active,
    Completed,
    Cancelled,
}

impl From<&CircleStatus> for Phase {
    fn from(status: &CircleStatus) -> Self {
        // Exhaustive on purpose: a new CircleStatus must be placed here.
        match status {
            CircleStatus::Pending => Phase::Pending,
            CircleStatus::Active => Phase::Active,
            CircleStatus::Completed => Phase::Completed,
            CircleStatus::Cancelled => Phase::Cancelled,
        }
    }
}

impl From<Phase> for CircleStatus {
    fn from(phase: Phase) -> Self {
        match phase {
            Phase::Pending => CircleStatus::Pending,
            Phase::Active => CircleStatus::Active,
            Phase::Completed => CircleStatus::Completed,
            Phase::Cancelled => CircleStatus::Cancelled,
        }
    }
}

/// Base status plus the pause and closed overlays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lifecycle {
    pub phase: Phase,
    pub paused: bool,
    pub closed: bool,
}

impl Lifecycle {
    const fn open(phase: Phase) -> Self {
        Lifecycle { phase, paused: false, closed: false }
    }

    const fn with_pause(self, paused: bool) -> Self {
        Lifecycle { phase: self.phase, paused, closed: self.closed }
    }

    const fn closed(phase: Phase) -> Self {
        Lifecycle { phase, paused: false, closed: true }
    }
}

/// The six named conditions the matrix is written against.
///
/// Each names a concrete, reproducible fixture (built in the tests):
///
/// | Condition | Fixture |
/// |---|---|
/// | `Pending` | initialized, first member joined, others not |
/// | `Active` | all joined, round 0 open, no contributions, before deadline |
/// | `Completed` | every round paid out |
/// | `Cancelled` | first member joined, then cancelled |
/// | `Paused` | `Active` + `pause` |
/// | `Closed` | `Completed` + `close` |
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Condition {
    Pending,
    Active,
    Completed,
    Cancelled,
    Paused,
    Closed,
}

pub const CONDITION_COUNT: usize = 6;

impl Condition {
    pub const ALL: [Condition; CONDITION_COUNT] = [
        Condition::Pending,
        Condition::Active,
        Condition::Completed,
        Condition::Cancelled,
        Condition::Paused,
        Condition::Closed,
    ];

    pub const fn index(self) -> usize {
        match self {
            Condition::Pending => 0,
            Condition::Active => 1,
            Condition::Completed => 2,
            Condition::Cancelled => 3,
            Condition::Paused => 4,
            Condition::Closed => 5,
        }
    }

    /// The lifecycle state the condition's fixture is in.
    pub const fn lifecycle(self) -> Lifecycle {
        match self {
            Condition::Pending => Lifecycle::open(Phase::Pending),
            Condition::Active => Lifecycle::open(Phase::Active),
            Condition::Completed => Lifecycle::open(Phase::Completed),
            Condition::Cancelled => Lifecycle::open(Phase::Cancelled),
            Condition::Paused => Lifecycle::open(Phase::Active).with_pause(true),
            Condition::Closed => Lifecycle::closed(Phase::Completed),
        }
    }
}

/// Every externally callable entry-point that writes state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mutation {
    Initialize,
    Join,
    Cancel,
    Contribute,
    Payout,
    MarkDefault,
    SettleRound,
    Close,
    Pause,
    Resume,
}

pub const MUTATION_COUNT: usize = 10;

impl Mutation {
    pub const ALL: [Mutation; MUTATION_COUNT] = [
        Mutation::Initialize,
        Mutation::Join,
        Mutation::Cancel,
        Mutation::Contribute,
        Mutation::Payout,
        Mutation::MarkDefault,
        Mutation::SettleRound,
        Mutation::Close,
        Mutation::Pause,
        Mutation::Resume,
    ];

    pub const fn index(self) -> usize {
        match self {
            Mutation::Initialize => 0,
            Mutation::Join => 1,
            Mutation::Cancel => 2,
            Mutation::Contribute => 3,
            Mutation::Payout => 4,
            Mutation::MarkDefault => 5,
            Mutation::SettleRound => 6,
            Mutation::Close => 7,
            Mutation::Pause => 8,
            Mutation::Resume => 9,
        }
    }

    /// Contract entry-point name.
    pub const fn name(self) -> &'static str {
        match self {
            Mutation::Initialize => "initialize",
            Mutation::Join => "join",
            Mutation::Cancel => "cancel",
            Mutation::Contribute => "contribute",
            Mutation::Payout => "payout",
            Mutation::MarkDefault => "mark_default",
            Mutation::SettleRound => "settle_round",
            Mutation::Close => "close",
            Mutation::Pause => "pause",
            Mutation::Resume => "resume",
        }
    }

    /// Whether `assert_not_paused` gates this entry-point.
    pub const fn moves_funds(self) -> bool {
        match self {
            Mutation::Join
            | Mutation::Cancel
            | Mutation::Contribute
            | Mutation::Payout
            | Mutation::MarkDefault
            | Mutation::SettleRound
            | Mutation::Close => true,
            Mutation::Initialize | Mutation::Pause | Mutation::Resume => false,
        }
    }
}

/// The typed error a rejected mutation fails with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Rejection {
    Contract(ContractError),
    Close(CloseError),
    Pause(PauseError),
    Init(InitError),
}

impl Rejection {
    /// Stable contract error code as seen on the wire.
    pub const fn code(self) -> u32 {
        match self {
            Rejection::Contract(e) => e as u32,
            Rejection::Close(e) => e as u32,
            Rejection::Pause(e) => e as u32,
            Rejection::Init(e) => e as u32,
        }
    }
}

/// Result of applying one mutation in one condition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
    /// The call succeeds and leaves the circle in this state.
    Allowed(Lifecycle),
    /// The call fails with this error and changes nothing.
    Rejected(Rejection),
}

use Outcome::{Allowed, Rejected};

const ALREADY_INIT: Outcome = Rejected(Rejection::Init(InitError::AlreadyInitialized));
const WRONG_STATUS: Outcome = Rejected(Rejection::Contract(ContractError::WrongStatus));
const PAUSED: Outcome = Rejected(Rejection::Contract(ContractError::CirclePaused));
const NOT_TERMINAL: Outcome = Rejected(Rejection::Close(CloseError::NotTerminalState));
const NOT_PAUSED: Outcome = Rejected(Rejection::Pause(PauseError::NotPaused));

const PENDING: Lifecycle = Lifecycle::open(Phase::Pending);
const ACTIVE: Lifecycle = Lifecycle::open(Phase::Active);
const COMPLETED: Lifecycle = Lifecycle::open(Phase::Completed);
const CANCELLED: Lifecycle = Lifecycle::open(Phase::Cancelled);
const CLOSED: Lifecycle = Lifecycle::closed(Phase::Completed);

/// `TRANSITION_MATRIX[condition.index()][mutation.index()]`.
///
/// Columns: initialize, join, cancel, contribute, payout, mark_default,
/// settle_round, close, pause, resume.  Canonical callers: `join` by a member
/// who has not joined in `Pending`; `cancel`, `contribute`, `mark_default`,
/// `close` by the first member; `pause` / `resume` by the admin.
pub const TRANSITION_MATRIX: [[Outcome; MUTATION_COUNT]; CONDITION_COUNT] = [
    // Pending
    [
        ALREADY_INIT,
        Allowed(PENDING),
        Allowed(CANCELLED),
        WRONG_STATUS,
        WRONG_STATUS,
        WRONG_STATUS,
        WRONG_STATUS,
        NOT_TERMINAL,
        Allowed(PENDING.with_pause(true)),
        NOT_PAUSED,
    ],
    // Active
    [
        ALREADY_INIT,
        WRONG_STATUS,
        WRONG_STATUS,
        Allowed(ACTIVE),
        Rejected(Rejection::Contract(ContractError::IncompleteRound)),
        Rejected(Rejection::Contract(ContractError::DeadlineNotPassed)),
        Rejected(Rejection::Contract(ContractError::DeadlineNotPassed)),
        NOT_TERMINAL,
        Allowed(ACTIVE.with_pause(true)),
        NOT_PAUSED,
    ],
    // Completed
    [
        ALREADY_INIT,
        WRONG_STATUS,
        WRONG_STATUS,
        WRONG_STATUS,
        WRONG_STATUS,
        WRONG_STATUS,
        WRONG_STATUS,
        Allowed(Lifecycle::closed(Phase::Completed)),
        Allowed(COMPLETED.with_pause(true)),
        NOT_PAUSED,
    ],
    // Cancelled
    [
        ALREADY_INIT,
        WRONG_STATUS,
        WRONG_STATUS,
        WRONG_STATUS,
        WRONG_STATUS,
        WRONG_STATUS,
        WRONG_STATUS,
        Allowed(Lifecycle::closed(Phase::Cancelled)),
        Allowed(CANCELLED.with_pause(true)),
        NOT_PAUSED,
    ],
    // Paused (Active + paused)
    [
        ALREADY_INIT,
        PAUSED,
        PAUSED,
        PAUSED,
        PAUSED,
        PAUSED,
        PAUSED,
        PAUSED,
        Rejected(Rejection::Pause(PauseError::AlreadyPaused)),
        Allowed(ACTIVE),
    ],
    // Closed (Completed + closed)
    [
        ALREADY_INIT,
        WRONG_STATUS,
        WRONG_STATUS,
        WRONG_STATUS,
        WRONG_STATUS,
        WRONG_STATUS,
        WRONG_STATUS,
        Rejected(Rejection::Close(CloseError::AlreadyClosed)),
        Allowed(CLOSED.with_pause(true)),
        NOT_PAUSED,
    ],
];

/// Expected outcome of `mutation` in `condition`.
pub const fn expected(condition: Condition, mutation: Mutation) -> Outcome {
    TRANSITION_MATRIX[condition.index()][mutation.index()]
}

/// Expected outcome of `mutation` on `base` with the pause flag set.
///
/// Holds for every base state, paused or not — the pause check runs before
/// any status check.  The `Paused` row of [`TRANSITION_MATRIX`] is this
/// function applied to `Active`.
pub const fn paused_overlay(base: Lifecycle, mutation: Mutation) -> Outcome {
    if mutation.moves_funds() {
        return PAUSED;
    }
    match mutation {
        Mutation::Pause => Rejected(Rejection::Pause(PauseError::AlreadyPaused)),
        Mutation::Resume => Allowed(base.with_pause(false)),
        _ => ALREADY_INIT,
    }
}
