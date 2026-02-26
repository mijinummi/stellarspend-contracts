#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype, contracterror, panic_with_error, symbol_short,
    Address, Bytes, Env, Symbol, String,
};

const MAX_MEMO_TYPE_LEN: u32 = 32;
const MAX_REFERENCE_LEN: u32 = 64;
const MAX_TEXT_LEN: u32 = 256;
const MAX_TOTAL_MEMO_BYTES: u32 = 320;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum MemoError {
    NotInitialized = 1,
    Unauthorized = 2,
    InvalidMemoType = 3,
    InvalidReference = 4,
    InvalidText = 5,
    MemoTooLarge = 6,
    MalformedMetadata = 7,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Memo {
    pub memo_type: Symbol,
    pub reference: String,
    pub text: String,
    pub created_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Admin,
    MaxMemoTypeLen,
    MaxReferenceLen,
    MaxTextLen,
    Memo(Bytes),
}

#[contract]
pub struct MemoContract;

#[contractimpl]
impl MemoContract {
    pub fn initialize(
        env: Env,
        admin: Address,
        max_memo_type_len: u32,
        max_reference_len: u32,
        max_text_len: u32,
    ) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic_with_error!(&env, MemoError::MalformedMetadata);
        }
        admin.require_auth();
        if max_memo_type_len == 0 || max_text_len == 0 {
            panic_with_error!(&env, MemoError::MalformedMetadata);
        }
        if max_memo_type_len > 64 || max_reference_len > 128 || max_text_len > 1024 {
            panic_with_error!(&env, MemoError::MemoTooLarge);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::MaxMemoTypeLen, &max_memo_type_len);
        env.storage().instance().set(&DataKey::MaxReferenceLen, &max_reference_len);
        env.storage().instance().set(&DataKey::MaxTextLen, &max_text_len);
        env.events().publish(
            (symbol_short!("memo"), symbol_short!("init")),
            (admin, max_memo_type_len, max_reference_len, max_text_len),
        );
    }

    pub fn set_memo(
        env: Env,
        caller: Address,
        transaction_id: Bytes,
        memo_type: Symbol,
        reference: String,
        text: String,
    ) {
        caller.require_auth();
        Self::require_admin(&env, &caller);
        Self::validate_memo(&env, &memo_type, &reference, &text);
        let created_at = env.ledger().timestamp();
        let memo = Memo {
            memo_type: memo_type.clone(),
            reference: reference.clone(),
            text: text.clone(),
            created_at,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Memo(transaction_id.clone()), &memo);
        env.events().publish(
            (symbol_short!("memo"), symbol_short!("stored")),
            (transaction_id, memo_type, created_at),
        );
    }

    pub fn get_memo(env: Env, transaction_id: Bytes) -> Option<Memo> {
        env.storage().persistent().get(&DataKey::Memo(transaction_id))
    }

    pub fn get_admin(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::Admin)
    }

    pub fn get_limits(
        env: Env,
    ) -> Option<(u32, u32, u32)> {
        let a = env.storage().instance().get(&DataKey::MaxMemoTypeLen)?;
        let b = env.storage().instance().get(&DataKey::MaxReferenceLen)?;
        let c = env.storage().instance().get(&DataKey::MaxTextLen)?;
        Some((a, b, c))
    }

    fn require_admin(env: &Env, caller: &Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| panic_with_error!(env, MemoError::NotInitialized));
        if caller != &admin {
            panic_with_error!(env, MemoError::Unauthorized);
        }
    }

    fn validate_memo(
        env: &Env,
        memo_type: &Symbol,
        reference: &String,
        text: &String,
    ) {
        let max_type: u32 = env
            .storage()
            .instance()
            .get(&DataKey::MaxMemoTypeLen)
            .unwrap_or(MAX_MEMO_TYPE_LEN);
        let max_ref: u32 = env
            .storage()
            .instance()
            .get(&DataKey::MaxReferenceLen)
            .unwrap_or(MAX_REFERENCE_LEN);
        let max_text: u32 = env
            .storage()
            .instance()
            .get(&DataKey::MaxTextLen)
            .unwrap_or(MAX_TEXT_LEN);

        let type_str = memo_type.to_string();
        if type_str.len() == 0 {
            panic_with_error!(env, MemoError::InvalidMemoType);
        }
        if type_str.len() as u32 > max_type {
            panic_with_error!(env, MemoError::MemoTooLarge);
        }
        if reference.len() as u32 > max_ref {
            panic_with_error!(env, MemoError::InvalidReference);
        }
        if text.len() == 0 {
            panic_with_error!(env, MemoError::InvalidText);
        }
        if text.len() as u32 > max_text {
            panic_with_error!(env, MemoError::MemoTooLarge);
        }

        let total: u32 = type_str.len() as u32 + reference.len() as u32 + text.len() as u32;
        if total > MAX_TOTAL_MEMO_BYTES {
            panic_with_error!(env, MemoError::MemoTooLarge);
        }
    }
}
