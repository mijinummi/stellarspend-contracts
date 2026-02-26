//! Tests for contracts/conditional_payments.rs
//!
//! Run with: `cargo test --test conditional_payment_tests`
//!
//! Test taxonomy
//! ─────────────
//! happy_*     — correct flows
//! neg_*       — invalid inputs / calls that must be rejected
//! edge_*      — boundary / timing / combination corner cases
//! auth_*      — authorization guard tests
//! cond_*      — per-condition-type validation tests

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    token, vec, Address, Env, Vec,
};

use crate::conditional_payments::{
    Condition, ConditionalPaymentError, ConditionalPaymentsContract,
    ConditionalPaymentsContractClient, PaymentStatus, MAX_CONDITIONS, MAX_SIGNERS,
};

// ── Harness ───────────────────────────────────────────────────────────────────

const AMOUNT: i128 = 10_000;

struct Ctx {
    env: Env,
    client: ConditionalPaymentsContractClient<'static>,
    admin: Address,
    token_id: Address,
}

impl Ctx {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let token_admin = Address::generate(&env);
        let token_id = env.register_stellar_asset_contract(token_admin.clone());

        let contract_id = env.register_contract(None, ConditionalPaymentsContract);
        let client: ConditionalPaymentsContractClient =
            ConditionalPaymentsContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        client.initialize(&admin);

        let client: ConditionalPaymentsContractClient<'static> =
            unsafe { core::mem::transmute(client) };

        Self { env, client, admin, token_id }
    }

    /// Mint `balance` to `owner` and approve the contract to spend it.
    fn funded_payer(&self, balance: i128) -> Address {
        let owner = Address::generate(&self.env);
        token::StellarAssetClient::new(&self.env, &self.token_id).mint(&owner, &balance);
        token::Client::new(&self.env, &self.token_id).approve(
            &owner,
            &self.client.address,
            &balance,
            &(self.env.ledger().sequence() + 10_000),
        );
        owner
    }

    fn set_timestamp(&self, ts: u64) {
        self.env.ledger().set(LedgerInfo {
            timestamp: ts,
            ..self.env.ledger().get()
        });
    }

    fn advance_secs(&self, secs: u64) {
        let ts = self.env.ledger().timestamp();
        self.set_timestamp(ts + secs);
    }

    fn balance(&self, addr: &Address) -> i128 {
        token::Client::new(&self.env, &self.token_id).balance(addr)
    }

    /// Shorthand: create a payment with a single `TimeAfter` condition.
    fn create_time_after(&self, payer: &Address, recipient: &Address, after: u64) -> u64 {
        self.client.create_payment(
            payer,
            recipient,
            &self.token_id,
            &AMOUNT,
            &vec![&self.env, Condition::TimeAfter(after)],
        )
    }
}

// ── Initialization ────────────────────────────────────────────────────────────

#[test]
fn happy_initialize_sets_admin() {
    let ctx = Ctx::new();
    assert_eq!(ctx.client.payment_count(), 0);
}

#[test]
#[should_panic(expected = "AlreadyInitialized")]
fn neg_reinit_blocked() {
    let ctx = Ctx::new();
    ctx.client.initialize(&ctx.admin);
}

// ── create_payment ────────────────────────────────────────────────────────────

#[test]
fn happy_create_payment_stores_record_and_escrows_funds() {
    let ctx = Ctx::new();
    ctx.set_timestamp(1_000);
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);

    let id = ctx.create_time_after(&payer, &recipient, 2_000);

    let p = ctx.client.get_payment(&id);
    assert_eq!(p.payer, payer);
    assert_eq!(p.recipient, recipient);
    assert_eq!(p.amount, AMOUNT);
    assert_eq!(p.status, PaymentStatus::Pending);
    assert_eq!(p.created_at, 1_000);
    assert_eq!(p.settled_at, 0);

    // Funds left payer and arrived at contract.
    assert_eq!(ctx.balance(&payer), 0);
    assert_eq!(ctx.balance(&ctx.client.address), AMOUNT);
}

