use cosmwasm_std::Uint128;
use cw_storage_plus::Map;

pub const BALANCES: Map<&str, Uint128> = Map::new("balances");
pub const SAVINGS_GOALS: Map<&str, Uint128> = Map::new("savings_goals");