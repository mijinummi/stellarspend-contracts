use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, symbol_short, Address,
    Env, Vec,
};

// =============================================================================
// Priority Levels
// =============================================================================

/// Priority levels for transaction execution.
/// Higher priority levels result in higher fees for faster execution.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[contracttype]
pub enum PriorityLevel {
    /// Low priority - lowest fees, slowest execution
    Low = 0,
    /// Medium priority - standard fees, normal execution
    Medium = 1,
    /// High priority - higher fees, faster execution
    High = 2,
    /// Urgent priority - highest fees, fastest execution
    Urgent = 3,
}

impl Default for PriorityLevel {
    fn default() -> Self {
        PriorityLevel::Medium
    }
}

impl PriorityLevel {
    /// Convert from u32 to PriorityLevel
    pub fn from_u32(value: u32) -> Option<Self> {
        match value {
            0 => Some(PriorityLevel::Low),
            1 => Some(PriorityLevel::Medium),
            2 => Some(PriorityLevel::High),
            3 => Some(PriorityLevel::Urgent),
            _ => None,
        }
    }

    /// Convert PriorityLevel to u32
    pub fn to_u32(self) -> u32 {
        self as u32
    }
}

// =============================================================================
// Fee Configuration Structures
// =============================================================================

/// Represents a fee window with time-based rates.
#[derive(Clone, Debug)]
#[contracttype]
pub struct FeeWindow {
    /// Ledger timestamp start
    pub start: u64,
    /// Ledger timestamp end
    pub end: u64,
    /// Fee rate in basis points (e.g., 100 = 1%)
    pub fee_rate: u32,
}

/// Configuration for priority-based fee multipliers.
/// Each priority level has a multiplier applied to the base fee rate.
#[derive(Clone, Debug)]
#[contracttype]
pub struct PriorityFeeConfig {
    /// Multiplier for Low priority (e.g., 8000 = 0.8x, 80% of base fee)
    pub low_multiplier_bps: u32,
    /// Multiplier for Medium priority (e.g., 10000 = 1.0x, 100% of base fee)
    pub medium_multiplier_bps: u32,
    /// Multiplier for High priority (e.g., 15000 = 1.5x, 150% of base fee)
    pub high_multiplier_bps: u32,
    /// Multiplier for Urgent priority (e.g., 20000 = 2.0x, 200% of base fee)
    pub urgent_multiplier_bps: u32,
}

impl Default for PriorityFeeConfig {
    fn default() -> Self {
        Self {
            low_multiplier_bps: 8000,      // 0.8x - 20% discount
            medium_multiplier_bps: 10000,  // 1.0x - base rate
            high_multiplier_bps: 15000,    // 1.5x - 50% premium
            urgent_multiplier_bps: 20000,  // 2.0x - 100% premium
        }
    }
}

impl PriorityFeeConfig {
    /// Get the multiplier for a given priority level in basis points
    pub fn get_multiplier_bps(&self, priority: PriorityLevel) -> u32 {
        match priority {
            PriorityLevel::Low => self.low_multiplier_bps,
            PriorityLevel::Medium => self.medium_multiplier_bps,
            PriorityLevel::High => self.high_multiplier_bps,
            PriorityLevel::Urgent => self.urgent_multiplier_bps,
        }
    }

    /// Validate that multipliers are in ascending order (higher priority = higher fee)
    pub fn is_valid(&self) -> bool {
        self.low_multiplier_bps <= self.medium_multiplier_bps
            && self.medium_multiplier_bps <= self.high_multiplier_bps
            && self.high_multiplier_bps <= self.urgent_multiplier_bps
    }
}

/// Main fee configuration structure.
#[derive(Clone, Debug)]
#[contracttype]
pub struct FeeConfig {
    /// Default fee rate in basis points
    pub default_fee_rate: u32,
    /// Time-based fee windows
    pub windows: Vec<FeeWindow>,
    /// Priority-based fee multipliers
    pub priority_config: PriorityFeeConfig,
}

// =============================================================================
// Storage Keys
// =============================================================================

/// Storage keys used by the fee contract.
#[derive(Clone)]
#[contracttype]
pub enum DataKey {
    /// Admin address
    Admin,
    /// Fee configuration
    FeeConfig,
    /// Priority fee configuration
    PriorityFeeConfig,
    /// Total fees collected
    TotalFeesCollected,
    /// Per-user fee tracking
    UserFeesAccrued(Address),
    /// Minimum fee threshold
    MinFee,
    /// Maximum fee threshold
    MaxFee,
}