#[test]
fn happy_payment_ids_are_incrementing() {
    let ctx = Ctx::new();
    let payer = ctx.funded_payer(AMOUNT * 3);
    let recipient = Address::generate(&ctx.env);

    let id0 = ctx.create_time_after(&payer, &recipient, 1);
    let id1 = ctx.create_time_after(&payer, &recipient, 1);
    let id2 = ctx.create_time_after(&payer, &recipient, 1);

    assert_eq!((id0, id1, id2), (0, 1, 2));
    assert_eq!(ctx.client.payment_count(), 3);
}

#[test]
#[should_panic(expected = "InvalidAmount")]
fn neg_create_zero_amount() {
    let ctx = Ctx::new();
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);
    ctx.client.create_payment(
        &payer,
        &recipient,
        &ctx.token_id,
        &0,
        &vec![&ctx.env, Condition::TimeAfter(1)],
    );
}

#[test]
#[should_panic(expected = "InvalidAmount")]
fn neg_create_negative_amount() {
    let ctx = Ctx::new();
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);
    ctx.client.create_payment(
        &payer,
        &recipient,
        &ctx.token_id,
        &-1,
        &vec![&ctx.env, Condition::TimeAfter(1)],
    );
}

#[test]
#[should_panic(expected = "SamePayerRecipient")]
fn neg_payer_equals_recipient() {
    let ctx = Ctx::new();
    let payer = ctx.funded_payer(AMOUNT);
    ctx.client.create_payment(
        &payer,
        &payer,
        &ctx.token_id,
        &AMOUNT,
        &vec![&ctx.env, Condition::TimeAfter(1)],
    );
}

#[test]
#[should_panic(expected = "TooManyConditions")]
fn neg_too_many_conditions() {
    let ctx = Ctx::new();
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);
    let mut conds = Vec::new(&ctx.env);
    for i in 0..(MAX_CONDITIONS + 1) as u64 {
        conds.push_back(Condition::TimeAfter(i + 1));
    }
    ctx.client
        .create_payment(&payer, &recipient, &ctx.token_id, &AMOUNT, &conds);
}

#[test]
#[should_panic(expected = "TooManyConditions")]
fn neg_zero_conditions() {
    let ctx = Ctx::new();
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);
    ctx.client.create_payment(
        &payer,
        &recipient,
        &ctx.token_id,
        &AMOUNT,
        &Vec::new(&ctx.env),
    );
}

// ── cond_*: per-condition validation at creation ──────────────────────────────

#[test]
#[should_panic(expected = "InvalidCondition")]
fn cond_time_after_zero_timestamp_invalid() {
    let ctx = Ctx::new();
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);
    ctx.client.create_payment(
        &payer,
        &recipient,
        &ctx.token_id,
        &AMOUNT,
        &vec![&ctx.env, Condition::TimeAfter(0)],
    );
}

#[test]
#[should_panic(expected = "InvalidCondition")]
fn cond_time_before_zero_timestamp_invalid() {
    let ctx = Ctx::new();
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);
    ctx.client.create_payment(
        &payer,
        &recipient,
        &ctx.token_id,
        &AMOUNT,
        &vec![&ctx.env, Condition::TimeBefore(0)],
    );
}

#[test]
#[should_panic(expected = "InvalidCondition")]
fn cond_balance_above_zero_threshold_invalid() {
    let ctx = Ctx::new();
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);
    let watch = Address::generate(&ctx.env);
    ctx.client.create_payment(
        &payer,
        &recipient,
        &ctx.token_id,
        &AMOUNT,
        &vec![
            &ctx.env,
            Condition::BalanceAbove {
                token: ctx.token_id.clone(),
                watch_address: watch,
                threshold: 0,
            },
        ],
    );
}

#[test]
#[should_panic(expected = "TooManySigners")]
fn cond_multisig_no_signers_invalid() {
    let ctx = Ctx::new();
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);
    ctx.client.create_payment(
        &payer,
        &recipient,
        &ctx.token_id,
        &AMOUNT,
        &vec![
            &ctx.env,
            Condition::MultiSigApproved {
                signers: Vec::new(&ctx.env),
                required: 1,
            },
        ],
    );
}

