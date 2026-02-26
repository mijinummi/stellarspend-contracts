//! Tests for contracts/account_status.rs
//!
//! Run with: `cargo test --test account_status_tests`
//!
//! Test taxonomy
//! ─────────────
//! happy_*       — correct flows
//! neg_*         — inputs that must be rejected
//! edge_*        — boundary / timing corner cases
//! auth_*        — authorization and privilege separation tests

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Address, Env, String,
};

use crate::account_status::{
    AccountStatusContract, AccountStatusContractClient, AccountStatusError, MAX_REASON_LEN,
};

// ── Helpers ───────────────────────────────────────────────────────────────────

const REASON: &str = "Suspicious activity detected";

struct Ctx {
    env: Env,
    client: AccountStatusContractClient<'static>,
    super_admin: Address,
}

impl Ctx {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, AccountStatusContract);
        let client: AccountStatusContractClient =
            AccountStatusContractClient::new(&env, &contract_id);

        let super_admin = Address::generate(&env);
        client.initialize(&super_admin);

        let client: AccountStatusContractClient<'static> =
            unsafe { core::mem::transmute(client) };

        Self { env, client, super_admin }
    }

    fn reason(&self) -> String {
        String::from_str(&self.env, REASON)
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

    /// Create a new admin and register them via add_admin.
    fn make_admin(&self) -> Address {
        let admin = Address::generate(&self.env);
        self.client.add_admin(&self.super_admin, &admin);
        admin
    }

    /// Freeze `target` with no expiry using the super-admin.
    fn freeze(&self, target: &Address) {
        self.client
            .freeze_account(&self.super_admin, target, &self.reason(), &0);
    }
}

// ── Initialization ────────────────────────────────────────────────────────────

#[test]
fn happy_initialize_sets_super_admin() {
    let ctx = Ctx::new();
    assert!(ctx.client.is_admin(&ctx.super_admin));
}

#[test]
fn happy_initialize_empty_admin_list() {
    let ctx = Ctx::new();
    assert_eq!(ctx.client.get_admins().len(), 0);
}

#[test]
#[should_panic(expected = "AlreadyInitialized")]
fn neg_reinit_blocked() {
    let ctx = Ctx::new();
    ctx.client.initialize(&ctx.super_admin);
}

// ── Admin management ──────────────────────────────────────────────────────────

#[test]
fn happy_super_admin_can_add_admin() {
    let ctx = Ctx::new();
    let admin = Address::generate(&ctx.env);
    ctx.client.add_admin(&ctx.super_admin, &admin);
    assert!(ctx.client.is_admin(&admin));
    assert_eq!(ctx.client.get_admins().len(), 1);
}

#[test]
fn happy_super_admin_can_remove_admin() {
    let ctx = Ctx::new();
    let admin = ctx.make_admin();
    ctx.client.remove_admin(&ctx.super_admin, &admin);
    assert!(!ctx.client.is_admin(&admin));
    assert_eq!(ctx.client.get_admins().len(), 0);
}

#[test]
fn happy_multiple_admins_can_be_added() {
    let ctx = Ctx::new();
    let a1 = ctx.make_admin();
    let a2 = ctx.make_admin();
    let a3 = ctx.make_admin();
    assert!(ctx.client.is_admin(&a1));
    assert!(ctx.client.is_admin(&a2));
    assert!(ctx.client.is_admin(&a3));
    assert_eq!(ctx.client.get_admins().len(), 3);
}

#[test]
#[should_panic(expected = "AlreadyAdmin")]
fn neg_duplicate_admin_rejected() {
    let ctx = Ctx::new();
    let admin = ctx.make_admin();
    ctx.client.add_admin(&ctx.super_admin, &admin);
}

#[test]
#[should_panic(expected = "AdminNotFound")]
fn neg_remove_nonexistent_admin() {
    let ctx = Ctx::new();
    let random = Address::generate(&ctx.env);
    ctx.client.remove_admin(&ctx.super_admin, &random);
}

// ── auth: admin management privilege separation ───────────────────────────────