// =============================================================================
// Errors
// =============================================================================

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum FeeError {
    /// Contract not initialized
    NotInitialized = 1,
    /// Contract already initialized
    AlreadyInitialized = 2,
    /// Caller is not authorized
    Unauthorized = 3,
    /// Invalid fee percentage
    InvalidPercentage = 4,
    /// Invalid amount
    InvalidAmount = 5,
    /// Arithmetic overflow
    Overflow = 6,
    /// Invalid priority level
    InvalidPriorityLevel = 7,
    /// Invalid priority multiplier configuration
    InvalidPriorityConfig = 8,
    /// Invalid fee window
    InvalidFeeWindow = 9,
    /// Invalid fee bound
    InvalidFeeBound = 10,
    /// Invalid fee bound range
    InvalidFeeBoundRange = 11,
}

// =============================================================================
// Events
// =============================================================================

/// Events emitted by the fee contract.
pub struct FeeEvents;

impl FeeEvents {
    pub fn priority_config_updated(env: &Env, admin: &Address, config: &PriorityFeeConfig) {
        let topics = (symbol_short!("fee"), symbol_short!("pri_cfg"));
        env.events().publish(
            topics,
            (
                admin.clone(),
                config.low_multiplier_bps,
                config.medium_multiplier_bps,
                config.high_multiplier_bps,
                config.urgent_multiplier_bps,
                env.ledger().timestamp(),
            ),
        );
    }

    pub fn fee_deducted(
        env: &Env,
        payer: &Address,
        amount: i128,
        fee: i128,
        priority: PriorityLevel,
    ) {
        let topics = (symbol_short!("fee"), symbol_short!("deducted"));
        env.events().publish(
            topics,
            (payer.clone(), amount, fee, priority.to_u32(), env.ledger().timestamp()),
        );
    }

    pub fn config_updated(env: &Env, admin: &Address, fee_rate: u32) {
        let topics = (symbol_short!("fee"), symbol_short!("cfg_upd"));
        env.events().publish(topics, (admin.clone(), fee_rate, env.ledger().timestamp()));
    }
}

// =============================================================================
// Fee Calculation Functions
// =============================================================================

/// Calculate the fee rate for a given priority level.
/// Returns the adjusted fee rate in basis points.
pub fn calculate_priority_fee_rate(
    base_rate_bps: u32,
    priority: PriorityLevel,
    config: &PriorityFeeConfig,
) -> u32 {
    let multiplier_bps = config.get_multiplier_bps(priority);
    // Calculate: base_rate * multiplier / 10000
    // This gives us the adjusted fee rate
    (base_rate_bps as u64 * multiplier_bps as u64 / 10_000) as u32
}

/// Calculate fee for an amount with time-based windows and priority level.
pub fn calculate_fee(env: &Env, amount: i128, config: &FeeConfig) -> i128 {
    calculate_fee_with_priority(env, amount, config, PriorityLevel::default())
}

/// Calculate fee for an amount with priority level.
pub fn calculate_fee_with_priority(
    env: &Env,
    amount: i128,
    config: &FeeConfig,
    priority: PriorityLevel,
) -> i128 {
    if amount <= 0 {
        return 0;
    }

    let now = env.ledger().timestamp();

    // Find applicable fee rate from windows
    let mut base_fee_rate = config.default_fee_rate;
    for window in config.windows.iter() {
        if now >= window.start && now <= window.end {
            base_fee_rate = window.fee_rate;
            break;
        }
    }

    // Apply priority multiplier
    let adjusted_fee_rate =
        calculate_priority_fee_rate(base_fee_rate, priority, &config.priority_config);

    // Calculate fee: amount * rate / 10000
    (amount * adjusted_fee_rate as i128) / 10_000
}

/// Validate fee windows for correctness.
pub fn validate_windows(windows: &[FeeWindow]) -> bool {
    for w in windows {
        if w.start >= w.end {
            return false;
        }
    }
    true
}

// =============================================================================
// Safe Arithmetic Functions
// =============================================================================

pub fn safe_multiply(amount: i128, rate: u32) -> Option<i128> {
    amount.checked_mul(rate as i128)
}

pub fn safe_divide(value: i128, divisor: i128) -> Option<i128> {
    value.checked_div(divisor)
}

// =============================================================================
// Fee Contract
// =============================================================================

#[contract]
pub struct FeeContract;

