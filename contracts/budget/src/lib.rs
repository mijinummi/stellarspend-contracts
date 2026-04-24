//! # Budget Contract
//!
//! A Soroban smart contract for managing user budgets with validation and event emission.
//!
//! ## Features
//!
//! - **Individual Budget Updates**: Update single user budgets
//! - **Validation**: Prevents negative or zero allocations
//! - **Event Emission**: Tracks budget updates
//! - **Atomic Operations**: Ensures reliable state changes
//!
#![no_std]
use soroban_sdk::{contractimpl, Env, Address};

pub struct BudgetContract;

#[contractimpl]
impl BudgetContract {
    /// Set a budget limit for a given user
    pub fn set_budget(env: Env, user: Address, amount: i128) {
        // Store under a key derived from user address
        env.storage().set(&user, &amount);
    }

    /// Get the budget limit for a given user
    pub fn get_budget(env: Env, user: Address) -> i128 {
        env.storage().get(&user).unwrap_or(0)
    }
}

use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Env, panic_with_error};

/// Error codes for the budget contract.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum BudgetError {
    /// Contract not initialized
    NotInitialized = 1,
    /// Caller is not authorized
    Unauthorized = 2,
    /// Invalid budget amount (negative or zero)
    InvalidAmount = 3,
    /// User not found
    UserNotFound = 4,
}

impl From<BudgetError> for soroban_sdk::Error {
    fn from(e: BudgetError) -> Self {
        soroban_sdk::Error::from_contract_error(e as u32)
    }
}

/// Budget record for a user
#[derive(Clone, Debug)]
#[contracttype]
pub struct BudgetRecord {
    pub user: Address,
    pub amount: i128,
    pub last_updated: u64,
}

/// Storage keys for the contract
#[derive(Clone, Debug)]
#[contracttype]
pub enum DataKey {
    Admin,
    Budget(Address),
    TotalAllocated,
}

#[contract]
pub struct BudgetContract;

#[contractimpl]
impl BudgetContract {
    /// Initializes the contract with an admin address.
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("Already initialized");
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::TotalAllocated, &0i128);
    }

    /// Updates a single user's budget.
    ///
    /// # Arguments
    /// * `env` - The contract environment
    /// * `admin` - The admin address calling the function
    /// * `user` - The user address to update budget for
    /// * `amount` - The new budget amount
    pub fn update_budget(env: Env, admin: Address, user: Address, amount: i128) {
        // Verify admin authority
        admin.require_auth();
        Self::require_admin(&env, &admin);

        // Validate amount
        if amount <= 0 {
            panic_with_error!(&env, BudgetError::InvalidAmount);
        }

        let current_time = env.ledger().timestamp();
        
        // Get current total allocated
        let mut total_allocated: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalAllocated)
            .unwrap_or(0);

        // Check if user exists and get old amount
        if let Some(old_record) = env
            .storage()
            .persistent()
            .get::<DataKey, BudgetRecord>(&DataKey::Budget(user.clone())) {
            // Subtract old amount from total
            total_allocated = total_allocated.checked_sub(old_record.amount).unwrap_or(0);
        }

        // Add new amount to total
        total_allocated = total_allocated.checked_add(amount).unwrap_or(i128::MAX);

        // Create new budget record
        let record = BudgetRecord {
            user: user.clone(),
            amount,
            last_updated: current_time,
        };

        // Store the updated budget
        env.storage()
            .persistent()
            .set(&DataKey::Budget(user.clone()), &record);

        // Update total allocated
        env.storage()
            .instance()
            .set(&DataKey::TotalAllocated, &total_allocated);

        // Emit update event
        env.events().publish(
            (symbol_short!("budget"), symbol_short!("updated")),
            (user, amount, current_time),
        );
    }

    /// Retrieves the budget for a specific user.
    pub fn get_budget(env: Env, user: Address) -> Option<BudgetRecord> {
        env.storage().persistent().get(&DataKey::Budget(user))
    }

    /// Returns the admin address
    pub fn get_admin(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Not initialized")
    }

    /// Returns the total allocated budget amount
    pub fn get_total_allocated(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::TotalAllocated)
            .unwrap_or(0)
    }

    /// Internal helper to verify admin authority
    fn require_admin(env: &Env, caller: &Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Not initialized");

        if *caller != admin {
            panic_with_error!(env, BudgetError::Unauthorized);
        }
    }
}