#[test]
#[should_panic(expected = "Unauthorized")]
fn auth_ordinary_admin_cannot_add_admin() {
    let ctx = Ctx::new();
    let admin = ctx.make_admin();
    let new_admin = Address::generate(&ctx.env);
    // Ordinary admin tries to grant admin rights — must be rejected.
    ctx.client.add_admin(&admin, &new_admin);
}

#[test]
#[should_panic(expected = "Unauthorized")]
fn auth_ordinary_admin_cannot_remove_admin() {
    let ctx = Ctx::new();
    let admin1 = ctx.make_admin();
    let admin2 = ctx.make_admin();
    ctx.client.remove_admin(&admin1, &admin2);
}

#[test]
#[should_panic(expected = "Unauthorized")]
fn auth_non_admin_cannot_add_admin() {
    let ctx = Ctx::new();
    let attacker = Address::generate(&ctx.env);
    let victim = Address::generate(&ctx.env);
    ctx.client.add_admin(&attacker, &victim);
}

// ── freeze_account ────────────────────────────────────────────────────────────

#[test]
fn happy_super_admin_can_freeze_account() {
    let ctx = Ctx::new();
    let target = Address::generate(&ctx.env);
    ctx.freeze(&target);
    assert!(ctx.client.is_frozen(&target));
}

#[test]
fn happy_ordinary_admin_can_freeze_account() {
    let ctx = Ctx::new();
    let admin = ctx.make_admin();
    let target = Address::generate(&ctx.env);
    ctx.client
        .freeze_account(&admin, &target, &ctx.reason(), &0);
    assert!(ctx.client.is_frozen(&target));
}

#[test]
fn happy_freeze_stores_full_record() {
    let ctx = Ctx::new();
    ctx.set_timestamp(5_000);
    let target = Address::generate(&ctx.env);
    ctx.client.freeze_account(
        &ctx.super_admin,
        &target,
        &ctx.reason(),
        &10_000,
    );

    let record = ctx.client.get_status(&target);
    assert!(record.frozen);
    assert_eq!(record.frozen_by, Some(ctx.super_admin.clone()));
    assert_eq!(record.frozen_at, 5_000);
    assert_eq!(record.expires_at, 10_000);
    assert_eq!(record.freeze_count, 1);
}

#[test]
fn happy_freeze_increments_global_count() {
    let ctx = Ctx::new();
    let t1 = Address::generate(&ctx.env);
    let t2 = Address::generate(&ctx.env);
    ctx.freeze(&t1);
    ctx.freeze(&t2);
    assert_eq!(ctx.client.total_freeze_count(), 2);
}

#[test]
fn happy_freeze_count_per_account_increments_across_freeze_cycles() {
    let ctx = Ctx::new();
    let target = Address::generate(&ctx.env);

    ctx.freeze(&target);
    ctx.client.unfreeze_account(&ctx.super_admin, &target);
    ctx.freeze(&target);

    let record = ctx.client.get_status(&target);
    assert_eq!(record.freeze_count, 2);
}

// ── auth: freeze authorization ────────────────────────────────────────────────

#[test]
#[should_panic(expected = "Unauthorized")]
fn auth_non_admin_cannot_freeze() {
    let ctx = Ctx::new();
    let attacker = Address::generate(&ctx.env);
    let target = Address::generate(&ctx.env);
    ctx.client
        .freeze_account(&attacker, &target, &ctx.reason(), &0);
}

#[test]
#[should_panic(expected = "CannotFreezeSelf")]
fn auth_admin_cannot_freeze_self() {
    let ctx = Ctx::new();
    let admin = ctx.make_admin();
    ctx.client
        .freeze_account(&admin, &admin, &ctx.reason(), &0);
}

#[test]
#[should_panic(expected = "CannotFreezeSuperAdmin")]
fn auth_admin_cannot_freeze_super_admin() {
    let ctx = Ctx::new();
    let admin = ctx.make_admin();
    ctx.client
        .freeze_account(&admin, &ctx.super_admin, &ctx.reason(), &0);
}