#[test]
#[should_panic(expected = "InvalidThreshold")]
fn cond_multisig_required_exceeds_signers() {
    let ctx = Ctx::new();
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);
    let s1 = Address::generate(&ctx.env);
    ctx.client.create_payment(
        &payer,
        &recipient,
        &ctx.token_id,
        &AMOUNT,
        &vec![
            &ctx.env,
            Condition::MultiSigApproved {
                signers: vec![&ctx.env, s1],
                required: 2, // only 1 signer but requires 2
            },
        ],
    );
}

#[test]
#[should_panic(expected = "InvalidThreshold")]
fn cond_multisig_required_zero_invalid() {
    let ctx = Ctx::new();
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);
    let s1 = Address::generate(&ctx.env);
    ctx.client.create_payment(
        &payer,
        &recipient,
        &ctx.token_id,
        &AMOUNT,
        &vec![
            &ctx.env,
            Condition::MultiSigApproved {
                signers: vec![&ctx.env, s1],
                required: 0,
            },
        ],
    );
}

// ── cond_*: TimeAfter execution ───────────────────────────────────────────────

#[test]
fn cond_time_after_executes_when_due() {
    let ctx = Ctx::new();
    ctx.set_timestamp(500);
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);
    let id = ctx.create_time_after(&payer, &recipient, 1_000);

    ctx.set_timestamp(1_000);
    let paid = ctx.client.execute_payment(&id);
    assert_eq!(paid, AMOUNT);
    assert_eq!(ctx.balance(&recipient), AMOUNT);
}

#[test]
fn cond_time_after_blocks_before_threshold() {
    let ctx = Ctx::new();
    ctx.set_timestamp(500);
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);
    let id = ctx.create_time_after(&payer, &recipient, 1_000);

    ctx.set_timestamp(999);
    let result = ctx.client.try_execute_payment(&id);
    assert_eq!(result, Err(Ok(ConditionalPaymentError::ConditionNotMet)));
}

#[test]
fn cond_time_after_exactly_at_threshold_passes() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);
    let id = ctx.create_time_after(&payer, &recipient, 1_000);

    ctx.set_timestamp(1_000); // exactly at threshold
    let paid = ctx.client.execute_payment(&id);
    assert_eq!(paid, AMOUNT);
}

// ── cond_*: TimeBefore execution ──────────────────────────────────────────────

#[test]
fn cond_time_before_executes_before_deadline() {
    let ctx = Ctx::new();
    ctx.set_timestamp(500);
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);

    let id = ctx.client.create_payment(
        &payer,
        &recipient,
        &ctx.token_id,
        &AMOUNT,
        &vec![&ctx.env, Condition::TimeBefore(1_000)],
    );

    ctx.set_timestamp(999);
    let paid = ctx.client.execute_payment(&id);
    assert_eq!(paid, AMOUNT);
}

#[test]
fn cond_time_before_blocks_at_or_after_deadline() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);

    let id = ctx.client.create_payment(
        &payer,
        &recipient,
        &ctx.token_id,
        &AMOUNT,
        &vec![&ctx.env, Condition::TimeBefore(1_000)],
    );

    // At exactly the deadline — condition should FAIL (now < threshold is false).
    ctx.set_timestamp(1_000);
    let result = ctx.client.try_execute_payment(&id);
    assert_eq!(result, Err(Ok(ConditionalPaymentError::ConditionNotMet)));
}

// ── cond_*: BalanceAbove execution ────────────────────────────────────────────

#[test]
fn cond_balance_above_executes_when_balance_sufficient() {
    let ctx = Ctx::new();
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);
    let watch = Address::generate(&ctx.env);

    // Fund the watched address.
    token::StellarAssetClient::new(&ctx.env, &ctx.token_id).mint(&watch, &5_000);

    let id = ctx.client.create_payment(
        &payer,
        &recipient,
        &ctx.token_id,
        &AMOUNT,
        &vec![
            &ctx.env,
            Condition::BalanceAbove {
                token: ctx.token_id.clone(),
                watch_address: watch.clone(),
                threshold: 5_000,
            },
        ],
    );

    let paid = ctx.client.execute_payment(&id);
    assert_eq!(paid, AMOUNT);
}

