use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Env, Symbol,
};

use category_analytics::{
    CategoryAnalytics, CategoryAnalyticsClient,
};

fn setup_analytics_contract() -> (Env, Address, CategoryAnalyticsClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(CategoryAnalytics, ());
    let client = CategoryAnalyticsClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.init(&admin);

    (env, admin, client)
}

#[test]
fn test_integration_spending_trends() {
    let (env, _admin, client) = setup_analytics_contract();
    let user = Address::generate(&env);
    let category = Symbol::new(&env, "shopping");

    // Year/Month calculation in contract:
    // 2026: 1768608000
    env.ledger().set_timestamp(1768608000);

    client.record_spending(&user, &category, &5000);
    client.record_spending(&user, &category, &3000);

    let metrics = client.get_category_metrics(&user, &category, &2026, &2);
    assert_eq!(metrics.volume, 8000);
    assert_eq!(metrics.count, 2);

    // Advance time to March (approx 30 days)
    env.ledger().set_timestamp(1768608000 + 2592000);
    client.record_spending(&user, &category, &2000);

    let march_metrics = client.get_category_metrics(&user, &category, &2026, &3);
    assert_eq!(march_metrics.volume, 2000);

    let yearly = client.get_yearly_trend(&user, &category, &2026);
    assert_eq!(yearly.volume, 10000);
    assert_eq!(yearly.count, 3);
}
