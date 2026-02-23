#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype,
    Address, Env, Vec, Symbol
};

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    UserWallets(Address),
    WalletOwner(Address),
}

#[contract]
pub struct WalletLinkingContract;

#[contractimpl]
impl WalletLinkingContract {

    // 🔗 Link wallet to user identity
    pub fn link_wallet(env: Env, user: Address, wallet: Address) {
        // Require user auth
        user.require_auth();

        // Validate wallet not already linked
        if env.storage().instance().has(&DataKey::WalletOwner(wallet.clone())) {
            panic!("Wallet already linked");
        }

        // Store wallet → user
        env.storage()
            .instance()
            .set(&DataKey::WalletOwner(wallet.clone()), &user);

        // Update user wallet list
        let mut wallets: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::UserWallets(user.clone()))
            .unwrap_or(Vec::new(&env));

        wallets.push_back(wallet.clone());

        env.storage()
            .instance()
            .set(&DataKey::UserWallets(user.clone()), &wallets);

        // Emit event
        env.events().publish(
            (Symbol::new(&env, "wallet_linked"), user.clone()),
            wallet
        );
    }

    // ❌ Unlink wallet
    pub fn unlink_wallet(env: Env, user: Address, wallet: Address) {
        user.require_auth();

        let owner: Address = env.storage()
            .instance()
            .get(&DataKey::WalletOwner(wallet.clone()))
            .expect("Wallet not linked");

        if owner != user {
            panic!("Unauthorized unlink attempt");
        }

        // Remove wallet ownership
        env.storage()
            .instance()
            .remove(&DataKey::WalletOwner(wallet.clone()));

        // Remove from user wallet list
        let mut wallets: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::UserWallets(user.clone()))
            .unwrap_or(Vec::new(&env));

        wallets.retain(|w| w != wallet);

        env.storage()
            .instance()
            .set(&DataKey::UserWallets(user.clone()), &wallets);

        env.events().publish(
            (Symbol::new(&env, "wallet_unlinked"), user),
            wallet
        );
    }

    // 📖 Get all wallets for a user
    pub fn get_wallets(env: Env, user: Address) -> Vec<Address> {
        env.storage()
            .instance()
            .get(&DataKey::UserWallets(user))
            .unwrap_or(Vec::new(&env))
    }

    // 📖 Get owner of wallet
    pub fn get_wallet_owner(env: Env, wallet: Address) -> Option<Address> {
        env.storage()
            .instance()
            .get(&DataKey::WalletOwner(wallet))
    }
}