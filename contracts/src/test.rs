#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Events as _},
    Address, Env, Vec,
};

// =============================================================================
// Test Setup
// =============================================================================

fn setup_contract() -> (Env, Address, FeeContract) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(FeeContract, ());
    (env, admin, contract_id)
}

// =============================================================================
// PriorityLevel Tests
// =============================================================================

#[test]
fn test_priority_level_from_u32() {
    assert_eq!(PriorityLevel::from_u32(0), Some(PriorityLevel::Low));
    assert_eq!(PriorityLevel::from_u32(1), Some(PriorityLevel::Medium));
    assert_eq!(PriorityLevel::from_u32(2), Some(PriorityLevel::High));
    assert_eq!(PriorityLevel::from_u32(3), Some(PriorityLevel::Urgent));
    assert_eq!(PriorityLevel::from_u32(4), None);
    assert_eq!(PriorityLevel::from_u32(100), None);
}

#[test]
fn test_priority_level_to_u32() {
    assert_eq!(PriorityLevel::Low.to_u32(), 0);
    assert_eq!(PriorityLevel::Medium.to_u32(), 1);
    assert_eq!(PriorityLevel::High.to_u32(), 2);
    assert_eq!(PriorityLevel::Urgent.to_u32(), 3);
}

#[test]
fn test_priority_level_ordering() {
    assert!(PriorityLevel::Low < PriorityLevel::Medium);
    assert!(PriorityLevel::Medium < PriorityLevel::High);
    assert!(PriorityLevel::High < PriorityLevel::Urgent);
    assert!(PriorityLevel::Low < PriorityLevel::Urgent);
}

#[test]
fn test_priority_level_default() {
    assert_eq!(PriorityLevel::default(), PriorityLevel::Medium);
}

// =============================================================================
// PriorityFeeConfig Tests
// =============================================================================

#[test]
fn test_priority_fee_config_default() {
    let config = PriorityFeeConfig::default();
    
    // Default values should be ascending
    assert_eq!(config.low_multiplier_bps, 8000);
    assert_eq!(config.medium_multiplier_bps, 10000);
    assert_eq!(config.high_multiplier_bps, 15000);
    assert_eq!(config.urgent_multiplier_bps, 20000);
}

#[test]
fn test_priority_fee_config_is_valid() {
    // Valid: ascending order
    let valid_config = PriorityFeeConfig {
        low_multiplier_bps: 5000,
        medium_multiplier_bps: 10000,
        high_multiplier_bps: 15000,
        urgent_multiplier_bps: 20000,
    };
    assert!(valid_config.is_valid());

    // Valid: equal values allowed
    let equal_config = PriorityFeeConfig {
        low_multiplier_bps: 10000,
        medium_multiplier_bps: 10000,
        high_multiplier_bps: 10000,
        urgent_multiplier_bps: 10000,
    };
    assert!(equal_config.is_valid());
}

#[test]
fn test_priority_fee_config_is_invalid() {
    // Invalid: descending order
    let invalid_config = PriorityFeeConfig {
        low_multiplier_bps: 20000,
        medium_multiplier_bps: 15000,
        high_multiplier_bps: 10000,
        urgent_multiplier_bps: 5000,
    };
    assert!(!invalid_config.is_valid());

    // Invalid: high > urgent
    let invalid_config2 = PriorityFeeConfig {
        low_multiplier_bps: 8000,
        medium_multiplier_bps: 10000,
        high_multiplier_bps: 20000,
        urgent_multiplier_bps: 15000,
    };
    assert!(!invalid_config2.is_valid());
}

#[test]
fn test_priority_fee_config_get_multiplier() {
    let config = PriorityFeeConfig::default();
    
    assert_eq!(config.get_multiplier_bps(PriorityLevel::Low), 8000);
    assert_eq!(config.get_multiplier_bps(PriorityLevel::Medium), 10000);
    assert_eq!(config.get_multiplier_bps(PriorityLevel::High), 15000);
    assert_eq!(config.get_multiplier_bps(PriorityLevel::Urgent), 20000);
}

// =============================================================================
// Priority Fee Calculation Tests
// =============================================================================

