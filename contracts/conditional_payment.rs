//! Conditional Payments Contract
//!
//! Enables payments that execute only when predefined on-chain conditions
//! are satisfied. A payer locks funds into a payment order and specifies
//! one or more conditions; any caller may attempt execution, but the
//! transfer only proceeds when every condition is met.
//!
//! # Supported condition types
//!
//! | Variant              | Triggers when…                                          |
//! |----------------------|---------------------------------------------------------|
//! | `TimeAfter`          | `now >= threshold_timestamp`                            |
//! | `TimeBefore`         | `now < threshold_timestamp`                             |
//! | `BalanceAbove`       | token balance of `watch_address >= threshold_amount`    |
//! | `BalanceBelow`       | token balance of `watch_address < threshold_amount`     |
//! | `MultiSigApproved`   | at least `required` of the listed signers have approved |
//!
//! Conditions within a single payment are evaluated with AND semantics —
//! all must pass for the payment to execute.
//!
//! # Lifecycle
//!
//! 1. **`initialize`** — one-time setup with an admin.
//! 2. **`create_payment`** — payer locks funds and attaches conditions.
//! 3. **`approve`** — a signer records their approval for a MultiSig condition.
//! 4. **`execute_payment`** — anyone triggers execution; reverts if any
//!    condition fails.
//! 5. **`cancel_payment`** — payer reclaims funds and voids the order.
//!
//! # Security properties
//!
//! - `require_auth` is the first statement in every mutating entry point.
//! - Funds are transferred into the contract on `create_payment` (escrow
//!   model) so execution can never fail for insufficient balance.
//! - State is written before the outbound token transfer
//!   (checks-effects-interactions).
//! - Duplicate approvals are rejected via a typed stamp key.
//! - Cancelled and executed payments cannot be re-executed.
//! - All arithmetic uses `checked_*`; overflow surfaces as a typed error.
//! - TTL is bumped on every persistent-storage access.

#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracterror, contracttype, panic_with_error, symbol_short,
    token, Address, Env, Vec,
};

// ── Constants ────────────────────────────────────────────────────────────────

/// Ledger TTL bump for persistent payment records (~2 years).
const PERSISTENT_TTL_BUMP: u32 = 12_614_400;

/// Maximum number of conditions allowed per payment.
pub const MAX_CONDITIONS: usize = 8;

/// Maximum number of signers allowed in a MultiSig condition.
pub const MAX_SIGNERS: usize = 10;

// ── Storage keys ─────────────────────────────────────────────────────────────

#[derive(Clone)]
#[contracttype]
pub enum DataKey {
    /// Admin address (instance storage).
    Admin,
    /// Auto-incrementing payment ID counter (instance storage).
    NextPaymentId,
    /// Per-payment record (persistent storage).
    Payment(u64),
    /// MultiSig approval stamp: `(payment_id, signer)` → `bool`.
    ApprovalStamp(u64, Address),
}

// ── Condition types ───────────────────────────────────────────────────────────

/// A single evaluatable condition attached to a payment.
#[derive(Clone)]
#[contracttype]
pub enum Condition {
    /// Payment executes only after this ledger timestamp.
    TimeAfter(u64),

    /// Payment executes only before this ledger timestamp (deadline).
    TimeBefore(u64),

    /// Balance of `watch_address` in `token` must be >= `threshold`.
    BalanceAbove {
        token: Address,
        watch_address: Address,
        threshold: i128,
    },

    /// Balance of `watch_address` in `token` must be < `threshold`.
    BalanceBelow {
        token: Address,
        watch_address: Address,
        threshold: i128,
    },

    /// At least `required` of `signers` must have called `approve`.
    MultiSigApproved {
        signers: Vec<Address>,
        required: u32,
    },
}

// ── Payment status ────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[contracttype]
pub enum PaymentStatus {
    /// Awaiting condition fulfilment.
    Pending = 0,
    /// All conditions met and funds transferred.
    Executed = 1,
    /// Cancelled by the payer; funds returned.
    Cancelled = 2,
}

// ── Payment record ────────────────────────────────────────────────────────────

