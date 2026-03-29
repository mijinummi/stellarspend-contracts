#![allow(unused)]


pub mod storage;
//! StellarSpend fee contract crate root: re-exports the fee contract and contract metrics types.

pub mod auth;
pub mod fee;

pub use fee::*;

#[cfg(test)]
mod test;

#[cfg(test)]
mod contract_metrics_tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Address, Env};

    #[test]
    fn contract_metrics_total_matches_get_total_collected() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let _id = env.register(FeeContract, ());
        FeeContract::initialize(env.clone(), admin.clone(), 500);
        let payer = Address::generate(&env);

        let m0 = FeeContract::get_contract_metrics(env.clone());
        assert_eq!(m0.total_fees_collected, 0);
        assert_eq!(m0.default_fee_rate_bps, 500);

        FeeContract::deduct_fee_with_priority(
            env.clone(),
            payer.clone(),
            1000,
            PriorityLevel::Medium,
        );

        let m1 = FeeContract::get_contract_metrics(env.clone());
        assert_eq!(
            m1.total_fees_collected,
            FeeContract::get_total_collected(env.clone())
        );
        assert_eq!(m1.total_fees_collected, 50);
    }
}
