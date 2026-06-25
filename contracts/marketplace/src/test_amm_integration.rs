/// Tests for AMM integration with the marketplace (Issue #245).
///
/// Covers acceptance criteria:
/// - Users can pay with any token listed in the AMM
/// - Swaps are executed atomically during purchase
/// - Slippage protection for large swaps
/// - Transaction fails if swap can't complete within limits
/// - Slippage configuration per user
/// - Multi-token payment scenarios — successful and failed swaps
#![cfg(test)]

use soroban_sdk::{
    contract, contractimpl, contracttype,
    testutils::Address as _,
    token,
    Address, Env, Symbol,
};

use crate::{MarketplaceContract, MarketplaceContractClient};

// ─────────────────────────────────────────────────────────────────────────────
// Minimal mock token (SEP-41 compatible subset used in tests)
// ─────────────────────────────────────────────────────────────────────────────

#[contract]
pub struct MockToken;

#[contracttype]
#[derive(Clone)]
pub enum MockTokenKey {
    Balance(Address),
    Allowance(Address, Address), // (owner, spender)
}

#[contractimpl]
impl MockToken {
    pub fn mint(env: Env, to: Address, amount: i128) {
        let key = MockTokenKey::Balance(to.clone());
        let bal: i128 = env.storage().instance().get(&key).unwrap_or(0);
        env.storage().instance().set(&key, &(bal + amount));
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        env.storage()
            .instance()
            .get(&MockTokenKey::Balance(id))
            .unwrap_or(0)
    }

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        let from_key = MockTokenKey::Balance(from.clone());
        let from_bal: i128 = env.storage().instance().get(&from_key).unwrap_or(0);
        assert!(from_bal >= amount, "Insufficient balance");
        env.storage().instance().set(&from_key, &(from_bal - amount));