#[test]
fn test_calculate_priority_fee_rate() {
    let config = PriorityFeeConfig::default();
    let base_rate = 1000u32; // 10%
    
    // Low: 1000 * 8000 / 10000 = 800 (8%)
    assert_eq!(
        calculate_priority_fee_rate(base_rate, PriorityLevel::Low, &config),
        800
    );
    
    // Medium: 1000 * 10000 / 10000 = 1000 (10%)
    assert_eq!(
        calculate_priority_fee_rate(base_rate, PriorityLevel::Medium, &config),
        1000
    );
    
    // High: 1000 * 15000 / 10000 = 1500 (15%)
    assert_eq!(
        calculate_priority_fee_rate(base_rate, PriorityLevel::High, &config),
        1500
    );
    
    // Urgent: 1000 * 20000 / 10000 = 2000 (20%)
    assert_eq!(
        calculate_priority_fee_rate(base_rate, PriorityLevel::Urgent, &config),
        2000
    );
}

#[test]
fn test_calculate_fee_with_priority() {
    let env = Env::default();
    let priority_config = PriorityFeeConfig::default();
    
    let config = FeeConfig {
        default_fee_rate: 500, // 5%
        windows: Vec::new(&env),
        priority_config,
    };
    
    let amount = 10_000i128;
    
    // Low: 5% * 0.8 = 4% => 10000 * 0.04 = 400
    let low_fee = calculate_fee_with_priority(&env, amount, &config, PriorityLevel::Low);
    assert_eq!(low_fee, 400);
    
    // Medium: 5% * 1.0 = 5% => 10000 * 0.05 = 500
    let medium_fee = calculate_fee_with_priority(&env, amount, &config, PriorityLevel::Medium);
    assert_eq!(medium_fee, 500);
    
    // High: 5% * 1.5 = 7.5% => 10000 * 0.075 = 750
    let high_fee = calculate_fee_with_priority(&env, amount, &config, PriorityLevel::High);
    assert_eq!(high_fee, 750);
    
    // Urgent: 5% * 2.0 = 10% => 10000 * 0.10 = 1000
    let urgent_fee = calculate_fee_with_priority(&env, amount, &config, PriorityLevel::Urgent);
    assert_eq!(urgent_fee, 1000);
}

#[test]
fn test_priority_fees_scale_correctly() {
    let env = Env::default();
    let priority_config = PriorityFeeConfig::default();
    
    let config = FeeConfig {
        default_fee_rate: 1000, // 10%
        windows: Vec::new(&env),
        priority_config,
    };
    
    // Test that higher priority always results in higher fees
    let amount = 100_000i128;
    
    let low_fee = calculate_fee_with_priority(&env, amount, &config, PriorityLevel::Low);
    let medium_fee = calculate_fee_with_priority(&env, amount, &config, PriorityLevel::Medium);
    let high_fee = calculate_fee_with_priority(&env, amount, &config, PriorityLevel::High);
    let urgent_fee = calculate_fee_with_priority(&env, amount, &config, PriorityLevel::Urgent);
    
    // Verify ascending order
    assert!(low_fee < medium_fee);
    assert!(medium_fee < high_fee);
    assert!(high_fee < urgent_fee);
    
    // Verify specific values
    assert_eq!(low_fee, 8_000);    // 10% * 0.8 = 8%
    assert_eq!(medium_fee, 10_000); // 10% * 1.0 = 10%
    assert_eq!(high_fee, 15_000);   // 10% * 1.5 = 15%
    assert_eq!(urgent_fee, 20_000); // 10% * 2.0 = 20%
}

// =============================================================================
// Contract Tests
// =============================================================================

#[test]
fn test_contract_initialization() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(FeeContract, ());
    
    // Initialize with 5% fee rate
    FeeContract::initialize(env.clone(), admin.clone(), 500);
    
    let config = FeeContract::get_fee_config(env.clone());
    assert_eq!(config.default_fee_rate, 500);
    
    let priority_config = FeeContract::get_priority_config(env.clone());
    assert_eq!(priority_config.low_multiplier_bps, 8000);
    assert_eq!(priority_config.medium_multiplier_bps, 10000);
    assert_eq!(priority_config.high_multiplier_bps, 15000);
    assert_eq!(priority_config.urgent_multiplier_bps, 20000);
}

