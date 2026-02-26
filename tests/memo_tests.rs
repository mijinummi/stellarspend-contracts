#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Address, Bytes, Env, Symbol, String,
};

#[path = "../contracts/memo.rs"]
mod memo;

use memo::{MemoContract, MemoContractClient};

fn setup_env() -> Env {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set(LedgerInfo {
        timestamp: 1_700_000_000,
        protocol_version: 22,
        sequence_number: 1,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 16,
        min_persistent_entry_ttl: 4096,
        max_entry_ttl: 6_312_000,
    });
    env
}

fn deploy(env: &Env) -> (MemoContractClient<'_>, Address) {
    let admin = Address::generate(env);
    let contract_id = env.register(MemoContract, ());
    let client = MemoContractClient::new(env, &contract_id);
    (client, admin)
}

fn tx_id(env: &Env, s: &str) -> Bytes {
    let mut b = Bytes::new(env);
    b.extend_from_slice(s.as_bytes());
    b
}

#[test]
fn test_initialize() {
    let env = setup_env();
    let (client, admin) = deploy(&env);
    client.initialize(&admin, &32_u32, &64_u32, &256_u32);
    assert_eq!(client.get_admin(), Some(admin.clone()));
    let limits = client.get_limits().unwrap();
    assert_eq!(limits.0, 32);
    assert_eq!(limits.1, 64);
    assert_eq!(limits.2, 256);
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn test_double_initialize_fails() {
    let env = setup_env();
    let (client, admin) = deploy(&env);
    client.initialize(&admin, &32_u32, &64_u32, &256_u32);
    client.initialize(&admin, &16_u32, &32_u32, &128_u32);
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn test_initialize_zero_memo_type_len_fails() {
    let env = setup_env();
    let (client, admin) = deploy(&env);
    client.initialize(&admin, &0_u32, &64_u32, &256_u32);
}

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn test_initialize_excessive_limits_fail() {
    let env = setup_env();
    let (client, admin) = deploy(&env);
    client.initialize(&admin, &100_u32, &64_u32, &256_u32);
}

#[test]
fn test_set_and_get_memo() {
    let env = setup_env();
    let (client, admin) = deploy(&env);
    client.initialize(&admin, &32_u32, &64_u32, &256_u32);
    let id = tx_id(&env, "tx-1");
    let memo_type = Symbol::new(&env, "payment");
    let reference = String::from_str(&env, "ref-123");
    let text = String::from_str(&env, "Payment for services");
    client.set_memo(&admin, &id, &memo_type, &reference, &text);
    let memo = client.get_memo(&id).unwrap();
    assert_eq!(memo.memo_type, memo_type);
    assert_eq!(memo.reference, reference);
    assert_eq!(memo.text, text);
    assert_eq!(memo.created_at, 1_700_000_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn test_set_memo_empty_type_fails() {
    let env = setup_env();
    let (client, admin) = deploy(&env);
    client.initialize(&admin, &32_u32, &64_u32, &256_u32);
    let id = tx_id(&env, "tx-2");
    let memo_type = Symbol::new(&env, "");
    let reference = String::from_str(&env, "");
    let text = String::from_str(&env, "Some text");
    client.set_memo(&admin, &id, &memo_type, &reference, &text);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn test_set_memo_empty_text_fails() {
    let env = setup_env();
    let (client, admin) = deploy(&env);
    client.initialize(&admin, &32_u32, &64_u32, &256_u32);
    let id = tx_id(&env, "tx-3");
    let memo_type = Symbol::new(&env, "note");
    let reference = String::from_str(&env, "");
    let text = String::from_str(&env, "");
    client.set_memo(&admin, &id, &memo_type, &reference, &text);
}

#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn test_set_memo_unauthorized() {
    let env = setup_env();
    let (client, admin) = deploy(&env);
    client.initialize(&admin, &32_u32, &64_u32, &256_u32);
    let other = Address::generate(&env);
    let id = tx_id(&env, "tx-4");
    let memo_type = Symbol::new(&env, "payment");
    let reference = String::from_str(&env, "");
    let text = String::from_str(&env, "Text");
    client.set_memo(&other, &id, &memo_type, &reference, &text);
}

#[test]
fn test_get_memo_none_for_unknown_tx() {
    let env = setup_env();
    let (client, admin) = deploy(&env);
    client.initialize(&admin, &32_u32, &64_u32, &256_u32);
    let id = tx_id(&env, "nonexistent");
    assert!(client.get_memo(&id).is_none());
}

#[test]
fn test_memo_respects_configured_limits() {
    let env = setup_env();
    let (client, admin) = deploy(&env);
    client.initialize(&admin, &8_u32, &4_u32, &16_u32);
    let id = tx_id(&env, "tx-5");
    let memo_type = Symbol::new(&env, "refund");
    let reference = String::from_str(&env, "r1");
    let text = String::from_str(&env, "Refund issued");
    client.set_memo(&admin, &id, &memo_type, &reference, &text);
    let memo = client.get_memo(&id).unwrap();
    assert_eq!(memo.text, String::from_str(&env, "Refund issued"));
}

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn test_memo_text_over_limit_fails() {
    let env = setup_env();
    let (client, admin) = deploy(&env);
    client.initialize(&admin, &32_u32, &64_u32, &10_u32);
    let id = tx_id(&env, "tx-6");
    let memo_type = Symbol::new(&env, "x");
    let reference = String::from_str(&env, "");
    let text = String::from_str(&env, "way too long text");
    client.set_memo(&admin, &id, &memo_type, &reference, &text);
}
