#![no_std]

use soroban_sdk::{
    contracttype, contractimpl, contractevent, Address, Env, Symbol, Vec, Map
};

#[contracttype]
#[derive(Clone)]
pub struct Transaction {
    pub id: u64,
    pub creator: Address,
    pub dependency: Option<u64>,
    pub completed: bool,
}

#[contracttype]
pub enum DataKey {
    Transaction(u64),
    TransactionCount,
}

#[contractevent]
pub struct DependencyBlocked {
    pub tx_id: u64,
    pub missing_dependency: u64,
}

#[contractevent]
pub struct DependencyResolved {
    pub tx_id: u64,
}

pub struct DependencyContract;

#[contractimpl]
impl DependencyContract {

    // Create new transaction with optional dependency
    pub fn create_transaction(
        env: Env,
        creator: Address,
        dependency: Option<u64>,
    ) -> u64 {

        creator.require_auth();

        let mut count: u64 = env
            .storage()
            .instance()
            .get(&DataKey::TransactionCount)
            .unwrap_or(0);

        count += 1;

        let tx = Transaction {
            id: count,
            creator,
            dependency,
            completed: false,
        };

        env.storage()
            .instance()
            .set(&DataKey::Transaction(count), &tx);

        env.storage()
            .instance()
            .set(&DataKey::TransactionCount, &count);

        count
    }

    // Execute transaction (marks completed if dependency satisfied)
    pub fn execute_transaction(env: Env, tx_id: u64) {

        let mut tx: Transaction = env
            .storage()
            .instance()
            .get(&DataKey::Transaction(tx_id))
            .expect("Transaction not found");

        if tx.completed {
            return;
        }

        if let Some(dep_id) = tx.dependency {
            let dep_tx: Transaction = env
                .storage()
                .instance()
                .get(&DataKey::Transaction(dep_id))
                .expect("Dependency not found");

            if !dep_tx.completed {
                env.events().publish(
                    (Symbol::new(&env, "dependency_blocked"),),
                    DependencyBlocked {
                        tx_id,
                        missing_dependency: dep_id,
                    },
                );
                panic!("Dependency not completed");
            }
        }

        tx.completed = true;

        env.storage()
            .instance()
            .set(&DataKey::Transaction(tx_id), &tx);

        env.events().publish(
            (Symbol::new(&env, "dependency_resolved"),),
            DependencyResolved { tx_id },
        );
    }

    pub fn is_completed(env: Env, tx_id: u64) -> bool {
        let tx: Transaction = env
            .storage()
            .instance()
            .get(&DataKey::Transaction(tx_id))
            .expect("Transaction not found");

        tx.completed
    }
}