#[test]
#[should_panic(expected = "CannotFreezeSuperAdmin")]
fn auth_super_admin_cannot_freeze_themselves_via_super_admin_guard() {
    // Even the super-admin cannot use freeze on their own address — would
    // lock the contract permanently.
    let ctx = Ctx::new();
    ctx.client.freeze_account(
        &ctx.super_admin,
        &ctx.super_admin,
        &ctx.reason(),
        &0,
    );
}

#[test]
#[should_panic(expected = "AlreadyFrozen")]
fn neg_freeze_already_frozen_account() {
    let ctx = Ctx::new();
    let target = Address::generate(&ctx.env);
    ctx.freeze(&target);
    ctx.freeze(&target); // second freeze must fail
}

#[test]
#[should_panic(expected = "ReasonTooLong")]
fn neg_reason_exceeds_max_length() {
    let ctx = Ctx::new();
    let target = Address::generate(&ctx.env);
    // Build a reason string longer than MAX_REASON_LEN bytes.
    let long: soroban_sdk::String = {
        let s: std::string::String = "x".repeat(MAX_REASON_LEN + 1);
        soroban_sdk::String::from_str(&ctx.env, &s)
    };
    ctx.client
        .freeze_account(&ctx.super_admin, &target, &long, &0);
}

#[test]
#[should_panic(expected = "InvalidExpiry")]
fn neg_expiry_in_the_past() {
    let ctx = Ctx::new();
    ctx.set_timestamp(10_000);
    let target = Address::generate(&ctx.env);
    // expires_at is before current timestamp.
    ctx.client
        .freeze_account(&ctx.super_admin, &target, &ctx.reason(), &5_000);
}

#[test]
#[should_panic(expected = "InvalidExpiry")]
fn neg_expiry_equal_to_current_timestamp() {
    let ctx = Ctx::new();
    ctx.set_timestamp(10_000);
    let target = Address::generate(&ctx.env);
    ctx.client
        .freeze_account(&ctx.super_admin, &target, &ctx.reason(), &10_000);
}

// ── unfreeze_account ──────────────────────────────────────────────────────────

#[test]
fn happy_super_admin_can_unfreeze() {
    let ctx = Ctx::new();
    let target = Address::generate(&ctx.env);
    ctx.freeze(&target);
    ctx.client.unfreeze_account(&ctx.super_admin, &target);
    assert!(!ctx.client.is_frozen(&target));
}

#[test]
fn happy_ordinary_admin_can_unfreeze() {
    let ctx = Ctx::new();
    let admin = ctx.make_admin();
    let target = Address::generate(&ctx.env);
    ctx.freeze(&target);
    ctx.client.unfreeze_account(&admin, &target);
    assert!(!ctx.client.is_frozen(&target));
}

#[test]
fn happy_unfreeze_clears_expires_at() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let target = Address::generate(&ctx.env);
    ctx.client
        .freeze_account(&ctx.super_admin, &target, &ctx.reason(), &50_000);
    ctx.client.unfreeze_account(&ctx.super_admin, &target);

    let record = ctx.client.get_status(&target);
    assert!(!record.frozen);
    assert_eq!(record.expires_at, 0);
}

#[test]
#[should_panic(expected = "NotFrozen")]
fn neg_unfreeze_account_that_is_not_frozen() {
    let ctx = Ctx::new();
    let target = Address::generate(&ctx.env);
    ctx.client.unfreeze_account(&ctx.super_admin, &target);
}

#[test]
#[should_panic(expected = "Unauthorized")]
fn auth_non_admin_cannot_unfreeze() {
    let ctx = Ctx::new();
    let attacker = Address::generate(&ctx.env);
    let target = Address::generate(&ctx.env);
    ctx.freeze(&target);
    ctx.client.unfreeze_account(&attacker, &target);
}

#[test]
fn happy_account_can_be_refrozen_after_unfreeze() {
    let ctx = Ctx::new();
    let target = Address::generate(&ctx.env);
    ctx.freeze(&target);
    ctx.client.unfreeze_account(&ctx.super_admin, &target);
    ctx.freeze(&target);
    assert!(ctx.client.is_frozen(&target));
}

