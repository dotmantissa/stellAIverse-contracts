#![cfg(test)]

use soroban_sdk::{testutils::{Ledger as _, Address as _}, token, Address, Env};
use stellai_lib::AuctionType;

use crate::{Marketplace, MarketplaceClient};

// ── helpers ────────────────────────────────────────────────────────────────

fn setup() -> (Env, MarketplaceClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let token_address = env.register_stellar_asset_contract(admin.clone());
    let id = env.register_contract(None, Marketplace);
    let client = MarketplaceClient::new(&env, &id);
    client.initialize(&admin, &token_address, &250);
    (env, client, token_address)
}

fn create_english_auction(env: &Env, client: &MarketplaceClient, seller: &Address) -> u64 {
    client.create_auction(
        &1u64,          // agent_id
        seller,
        &AuctionType::English,
        &1_000i128,     // start_price
        &500i128,       // reserve_price
        &3_600u64,      // duration (1 hour)
        &0u32,          // min_bid_increment_bps (no enforced minimum)
    )
}

// ── tests ──────────────────────────────────────────────────────────────────

#[test]
fn test_first_bid_recorded_with_zero_increment() {
    let (env, client, token_address) = setup();
    let seller = Address::generate(&env);
    let bidder = Address::generate(&env);
    let token_client = token::StellarAssetClient::new(&env, &token_address);
    token_client.mint(&bidder, &10_000);

    let auction_id = create_english_auction(&env, &client, &seller);

    client.place_bid(&auction_id, &bidder, &1_000i128);

    let count = client.get_bid_count(&auction_id);
    assert_eq!(count, 1);

    let entry = client.get_bid_history_entry_at(&auction_id, &0).unwrap();
    assert_eq!(entry.bidder, bidder);
    assert_eq!(entry.amount, 1_000);
    assert_eq!(entry.bid_increment, 0, "first bid increment must be 0");
    assert_eq!(entry.sequence, 1);
}

#[test]
fn test_multiple_bids_increment_and_sequence() {
    let (env, client, token_address) = setup();
    let seller = Address::generate(&env);
    let b1 = Address::generate(&env);
    let b2 = Address::generate(&env);
    let b3 = Address::generate(&env);
    let token_client = token::StellarAssetClient::new(&env, &token_address);
    token_client.mint(&b1, &10_000);
    token_client.mint(&b2, &10_000);
    token_client.mint(&b3, &10_000);

    let auction_id = create_english_auction(&env, &client, &seller);

    client.place_bid(&auction_id, &b1, &1_000i128);
    client.place_bid(&auction_id, &b2, &2_000i128);
    client.place_bid(&auction_id, &b3, &3_500i128);

    let count = client.get_bid_count(&auction_id);
    assert_eq!(count, 3);

    let history = client.get_bid_history(&auction_id);
    assert_eq!(history.len(), 3);

    // First bid
    assert_eq!(history.get(0).unwrap().sequence, 1);
    assert_eq!(history.get(0).unwrap().amount, 1_000);
    assert_eq!(history.get(0).unwrap().bid_increment, 0);

    // Second bid: +1_000 above first
    assert_eq!(history.get(1).unwrap().sequence, 2);
    assert_eq!(history.get(1).unwrap().amount, 2_000);
    assert_eq!(history.get(1).unwrap().bid_increment, 1_000);

    // Third bid: +1_500 above second
    assert_eq!(history.get(2).unwrap().sequence, 3);
    assert_eq!(history.get(2).unwrap().amount, 3_500);
    assert_eq!(history.get(2).unwrap().bid_increment, 1_500);
}

#[test]
fn test_bid_history_preserves_bidder_identity() {
    let (env, client, token_address) = setup();
    let seller = Address::generate(&env);
    let b1 = Address::generate(&env);
    let b2 = Address::generate(&env);
    let token_client = token::StellarAssetClient::new(&env, &token_address);
    token_client.mint(&b1, &10_000);
    token_client.mint(&b2, &10_000);

    let auction_id = create_english_auction(&env, &client, &seller);

    client.place_bid(&auction_id, &b1, &1_000i128);
    client.place_bid(&auction_id, &b2, &2_000i128);

    let history = client.get_bid_history(&auction_id);
    assert_eq!(history.get(0).unwrap().bidder, b1);
    assert_eq!(history.get(1).unwrap().bidder, b2);
}

#[test]
fn test_bid_history_timestamps_non_decreasing() {
    let (env, client, token_address) = setup();
    let seller = Address::generate(&env);
    let b1 = Address::generate(&env);
    let b2 = Address::generate(&env);
    let token_client = token::StellarAssetClient::new(&env, &token_address);
    token_client.mint(&b1, &10_000);
    token_client.mint(&b2, &10_000);

    let auction_id = create_english_auction(&env, &client, &seller);

    client.place_bid(&auction_id, &b1, &1_000i128);

    // Advance time
    env.ledger().set_timestamp(env.ledger().timestamp() + 60);

    client.place_bid(&auction_id, &b2, &2_000i128);

    let history = client.get_bid_history(&auction_id);
    assert!(
        history.get(1).unwrap().timestamp >= history.get(0).unwrap().timestamp,
        "timestamps must be non-decreasing"
    );
}

#[test]
fn test_no_bids_returns_empty_history() {
    let (env, client, _token_address) = setup();
    let seller = Address::generate(&env);

    let auction_id = create_english_auction(&env, &client, &seller);

    assert_eq!(client.get_bid_count(&auction_id), 0);
    assert_eq!(client.get_bid_history(&auction_id).len(), 0);
    assert!(client.get_bid_history_entry_at(&auction_id, &0).is_none());
}