        let to_key = MockTokenKey::Balance(to.clone());
        let to_bal: i128 = env.storage().instance().get(&to_key).unwrap_or(0);
        env.storage().instance().set(&to_key, &(to_bal + amount));
    }

    pub fn approve(env: Env, owner: Address, spender: Address, amount: i128, _expiry: u32) {
        owner.require_auth();
        env.storage()
            .instance()
            .set(&MockTokenKey::Allowance(owner, spender), &amount);
    }

    pub fn allowance(env: Env, owner: Address, spender: Address) -> i128 {
        env.storage()
            .instance()
            .get(&MockTokenKey::Allowance(owner, spender))
            .unwrap_or(0)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Minimal mock AMM (mirrors the real AMM's `swap` signature)
// ─────────────────────────────────────────────────────────────────────────────
//
// The mock AMM swaps token_in for a hard-coded payment token at a 1:1 ratio
// (minus a configurable fee) so tests are fully deterministic.

#[contract]
pub struct MockAmm;

#[contracttype]
#[derive(Clone)]
pub enum MockAmmKey {
    PaymentToken,
    FeeNumerator, // fee = amount_in * fee_num / 10_000
    ShouldFail,   // if true, swap always panics
}

#[contractimpl]
impl MockAmm {
    /// One-time setup called by the test helper.
    pub fn init(env: Env, payment_token: Address, fee_bps: u32) {
        env.storage()
            .instance()
            .set(&MockAmmKey::PaymentToken, &payment_token);
        env.storage()
            .instance()
            .set(&MockAmmKey::FeeNumerator, &fee_bps);
        env.storage()
            .instance()
            .set(&MockAmmKey::ShouldFail, &false);
    }

    /// Make the next swap call panic (used to test failure path).
    pub fn set_should_fail(env: Env, fail: bool) {
        env.storage()
            .instance()
            .set(&MockAmmKey::ShouldFail, &fail);
    }

    /// Mirrors `Amm::swap`.
    /// Pulls `amount_in` of `token_in` from `user`, deposits `amount_out` of
    /// payment_token back to `user`.
    /// Panics on slippage violation or when `should_fail` is set.
    pub fn swap(
        env: Env,
        user: Address,
        _pool_id: u64,
        token_in: Address,
        amount_in: i128,
        min_amount_out: i128,
    ) -> i128 {
        let should_fail: bool = env
            .storage()
            .instance()
            .get(&MockAmmKey::ShouldFail)
            .unwrap_or(false);
        if should_fail {
            panic!("MockAmm: swap failed");
        }

        let fee_bps: u32 = env
            .storage()
            .instance()
            .get(&MockAmmKey::FeeNumerator)
            .unwrap_or(30);
        let payment_token: Address = env
            .storage()
            .instance()
            .get(&MockAmmKey::PaymentToken)
            .expect("MockAmm not initialized");

        // amount_out = amount_in * (10_000 - fee_bps) / 10_000  (1:1 ratio minus fee)
        let amount_out = amount_in * (10_000 - fee_bps as i128) / 10_000;
        assert!(amount_out >= min_amount_out, "Slippage tolerance exceeded");

        // Pull token_in from user.
        let token_in_client = MockTokenClient::new(&env, &token_in);
        token_in_client.transfer(&user, &env.current_contract_address(), &amount_in);

        // Push payment_token to user.
        let payment_client = MockTokenClient::new(&env, &payment_token);
        payment_client.transfer(&env.current_contract_address(), &user, &amount_out);

        amount_out
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Test helper
// ─────────────────────────────────────────────────────────────────────────────

struct TestCtx {
    env: Env,
    admin: Address,
    marketplace: MarketplaceContractClient<'static>,
    marketplace_id: Address,
    payment_token: Address,
    alt_token: Address,
    amm_id: Address,
}

fn setup() -> TestCtx {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);

    // Deploy mock payment token and alt token.
    let payment_token = env.register(MockToken, ());
    let alt_token = env.register(MockToken, ());

    // Deploy mock AMM and initialize it (no fee = 0 bps, gives clean 1:1 outputs).
    let amm_id = env.register(MockAmm, ());
    MockAmmClient::new(&env, &amm_id).init(&payment_token, &0);

    // Pre-fund the mock AMM with payment tokens so it can pay out on swaps.
    MockTokenClient::new(&env, &payment_token).mint(&amm_id, &10_000_000);

    // Deploy marketplace and initialize.
    let marketplace_id = env.register(MarketplaceContract, ());
    let marketplace = MarketplaceContractClient::new(&env, &marketplace_id);
    marketplace.initialize(&admin, &payment_token, &250); // 2.5% platform fee

    // Wire the AMM into the marketplace.
    marketplace.set_amm_contract(&admin, &amm_id);

    // Register alt_token as accepted and map it to pool 0.
    marketplace.add_accepted_token(&admin, &alt_token);
    marketplace.set_token_pool_id(&admin, &alt_token, &0u64);

    TestCtx {
        env,
        admin,
        marketplace,
        marketplace_id,
        payment_token,
        alt_token,
        amm_id,
    }
}

/// Create a standard Sale listing priced at `price` and return its listing_id.
fn create_listing(ctx: &TestCtx, seller: &Address, price: i128) -> u64 {
    ctx.marketplace
        .create_listing(&1u64, seller, &0u32, &price) // listing_type 0 = Sale
}

// ─────────────────────────────────────────────────────────────────────────────
// Admin configuration tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_set_and_get_amm_contract() {
    let ctx = setup();
    let amm_addr = ctx.marketplace.get_amm_contract();
    assert_eq!(amm_addr, Some(ctx.amm_id.clone()));
}

#[test]
fn test_add_and_get_accepted_tokens() {
    let ctx = setup();
    let tokens = ctx.marketplace.get_accepted_tokens();
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens.get(0).unwrap(), ctx.alt_token);
}

#[test]
fn test_remove_accepted_token() {
    let ctx = setup();
    ctx.marketplace
        .remove_accepted_token(&ctx.admin, &ctx.alt_token);
    let tokens = ctx.marketplace.get_accepted_tokens();
    assert_eq!(tokens.len(), 0);
}

#[test]
fn test_add_accepted_token_idempotent() {
    let ctx = setup();
    // Adding the same token twice should not duplicate it.
    ctx.marketplace
        .add_accepted_token(&ctx.admin, &ctx.alt_token);
    let tokens = ctx.marketplace.get_accepted_tokens();
    assert_eq!(tokens.len(), 1);
}

#[test]
fn test_set_and_get_token_pool_id() {
    let ctx = setup();
    let pool = ctx.marketplace.get_token_pool_id(&ctx.alt_token);
    assert_eq!(pool, Some(0u64));
}

// ─────────────────────────────────────────────────────────────────────────────
// Slippage configuration tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_default_slippage_is_100_bps() {
    let ctx = setup();
    let buyer = Address::generate(&ctx.env);
    let slippage = ctx.marketplace.get_user_slippage(&buyer);
    assert_eq!(slippage, 100); // DEFAULT_SLIPPAGE_BPS
}

