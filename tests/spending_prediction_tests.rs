#![cfg(test)]
use soroban_sdk::{Env, Address};
use crate::PredictionEngine;

#[test]
fn test_prediction_basic() {
    let env = Env::default();
    let user = Address::random(&env);

    // Add transactions
    PredictionEngine::add_transaction(env.clone(), user.clone(), 100, 1);
    PredictionEngine::add_transaction(env.clone(), user.clone(), 200, 2);
    PredictionEngine::add_transaction(env.clone(), user.clone(), 300, 3);

    // Prediction should be average
    let projected = PredictionEngine::predict_spending(env.clone(), user.clone());
    assert_eq!(projected, 200);

    // Check transaction storage
    let txs = PredictionEngine::get_transactions(env.clone(), user.clone());
    assert_eq!(txs.len(), 3);
}

#[test]
fn test_storage_limit() {
    let env = Env::default();
    let user = Address::random(&env);

    // Add 60 transactions
    for i in 0..60 {
        PredictionEngine::add_transaction(env.clone(), user.clone(), i, i as u64);
    }

    // Only last 50 should remain
    let txs = PredictionEngine::get_transactions(env.clone(), user.clone());
    assert_eq!(txs.len(), 50);
    assert_eq!(txs[0].amount, 10); // first 10 removed
}