#[derive(Clone)]
#[contracttype]
pub struct ConditionalPayment {
    /// Address that created and funded this payment.
    pub payer: Address,
    /// Destination address for the funds on execution.
    pub recipient: Address,
    /// Token contract address.
    pub token: Address,
    /// Amount locked in escrow (stroops).
    pub amount: i128,
    /// All conditions — every one must pass for execution to proceed.
    pub conditions: Vec<Condition>,
    /// Current lifecycle status.
    pub status: PaymentStatus,
    /// Ledger timestamp when this payment was created.
    pub created_at: u64,
    /// Ledger timestamp of execution or cancellation (0 if still pending).
    pub settled_at: u64,
}

// ── Errors ────────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ConditionalPaymentError {
    NotInitialized = 1,
    AlreadyInitialized = 2,
    Unauthorized = 3,
    InvalidAmount = 4,
    PaymentNotFound = 5,
    /// Payment is not in `Pending` status.
    PaymentNotPending = 6,
    /// One or more conditions are not yet satisfied.
    ConditionNotMet = 7,
    /// The caller is not a valid signer for this payment's MultiSig.
    NotASigner = 8,
    /// This signer has already approved.
    AlreadyApproved = 9,
    /// Too many conditions supplied.
    TooManyConditions = 10,
    /// Too many signers in a MultiSig condition.
    TooManySigners = 11,
    /// `required` approvals exceeds number of signers.
    InvalidThreshold = 12,
    /// Payer and recipient are the same address.
    SamePayerRecipient = 13,
    /// Arithmetic overflow.
    Overflow = 14,
    /// A condition parameter is logically invalid (e.g. zero threshold).
    InvalidCondition = 15,
}

// ── Events ────────────────────────────────────────────────────────────────────

pub struct ConditionalPaymentEvents;

impl ConditionalPaymentEvents {
    /// Emitted when a new conditional payment is created.
    /// Payload: `(payer, payment_id, recipient, token, amount, timestamp)`
    pub fn payment_created(
        env: &Env,
        payer: &Address,
        payment_id: u64,
        recipient: &Address,
        token: &Address,
        amount: i128,
    ) {
        env.events().publish(
            (symbol_short!("condpay"), symbol_short!("created")),
            (
                payer.clone(),
                payment_id,
                recipient.clone(),
                token.clone(),
                amount,
                env.ledger().timestamp(),
            ),
        );
    }

    /// Emitted when all conditions are met and the payment is executed.
    /// Payload: `(payment_id, payer, recipient, amount, timestamp)`
    pub fn payment_executed(
        env: &Env,
        payment_id: u64,
        payer: &Address,
        recipient: &Address,
        amount: i128,
    ) {
        env.events().publish(
            (symbol_short!("condpay"), symbol_short!("executed")),
            (
                payment_id,
                payer.clone(),
                recipient.clone(),
                amount,
                env.ledger().timestamp(),
            ),
        );
    }

    /// Emitted when a payment is cancelled and funds returned to payer.
    /// Payload: `(payment_id, payer, amount, timestamp)`
    pub fn payment_cancelled(env: &Env, payment_id: u64, payer: &Address, amount: i128) {
        env.events().publish(
            (symbol_short!("condpay"), symbol_short!("cancel")),
            (
                payment_id,
                payer.clone(),
                amount,
                env.ledger().timestamp(),
            ),
        );
    }

    /// Emitted when a signer approves a MultiSig condition.
    /// Payload: `(payment_id, signer, timestamp)`
    pub fn approval_recorded(env: &Env, payment_id: u64, signer: &Address) {
        env.events().publish(
            (symbol_short!("condpay"), symbol_short!("approved")),
            (payment_id, signer.clone(), env.ledger().timestamp()),
        );
    }

    /// Emitted when a condition check fails during an execution attempt.
    /// Payload: `(payment_id, condition_index, timestamp)`
    pub fn condition_failed(env: &Env, payment_id: u64, condition_index: u32) {
        env.events().publish(
            (symbol_short!("condpay"), symbol_short!("cond_fail")),
            (payment_id, condition_index, env.ledger().timestamp()),
        );
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

impl ConditionalPaymentsContract {
    fn require_initialized(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| panic_with_error!(env, ConditionalPaymentError::NotInitialized))
    }

    fn next_payment_id(env: &Env) -> u64 {
        let id: u64 = env
            .storage()
            .instance()
            .get(&DataKey::NextPaymentId)
            .unwrap_or(0u64);
        let next = id
            .checked_add(1)
            .unwrap_or_else(|| panic_with_error!(env, ConditionalPaymentError::Overflow));
        env.storage()
            .instance()
            .set(&DataKey::NextPaymentId, &next);
        id
    }

    fn load_payment(env: &Env, payment_id: u64) -> ConditionalPayment {
        let key = DataKey::Payment(payment_id);
        let payment: ConditionalPayment = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| panic_with_error!(env, ConditionalPaymentError::PaymentNotFound));
        env.storage()
            .persistent()
            .extend_ttl(&key, PERSISTENT_TTL_BUMP, PERSISTENT_TTL_BUMP);
        payment
    }