#[test]
fn test_set_user_slippage() {
    let ctx = setup();
    let buyer = Address::generate(&ctx.env);
    ctx.marketplace.set_user_slippage(&buyer, &200u32);
    assert_eq!(ctx.marketplace.get_user_slippage(&buyer), 200);
}

#[test]
fn test_set_user_slippage_zero() {
    let ctx = setup();
    let buyer = Address::generate(&ctx.env);
    ctx.marketplace.set_user_slippage(&buyer, &0u32);
    assert_eq!(ctx.marketplace.get_user_slippage(&buyer), 0);
}

#[test]
fn test_set_user_slippage_max() {
    let ctx = setup();
    let buyer = Address::generate(&ctx.env);
    ctx.marketplace.set_user_slippage(&buyer, &5000u32);
    assert_eq!(ctx.marketplace.get_user_slippage(&buyer), 5000);
}

#[test]
#[should_panic(expected = "Slippage tolerance exceeds maximum")]
fn test_set_user_slippage_exceeds_max() {
    let ctx = setup();
    let buyer = Address::generate(&ctx.env);
    ctx.marketplace.set_user_slippage(&buyer, &5001u32);
}

// ─────────────────────────────────────────────────────────────────────────────
// Successful multi-token purchase tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_buy_agent_with_swap_success() {
    let ctx = setup();
    let seller = Address::generate(&ctx.env);
    let buyer = Address::generate(&ctx.env);

    // Listing priced at 10_000 payment tokens.
    let listing_id = create_listing(&ctx, &seller, 10_000);

    // Buyer holds 10_100 alt_tokens (slightly more to cover any rounding/fees).
    MockTokenClient::new(&ctx.env, &ctx.alt_token).mint(&buyer, &10_100);

    // Execute swap-purchase.
    let escrow_id = ctx
        .marketplace
        .buy_agent_with_swap(&listing_id, &buyer, &ctx.alt_token, &10_100);

    // Escrow should exist and hold exactly listing.price = 10_000.
    let escrow = ctx
        .marketplace
        .get_escrow(&escrow_id)
        .expect("Escrow not created");
    assert_eq!(escrow.buyer, buyer);
    assert_eq!(escrow.seller, seller);
    assert_eq!(escrow.amount, 10_000);
}

#[test]
fn test_listing_deactivated_after_swap_purchase() {
    let ctx = setup();
    let seller = Address::generate(&ctx.env);
    let buyer = Address::generate(&ctx.env);

    let listing_id = create_listing(&ctx, &seller, 5_000);
    MockTokenClient::new(&ctx.env, &ctx.alt_token).mint(&buyer, &5_000);

    ctx.marketplace
        .buy_agent_with_swap(&listing_id, &buyer, &ctx.alt_token, &5_000);

    let listing = ctx
        .marketplace
        .get_listing(&listing_id)
        .expect("Listing not found");
    assert!(!listing.active, "Listing should be deactivated after purchase");
}

#[test]
fn test_excess_payment_token_refunded_to_buyer() {
    let ctx = setup();
    // MockAmm fee = 0 bps, so 1:1. Buyer sends 12_000, listing is 10_000.
    // Surplus = 2_000 should be refunded as payment_token.
    let seller = Address::generate(&ctx.env);
    let buyer = Address::generate(&ctx.env);

    let listing_id = create_listing(&ctx, &seller, 10_000);
    MockTokenClient::new(&ctx.env, &ctx.alt_token).mint(&buyer, &12_000);

    ctx.marketplace
        .buy_agent_with_swap(&listing_id, &buyer, &ctx.alt_token, &12_000);

    // Buyer should have received 2_000 payment tokens as a refund.
    let refund = MockTokenClient::new(&ctx.env, &ctx.payment_token).balance(&buyer);
    assert_eq!(refund, 2_000);
}

