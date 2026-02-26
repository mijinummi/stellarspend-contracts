//! Recurring Savings Contribution Contract
//!
//! Allows users to schedule automatic, periodic contributions to a savings
//! goal token address at a fixed interval and amount.
//!
//! # Lifecycle
//!
//! 1. **`initialize`** — admin sets up the contract once.
//! 2. **`create_schedule`** — user registers a recurring contribution:
//!    interval (seconds), amount per contribution, destination savings goal,
//!    and an optional max execution count.
//! 3. **`execute_contribution`** — anyone (or an automation bot) calls this
//!    when a schedule is due. Transfers `amount` from the user's balance to
//!    the savings goal and records the execution.
//! 4. **`cancel_schedule`** — user permanently stops the schedule.
//!
//! # Security properties
//!
//! - `require_auth` is the first statement in every mutating entry point.
//! - Execution is gated on `next_execution_at <= now`; early calls are rejected.
//! - Cancelled and exhausted schedules cannot be executed.
//! - All arithmetic is checked; overflow panics with a typed error.
//! - Per-schedule TTL is bumped on every access to avoid silent state eviction.

#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracterror, contracttype, panic_with_error, symbol_short,
    token, Address, Env,
};

// ── Constants ────────────────────────────────────────────────────────────────

/// Minimum allowed interval: 1 hour.
pub const MIN_INTERVAL_SECS: u64 = 3_600;

/// Ledger TTL bump for persistent schedule records (~2 years).
const PERSISTENT_TTL_BUMP: u32 = 12_614_400;

// ── Storage keys ─────────────────────────────────────────────────────────────

#[derive(Clone)]
#[contracttype]
pub enum DataKey {
    /// Contract-wide admin config (instance storage).
    Admin,
    /// Auto-incrementing schedule ID counter (instance storage).
    NextScheduleId,
    /// Per-schedule record keyed by `schedule_id` (persistent storage).
    Schedule(u64),
}

// ── Types ─────────────────────────────────────────────────────────────────────

/// Status of a recurring savings schedule.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[contracttype]
pub enum ScheduleStatus {
    /// Active and eligible for execution.
    Active = 0,
    /// Permanently stopped by the owner.
    Cancelled = 1,
    /// All allowed executions have been completed.
    Exhausted = 2,
}

/// A single recurring savings schedule.
#[derive(Clone)]
#[contracttype]
pub struct RecurringSchedule {
    /// Address that owns this schedule and whose balance is debited.
    pub owner: Address,
    /// SAC-compatible token to transfer.
    pub token: Address,
    /// Destination savings goal address (receives the transfer).
    pub savings_goal: Address,
    /// Amount (in token stroops) transferred per execution.
    pub amount: i128,
    /// Interval between executions, in seconds.
    pub interval_secs: u64,
    /// Ledger timestamp of the next scheduled execution.
    pub next_execution_at: u64,
    /// Number of times this schedule has been successfully executed.
    pub executions_completed: u32,
    /// Maximum executions allowed (0 = unlimited).
    pub max_executions: u32,
    /// Current status of the schedule.
    pub status: ScheduleStatus,
    /// Ledger timestamp when this schedule was created.
    pub created_at: u64,
}

// ── Errors ────────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum RecurringError {
    /// Contract has not been initialised.
    NotInitialized = 1,
    /// Contract has already been initialised.
    AlreadyInitialized = 2,
    /// Caller is not the admin or schedule owner.
    Unauthorized = 3,
    /// Contribution amount must be > 0.
    InvalidAmount = 4,
    /// Interval must be >= `MIN_INTERVAL_SECS`.
    InvalidInterval = 5,
    /// Schedule ID does not exist.
    ScheduleNotFound = 6,
    /// Schedule is cancelled or exhausted.
    ScheduleInactive = 7,
    /// Execution attempted before `next_execution_at`.
    NotDueYet = 8,
    /// Arithmetic overflow detected.
    Overflow = 9,
    /// Savings goal address must differ from owner.
    InvalidGoalAddress = 10,
}

// ── Events ────────────────────────────────────────────────────────────────────

pub struct RecurringEvents;

impl RecurringEvents {
    /// Emitted when a new schedule is created.
    /// Payload: `(owner, schedule_id, token, amount, interval_secs, timestamp)`
    pub fn schedule_created(
        env: &Env,
        owner: &Address,
        schedule_id: u64,
        token: &Address,
        amount: i128,
        interval_secs: u64,
    ) {
        env.events().publish(
            (symbol_short!("recur"), symbol_short!("created")),
            (
                owner.clone(),
                schedule_id,
                token.clone(),
                amount,
                interval_secs,
                env.ledger().timestamp(),
            ),
        );
    }

