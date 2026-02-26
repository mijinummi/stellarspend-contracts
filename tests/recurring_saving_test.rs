//! Tests for contracts/recurring_savings.rs
//!
//! Run with: `cargo test --test recurring_savings_tests`
//!
//! Test taxonomy
//! ─────────────
//! happy_*     — correct flows
//! neg_*       — inputs / calls that must be rejected
//! edge_*      — boundary and scheduling corner cases
//! security_*  — auth guards, duplicate-execution, state isolation

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    token, Address, Env,
};

use crate::recurring_savings::{
    RecurringError, RecurringSavingsContract, RecurringSavingsContractClient,
    ScheduleStatus, MIN_INTERVAL_SECS,
};

// ── Constants ─────────────────────────────────────────────────────────────────

const AMOUNT: i128 = 1_000;
const INTERVAL: u64 = MIN_INTERVAL_SECS; // 1 hour

// ── Test harness ──────────────────────────────────────────────────────────────

struct Ctx {
    env: Env,
    client: RecurringSavingsContractClient<'static>,
    admin: Address,
    token_id: Address,
}

impl Ctx {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        // Deploy a test SAC token.
        let token_admin = Address::generate(&env);
        let token_id = env.register_stellar_asset_contract(token_admin.clone());

        // Deploy the recurring savings contract.
        let contract_id = env.register_contract(None, RecurringSavingsContract);
        let client: RecurringSavingsContractClient =
            RecurringSavingsContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        client.initialize(&admin);

        let client: RecurringSavingsContractClient<'static> =
            unsafe { core::mem::transmute(client) };

        Self { env, client, admin, token_id }
    }

    /// Create an owner with `balance` tokens already approved for the contract.
    fn funded_owner(&self, balance: i128) -> Address {
        let owner = Address::generate(&self.env);
        let sac = token::StellarAssetClient::new(&self.env, &self.token_id);
        sac.mint(&owner, &balance);
        // Approve the contract to spend on owner's behalf.
        let token = token::Client::new(&self.env, &self.token_id);
        token.approve(
            &owner,
            &self.client.address,
            &balance,
            &(self.env.ledger().sequence() + 10_000),
        );
        owner
    }

    fn advance_secs(&self, secs: u64) {
        let ts = self.env.ledger().timestamp();
        self.env.ledger().set(LedgerInfo {
            timestamp: ts + secs,
            ..self.env.ledger().get()
        });
    }

    fn set_timestamp(&self, ts: u64) {
        self.env.ledger().set(LedgerInfo {
            timestamp: ts,
            ..self.env.ledger().get()
        });
    }

    fn balance(&self, addr: &Address) -> i128 {
        token::Client::new(&self.env, &self.token_id).balance(addr)
    }

    /// Helper: create a basic schedule and return its ID.
    fn create_basic_schedule(&self, owner: &Address, goal: &Address) -> u64 {
        self.client.create_schedule(
            owner,
            &self.token_id,
            goal,
            &AMOUNT,
            &INTERVAL,
            &0, // unlimited
        )
    }
}

// ── Initialization ────────────────────────────────────────────────────────────

#[test]
fn happy_initialize_sets_admin() {
    let ctx = Ctx::new();
    // Contract is initialized — schedule_count should be 0.
    assert_eq!(ctx.client.schedule_count(), 0);
}

#[test]
#[should_panic(expected = "AlreadyInitialized")]
fn neg_reinit_blocked() {
    let ctx = Ctx::new();
    ctx.client.initialize(&ctx.admin);
}

// ── create_schedule ───────────────────────────────────────────────────────────

#[test]
fn happy_create_schedule_returns_incrementing_ids() {
    let ctx = Ctx::new();
    ctx.set_timestamp(1_000_000);
    let owner = ctx.funded_owner(10_000);
    let goal = Address::generate(&ctx.env);

    let id0 = ctx.create_basic_schedule(&owner, &goal);
    let id1 = ctx.create_basic_schedule(&owner, &goal);
    assert_eq!(id0, 0);
    assert_eq!(id1, 1);
    assert_eq!(ctx.client.schedule_count(), 2);
}