#[test]
fn cond_balance_above_blocks_when_balance_insufficient() {
    let ctx = Ctx::new();
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);
    let watch = Address::generate(&ctx.env);

    // watched address has 4_999 — below threshold of 5_000.
    token::StellarAssetClient::new(&ctx.env, &ctx.token_id).mint(&watch, &4_999);

    let id = ctx.client.create_payment(
        &payer,
        &recipient,
        &ctx.token_id,
        &AMOUNT,
        &vec![
            &ctx.env,
            Condition::BalanceAbove {
                token: ctx.token_id.clone(),
                watch_address: watch,
                threshold: 5_000,
            },
        ],
    );

    let result = ctx.client.try_execute_payment(&id);
    assert_eq!(result, Err(Ok(ConditionalPaymentError::ConditionNotMet)));
}

// ── cond_*: BalanceBelow execution ────────────────────────────────────────────

#[test]
fn cond_balance_below_executes_when_balance_low() {
    let ctx = Ctx::new();
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);
    let watch = Address::generate(&ctx.env); // zero balance

    let id = ctx.client.create_payment(
        &payer,
        &recipient,
        &ctx.token_id,
        &AMOUNT,
        &vec![
            &ctx.env,
            Condition::BalanceBelow {
                token: ctx.token_id.clone(),
                watch_address: watch,
                threshold: 1_000,
            },
        ],
    );

    let paid = ctx.client.execute_payment(&id);
    assert_eq!(paid, AMOUNT);
}

#[test]
fn cond_balance_below_blocks_when_balance_high() {
    let ctx = Ctx::new();
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);
    let watch = Address::generate(&ctx.env);

    token::StellarAssetClient::new(&ctx.env, &ctx.token_id).mint(&watch, &2_000);

    let id = ctx.client.create_payment(
        &payer,
        &recipient,
        &ctx.token_id,
        &AMOUNT,
        &vec![
            &ctx.env,
            Condition::BalanceBelow {
                token: ctx.token_id.clone(),
                watch_address: watch,
                threshold: 1_000, // watch has 2_000 >= 1_000, so "below" fails
            },
        ],
    );

    let result = ctx.client.try_execute_payment(&id);
    assert_eq!(result, Err(Ok(ConditionalPaymentError::ConditionNotMet)));
}

// ── cond_*: MultiSig execution ────────────────────────────────────────────────

#[test]
fn cond_multisig_executes_when_threshold_met() {
    let ctx = Ctx::new();
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);
    let s1 = Address::generate(&ctx.env);
    let s2 = Address::generate(&ctx.env);
    let s3 = Address::generate(&ctx.env);

    let id = ctx.client.create_payment(
        &payer,
        &recipient,
        &ctx.token_id,
        &AMOUNT,
        &vec![
            &ctx.env,
            Condition::MultiSigApproved {
                signers: vec![&ctx.env, s1.clone(), s2.clone(), s3.clone()],
                required: 2,
            },
        ],
    );

    // Only 2-of-3 needed.
    ctx.client.approve(&s1, &id);
    ctx.client.approve(&s2, &id);

    let paid = ctx.client.execute_payment(&id);
    assert_eq!(paid, AMOUNT);
}

#[test]
fn cond_multisig_blocks_when_insufficient_approvals() {
    let ctx = Ctx::new();
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);
    let s1 = Address::generate(&ctx.env);
    let s2 = Address::generate(&ctx.env);

    let id = ctx.client.create_payment(
        &payer,
        &recipient,
        &ctx.token_id,
        &AMOUNT,
        &vec![
            &ctx.env,
            Condition::MultiSigApproved {
                signers: vec![&ctx.env, s1.clone(), s2.clone()],
                required: 2,
            },
        ],
    );

    // Only 1 of 2 approvals.
    ctx.client.approve(&s1, &id);

    let result = ctx.client.try_execute_payment(&id);
    assert_eq!(result, Err(Ok(ConditionalPaymentError::ConditionNotMet)));
}

#[test]
fn cond_multisig_approval_count_tracks_correctly() {
    let ctx = Ctx::new();
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);
    let s1 = Address::generate(&ctx.env);
    let s2 = Address::generate(&ctx.env);

    let id = ctx.client.create_payment(
        &payer,
        &recipient,
        &ctx.token_id,
        &AMOUNT,
        &vec![
            &ctx.env,
            Condition::MultiSigApproved {
                signers: vec![&ctx.env, s1.clone(), s2.clone()],
                required: 2,
            },
        ],
    );

    assert_eq!(ctx.client.approval_count(&id, &0), 0);
    ctx.client.approve(&s1, &id);
    assert_eq!(ctx.client.approval_count(&id, &0), 1);
    ctx.client.approve(&s2, &id);
    assert_eq!(ctx.client.approval_count(&id, &0), 2);
}