// ── assert_not_frozen ─────────────────────────────────────────────────────────

#[test]
fn happy_assert_not_frozen_passes_for_clean_account() {
    let ctx = Ctx::new();
    let account = Address::generate(&ctx.env);
    // Should not panic.
    ctx.client.assert_not_frozen(&account);
}

#[test]
#[should_panic(expected = "AccountFrozen")]
fn neg_assert_not_frozen_panics_for_frozen_account() {
    let ctx = Ctx::new();
    let account = Address::generate(&ctx.env);
    ctx.freeze(&account);
    ctx.client.assert_not_frozen(&account);
}

#[test]
fn happy_assert_not_frozen_passes_after_unfreeze() {
    let ctx = Ctx::new();
    let account = Address::generate(&ctx.env);
    ctx.freeze(&account);
    ctx.client.unfreeze_account(&ctx.super_admin, &account);
    // Should not panic.
    ctx.client.assert_not_frozen(&account);
}

// ── Expiry / time-based freeze ────────────────────────────────────────────────

#[test]
fn edge_frozen_account_is_blocked_before_expiry() {
    let ctx = Ctx::new();
    ctx.set_timestamp(1_000);
    let target = Address::generate(&ctx.env);
    // Freeze with expiry at t=2000.
    ctx.client
        .freeze_account(&ctx.super_admin, &target, &ctx.reason(), &2_000);

    // At t=1999 — still frozen.
    ctx.set_timestamp(1_999);
    assert!(ctx.client.is_frozen(&target));

    let result = ctx.client.try_assert_not_frozen(&target);
    assert_eq!(result, Err(Ok(AccountStatusError::AccountFrozen)));
}

#[test]
fn edge_frozen_account_auto_expires_at_expiry_timestamp() {
    let ctx = Ctx::new();
    ctx.set_timestamp(1_000);
    let target = Address::generate(&ctx.env);
    ctx.client
        .freeze_account(&ctx.super_admin, &target, &ctx.reason(), &2_000);

    // At exactly t=2000 the freeze is expired (timestamp >= expires_at).
    ctx.set_timestamp(2_000);
    assert!(!ctx.client.is_frozen(&target));
    ctx.client.assert_not_frozen(&target); // should not panic
}

#[test]
fn edge_expired_freeze_allows_re_freeze() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let target = Address::generate(&ctx.env);
    ctx.client
        .freeze_account(&ctx.super_admin, &target, &ctx.reason(), &1_000);

    // Let freeze expire.
    ctx.set_timestamp(1_000);
    assert!(!ctx.client.is_frozen(&target));

    // Should be able to freeze again without error.
    ctx.client
        .freeze_account(&ctx.super_admin, &target, &ctx.reason(), &0);
    assert!(ctx.client.is_frozen(&target));
}

#[test]
fn edge_unfreeze_expired_freeze_returns_not_frozen_error() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let target = Address::generate(&ctx.env);
    ctx.client
        .freeze_account(&ctx.super_admin, &target, &ctx.reason(), &1_000);

    // Let it expire — trying to unfreeze an already-expired freeze should fail.
    ctx.set_timestamp(1_000);
    let result = ctx.client.try_unfreeze_account(&ctx.super_admin, &target);
    assert_eq!(result, Err(Ok(AccountStatusError::NotFrozen)));
}

#[test]
fn edge_indefinite_freeze_never_auto_expires() {
    let ctx = Ctx::new();
    ctx.set_timestamp(0);
    let target = Address::generate(&ctx.env);
    // expires_at = 0 → indefinite.
    ctx.client
        .freeze_account(&ctx.super_admin, &target, &ctx.reason(), &0);

    // Jump far into the future.
    ctx.set_timestamp(u64::MAX / 2);
    assert!(ctx.client.is_frozen(&target));
}

// ── Removed admin loses freeze privileges ─────────────────────────────────────