#[test]
fn happy_create_schedule_stores_correct_fields() {
    let ctx = Ctx::new();
    ctx.set_timestamp(1_000_000);
    let owner = ctx.funded_owner(10_000);
    let goal = Address::generate(&ctx.env);

    let id = ctx.client.create_schedule(
        &owner,
        &ctx.token_id,
        &goal,
        &AMOUNT,
        &INTERVAL,
        &5,
    );

    let s = ctx.client.get_schedule(&id);
    assert_eq!(s.owner, owner);
    assert_eq!(s.token, ctx.token_id);
    assert_eq!(s.savings_goal, goal);
    assert_eq!(s.amount, AMOUNT);
    assert_eq!(s.interval_secs, INTERVAL);
    assert_eq!(s.max_executions, 5);
    assert_eq!(s.executions_completed, 0);
    assert_eq!(s.status, ScheduleStatus::Active);
    assert_eq!(s.next_execution_at, 1_000_000 + INTERVAL);
    assert_eq!(s.created_at, 1_000_000);
}

#[test]
#[should_panic(expected = "InvalidAmount")]
fn neg_create_zero_amount() {
    let ctx = Ctx::new();
    let owner = ctx.funded_owner(10_000);
    let goal = Address::generate(&ctx.env);
    ctx.client.create_schedule(&owner, &ctx.token_id, &goal, &0, &INTERVAL, &0);
}

#[test]
#[should_panic(expected = "InvalidAmount")]
fn neg_create_negative_amount() {
    let ctx = Ctx::new();
    let owner = ctx.funded_owner(10_000);
    let goal = Address::generate(&ctx.env);
    ctx.client.create_schedule(&owner, &ctx.token_id, &goal, &-1, &INTERVAL, &0);
}

#[test]
#[should_panic(expected = "InvalidInterval")]
fn neg_create_interval_below_minimum() {
    let ctx = Ctx::new();
    let owner = ctx.funded_owner(10_000);
    let goal = Address::generate(&ctx.env);
    ctx.client.create_schedule(
        &owner,
        &ctx.token_id,
        &goal,
        &AMOUNT,
        &(MIN_INTERVAL_SECS - 1),
        &0,
    );
}

#[test]
fn edge_create_interval_exactly_at_minimum() {
    let ctx = Ctx::new();
    ctx.set_timestamp(1_000_000);
    let owner = ctx.funded_owner(10_000);
    let goal = Address::generate(&ctx.env);
    // Exactly MIN_INTERVAL_SECS should succeed.
    let id = ctx.client.create_schedule(
        &owner,
        &ctx.token_id,
        &goal,
        &AMOUNT,
        &MIN_INTERVAL_SECS,
        &0,
    );
    assert_eq!(ctx.client.get_schedule(&id).interval_secs, MIN_INTERVAL_SECS);
}

#[test]
#[should_panic(expected = "InvalidGoalAddress")]
fn neg_savings_goal_same_as_owner() {
    let ctx = Ctx::new();
    let owner = ctx.funded_owner(10_000);
    // Circular self-contribution must be rejected.
    ctx.client.create_schedule(&owner, &ctx.token_id, &owner, &AMOUNT, &INTERVAL, &0);
}

// ── execute_contribution ──────────────────────────────────────────────────────

#[test]
fn happy_execute_transfers_tokens_and_advances_schedule() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let owner = ctx.funded_owner(10_000);
    let goal = Address::generate(&ctx.env);
    let id = ctx.create_basic_schedule(&owner, &goal);

    // Advance past the first interval.
    ctx.advance_secs(INTERVAL);

    let goal_balance_before = ctx.balance(&goal);
    let returned = ctx.client.execute_contribution(&id);
    assert_eq!(returned, AMOUNT);

    // Goal received the tokens.
    assert_eq!(ctx.balance(&goal) - goal_balance_before, AMOUNT);
    // Owner was debited.
    assert_eq!(ctx.balance(&owner), 10_000 - AMOUNT);

    // Schedule state updated.
    let s = ctx.client.get_schedule(&id);
    assert_eq!(s.executions_completed, 1);
    assert_eq!(s.next_execution_at, INTERVAL + INTERVAL);
    assert_eq!(s.status, ScheduleStatus::Active);
}