#[test]
#[should_panic(expected = "NotASigner")]
fn auth_non_signer_cannot_approve() {
    let ctx = Ctx::new();
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);
    let s1 = Address::generate(&ctx.env);
    let interloper = Address::generate(&ctx.env);

    let id = ctx.client.create_payment(
        &payer,
        &recipient,
        &ctx.token_id,
        &AMOUNT,
        &vec![
            &ctx.env,
            Condition::MultiSigApproved {
                signers: vec![&ctx.env, s1],
                required: 1,
            },
        ],
    );

    ctx.client.approve(&interloper, &id);
}

#[test]
#[should_panic(expected = "AlreadyApproved")]
fn neg_duplicate_approval_rejected() {
    let ctx = Ctx::new();
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);
    let s1 = Address::generate(&ctx.env);

    let id = ctx.client.create_payment(
        &payer,
        &recipient,
        &ctx.token_id,
        &AMOUNT,
        &vec![
            &ctx.env,
            Condition::MultiSigApproved {
                signers: vec![&ctx.env, s1.clone()],
                required: 1,
            },
        ],
    );

    ctx.client.approve(&s1, &id);
    ctx.client.approve(&s1, &id); // must fail
}

// ── execute_payment: state transitions ───────────────────────────────────────

#[test]
fn happy_execution_marks_payment_executed() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);
    let id = ctx.create_time_after(&payer, &recipient, 0);

    ctx.set_timestamp(1);
    ctx.client.execute_payment(&id);

    let p = ctx.client.get_payment(&id);
    assert_eq!(p.status, PaymentStatus::Executed);
    assert!(p.settled_at > 0);
}

#[test]
#[should_panic(expected = "PaymentNotPending")]
fn neg_execute_already_executed_payment() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);
    let id = ctx.create_time_after(&payer, &recipient, 1);

    ctx.set_timestamp(1);
    ctx.client.execute_payment(&id);
    ctx.client.execute_payment(&id); // second attempt must fail
}

#[test]
#[should_panic(expected = "PaymentNotPending")]
fn neg_execute_cancelled_payment() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);
    let id = ctx.create_time_after(&payer, &recipient, 1_000);

    ctx.client.cancel_payment(&payer, &id);

    ctx.set_timestamp(1_000);
    ctx.client.execute_payment(&id);
}

// ── cancel_payment ────────────────────────────────────────────────────────────

#[test]
fn happy_payer_can_cancel_and_funds_returned() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);

    assert_eq!(ctx.balance(&payer), 0); // funds in escrow after create
    let id = ctx.create_time_after(&payer, &recipient, 1_000);

    ctx.client.cancel_payment(&payer, &id);

    // Funds returned.
    assert_eq!(ctx.balance(&payer), AMOUNT);
    assert_eq!(ctx.balance(&ctx.client.address), 0);

    let p = ctx.client.get_payment(&id);
    assert_eq!(p.status, PaymentStatus::Cancelled);
    assert!(p.settled_at > 0);
}

#[test]
#[should_panic(expected = "Unauthorized")]
fn auth_non_payer_cannot_cancel() {
    let ctx = Ctx::new();
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);
    let id = ctx.create_time_after(&payer, &recipient, 1_000);

    let attacker = Address::generate(&ctx.env);
    ctx.client.cancel_payment(&attacker, &id);
}

#[test]
#[should_panic(expected = "PaymentNotPending")]
fn neg_cancel_executed_payment() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);
    let id = ctx.create_time_after(&payer, &recipient, 1);

    ctx.set_timestamp(1);
    ctx.client.execute_payment(&id);
    ctx.client.cancel_payment(&payer, &id); // must fail
}

#[test]
#[should_panic(expected = "PaymentNotPending")]
fn neg_cancel_twice() {
    let ctx = Ctx::new();
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);
    let id = ctx.create_time_after(&payer, &recipient, 1_000);

    ctx.client.cancel_payment(&payer, &id);
    ctx.client.cancel_payment(&payer, &id); // must fail
}

