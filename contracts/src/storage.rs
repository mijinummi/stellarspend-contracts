use soroban_sdk::{contracttype, Address, Env, Vec};

#[derive(Clone)]
#[contracttype]
pub enum FeeLogKind {
    Charge,
    Refund,
}

#[derive(Clone)]
#[contracttype]
pub struct FeeLog {
    pub id: u64,
    pub payer: Option<Address>,
    pub gross_amount: i128,
    pub fee_amount: i128,
    pub timestamp: u64,
    pub ledger_sequence: u32,
    pub kind: FeeLogKind,
}

#[derive(Clone)]
#[contracttype]
pub enum DataKey {
    FeeLogCount,
    FeeLog(u64),
}

pub fn append_fee_log(
    env: &Env,
    payer: Option<Address>,
    gross_amount: i128,
    fee_amount: i128,
    kind: FeeLogKind,
) -> FeeLog {
    let mut count: u64 = env
        .storage()
        .persistent()
        .get(&DataKey::FeeLogCount)
        .unwrap_or(0);

    count += 1;

    let fee_log = FeeLog {
        id: count,
        payer,
        gross_amount,
        fee_amount,
        timestamp: env.ledger().timestamp(),
        ledger_sequence: env.ledger().sequence(),
        kind,
    };

    env.storage()
        .persistent()
        .set(&DataKey::FeeLog(count), &fee_log);
    env.storage()
        .persistent()
        .set(&DataKey::FeeLogCount, &count);

    fee_log
}

pub fn get_fee_log(env: &Env, id: u64) -> Option<FeeLog> {
    env.storage().persistent().get(&DataKey::FeeLog(id))
}

pub fn get_fee_log_count(env: &Env) -> u64 {
    env.storage()
        .persistent()
        .get(&DataKey::FeeLogCount)
        .unwrap_or(0)
}

pub fn get_fee_logs(env: &Env, start: u64, end: u64) -> Vec<FeeLog> {
    if start == 0 || end < start {
        return Vec::new(env);
    }

    let mut logs = Vec::new(env);
    let total = get_fee_log_count(env);
    let capped_end = if end > total { total } else { end };

    for id in start..=capped_end {
        if let Some(log) = get_fee_log(env, id) {
            logs.push_back(log);
        }
    }

    logs
}