#[test]
fn happy_multiple_executions_over_time() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let owner = ctx.funded_owner(10_000);
    let goal = Address::generate(&ctx.env);
    let id = ctx.create_basic_schedule(&owner, &goal);

    for i in 1u64..=5 {
        ctx.set_timestamp(i * INTERVAL);
        ctx.client.execute_contribution(&id);
    }

    let s = ctx.client.get_schedule(&id);
    assert_eq!(s.executions_completed, 5);
    assert_eq!(ctx.balance(&goal), AMOUNT * 5);
    assert_eq!(ctx.balance(&owner), 10_000 - AMOUNT * 5);
}

#[test]
#[should_panic(expected = "NotDueYet")]
fn neg_execute_before_interval_elapses() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let owner = ctx.funded_owner(10_000);
    let goal = Address::generate(&ctx.env);
    let id = ctx.create_basic_schedule(&owner, &goal);

    // Only advance half the interval.
    ctx.advance_secs(INTERVAL / 2);
    ctx.client.execute_contribution(&id);
}

#[test]
fn edge_execute_exactly_at_next_execution_at() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let owner = ctx.funded_owner(10_000);
    let goal = Address::generate(&ctx.env);
    let id = ctx.create_basic_schedule(&owner, &goal);

    // Advance to exactly `next_execution_at`.
    ctx.set_timestamp(INTERVAL);
    let result = ctx.client.execute_contribution(&id);
    assert_eq!(result, AMOUNT);
}

#[test]
fn edge_late_execution_advances_from_scheduled_time_not_now() {
    // If an executor is 3 intervals late, the next due time should be
    // scheduled_time + interval, NOT now + interval. This keeps the cadence
    // anchored and prevents drift.
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let owner = ctx.funded_owner(10_000);
    let goal = Address::generate(&ctx.env);
    let id = ctx.create_basic_schedule(&owner, &goal);

    // Execute 3 intervals late.
    let late_now = INTERVAL * 4;
    ctx.set_timestamp(late_now);
    ctx.client.execute_contribution(&id);

    let s = ctx.client.get_schedule(&id);
    // next should be the originally scheduled slot + one interval, not late_now + interval.
    assert_eq!(s.next_execution_at, INTERVAL + INTERVAL);
}

#[test]
fn edge_execute_exactly_at_second_due_time() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let owner = ctx.funded_owner(10_000);
    let goal = Address::generate(&ctx.env);
    let id = ctx.create_basic_schedule(&owner, &goal);

    ctx.set_timestamp(INTERVAL);
    ctx.client.execute_contribution(&id);

    ctx.set_timestamp(INTERVAL * 2);
    let result = ctx.client.execute_contribution(&id);
    assert_eq!(result, AMOUNT);

    let s = ctx.client.get_schedule(&id);
    assert_eq!(s.executions_completed, 2);
}

// ── max_executions / exhaustion ───────────────────────────────────────────────

#[test]
fn happy_schedule_exhausts_after_max_executions() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let owner = ctx.funded_owner(10_000);
    let goal = Address::generate(&ctx.env);

    let id = ctx.client.create_schedule(
        &owner,
        &ctx.token_id,
        &goal,
        &AMOUNT,
        &INTERVAL,
        &3, // only 3 allowed
    );

    for i in 1u64..=3 {
        ctx.set_timestamp(i * INTERVAL);
        ctx.client.execute_contribution(&id);
    }

    let s = ctx.client.get_schedule(&id);
    assert_eq!(s.status, ScheduleStatus::Exhausted);
    assert_eq!(s.executions_completed, 3);
}

#[test]
#[should_panic(expected = "ScheduleInactive")]
fn neg_execute_exhausted_schedule() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let owner = ctx.funded_owner(10_000);
    let goal = Address::generate(&ctx.env);

    let id = ctx.client.create_schedule(
        &owner,
        &ctx.token_id,
        &goal,
        &AMOUNT,
        &INTERVAL,
        &1,
    );

    ctx.set_timestamp(INTERVAL);
    ctx.client.execute_contribution(&id);
    // Second attempt on exhausted schedule must fail.
    ctx.set_timestamp(INTERVAL * 2);
    ctx.client.execute_contribution(&id);
}