// ── check_conditions dry-run ──────────────────────────────────────────────────

#[test]
fn happy_check_conditions_returns_per_condition_results() {
    let ctx = Ctx::new();
    ctx.set_timestamp(500);
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);

    // Two conditions: TimeAfter(1000) [fails now] and TimeBefore(2000) [passes now].
    let id = ctx.client.create_payment(
        &payer,
        &recipient,
        &ctx.token_id,
        &AMOUNT,
        &vec![
            &ctx.env,
            Condition::TimeAfter(1_000),
            Condition::TimeBefore(2_000),
        ],
    );

    let results = ctx.client.check_conditions(&id);
    assert_eq!(results.get(0), Some(false)); // TimeAfter(1000) not yet
    assert_eq!(results.get(1), Some(true));  // TimeBefore(2000) passes

    // Advance past TimeAfter threshold.
    ctx.set_timestamp(1_000);
    let results2 = ctx.client.check_conditions(&id);
    assert_eq!(results2.get(0), Some(true));
    assert_eq!(results2.get(1), Some(true));
}

// ── edge_*: compound AND conditions ──────────────────────────────────────────

#[test]
fn edge_all_conditions_must_pass_for_execution() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);
    let s1 = Address::generate(&ctx.env);

    // TimeAfter(1000) AND MultiSig(1-of-1).
    let id = ctx.client.create_payment(
        &payer,
        &recipient,
        &ctx.token_id,
        &AMOUNT,
        &vec![
            &ctx.env,
            Condition::TimeAfter(1_000),
            Condition::MultiSigApproved {
                signers: vec![&ctx.env, s1.clone()],
                required: 1,
            },
        ],
    );

    // Advance time but don't approve — should fail.
    ctx.set_timestamp(1_000);
    let result = ctx.client.try_execute_payment(&id);
    assert_eq!(result, Err(Ok(ConditionalPaymentError::ConditionNotMet)));

    // Approve but reset time — should fail.
    ctx.set_timestamp(0);
    ctx.client.approve(&s1, &id);
    let result2 = ctx.client.try_execute_payment(&id);
    assert_eq!(result2, Err(Ok(ConditionalPaymentError::ConditionNotMet)));

    // Both satisfied — should execute.
    ctx.set_timestamp(1_000);
    let paid = ctx.client.execute_payment(&id);
    assert_eq!(paid, AMOUNT);
}

#[test]
fn edge_time_window_both_after_and_before() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);

    // Only executable in window [1000, 2000).
    let id = ctx.client.create_payment(
        &payer,
        &recipient,
        &ctx.token_id,
        &AMOUNT,
        &vec![
            &ctx.env,
            Condition::TimeAfter(1_000),
            Condition::TimeBefore(2_000),
        ],
    );

    // Before window.
    ctx.set_timestamp(999);
    assert_eq!(
        ctx.client.try_execute_payment(&id),
        Err(Ok(ConditionalPaymentError::ConditionNotMet))
    );

    // Inside window.
    ctx.set_timestamp(1_500);
    let paid = ctx.client.execute_payment(&id);
    assert_eq!(paid, AMOUNT);
}

#[test]
fn edge_time_window_expired_before_execution() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let payer = ctx.funded_payer(AMOUNT * 2);
    let recipient = Address::generate(&ctx.env);

    let id = ctx.client.create_payment(
        &payer,
        &recipient,
        &ctx.token_id,
        &AMOUNT,
        &vec![
            &ctx.env,
            Condition::TimeAfter(1_000),
            Condition::TimeBefore(2_000),
        ],
    );

    // Past the window — TimeBefore fails.
    ctx.set_timestamp(2_001);
    let result = ctx.client.try_execute_payment(&id);
    assert_eq!(result, Err(Ok(ConditionalPaymentError::ConditionNotMet)));

    // Payer can still cancel and recover funds.
    ctx.client.cancel_payment(&payer, &id);
    assert_eq!(ctx.balance(&payer), AMOUNT); // got back the one payment (other was already spent at funded_payer)
}