#[test]
fn test_swap_purchase_with_amm_fee() {
    let ctx = setup();
    // Re-initialise the mock AMM with a 100 bps (1%) fee.
    MockAmmClient::new(&ctx.env, &ctx.amm_id).init(&ctx.payment_token, &100u32);
    // Pre-fund AMM again after re-init.
    MockTokenClient::new(&ctx.env, &ctx.payment_token).mint(&ctx.amm_id, &10_000_000);

    let seller = Address::generate(&ctx.env);
    let buyer = Address::generate(&ctx.env);

    // Listing priced at 9_900.
    // At 1% fee, sending 10_000 alt_tokens yields 9_900 payment tokens (exact).
    let listing_id = create_listing(&ctx, &seller, 9_900);
    MockTokenClient::new(&ctx.env, &ctx.alt_token).mint(&buyer, &10_000);

    let escrow_id = ctx
        .marketplace
        .buy_agent_with_swap(&listing_id, &buyer, &ctx.alt_token, &10_000);

    let escrow = ctx
        .marketplace
        .get_escrow(&escrow_id)
        .expect("Escrow not created");
    assert_eq!(escrow.amount, 9_900);
}

// ─────────────────────────────────────────────────────────────────────────────
// Slippage-induced failure tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
#[should_panic(expected = "Swap output insufficient to cover listing price")]
fn test_swap_output_below_listing_price_fails() {
    let ctx = setup();
    // Set a high AMM fee (20%) so output is less than listing price.
    MockAmmClient::new(&ctx.env, &ctx.amm_id).init(&ctx.payment_token, &2000u32);
    MockTokenClient::new(&ctx.env, &ctx.payment_token).mint(&ctx.amm_id, &10_000_000);

    let seller = Address::generate(&ctx.env);
    let buyer = Address::generate(&ctx.env);

    // Listing = 10_000. Sending 10_000 alt_tokens with 20% AMM fee yields 8_000 payment
    // tokens — less than listing.price. The assertion in buy_agent_with_swap should panic.
    let listing_id = create_listing(&ctx, &seller, 10_000);
    MockTokenClient::new(&ctx.env, &ctx.alt_token).mint(&buyer, &10_000);

    ctx.marketplace
        .buy_agent_with_swap(&listing_id, &buyer, &ctx.alt_token, &10_000);
}

#[test]
#[should_panic(expected = "Slippage tolerance exceeded")]
fn test_strict_zero_slippage_fails_with_fee() {
    // User sets 0% slippage. MockAmm fee = 100 bps.
    // min_out = listing.price * 10_000/10_000 = listing.price exactly.
    // AMM output = amount_in * 0.99 < listing.price → slippage guard fires inside AMM.
    let ctx = setup();
    MockAmmClient::new(&ctx.env, &ctx.amm_id).init(&ctx.payment_token, &100u32);
    MockTokenClient::new(&ctx.env, &ctx.payment_token).mint(&ctx.amm_id, &10_000_000);

    let seller = Address::generate(&ctx.env);
    let buyer = Address::generate(&ctx.env);

    // Set buyer slippage to 0.
    ctx.marketplace.set_user_slippage(&buyer, &0u32);

    // Listing = 10_000. Buyer sends 10_000 alt_tokens.
    // min_out = 10_000 * (10_000 - 0) / 10_000 = 10_000.
    // AMM output = 10_000 * 9_900 / 10_000 = 9_900 < 10_000 → slippage guard panics.
    let listing_id = create_listing(&ctx, &seller, 10_000);
    MockTokenClient::new(&ctx.env, &ctx.alt_token).mint(&buyer, &10_000);

    ctx.marketplace
        .buy_agent_with_swap(&listing_id, &buyer, &ctx.alt_token, &10_000);
}

#[test]
#[should_panic]
fn test_amm_swap_failure_reverts_entire_transaction() {
    // When the AMM itself panics (e.g. no liquidity), the whole tx should revert.
    let ctx = setup();
    MockAmmClient::new(&ctx.env, &ctx.amm_id).set_should_fail(&true);

    let seller = Address::generate(&ctx.env);
    let buyer = Address::generate(&ctx.env);

    let listing_id = create_listing(&ctx, &seller, 5_000);
    MockTokenClient::new(&ctx.env, &ctx.alt_token).mint(&buyer, &5_000);

    // Should panic — AMM is set to fail.
    ctx.marketplace
        .buy_agent_with_swap(&listing_id, &buyer, &ctx.alt_token, &5_000);
}

// ─────────────────────────────────────────────────────────────────────────────
// Guard / validation tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
#[should_panic(expected = "Payment token not in accepted list")]
fn test_buy_with_unaccepted_token_fails() {
    let ctx = setup();
    let seller = Address::generate(&ctx.env);
    let buyer = Address::generate(&ctx.env);
    let unknown_token = env_register_mock(&ctx.env);

    let listing_id = create_listing(&ctx, &seller, 1_000);
    MockTokenClient::new(&ctx.env, &unknown_token).mint(&buyer, &1_000);

    ctx.marketplace
        .buy_agent_with_swap(&listing_id, &buyer, &unknown_token, &1_000);
}

