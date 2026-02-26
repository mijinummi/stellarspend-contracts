#![cfg(test)]

use soroban_sdk::{Env, Address};
use crate::DependencyContract;

#[test]
fn test_simple_dependency_flow() {
    let env = Env::default();
    let contract_id = env.register_contract(None, DependencyContract);
    let client = DependencyContractClient::new(&env, &contract_id);

    let user = Address::generate(&env);

    // Create base transaction
    let tx1 = client.create_transaction(&user, &None);

    // Create dependent transaction
    let tx2 = client.create_transaction(&user, &Some(tx1));

    // Attempt execution of dependent first (should panic)
    let result = std::panic::catch_unwind(|| {
        client.execute_transaction(&tx2);
    });

    assert!(result.is_err());

    // Complete base transaction
    client.execute_transaction(&tx1);
    assert!(client.is_completed(&tx1));

    // Now execute dependent
    client.execute_transaction(&tx2);
    assert!(client.is_completed(&tx2));
}

#[test]
fn test_no_dependency_execution() {
    let env = Env::default();
    let contract_id = env.register_contract(None, DependencyContract);
    let client = DependencyContractClient::new(&env, &contract_id);

    let user = Address::generate(&env);

    let tx = client.create_transaction(&user, &None);

    client.execute_transaction(&tx);

    assert!(client.is_completed(&tx));
}