#[test]
fn edge_max_conditions_boundary_is_accepted() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);

    let mut conds = Vec::new(&ctx.env);
    for i in 1..=(MAX_CONDITIONS as u64) {
        conds.push_back(Condition::TimeAfter(i));
    }
    // Should not panic.
    let id = ctx.client.create_payment(
        &payer,
        &recipient,
        &ctx.token_id,
        &AMOUNT,
        &conds,
    );

    ctx.set_timestamp(MAX_CONDITIONS as u64);
    let paid = ctx.client.execute_payment(&id);
    assert_eq!(paid, AMOUNT);
}

#[test]
fn edge_multisig_1_of_n_any_signer_suffices() {
    let ctx = Ctx::new();
    let payer = ctx.funded_payer(AMOUNT * 3);
    let recipient = Address::generate(&ctx.env);
    let signers: Vec<Address> = (0..5).map(|_| Address::generate(&ctx.env)).collect();

    for (i, signer) in signers.iter().enumerate() {
        // Fresh payment per signer.
        let mut signer_vec = Vec::new(&ctx.env);
        for s in signers.iter() {
            signer_vec.push_back(s.clone());
        }
        let id = ctx.client.create_payment(
            &payer,
            &recipient,
            &ctx.token_id,
            &1, // small amount per test
            &vec![
                &ctx.env,
                Condition::MultiSigApproved {
                    signers: signer_vec,
                    required: 1,
                },
            ],
        );
        // Any single signer should unlock it.
        ctx.client.approve(signer, &id);
        ctx.client.execute_payment(&id);
    }
}

#[test]
fn edge_approve_on_cancelled_payment_rejected() {
    let ctx = Ctx::new();
    let payer = ctx.funded_payer(AMOUNT);
    let recipient = Address::generate(&ctx.env);
    let s1 = Address::generate(&ctx.env);

    let id = ctx.client.create_payment(
        &payer,
        &recipient,
        &ctx.token_id,
        &AMOUNT,
        &vec![
            &ctx.env,
            Condition::MultiSigApproved {
                signers: vec![&ctx.env, s1.clone()],
                required: 1,
            },
        ],
    );

    ctx.client.cancel_payment(&payer, &id);

    let result = ctx.client.try_approve(&s1, &id);
    assert_eq!(result, Err(Ok(ConditionalPaymentError::PaymentNotPending)));
}

// ── auth_*: require_auth guards ───────────────────────────────────────────────

#[test]
#[should_panic]
fn auth_create_payment_requires_auth() {
    let env = Env::default();
    // No mock_all_auths.
    let admin = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract(admin.clone());
    let contract_id = env.register_contract(None, ConditionalPaymentsContract);
    let client = ConditionalPaymentsContractClient::new(&env, &contract_id);
    {
        env.mock_all_auths();
        client.initialize(&admin);
    }
    let payer = Address::generate(&env);
    let recipient = Address::generate(&env);
    // No auth mocked — must panic.
    client.create_payment(
        &payer,
        &recipient,
        &token_id,
        &AMOUNT,
        &vec![&env, Condition::TimeAfter(1)],
    );
}

#[test]
#[should_panic]
fn auth_cancel_payment_requires_auth() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract(admin.clone());
    let contract_id = env.register_contract(None, ConditionalPaymentsContract);
    let client = ConditionalPaymentsContractClient::new(&env, &contract_id);
    let payer;
    let id;
    {
        env.mock_all_auths();
        client.initialize(&admin);
        payer = Address::generate(&env);
        token::StellarAssetClient::new(&env, &token_id).mint(&payer, &AMOUNT);
        token::Client::new(&env, &token_id).approve(
            &payer,
            &client.address,
            &AMOUNT,
            &(env.ledger().sequence() + 10_000),
        );
        id = client.create_payment(
            &payer,
            &Address::generate(&env),
            &token_id,
            &AMOUNT,
            &vec![&env, Condition::TimeAfter(1_000)],
        );
    }
    // No auth mocked for cancel — must panic.
    client.cancel_payment(&payer, &id);
}

#[test]
#[should_panic(expected = "PaymentNotFound")]
fn neg_get_nonexistent_payment() {
    let ctx = Ctx::new();
    ctx.client.get_payment(&9999);
}