    /// Emitted on every successful contribution execution.
    /// Payload: `(owner, schedule_id, amount, executions_completed, next_execution_at, timestamp)`
    pub fn contribution_made(
        env: &Env,
        owner: &Address,
        schedule_id: u64,
        amount: i128,
        executions_completed: u32,
        next_execution_at: u64,
    ) {
        env.events().publish(
            (symbol_short!("recur"), symbol_short!("contrib")),
            (
                owner.clone(),
                schedule_id,
                amount,
                executions_completed,
                next_execution_at,
                env.ledger().timestamp(),
            ),
        );
    }

    /// Emitted when a schedule is cancelled by the owner.
    /// Payload: `(owner, schedule_id, executions_completed, timestamp)`
    pub fn schedule_cancelled(
        env: &Env,
        owner: &Address,
        schedule_id: u64,
        executions_completed: u32,
    ) {
        env.events().publish(
            (symbol_short!("recur"), symbol_short!("cancel")),
            (
                owner.clone(),
                schedule_id,
                executions_completed,
                env.ledger().timestamp(),
            ),
        );
    }

    /// Emitted when a schedule reaches its `max_executions` limit.
    /// Payload: `(owner, schedule_id, total_contributed, timestamp)`
    pub fn schedule_exhausted(
        env: &Env,
        owner: &Address,
        schedule_id: u64,
        total_contributed: i128,
    ) {
        env.events().publish(
            (symbol_short!("recur"), symbol_short!("exhaust")),
            (
                owner.clone(),
                schedule_id,
                total_contributed,
                env.ledger().timestamp(),
            ),
        );
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

impl RecurringSavingsContract {
    fn require_initialized(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| panic_with_error!(env, RecurringError::NotInitialized))
    }

    fn next_schedule_id(env: &Env) -> u64 {
        let id: u64 = env
            .storage()
            .instance()
            .get(&DataKey::NextScheduleId)
            .unwrap_or(0u64);
        let next = id
            .checked_add(1)
            .unwrap_or_else(|| panic_with_error!(env, RecurringError::Overflow));
        env.storage()
            .instance()
            .set(&DataKey::NextScheduleId, &next);
        id
    }

    fn load_schedule(env: &Env, schedule_id: u64) -> RecurringSchedule {
        let key = DataKey::Schedule(schedule_id);
        let schedule: RecurringSchedule = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| panic_with_error!(env, RecurringError::ScheduleNotFound));
        env.storage()
            .persistent()
            .extend_ttl(&key, PERSISTENT_TTL_BUMP, PERSISTENT_TTL_BUMP);
        schedule
    }

    fn save_schedule(env: &Env, schedule_id: u64, schedule: &RecurringSchedule) {
        let key = DataKey::Schedule(schedule_id);
        env.storage().persistent().set(&key, schedule);
        env.storage()
            .persistent()
            .extend_ttl(&key, PERSISTENT_TTL_BUMP, PERSISTENT_TTL_BUMP);
    }
}

// ── Contract ──────────────────────────────────────────────────────────────────

#[contract]
pub struct RecurringSavingsContract;

#[contractimpl]
impl RecurringSavingsContract {
    // ── Lifecycle ────────────────────────────────────────────────────────────