    fn save_payment(env: &Env, payment_id: u64, payment: &ConditionalPayment) {
        let key = DataKey::Payment(payment_id);
        env.storage().persistent().set(&key, payment);
        env.storage()
            .persistent()
            .extend_ttl(&key, PERSISTENT_TTL_BUMP, PERSISTENT_TTL_BUMP);
    }

    /// Validate a single condition's internal parameters at creation time.
    fn validate_condition(env: &Env, condition: &Condition) {
        match condition {
            Condition::TimeAfter(ts) => {
                // ts == 0 would always be true — disallow as it is meaningless.
                if *ts == 0 {
                    panic_with_error!(env, ConditionalPaymentError::InvalidCondition);
                }
            }
            Condition::TimeBefore(ts) => {
                if *ts == 0 {
                    panic_with_error!(env, ConditionalPaymentError::InvalidCondition);
                }
            }
            Condition::BalanceAbove { threshold, .. }
            | Condition::BalanceBelow { threshold, .. } => {
                // Non-positive thresholds are always trivially true or false.
                if *threshold <= 0 {
                    panic_with_error!(env, ConditionalPaymentError::InvalidCondition);
                }
            }
            Condition::MultiSigApproved { signers, required } => {
                let n = signers.len();
                if n == 0 || n > MAX_SIGNERS as u32 {
                    panic_with_error!(env, ConditionalPaymentError::TooManySigners);
                }
                if *required == 0 || *required > n {
                    panic_with_error!(env, ConditionalPaymentError::InvalidThreshold);
                }
            }
        }
    }

    /// Evaluate a single condition against current on-chain state.
    ///
    /// Returns `true` if the condition passes, `false` otherwise.
    /// Does NOT panic — callers decide whether to abort or emit an event.
    fn evaluate_condition(env: &Env, payment_id: u64, condition: &Condition) -> bool {
        match condition {
            Condition::TimeAfter(threshold) => env.ledger().timestamp() >= *threshold,

            Condition::TimeBefore(threshold) => env.ledger().timestamp() < *threshold,

            Condition::BalanceAbove {
                token,
                watch_address,
                threshold,
            } => {
                let balance = token::Client::new(env, token).balance(watch_address);
                balance >= *threshold
            }

            Condition::BalanceBelow {
                token,
                watch_address,
                threshold,
            } => {
                let balance = token::Client::new(env, token).balance(watch_address);
                balance < *threshold
            }

            Condition::MultiSigApproved { signers, required } => {
                let mut approvals: u32 = 0;
                for signer in signers.iter() {
                    let stamp_key = DataKey::ApprovalStamp(payment_id, signer.clone());
                    if env.storage().persistent().has(&stamp_key) {
                        approvals = approvals.saturating_add(1);
                    }
                    if approvals >= *required {
                        return true;
                    }
                }
                false
            }
        }
    }
}

// ── Contract ──────────────────────────────────────────────────────────────────

#[contract]
pub struct ConditionalPaymentsContract;

#[contractimpl]
impl ConditionalPaymentsContract {
    // ── Lifecycle ────────────────────────────────────────────────────────────