#[test]
fn test_set_priority_multipliers() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(FeeContract, ());
    
    FeeContract::initialize(env.clone(), admin.clone(), 500);
    
    // Set custom priority multipliers
    FeeContract::set_priority_multipliers(
        env.clone(),
        admin.clone(),
        5000,   // Low: 0.5x
        10000,  // Medium: 1.0x
        20000,  // High: 2.0x
        30000,  // Urgent: 3.0x
    );
    
    let config = FeeContract::get_priority_config(env.clone());
    assert_eq!(config.low_multiplier_bps, 5000);
    assert_eq!(config.medium_multiplier_bps, 10000);
    assert_eq!(config.high_multiplier_bps, 20000);
    assert_eq!(config.urgent_multiplier_bps, 30000);
}

#[test]
#[should_panic(expected = "InvalidPriorityConfig")]
fn test_set_invalid_priority_multipliers_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(FeeContract, ());
    
    FeeContract::initialize(env.clone(), admin.clone(), 500);
    
    // Try to set invalid multipliers (descending order)
    FeeContract::set_priority_multipliers(
        env.clone(),
        admin.clone(),
        30000,  // Low: 3.0x (higher than urgent!)
        20000,  // Medium: 2.0x
        10000,  // High: 1.0x
        5000,   // Urgent: 0.5x
    );
}

#[test]
fn test_get_priority_multiplier() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(FeeContract, ());
    
    FeeContract::initialize(env.clone(), admin.clone(), 500);
    
    assert_eq!(
        FeeContract::get_priority_multiplier(env.clone(), PriorityLevel::Low),
        8000
    );
    assert_eq!(
        FeeContract::get_priority_multiplier(env.clone(), PriorityLevel::Medium),
        10000
    );
    assert_eq!(
        FeeContract::get_priority_multiplier(env.clone(), PriorityLevel::High),
        15000
    );
    assert_eq!(
        FeeContract::get_priority_multiplier(env.clone(), PriorityLevel::Urgent),
        20000
    );
}

#[test]
fn test_calculate_fee_with_priority_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(FeeContract, ());
    
    FeeContract::initialize(env.clone(), admin.clone(), 1000); // 10% base rate
    
    let amount = 10_000i128;
    
    // Low: 10% * 0.8 = 8% => 800
    let low_fee = FeeContract::calculate_fee_with_priority(
        env.clone(),
        amount,
        PriorityLevel::Low,
    );
    assert_eq!(low_fee, 800);
    
    // Medium: 10% * 1.0 = 10% => 1000
    let medium_fee = FeeContract::calculate_fee_with_priority(
        env.clone(),
        amount,
        PriorityLevel::Medium,
    );
    assert_eq!(medium_fee, 1000);
    
    // High: 10% * 1.5 = 15% => 1500
    let high_fee = FeeContract::calculate_fee_with_priority(
        env.clone(),
        amount,
        PriorityLevel::High,
    );
    assert_eq!(high_fee, 1500);
    
    // Urgent: 10% * 2.0 = 20% => 2000
    let urgent_fee = FeeContract::calculate_fee_with_priority(
        env.clone(),
        amount,
        PriorityLevel::Urgent,
    );
    assert_eq!(urgent_fee, 2000);
}

#[test]
fn test_deduct_fee_with_priority() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let payer = Address::generate(&env);
    let contract_id = env.register(FeeContract, ());
    
    FeeContract::initialize(env.clone(), admin.clone(), 1000); // 10% base rate
    
    let amount = 10_000i128;
    
    // Deduct with High priority (15% fee)
    let (net, fee) = FeeContract::deduct_fee_with_priority(
        env.clone(),
        payer.clone(),
        amount,
        PriorityLevel::High,
    );
    
    assert_eq!(fee, 1500);
    assert_eq!(net, 8500);
    assert_eq!(FeeContract::get_total_collected(env.clone()), 1500);
    assert_eq!(FeeContract::get_user_fees_accrued(env.clone(), payer.clone()), 1500);
}

#[test]
fn test_priority_fee_with_bounds() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(FeeContract, ());
    
    FeeContract::initialize(env.clone(), admin.clone(), 1000);
    
    // Set fee bounds
    FeeContract::set_fee_bounds(env.clone(), admin.clone(), 500, 2000);
    
    // Low priority would calculate to 400 (below min)
    // Should be clamped to min 500
    let low_fee = FeeContract::calculate_fee_with_priority(
        env.clone(),
        5000,
        PriorityLevel::Low,
    );
    assert_eq!(low_fee, 500);
    
    // Urgent priority would calculate to 4000 (above max)
    // Should be clamped to max 2000
    let urgent_fee = FeeContract::calculate_fee_with_priority(
        env.clone(),
        20000,
        PriorityLevel::Urgent,
    );
    assert_eq!(urgent_fee, 2000);
}

