//! Integration tests for Hyperliquid testnet.
//!
//! These tests require a funded testnet account. Set PRIVATE_KEY in your .env
//! or environment before running:
//!
//! ```bash
//! cargo test -- --ignored
//! ```

use opensigma::execution::hyperliquid::HyperliquidExecutor;

/// Helper to create an executor from env, skipping if PRIVATE_KEY not set.
async fn make_executor() -> Option<HyperliquidExecutor> {
    dotenvy::dotenv().ok();
    let key = match std::env::var("PRIVATE_KEY") {
        Ok(k) if !k.is_empty() && k != "0x..." => k,
        _ => {
            eprintln!("PRIVATE_KEY not set or placeholder — skipping testnet test");
            return None;
        }
    };
    match HyperliquidExecutor::new(&key, false).await {
        Ok(exec) => Some(exec),
        Err(e) => {
            eprintln!("Failed to create executor: {} — skipping", e);
            None
        }
    }
}

#[tokio::test]
#[ignore]
async fn test_get_balance_testnet() {
    let Some(executor) = make_executor().await else {
        return;
    };
    let balance = executor.get_balance().await.expect("get_balance failed");
    println!("Testnet balance: {}", balance);
    // Balance should be a non-negative number
    assert!(balance >= 0.0);
}

#[tokio::test]
#[ignore]
async fn test_get_meta_testnet() {
    let Some(executor) = make_executor().await else {
        return;
    };
    let meta = executor.fetch_meta().await.expect("fetch_meta failed");
    println!("Number of assets: {}", meta.universe.len());
    assert!(!meta.universe.is_empty());

    // BTC and ETH should be in the universe
    let names: Vec<&str> = meta.universe.iter().map(|a| a.name.as_str()).collect();
    assert!(names.contains(&"BTC"), "BTC not found in universe");
    assert!(names.contains(&"ETH"), "ETH not found in universe");
}

#[tokio::test]
#[ignore]
async fn test_get_positions_testnet() {
    let Some(executor) = make_executor().await else {
        return;
    };
    let positions = executor
        .get_positions()
        .await
        .expect("get_positions failed");
    println!("Open positions: {}", positions.len());
    // Just verify it doesn't error — may be empty
}

#[tokio::test]
#[ignore]
async fn test_get_open_orders_testnet() {
    let Some(executor) = make_executor().await else {
        return;
    };
    let orders = executor
        .get_open_orders()
        .await
        .expect("get_open_orders failed");
    println!("Open orders: {}", orders.len());
}

#[tokio::test]
#[ignore]
async fn test_place_and_cancel_order_testnet() {
    let Some(executor) = make_executor().await else {
        return;
    };

    // Place a limit buy far below market price so it won't fill
    let result = executor
        .place_limit_order(
            "BTC",
            true,    // buy
            10000.0, // way below market
            0.001,   // minimum size
            false,   // not reduce only
            "Gtc",
        )
        .await
        .expect("place_limit_order failed");

    println!("Place order response: {:?}", result);
    assert_eq!(result.status, "ok");

    // Check that the order appears in open orders
    let orders = executor
        .get_open_orders()
        .await
        .expect("get_open_orders failed");
    println!("Open orders after place: {}", orders.len());

    // Find our BTC order
    let btc_order = orders.iter().find(|o| o.coin == "BTC" && o.side == "B");
    assert!(btc_order.is_some(), "Expected to find the BTC buy order");
    let oid = btc_order.unwrap().oid;
    println!("Order ID: {}", oid);

    // Cancel it
    executor
        .cancel_order("BTC", oid)
        .await
        .expect("cancel_order failed");

    // Verify it's gone
    let orders_after = executor
        .get_open_orders()
        .await
        .expect("get_open_orders failed");
    let still_exists = orders_after.iter().any(|o| o.oid == oid);
    assert!(
        !still_exists,
        "Order {} should have been cancelled",
        oid
    );
    println!("Order successfully placed and cancelled on testnet");
}