#[test]
#[should_panic(expected = "Listing is not active")]
fn test_buy_inactive_listing_fails() {
    let ctx = setup();
    let seller = Address::generate(&ctx.env);
    let buyer = Address::generate(&ctx.env);

    let listing_id = create_listing(&ctx, &seller, 1_000);
    // Cancel the listing first.
    ctx.marketplace.cancel_listing(&listing_id, &seller);

    MockTokenClient::new(&ctx.env, &ctx.alt_token).mint(&buyer, &1_000);
    ctx.marketplace
        .buy_agent_with_swap(&listing_id, &buyer, &ctx.alt_token, &1_000);
}

#[test]
#[should_panic(expected = "AMM contract not configured")]
fn test_buy_without_amm_configured_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let payment_token = env.register(MockToken, ());
    let alt_token = env.register(MockToken, ());

    let marketplace_id = env.register(MarketplaceContract, ());
    let marketplace = MarketplaceContractClient::new(&env, &marketplace_id);
    marketplace.initialize(&admin, &payment_token, &250);

    // Deliberately skip set_amm_contract.
    marketplace.add_accepted_token(&admin, &alt_token);
    marketplace.set_token_pool_id(&admin, &alt_token, &0u64);

    let seller = Address::generate(&env);
    let buyer = Address::generate(&env);
    let listing_id = marketplace.create_listing(&1u64, &seller, &0u32, &1_000);

    MockTokenClient::new(&env, &alt_token).mint(&buyer, &1_000);
    marketplace.buy_agent_with_swap(&listing_id, &buyer, &alt_token, &1_000);
}

#[test]
#[should_panic(expected = "No pool configured for this token")]
fn test_buy_token_without_pool_mapping_fails() {
    let ctx = setup();
    let seller = Address::generate(&ctx.env);
    let buyer = Address::generate(&ctx.env);

    // Register an extra token as accepted but intentionally skip set_token_pool_id.
    let extra_token = env_register_mock(&ctx.env);
    ctx.marketplace
        .add_accepted_token(&ctx.admin, &extra_token);

    let listing_id = create_listing(&ctx, &seller, 1_000);
    MockTokenClient::new(&ctx.env, &extra_token).mint(&buyer, &1_000);

    ctx.marketplace
        .buy_agent_with_swap(&listing_id, &buyer, &extra_token, &1_000);
}

#[test]
#[should_panic(expected = "amount_in must be positive")]
fn test_buy_with_zero_amount_fails() {
    let ctx = setup();
    let seller = Address::generate(&ctx.env);
    let buyer = Address::generate(&ctx.env);

    let listing_id = create_listing(&ctx, &seller, 1_000);
    ctx.marketplace
        .buy_agent_with_swap(&listing_id, &buyer, &ctx.alt_token, &0i128);
}

// ─────────────────────────────────────────────────────────────────────────────
// Oracle-price integration: confirm_receipt after swap purchase still works
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_confirm_receipt_after_swap_purchase_releases_escrow() {
    let ctx = setup();
    let seller = Address::generate(&ctx.env);
    let buyer = Address::generate(&ctx.env);

    // Fund the marketplace with payment_token so route_sale_payment can transfer out.
    MockTokenClient::new(&ctx.env, &ctx.payment_token)
        .mint(&ctx.marketplace_id, &100_000);

    let listing_id = create_listing(&ctx, &seller, 5_000);
    MockTokenClient::new(&ctx.env, &ctx.alt_token).mint(&buyer, &5_000);

    let escrow_id = ctx
        .marketplace
        .buy_agent_with_swap(&listing_id, &buyer, &ctx.alt_token, &5_000);

    // Buyer confirms receipt — this should route payment to the seller.
    ctx.marketplace.confirm_receipt(&escrow_id, &buyer);

    let escrow = ctx
        .marketplace
        .get_escrow(&escrow_id)
        .expect("Escrow not found");
    // Status 1 = Released
    assert_eq!(escrow.status as u32, 1u32);
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn env_register_mock(env: &Env) -> Address {
    env.register(MockToken, ())
}