#[test]
fn edge_unlimited_schedule_max_zero_runs_indefinitely() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let owner = ctx.funded_owner(1_000_000);
    let goal = Address::generate(&ctx.env);

    let id = ctx.client.create_schedule(
        &owner,
        &ctx.token_id,
        &goal,
        &AMOUNT,
        &INTERVAL,
        &0, // unlimited
    );

    // Run 50 times — schedule should remain Active throughout.
    for i in 1u64..=50 {
        ctx.set_timestamp(i * INTERVAL);
        ctx.client.execute_contribution(&id);
    }

    let s = ctx.client.get_schedule(&id);
    assert_eq!(s.status, ScheduleStatus::Active);
    assert_eq!(s.executions_completed, 50);
}

#[test]
fn edge_single_execution_schedule() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let owner = ctx.funded_owner(10_000);
    let goal = Address::generate(&ctx.env);

    let id = ctx.client.create_schedule(
        &owner,
        &ctx.token_id,
        &goal,
        &AMOUNT,
        &INTERVAL,
        &1,
    );

    ctx.set_timestamp(INTERVAL);
    let paid = ctx.client.execute_contribution(&id);
    assert_eq!(paid, AMOUNT);

    let s = ctx.client.get_schedule(&id);
    assert_eq!(s.status, ScheduleStatus::Exhausted);

    let result = ctx.client.try_execute_contribution(&id);
    assert_eq!(result, Err(Ok(RecurringError::ScheduleInactive)));
}

// ── cancel_schedule ───────────────────────────────────────────────────────────

#[test]
fn happy_owner_can_cancel_active_schedule() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let owner = ctx.funded_owner(10_000);
    let goal = Address::generate(&ctx.env);
    let id = ctx.create_basic_schedule(&owner, &goal);

    ctx.client.cancel_schedule(&owner, &id);

    let s = ctx.client.get_schedule(&id);
    assert_eq!(s.status, ScheduleStatus::Cancelled);
}

#[test]
#[should_panic(expected = "ScheduleInactive")]
fn neg_execute_cancelled_schedule() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let owner = ctx.funded_owner(10_000);
    let goal = Address::generate(&ctx.env);
    let id = ctx.create_basic_schedule(&owner, &goal);

    ctx.client.cancel_schedule(&owner, &id);

    ctx.advance_secs(INTERVAL);
    ctx.client.execute_contribution(&id); // must fail
}

#[test]
#[should_panic(expected = "Unauthorized")]
fn neg_non_owner_cannot_cancel() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let owner = ctx.funded_owner(10_000);
    let goal = Address::generate(&ctx.env);
    let id = ctx.create_basic_schedule(&owner, &goal);

    let attacker = Address::generate(&ctx.env);
    ctx.client.cancel_schedule(&attacker, &id);
}

#[test]
#[should_panic(expected = "ScheduleInactive")]
fn neg_cancel_already_cancelled_schedule() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let owner = ctx.funded_owner(10_000);
    let goal = Address::generate(&ctx.env);
    let id = ctx.create_basic_schedule(&owner, &goal);

    ctx.client.cancel_schedule(&owner, &id);
    ctx.client.cancel_schedule(&owner, &id); // second cancel must fail
}

#[test]
fn happy_cancel_preserves_execution_history() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let owner = ctx.funded_owner(10_000);
    let goal = Address::generate(&ctx.env);
    let id = ctx.create_basic_schedule(&owner, &goal);

    // Execute twice, then cancel.
    for i in 1u64..=2 {
        ctx.set_timestamp(i * INTERVAL);
        ctx.client.execute_contribution(&id);
    }
    ctx.set_timestamp(INTERVAL * 3);
    ctx.client.cancel_schedule(&owner, &id);

    let s = ctx.client.get_schedule(&id);
    assert_eq!(s.executions_completed, 2, "history must be preserved after cancel");
    assert_eq!(s.status, ScheduleStatus::Cancelled);
}

// ── is_due ────────────────────────────────────────────────────────────────────

#[test]
fn happy_is_due_false_before_interval() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let owner = ctx.funded_owner(10_000);
    let goal = Address::generate(&ctx.env);
    let id = ctx.create_basic_schedule(&owner, &goal);

    assert!(!ctx.client.is_due(&id));
}