#[contractimpl]
impl FeeContract {
    /// Initialize the fee contract with admin and default fee rate.
    pub fn initialize(env: Env, admin: Address, default_fee_rate: u32) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic_with_error!(&env, FeeError::AlreadyInitialized);
        }

        if default_fee_rate > 10_000 {
            panic_with_error!(&env, FeeError::InvalidPercentage);
        }

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::TotalFeesCollected, &0i128);

        // Initialize default priority configuration
        let priority_config = PriorityFeeConfig::default();
        env.storage()
            .instance()
            .set(&DataKey::PriorityFeeConfig, &priority_config);

        // Initialize fee config with default rate
        let config = FeeConfig {
            default_fee_rate,
            windows: Vec::new(&env),
            priority_config: priority_config.clone(),
        };
        env.storage().instance().set(&DataKey::FeeConfig, &config);

        FeeEvents::config_updated(&env, &admin, default_fee_rate);
    }

    /// Set the priority fee multipliers.
    /// Only admin can call this function.
    ///
    /// # Arguments
    /// * `caller` - The admin address
    /// * `low_multiplier_bps` - Multiplier for Low priority (e.g., 8000 = 0.8x)
    /// * `medium_multiplier_bps` - Multiplier for Medium priority (e.g., 10000 = 1.0x)
    /// * `high_multiplier_bps` - Multiplier for High priority (e.g., 15000 = 1.5x)
    /// * `urgent_multiplier_bps` - Multiplier for Urgent priority (e.g., 20000 = 2.0x)
    pub fn set_priority_multipliers(
        env: Env,
        caller: Address,
        low_multiplier_bps: u32,
        medium_multiplier_bps: u32,
        high_multiplier_bps: u32,
        urgent_multiplier_bps: u32,
    ) {
        caller.require_auth();
        Self::require_admin(&env, &caller);

        let config = PriorityFeeConfig {
            low_multiplier_bps,
            medium_multiplier_bps,
            high_multiplier_bps,
            urgent_multiplier_bps,
        };

        if !config.is_valid() {
            panic_with_error!(&env, FeeError::InvalidPriorityConfig);
        }

        env.storage()
            .instance()
            .set(&DataKey::PriorityFeeConfig, &config);

        // Also update the FeeConfig
        let mut fee_config: FeeConfig = env
            .storage()
            .instance()
            .get(&DataKey::FeeConfig)
            .unwrap_or_else(|| panic_with_error!(&env, FeeError::NotInitialized));
        fee_config.priority_config = config.clone();
        env.storage().instance().set(&DataKey::FeeConfig, &fee_config);

        FeeEvents::priority_config_updated(&env, &caller, &config);
    }

    /// Get the current priority fee configuration.
    pub fn get_priority_config(env: Env) -> PriorityFeeConfig {
        env.storage()
            .instance()
            .get(&DataKey::PriorityFeeConfig)
            .unwrap_or_else(PriorityFeeConfig::default)
    }

    /// Get the fee multiplier for a specific priority level.
    pub fn get_priority_multiplier(env: Env, priority: PriorityLevel) -> u32 {
        let config = Self::get_priority_config(&env);
        config.get_multiplier_bps(priority)
    }

    /// Calculate fee for an amount with a specific priority level.
    pub fn calculate_fee_with_priority(
        env: Env,
        amount: i128,
        priority: PriorityLevel,
    ) -> i128 {
        if amount <= 0 {
            panic_with_error!(&env, FeeError::InvalidAmount);
        }

        let config: FeeConfig = env
            .storage()
            .instance()
            .get(&DataKey::FeeConfig)
            .unwrap_or_else(|| panic_with_error!(&env, FeeError::NotInitialized));

        let fee = calculate_fee_with_priority(&env, amount, &config, priority);

        // Apply min/max bounds
        let min_fee: i128 = env
            .storage()
            .instance()
            .get(&DataKey::MinFee)
            .unwrap_or(0);
        let max_fee: i128 = env
            .storage()
            .instance()
            .get(&DataKey::MaxFee)
            .unwrap_or(i128::MAX);

        fee.max(min_fee).min(max_fee)
    }

    /// Deduct fee with priority level.
    /// Returns (net_amount, fee_charged).
    pub fn deduct_fee_with_priority(
        env: Env,
        payer: Address,
        amount: i128,
        priority: PriorityLevel,
    ) -> (i128, i128) {
        payer.require_auth();
        Self::require_initialized(&env);

        let fee = Self::calculate_fee_with_priority(env.clone(), amount, priority);

        let net = amount
            .checked_sub(fee)
            .unwrap_or_else(|| panic_with_error!(&env, FeeError::Overflow));

        // Update total collected
        let mut total: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalFeesCollected)
            .unwrap_or(0);
        total = total
            .checked_add(fee)
            .unwrap_or_else(|| panic_with_error!(&env, FeeError::Overflow));
        env.storage()
            .instance()
            .set(&DataKey::TotalFeesCollected, &total);

        // Update user fees accrued
        let mut user_fees: i128 = env
            .storage()
            .instance()
            .get(&DataKey::UserFeesAccrued(payer.clone()))
            .unwrap_or(0);
        user_fees = user_fees
            .checked_add(fee)
            .unwrap_or_else(|| panic_with_error!(&env, FeeError::Overflow));
        env.storage()
            .instance()
            .set(&DataKey::UserFeesAccrued(payer.clone()), &user_fees);

        FeeEvents::fee_deducted(&env, &payer, amount, fee, priority);
        (net, fee)
    }

    /// Simulate fee calculation (read-only).
    pub fn simulate_fee(env: Env, amount: i128, user: Address) -> i128 {
        let config: FeeConfig = env
            .storage()
            .instance()
            .get(&DataKey::FeeConfig)
            .unwrap_or_else(|| panic_with_error!(&env, FeeError::NotInitialized));
        calculate_fee(&env, amount, &config)
    }

    /// Get fee for an amount with default (Medium) priority.
    pub fn get_fee(env: Env, amount: i128) -> i128 {
        let config: FeeConfig = env
            .storage()
            .instance()
            .get(&DataKey::FeeConfig)
            .unwrap_or_else(|| panic_with_error!(&env, FeeError::NotInitialized));
        calculate_fee(&env, amount, &config)
    }

    /// Get total fees collected.
    pub fn get_total_collected(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::TotalFeesCollected)
            .unwrap_or(0)
    }

    /// Get user fees accrued.
    pub fn get_user_fees_accrued(env: Env, user: Address) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::UserFeesAccrued(user))
            .unwrap_or(0)
    }

    /// Set fee bounds (min/max).
    pub fn set_fee_bounds(env: Env, caller: Address, min_fee: i128, max_fee: i128) {
        caller.require_auth();
        Self::require_admin(&env, &caller);

        if min_fee < 0 || max_fee < 0 {
            panic_with_error!(&env, FeeError::InvalidFeeBound);
        }
        if max_fee < min_fee {
            panic_with_error!(&env, FeeError::InvalidFeeBoundRange);
        }

        env.storage().instance().set(&DataKey::MinFee, &min_fee);
        env.storage().instance().set(&DataKey::MaxFee, &max_fee);
    }

    /// Get minimum fee.
    pub fn get_min_fee(env: Env) -> i128 {
        env.storage().instance().get(&DataKey::MinFee).unwrap_or(0)
    }

    /// Get maximum fee.
    pub fn get_max_fee(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::MaxFee)
            .unwrap_or(i128::MAX)
    }

    /// Update the default fee rate.
    pub fn set_fee_rate(env: Env, caller: Address, fee_rate: u32) {
        caller.require_auth();
        Self::require_admin(&env, &caller);

        if fee_rate > 10_000 {
            panic_with_error!(&env, FeeError::InvalidPercentage);
        }

        let mut config: FeeConfig = env
            .storage()
            .instance()
            .get(&DataKey::FeeConfig)
            .unwrap_or_else(|| panic_with_error!(&env, FeeError::NotInitialized));
        config.default_fee_rate = fee_rate;
        env.storage().instance().set(&DataKey::FeeConfig, &config);

        FeeEvents::config_updated(&env, &caller, fee_rate);
    }

    /// Get the current fee configuration.
    pub fn get_fee_config(env: Env) -> FeeConfig {
        env.storage()
            .instance()
            .get(&DataKey::FeeConfig)
            .unwrap_or_else(|| panic_with_error!(&env, FeeError::NotInitialized))
    }
}

// =============================================================================
// Internal Helpers
// =============================================================================

impl FeeContract {
    fn require_initialized(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| panic_with_error!(env, FeeError::NotInitialized))
    }

    fn require_admin(env: &Env, caller: &Address) {
        let admin = Self::require_initialized(env);
        if caller != &admin {
            panic_with_error!(env, FeeError::Unauthorized);
        }
    }
}

// =============================================================================
// Tests Module
// =============================================================================

#[cfg(test)]
mod test;