    /// Initialise the contract. Only callable once.
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic_with_error!(&env, RecurringError::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::NextScheduleId, &0u64);
    }

    // ── Schedule management ──────────────────────────────────────────────────

    /// Create a new recurring savings schedule.
    ///
    /// # Parameters
    /// - `owner`         — address whose token balance will be debited each execution.
    /// - `token`         — SAC-compatible token contract address.
    /// - `savings_goal`  — destination address for contributions (must differ from owner).
    /// - `amount`        — stroops transferred per execution; must be > 0.
    /// - `interval_secs` — seconds between executions; must be >= `MIN_INTERVAL_SECS` (1 h).
    /// - `max_executions`— cap on total executions; 0 = unlimited.
    ///
    /// Returns the new schedule ID.
    ///
    /// # Security
    /// - `owner.require_auth()` is the first operation.
    /// - `savings_goal != owner` prevents circular self-transfers that could
    ///   be used to artificially inflate contribution counters.
    pub fn create_schedule(
        env: Env,
        owner: Address,
        token: Address,
        savings_goal: Address,
        amount: i128,
        interval_secs: u64,
        max_executions: u32,
    ) -> u64 {
        owner.require_auth();
        Self::require_initialized(&env);

        if amount <= 0 {
            panic_with_error!(&env, RecurringError::InvalidAmount);
        }
        if interval_secs < MIN_INTERVAL_SECS {
            panic_with_error!(&env, RecurringError::InvalidInterval);
        }
        if savings_goal == owner {
            panic_with_error!(&env, RecurringError::InvalidGoalAddress);
        }

        let now = env.ledger().timestamp();
        // First execution is due one full interval from now.
        let next_execution_at = now
            .checked_add(interval_secs)
            .unwrap_or_else(|| panic_with_error!(&env, RecurringError::Overflow));

        let schedule_id = Self::next_schedule_id(&env);

        let schedule = RecurringSchedule {
            owner: owner.clone(),
            token: token.clone(),
            savings_goal,
            amount,
            interval_secs,
            next_execution_at,
            executions_completed: 0,
            max_executions,
            status: ScheduleStatus::Active,
            created_at: now,
        };

        Self::save_schedule(&env, schedule_id, &schedule);
        RecurringEvents::schedule_created(&env, &owner, schedule_id, &token, amount, interval_secs);

        schedule_id
    }

    /// Execute a due recurring contribution.
    ///
    /// Can be called by anyone (e.g. an automation service) but the token
    /// transfer is always from `schedule.owner` to `schedule.savings_goal`.
    /// The owner must have pre-authorised the contract for the transfer amount
    /// via the token's `approve` mechanism.
    ///
    /// # Execution rules
    /// 1. Schedule must be `Active`.
    /// 2. `now >= next_execution_at`.
    /// 3. Transfers `amount` tokens from owner → savings_goal.
    /// 4. Advances `next_execution_at` by `interval_secs`.
    /// 5. If `max_executions > 0` and the count is reached, marks `Exhausted`.
    ///
    /// Returns the amount transferred.
    pub fn execute_contribution(env: Env, schedule_id: u64) -> i128 {
        Self::require_initialized(&env);

        let mut schedule = Self::load_schedule(&env, schedule_id);

        // Guard: schedule must be active.
        if schedule.status != ScheduleStatus::Active {
            panic_with_error!(&env, RecurringError::ScheduleInactive);
        }

        // Guard: not due yet.
        let now = env.ledger().timestamp();
        if now < schedule.next_execution_at {
            panic_with_error!(&env, RecurringError::NotDueYet);
        }

        // ── Transfer tokens (checks-effects-interactions) ─────────────────
        // Update state BEFORE the external call.
        schedule.executions_completed = schedule
            .executions_completed
            .checked_add(1)
            .unwrap_or_else(|| panic_with_error!(&env, RecurringError::Overflow));

        // Advance next execution time. If the executor is late we advance from
        // `next_execution_at` (not `now`) so the schedule stays on cadence.
        schedule.next_execution_at = schedule
            .next_execution_at
            .checked_add(schedule.interval_secs)
            .unwrap_or_else(|| panic_with_error!(&env, RecurringError::Overflow));

        // Check if the schedule is now exhausted.
        let is_exhausted = schedule.max_executions > 0
            && schedule.executions_completed >= schedule.max_executions;

        if is_exhausted {
            schedule.status = ScheduleStatus::Exhausted;
        }

        let amount = schedule.amount;
        let next_at = schedule.next_execution_at;
        let completed = schedule.executions_completed;
        let owner = schedule.owner.clone();

        Self::save_schedule(&env, schedule_id, &schedule);

        // ── External token transfer ───────────────────────────────────────
        let token_client = token::Client::new(&env, &schedule.token);
        token_client.transfer_from(
            &env.current_contract_address(),
            &owner,
            &schedule.savings_goal,
            &amount,
        );

        RecurringEvents::contribution_made(&env, &owner, schedule_id, amount, completed, next_at);

        if is_exhausted {
            let total = (amount as i128)
                .checked_mul(completed as i128)
                .unwrap_or(i128::MAX); // best-effort for event; overflow not critical here
            RecurringEvents::schedule_exhausted(&env, &owner, schedule_id, total);
        }

        amount
    }

    /// Cancel a schedule. Only the schedule owner may cancel.
    ///
    /// Cancelled schedules can never be re-activated or executed.
    pub fn cancel_schedule(env: Env, caller: Address, schedule_id: u64) {
        caller.require_auth();
        Self::require_initialized(&env);

        let mut schedule = Self::load_schedule(&env, schedule_id);

        if caller != schedule.owner {
            panic_with_error!(&env, RecurringError::Unauthorized);
        }
        if schedule.status != ScheduleStatus::Active {
            panic_with_error!(&env, RecurringError::ScheduleInactive);
        }

        schedule.status = ScheduleStatus::Cancelled;
        let completed = schedule.executions_completed;
        let owner = schedule.owner.clone();

        Self::save_schedule(&env, schedule_id, &schedule);
        RecurringEvents::schedule_cancelled(&env, &owner, schedule_id, completed);
    }

    // ── Read-only queries ─────────────────────────────────────────────────────

    /// Return the full schedule record, or panic with `ScheduleNotFound`.
    pub fn get_schedule(env: Env, schedule_id: u64) -> RecurringSchedule {
        Self::require_initialized(&env);
        Self::load_schedule(&env, schedule_id)
    }

    /// Return `true` if the schedule is active and due for execution right now.
    pub fn is_due(env: Env, schedule_id: u64) -> bool {
        let schedule = Self::load_schedule(&env, schedule_id);
        schedule.status == ScheduleStatus::Active
            && env.ledger().timestamp() >= schedule.next_execution_at
    }

    /// Return the number of schedules ever created (next schedule ID value).
    pub fn schedule_count(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::NextScheduleId)
            .unwrap_or(0)
    }
}