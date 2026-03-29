#![no_std]

mod escrow;
mod storage;

use soroban_sdk::{contract, contractimpl, panic_with_error, symbol_short, Address, Env, Vec};

use crate::escrow::{
    collect_batch_to_escrow, collect_to_escrow, release_cycle_fees, rollover_cycle_fees,
};
use crate::storage::{
    has_admin, read_admin, read_current_cycle, read_escrow_balance, read_fee_bps, read_locked,
		read_min_fee, read_pending_fees, read_token, read_total_batch_calls, read_total_collected,
    read_total_released, read_treasury, write_admin, write_current_cycle, write_fee_bps,
	write_locked, write_min_fee, write_token, write_treasury,
};
pub use crate::storage::{BatchFeeResult, DataKey, MAX_BATCH_SIZE, MAX_FEE_BPS};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum FeeContractError {
    NotInitialized = 1,
    Unauthorized = 2,
    Locked = 3,
    InvalidAmount = 4,
    EmptyBatch = 5,
    BatchTooLarge = 6,
    Overflow = 7,
    InsufficientEscrow = 8,
    InvalidCycle = 9,
    InvalidConfig = 10,
    NoPendingFees = 11,
}

impl From<FeeContractError> for soroban_sdk::Error {
    fn from(value: FeeContractError) -> Self {
        soroban_sdk::Error::from_contract_error(value as u32)
    }
}

pub struct FeeEvents;

impl FeeEvents {
    pub fn fee_escrowed(env: &Env, payer: &Address, amount: i128, cycle: u64) {
        let topics = (symbol_short!("fee"), symbol_short!("escrowed"));
        env.events().publish(topics, (payer.clone(), amount, cycle));
    }

    pub fn fee_batched(
        env: &Env,
        payer: &Address,
        total_amount: i128,
        batch_size: u32,
        cycle: u64,
    ) {
        let topics = (symbol_short!("fee"), symbol_short!("batched"));
        env.events()
            .publish(topics, (payer.clone(), total_amount, batch_size, cycle));
    }

    pub fn fee_released(env: &Env, cycle: u64, amount: i128, treasury: &Address) {
        let topics = (symbol_short!("fee"), symbol_short!("released"));
        env.events()
            .publish(topics, (cycle, amount, treasury.clone()));
    }

    pub fn fee_rolled(env: &Env, from_cycle: u64, to_cycle: u64, amount: i128) {
        let topics = (symbol_short!("fee"), symbol_short!("rollover"));
        env.events().publish(topics, (from_cycle, to_cycle, amount));
    }

    pub fn locked(env: &Env) {
        let topics = (symbol_short!("fee"), symbol_short!("locked"));
        env.events().publish(topics, ());
    }

    pub fn unlocked(env: &Env) {
        let topics = (symbol_short!("fee"), symbol_short!("unlocked"));
        env.events().publish(topics, ());
    }

    pub fn fee_bps_updated(env: &Env, fee_bps: u32) {
        let topics = (symbol_short!("fee"), symbol_short!("config"));
        env.events()
            .publish(topics, (symbol_short!("bps"), fee_bps));
    }

    pub fn treasury_updated(env: &Env, treasury: &Address) {
        let topics = (symbol_short!("fee"), symbol_short!("config"));
        env.events()
            .publish(topics, (symbol_short!("treasury"), treasury.clone()));
    }

	pub fn min_fee_updated(env: &Env, min_fee: i128) {
		let topics = (symbol_short!("fee"), symbol_short!("config"));
		env.events().publish(topics, (symbol_short!("min_fee"), min_fee));
	}
}

#[contract]
pub struct FeeContract;

#[contractimpl]
impl FeeContract {
    pub fn initialize(
        env: Env,
        admin: Address,
        token: Address,
        treasury: Address,
        fee_bps: u32,
        initial_cycle: u64,
    ) {
        if has_admin(&env) {
            panic!("Contract already initialized");
        }
        if fee_bps > MAX_FEE_BPS || initial_cycle == 0 {
            panic_with_error!(&env, FeeContractError::InvalidConfig);
        }

        write_admin(&env, &admin);
        write_token(&env, &token);
        write_treasury(&env, &treasury);
        write_fee_bps(&env, fee_bps);
        write_locked(&env, false);
        write_current_cycle(&env, initial_cycle);
    }

    pub fn collect_fee(env: Env, payer: Address, amount: i128) -> i128 {
        payer.require_auth();
        let pending = collect_to_escrow(&env, &payer, amount);
        FeeEvents::fee_escrowed(&env, &payer, amount, read_current_cycle(&env));
        pending
    }

    pub fn collect_fee_batch(env: Env, payer: Address, amounts: Vec<i128>) -> BatchFeeResult {
        payer.require_auth();

        let batch_size = amounts.len();
        if batch_size == 0 {
            panic_with_error!(&env, FeeContractError::EmptyBatch);
        }
        if batch_size > MAX_BATCH_SIZE {
            panic_with_error!(&env, FeeContractError::BatchTooLarge);
        }

        let result = collect_batch_to_escrow(&env, &payer, &amounts);
        FeeEvents::fee_batched(
            &env,
            &payer,
            result.total_amount,
            result.batch_size,
            result.cycle,
        );
        result
    }