#[test]
fn happy_is_due_true_after_interval() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let owner = ctx.funded_owner(10_000);
    let goal = Address::generate(&ctx.env);
    let id = ctx.create_basic_schedule(&owner, &goal);

    ctx.advance_secs(INTERVAL);
    assert!(ctx.client.is_due(&id));
}

#[test]
fn happy_is_due_false_after_cancel() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let owner = ctx.funded_owner(10_000);
    let goal = Address::generate(&ctx.env);
    let id = ctx.create_basic_schedule(&owner, &goal);

    ctx.client.cancel_schedule(&owner, &id);
    ctx.advance_secs(INTERVAL * 10);
    assert!(!ctx.client.is_due(&id));
}

#[test]
fn happy_is_due_false_after_exhaustion() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let owner = ctx.funded_owner(10_000);
    let goal = Address::generate(&ctx.env);

    let id = ctx.client.create_schedule(
        &owner,
        &ctx.token_id,
        &goal,
        &AMOUNT,
        &INTERVAL,
        &1,
    );

    ctx.set_timestamp(INTERVAL);
    ctx.client.execute_contribution(&id);

    ctx.advance_secs(INTERVAL * 10);
    assert!(!ctx.client.is_due(&id));
}

// ── get_schedule error paths ───────────────────────────────────────────────────

#[test]
#[should_panic(expected = "ScheduleNotFound")]
fn neg_get_nonexistent_schedule() {
    let ctx = Ctx::new();
    ctx.client.get_schedule(&9999);
}

#[test]
#[should_panic(expected = "ScheduleNotFound")]
fn neg_execute_nonexistent_schedule() {
    let ctx = Ctx::new();
    ctx.client.execute_contribution(&9999);
}

// ── Multi-user isolation ──────────────────────────────────────────────────────

#[test]
fn security_schedules_are_isolated_per_user() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);

    let alice = ctx.funded_owner(10_000);
    let bob = ctx.funded_owner(10_000);
    let alice_goal = Address::generate(&ctx.env);
    let bob_goal = Address::generate(&ctx.env);

    let alice_id = ctx.create_basic_schedule(&alice, &alice_goal);
    let bob_id = ctx.create_basic_schedule(&bob, &bob_goal);

    // Advance time.
    ctx.advance_secs(INTERVAL);

    ctx.client.execute_contribution(&alice_id);
    ctx.client.execute_contribution(&bob_id);

    // Each user's goal received only their own contribution.
    assert_eq!(ctx.balance(&alice_goal), AMOUNT);
    assert_eq!(ctx.balance(&bob_goal), AMOUNT);
    // Bob cancelling her schedule doesn't affect Alice.
    ctx.client.cancel_schedule(&bob, &bob_id);
    let alice_s = ctx.client.get_schedule(&alice_id);
    assert_eq!(alice_s.status, ScheduleStatus::Active);
}

#[test]
fn security_one_user_cancel_cannot_cancel_another_schedule() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);

    let alice = ctx.funded_owner(10_000);
    let bob = ctx.funded_owner(10_000);
    let goal = Address::generate(&ctx.env);

    let alice_id = ctx.create_basic_schedule(&alice, &goal);
    let _bob_id = ctx.create_basic_schedule(&bob, &goal);

    // Bob tries to cancel Alice's schedule — must be rejected.
    let result = ctx.client.try_cancel_schedule(&bob, &alice_id);
    assert_eq!(result, Err(Ok(RecurringError::Unauthorized)));
}

// ── Auth guards ───────────────────────────────────────────────────────────────

#[test]
#[should_panic]
fn security_create_requires_auth() {
    let env = Env::default();
    // No mock_all_auths.
    let admin = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract(admin.clone());
    let contract_id = env.register_contract(None, RecurringSavingsContract);
    let client = RecurringSavingsContractClient::new(&env, &contract_id);
    {
        env.mock_all_auths();
        client.initialize(&admin);
    }
    // Auth not mocked — must panic.
    let owner = Address::generate(&env);
    let goal = Address::generate(&env);
    client.create_schedule(&owner, &token_id, &goal, &AMOUNT, &INTERVAL, &0);
}