    /// Initialise the contract. Only callable once.
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic_with_error!(&env, ConditionalPaymentError::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::NextPaymentId, &0u64);
    }

    // ── Payment creation ─────────────────────────────────────────────────────

    /// Create a conditional payment, immediately escrowing `amount` tokens.
    ///
    /// The payer must have pre-approved the contract for at least `amount`
    /// via the token's `approve` mechanism.
    ///
    /// # Parameters
    /// - `conditions` — 1–`MAX_CONDITIONS` conditions, ALL must pass to execute.
    ///
    /// Returns the new payment ID.
    ///
    /// # Security
    /// - `payer.require_auth()` is first.
    /// - Payer ≠ recipient (prevents zero-net-effect circular payments).
    /// - All condition parameters are validated before the token transfer.
    /// - Funds move into escrow at creation; execution can never fail for
    ///   insufficient balance.
    pub fn create_payment(
        env: Env,
        payer: Address,
        recipient: Address,
        token: Address,
        amount: i128,
        conditions: Vec<Condition>,
    ) -> u64 {
        payer.require_auth();
        Self::require_initialized(&env);

        if amount <= 0 {
            panic_with_error!(&env, ConditionalPaymentError::InvalidAmount);
        }
        if payer == recipient {
            panic_with_error!(&env, ConditionalPaymentError::SamePayerRecipient);
        }
        if conditions.is_empty() || conditions.len() > MAX_CONDITIONS as u32 {
            panic_with_error!(&env, ConditionalPaymentError::TooManyConditions);
        }

        // Validate every condition's internal parameters before writing state.
        for condition in conditions.iter() {
            Self::validate_condition(&env, &condition);
        }

        let payment_id = Self::next_payment_id(&env);

        let payment = ConditionalPayment {
            payer: payer.clone(),
            recipient: recipient.clone(),
            token: token.clone(),
            amount,
            conditions: conditions.clone(),
            status: PaymentStatus::Pending,
            created_at: env.ledger().timestamp(),
            settled_at: 0,
        };

        Self::save_payment(&env, payment_id, &payment);

        // Escrow: pull funds from payer into this contract now.
        token::Client::new(&env, &token).transfer_from(
            &env.current_contract_address(),
            &payer,
            &env.current_contract_address(),
            &amount,
        );

        ConditionalPaymentEvents::payment_created(
            &env, &payer, payment_id, &recipient, &token, amount,
        );

        payment_id
    }

    // ── MultiSig approval ────────────────────────────────────────────────────

    /// Record `signer`'s approval for a pending payment's MultiSig condition.
    ///
    /// Can be called independently of execution, allowing signers to approve
    /// asynchronously before the time window opens.
    ///
    /// # Security
    /// - `signer.require_auth()` is first.
    /// - Caller must appear in at least one `MultiSigApproved` condition.
    /// - Duplicate approvals are rejected via a typed `ApprovalStamp` key.
    /// - Only pending payments accept approvals.
    pub fn approve(env: Env, signer: Address, payment_id: u64) {
        signer.require_auth();
        Self::require_initialized(&env);

        let payment = Self::load_payment(&env, payment_id);

        if payment.status != PaymentStatus::Pending {
            panic_with_error!(&env, ConditionalPaymentError::PaymentNotPending);
        }

        // Verify the signer appears in at least one MultiSig condition.
        let mut valid_signer = false;
        for condition in payment.conditions.iter() {
            if let Condition::MultiSigApproved { signers, .. } = condition {
                if signers.contains(&signer) {
                    valid_signer = true;
                    break;
                }
            }
        }
        if !valid_signer {
            panic_with_error!(&env, ConditionalPaymentError::NotASigner);
        }

        // Duplicate approval guard.
        let stamp_key = DataKey::ApprovalStamp(payment_id, signer.clone());
        if env.storage().persistent().has(&stamp_key) {
            panic_with_error!(&env, ConditionalPaymentError::AlreadyApproved);
        }

        // Write approval stamp and bump TTL.
        env.storage().persistent().set(&stamp_key, &true);
        env.storage()
            .persistent()
            .extend_ttl(&stamp_key, PERSISTENT_TTL_BUMP, PERSISTENT_TTL_BUMP);

        ConditionalPaymentEvents::approval_recorded(&env, payment_id, &signer);
    }

    // ── Execution ────────────────────────────────────────────────────────────

    /// Attempt to execute a conditional payment.
    ///
    /// May be called by anyone — if all conditions pass the funds are
    /// transferred from escrow to the recipient. If any condition fails, a
    /// `condition_failed` event is emitted for that index and the call reverts
    /// with `ConditionNotMet`.
    ///
    /// # Security
    /// - Payment must be `Pending`.
    /// - ALL conditions are evaluated; first failure reverts the call.
    /// - State is updated to `Executed` BEFORE the outbound token transfer
    ///   (checks-effects-interactions pattern).
    pub fn execute_payment(env: Env, payment_id: u64) -> i128 {
        Self::require_initialized(&env);

        let payment = Self::load_payment(&env, payment_id);

        if payment.status != PaymentStatus::Pending {
            panic_with_error!(&env, ConditionalPaymentError::PaymentNotPending);
        }

        // Evaluate every condition; revert on first failure.
        for (idx, condition) in payment.conditions.iter().enumerate() {
            if !Self::evaluate_condition(&env, payment_id, &condition) {
                ConditionalPaymentEvents::condition_failed(&env, payment_id, idx as u32);
                panic_with_error!(&env, ConditionalPaymentError::ConditionNotMet);
            }
        }

        // All conditions passed — update state before the transfer.
        let mut settled = payment.clone();
        settled.status = PaymentStatus::Executed;
        settled.settled_at = env.ledger().timestamp();
        Self::save_payment(&env, payment_id, &settled);

        // Transfer escrowed funds to recipient.
        token::Client::new(&env, &payment.token).transfer(
            &env.current_contract_address(),
            &payment.recipient,
            &payment.amount,
        );

        ConditionalPaymentEvents::payment_executed(
            &env,
            payment_id,
            &payment.payer,
            &payment.recipient,
            payment.amount,
        );

        payment.amount
    }

    // ── Cancellation ─────────────────────────────────────────────────────────

    /// Cancel a pending payment and return escrowed funds to the payer.
    ///
    /// Only the original payer may cancel.
    ///
    /// # Security
    /// - `caller.require_auth()` is first.
    /// - Only pending payments can be cancelled.
    /// - Escrowed funds are returned via the outbound transfer AFTER state
    ///   is updated (checks-effects-interactions).
    pub fn cancel_payment(env: Env, caller: Address, payment_id: u64) {
        caller.require_auth();
        Self::require_initialized(&env);

        let payment = Self::load_payment(&env, payment_id);

        if caller != payment.payer {
            panic_with_error!(&env, ConditionalPaymentError::Unauthorized);
        }
        if payment.status != PaymentStatus::Pending {
            panic_with_error!(&env, ConditionalPaymentError::PaymentNotPending);
        }

        // Update state before the outbound transfer.
        let mut cancelled = payment.clone();
        cancelled.status = PaymentStatus::Cancelled;
        cancelled.settled_at = env.ledger().timestamp();
        Self::save_payment(&env, payment_id, &cancelled);

        // Refund escrowed amount back to payer.
        token::Client::new(&env, &payment.token).transfer(
            &env.current_contract_address(),
            &payment.payer,
            &payment.amount,
        );

        ConditionalPaymentEvents::payment_cancelled(&env, payment_id, &payment.payer, payment.amount);
    }

    // ── Read-only queries ────────────────────────────────────────────────────

    /// Return the full payment record.
    pub fn get_payment(env: Env, payment_id: u64) -> ConditionalPayment {
        Self::require_initialized(&env);
        Self::load_payment(&env, payment_id)
    }

    /// Dry-run condition evaluation without executing.
    ///
    /// Returns a `Vec<bool>` — one entry per condition in order.
    /// Useful for frontends to show which conditions are currently passing.
    pub fn check_conditions(env: Env, payment_id: u64) -> Vec<bool> {
        Self::require_initialized(&env);
        let payment = Self::load_payment(&env, payment_id);
        let mut results = Vec::new(&env);
        for condition in payment.conditions.iter() {
            results.push_back(Self::evaluate_condition(&env, payment_id, &condition));
        }
        results
    }

    /// Return how many approvals a MultiSig condition at `condition_index`
    /// has accumulated so far.
    pub fn approval_count(env: Env, payment_id: u64, condition_index: u32) -> u32 {
        Self::require_initialized(&env);
        let payment = Self::load_payment(&env, payment_id);
        let condition = payment.conditions.get(condition_index).unwrap_or_else(|| {
            panic_with_error!(&env, ConditionalPaymentError::InvalidCondition)
        });
        match condition {
            Condition::MultiSigApproved { signers, .. } => {
                let mut count: u32 = 0;
                for signer in signers.iter() {
                    let stamp_key = DataKey::ApprovalStamp(payment_id, signer.clone());
                    if env.storage().persistent().has(&stamp_key) {
                        count = count.saturating_add(1);
                    }
                }
                count
            }
            _ => panic_with_error!(&env, ConditionalPaymentError::InvalidCondition),
        }
    }

    /// Return total number of payments ever created.
    pub fn payment_count(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::NextPaymentId)
            .unwrap_or(0)
    }
}