    pub fn release_fees(env: Env, admin: Address, cycle: u64) -> i128 {
        admin.require_auth();
        Self::require_admin(&env, &admin);

        let released = release_cycle_fees(&env, cycle);
        FeeEvents::fee_released(&env, cycle, released, &read_treasury(&env));
        released
    }

    pub fn rollover_fees(env: Env, admin: Address, next_cycle: u64) -> i128 {
        admin.require_auth();
        Self::require_admin(&env, &admin);

        let current_cycle = read_current_cycle(&env);
        if next_cycle <= current_cycle {
            panic_with_error!(&env, FeeContractError::InvalidCycle);
        }

        let rolled = rollover_cycle_fees(&env, current_cycle, next_cycle);
        write_current_cycle(&env, next_cycle);
        FeeEvents::fee_rolled(&env, current_cycle, next_cycle, rolled);
        rolled
    }

    pub fn lock(env: Env, admin: Address) {
        admin.require_auth();
        Self::require_admin(&env, &admin);

        write_locked(&env, true);
        FeeEvents::locked(&env);
    }

    pub fn unlock(env: Env, admin: Address) {
        admin.require_auth();
        Self::require_admin(&env, &admin);

        write_locked(&env, false);
        FeeEvents::unlocked(&env);
    }

    pub fn set_fee_bps(env: Env, admin: Address, fee_bps: u32) {
        admin.require_auth();
        Self::require_admin(&env, &admin);
        Self::require_unlocked(&env);

        if fee_bps > MAX_FEE_BPS {
            panic_with_error!(&env, FeeContractError::InvalidConfig);
        }

        write_fee_bps(&env, fee_bps);
        FeeEvents::fee_bps_updated(&env, fee_bps);
    }

    pub fn set_treasury(env: Env, admin: Address, treasury: Address) {
        admin.require_auth();
        Self::require_admin(&env, &admin);
        Self::require_unlocked(&env);

        write_treasury(&env, &treasury);
        FeeEvents::treasury_updated(&env, &treasury);
    }

	pub fn set_min_fee(env: Env, admin: Address, min_fee: i128) {
		admin.require_auth();
		Self::require_admin(&env, &admin);
		Self::require_unlocked(&env);

		if min_fee < 0 {
			panic_with_error!(&env, FeeContractError::InvalidConfig);
		}

		write_min_fee(&env, min_fee);
		FeeEvents::min_fee_updated(&env, min_fee);
	}

    pub fn get_admin(env: Env) -> Address {
        read_admin(&env)
    }

    pub fn get_token(env: Env) -> Address {
        read_token(&env)
    }

    pub fn get_treasury(env: Env) -> Address {
        read_treasury(&env)
    }

    pub fn get_fee_bps(env: Env) -> u32 {
        read_fee_bps(&env)
    }

	pub fn get_min_fee(env: Env) -> i128 {
		read_min_fee(&env)
	}

    pub fn is_locked(env: Env) -> bool {
        read_locked(&env)
    }

    pub fn get_current_cycle(env: Env) -> u64 {
        read_current_cycle(&env)
    }

    pub fn get_escrow_balance(env: Env) -> i128 {
        read_escrow_balance(&env)
    }

    pub fn get_pending_fees(env: Env, cycle: u64) -> i128 {
        read_pending_fees(&env, cycle)
    }

    pub fn get_total_collected(env: Env) -> i128 {
        read_total_collected(&env)
    }

    pub fn get_total_released(env: Env) -> i128 {
        read_total_released(&env)
    }

    pub fn get_total_batch_calls(env: Env) -> u64 {
        read_total_batch_calls(&env)
    }

	/// Preview the total fees for a batch of operations without mutating state.
	///
	/// This is a view/read method intended for clients to estimate the aggregate fee
	/// they will be charged when submitting a batch via `collect_fee_batch`. It performs
	/// identical validations (non-empty, size cap, per-item minimum and positivity) but
	/// does not transfer tokens or write to storage.
	///
	/// Validations mirror `collect_fee_batch`:
	/// - Batch must be non-empty and not exceed `MAX_BATCH_SIZE`
	/// - Each item must be positive and meet the configured `min_fee`
	///
	/// Returns the sum of all amounts if valid.
	pub fn preview_batch_fee(env: Env, _user: Address, amounts: Vec<i128>) -> i128 {
		let batch_size = amounts.len();
		if batch_size == 0 {
			panic_with_error!(&env, FeeContractError::EmptyBatch);
		}
		if batch_size > MAX_BATCH_SIZE {
			panic_with_error!(&env, FeeContractError::BatchTooLarge);
		}

		let min_fee = read_min_fee(&env);
		let mut total: i128 = 0;
		for amount in amounts.iter() {
			if amount <= 0 {
				panic_with_error!(&env, FeeContractError::InvalidAmount);
			}
			if amount < min_fee {
				panic_with_error!(&env, FeeContractError::InvalidAmount);
			}
			total = total
				.checked_add(amount)
				.unwrap_or_else(|| panic_with_error!(&env, FeeContractError::Overflow));
		}
		total
	}

    fn require_admin(env: &Env, caller: &Address) {
        if !has_admin(env) {
            panic_with_error!(env, FeeContractError::NotInitialized);
        }

        let admin = read_admin(env);
        if admin != *caller {
            panic_with_error!(env, FeeContractError::Unauthorized);
        }
    }

    fn require_unlocked(env: &Env) {
        if read_locked(env) {
            panic_with_error!(env, FeeContractError::Locked);
        }
    }
}