#[test]
fn test_priority_fee_events() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(FeeContract, ());
    
    FeeContract::initialize(env.clone(), admin.clone(), 1000);
    
    // Set priority multipliers
    FeeContract::set_priority_multipliers(
        env.clone(),
        admin.clone(),
        5000,
        10000,
        15000,
        20000,
    );
    
    // Check event was emitted
    let events = env.events().all();
    assert!(events.iter().any(|e| e.topics.0 == symbol_short!("fee") 
        && e.topics.1 == symbol_short!("pri_cfg")));
}

// =============================================================================
// Edge Case Tests
// =============================================================================

#[test]
fn test_zero_amount_fee() {
    let env = Env::default();
    let priority_config = PriorityFeeConfig::default();
    
    let config = FeeConfig {
        default_fee_rate: 1000,
        windows: Vec::new(&env),
        priority_config,
    };
    
    // Zero amount should return 0 fee
    let fee = calculate_fee_with_priority(&env, 0, &config, PriorityLevel::Urgent);
    assert_eq!(fee, 0);
    
    // Negative amount should return 0 fee
    let fee = calculate_fee_with_priority(&env, -1000, &config, PriorityLevel::Urgent);
    assert_eq!(fee, 0);
}

#[test]
fn test_large_amount_with_priority() {
    let env = Env::default();
    let priority_config = PriorityFeeConfig::default();
    
    let config = FeeConfig {
        default_fee_rate: 100, // 1%
        windows: Vec::new(&env),
        priority_config,
    };
    
    let large_amount = 1_000_000_000_000i128;
    
    // Urgent: 1% * 2.0 = 2% => 20_000_000_000
    let fee = calculate_fee_with_priority(&env, large_amount, &config, PriorityLevel::Urgent);
    assert_eq!(fee, 20_000_000_000);
}

#[test]
fn test_custom_priority_multipliers() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(FeeContract, ());
    
    FeeContract::initialize(env.clone(), admin.clone(), 1000);
    
    // Set custom multipliers with larger spread
    FeeContract::set_priority_multipliers(
        env.clone(),
        admin.clone(),
        2500,   // Low: 0.25x (75% discount)
        10000,  // Medium: 1.0x
        25000,  // High: 2.5x (150% premium)
        50000,  // Urgent: 5.0x (400% premium)
    );
    
    let amount = 10_000i128;
    
    // Low: 10% * 0.25 = 2.5% => 250
    let low_fee = FeeContract::calculate_fee_with_priority(
        env.clone(),
        amount,
        PriorityLevel::Low,
    );
    assert_eq!(low_fee, 250);
    
    // Urgent: 10% * 5.0 = 50% => 5000
    let urgent_fee = FeeContract::calculate_fee_with_priority(
        env.clone(),
        amount,
        PriorityLevel::Urgent,
    );
    assert_eq!(urgent_fee, 5000);
}

#[test]
fn test_multiple_priority_transactions() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let payer = Address::generate(&env);
    let contract_id = env.register(FeeContract, ());
    
    FeeContract::initialize(env.clone(), admin.clone(), 1000);
    
    // Execute transactions with different priorities
    let (_, low_fee) = FeeContract::deduct_fee_with_priority(
        env.clone(),
        payer.clone(),
        10_000,
        PriorityLevel::Low,
    );
    assert_eq!(low_fee, 800);
    
    let (_, medium_fee) = FeeContract::deduct_fee_with_priority(
        env.clone(),
        payer.clone(),
        10_000,
        PriorityLevel::Medium,
    );
    assert_eq!(medium_fee, 1000);
    
    let (_, high_fee) = FeeContract::deduct_fee_with_priority(
        env.clone(),
        payer.clone(),
        10_000,
        PriorityLevel::High,
    );
    assert_eq!(high_fee, 1500);
    
    let (_, urgent_fee) = FeeContract::deduct_fee_with_priority(
        env.clone(),
        payer.clone(),
        10_000,
        PriorityLevel::Urgent,
    );
    assert_eq!(urgent_fee, 2000);
    
    // Total collected should be sum of all fees
    assert_eq!(FeeContract::get_total_collected(env.clone()), 5300);
    
    // User fees accrued should match
    assert_eq!(FeeContract::get_user_fees_accrued(env.clone(), payer.clone()), 5300);
}
