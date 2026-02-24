use crate::roundup::execute_roundup;

match msg {
    ExecuteMsg::RoundUp { transaction_amount } => {
        execute_roundup(deps, env, info, transaction_amount)
    }
}