#[test]
fn auth_removed_admin_loses_freeze_privilege() {
    let ctx = Ctx::new();
    let admin = ctx.make_admin();

    // Remove them.
    ctx.client.remove_admin(&ctx.super_admin, &admin);

    // Now their freeze attempt must fail.
    let target = Address::generate(&ctx.env);
    let result = ctx
        .client
        .try_freeze_account(&admin, &target, &ctx.reason(), &0);
    assert_eq!(result, Err(Ok(AccountStatusError::Unauthorized)));
}

#[test]
fn auth_removed_admin_loses_unfreeze_privilege() {
    let ctx = Ctx::new();
    let admin = ctx.make_admin();
    let target = Address::generate(&ctx.env);
    ctx.freeze(&target);

    ctx.client.remove_admin(&ctx.super_admin, &admin);

    let result = ctx.client.try_unfreeze_account(&admin, &target);
    assert_eq!(result, Err(Ok(AccountStatusError::Unauthorized)));
}

// ── Multi-account isolation ────────────────────────────────────────────────────

#[test]
fn edge_freezing_one_account_does_not_affect_others() {
    let ctx = Ctx::new();
    let alice = Address::generate(&ctx.env);
    let bob = Address::generate(&ctx.env);

    ctx.freeze(&alice);

    assert!(ctx.client.is_frozen(&alice));
    assert!(!ctx.client.is_frozen(&bob));

    ctx.client.assert_not_frozen(&bob); // must not panic
}

#[test]
fn edge_unfreezing_one_account_does_not_affect_others() {
    let ctx = Ctx::new();
    let alice = Address::generate(&ctx.env);
    let bob = Address::generate(&ctx.env);

    ctx.freeze(&alice);
    ctx.freeze(&bob);
    ctx.client.unfreeze_account(&ctx.super_admin, &alice);

    assert!(!ctx.client.is_frozen(&alice));
    assert!(ctx.client.is_frozen(&bob));
}

// ── Reason string edge cases ──────────────────────────────────────────────────

#[test]
fn edge_empty_reason_is_valid() {
    let ctx = Ctx::new();
    let target = Address::generate(&ctx.env);
    let empty = soroban_sdk::String::from_str(&ctx.env, "");
    ctx.client
        .freeze_account(&ctx.super_admin, &target, &empty, &0);
    assert!(ctx.client.is_frozen(&target));
}

#[test]
fn edge_reason_exactly_at_max_length_is_valid() {
    let ctx = Ctx::new();
    let target = Address::generate(&ctx.env);
    let exact: soroban_sdk::String = {
        let s: std::string::String = "x".repeat(MAX_REASON_LEN);
        soroban_sdk::String::from_str(&ctx.env, &s)
    };
    ctx.client
        .freeze_account(&ctx.super_admin, &target, &exact, &0);
    assert!(ctx.client.is_frozen(&target));
}

// ── require_auth guards ───────────────────────────────────────────────────────

#[test]
#[should_panic]
fn auth_freeze_requires_auth() {
    let env = Env::default();
    // No mock_all_auths.
    let super_admin = Address::generate(&env);
    let contract_id = env.register_contract(None, AccountStatusContract);
    let client = AccountStatusContractClient::new(&env, &contract_id);
    {
        env.mock_all_auths();
        client.initialize(&super_admin);
    }
    let target = Address::generate(&env);
    let reason = soroban_sdk::String::from_str(&env, REASON);
    // Auth not mocked — must panic.
    client.freeze_account(&super_admin, &target, &reason, &0);
}

#[test]
#[should_panic]
fn auth_unfreeze_requires_auth() {
    let env = Env::default();
    let super_admin = Address::generate(&env);
    let contract_id = env.register_contract(None, AccountStatusContract);
    let client = AccountStatusContractClient::new(&env, &contract_id);
    {
        env.mock_all_auths();
        client.initialize(&super_admin);
        let target = Address::generate(&env);
        let reason = soroban_sdk::String::from_str(&env, REASON);
        client.freeze_account(&super_admin, &target, &reason, &0);
        // Now try to unfreeze without auth.
        client.unfreeze_account(&super_admin, &target);
    }
}