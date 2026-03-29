/// Property-based tests for the Marketplace contract.
///
/// Invariants verified:
///   MP-1  listing_price > 0 for every active listing
///   MP-2  royalty fee ≤ 2500 bps (25 %)
///   MP-3  platform fee ≤ 5000 bps (50 %)
///   MP-4  listing counter is strictly monotonically increasing
///   MP-5  a cancelled listing is never active
///   MP-6  set_royalty rejects any fee > 2500
#[cfg(test)]
mod prop_tests {
    use crate::{Marketplace, MarketplaceClient};
    use proptest::prelude::*;
    use soroban_sdk::{testutils::Address as _, Address, Env};

    // ── helpers ──────────────────────────────────────────────────────────────

    fn setup(env: &Env) -> (MarketplaceClient, Address) {
        let id = env.register_contract(None, Marketplace);
        let client = MarketplaceClient::new(env, &id);
        let admin = Address::generate(env);
        env.mock_all_auths();
        client.init_contract(&admin);
        (client, admin)
    }

    /// Create a listing and return its id.
    fn make_listing(env: &Env, client: &MarketplaceClient, agent_id: u64, price: i128) -> u64 {
        let seller = Address::generate(env);
        client.create_listing(&agent_id, &seller, &0u32, &price)
    }

    // ── MP-1  listing price is always positive ────────────────────────────────

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn prop_listing_price_always_positive(
            agent_id in 1..10_000u64,
            price in 1..i128::MAX,
        ) {
            let env = Env::default();
            let (client, _) = setup(&env);

            let listing_id = make_listing(&env, &client, agent_id, price);
            let listing = client.get_listing(&listing_id).unwrap();

            // MP-1: stored price must equal the requested price and be > 0
            prop_assert!(listing.price > 0);
            prop_assert_eq!(listing.price, price);
        }

        // ── MP-1b  zero / negative price is rejected ─────────────────────────

        #[test]
        fn prop_zero_or_negative_price_rejected(
            agent_id in 1..10_000u64,
            bad_price in i128::MIN..=0i128,
        ) {
            let env = Env::default();
            let (client, _) = setup(&env);
            let seller = Address::generate(&env);

            let result = client.try_create_listing(&agent_id, &seller, &0u32, &bad_price);
            prop_assert!(result.is_err(), "price={bad_price} should be rejected");
        }

        // ── MP-2  royalty fee ≤ 2500 bps ─────────────────────────────────────

        #[test]
        fn prop_valid_royalty_fee_accepted(fee in 0..=2500u32) {
            let env = Env::default();
            let (client, _) = setup(&env);
            let creator = Address::generate(&env);
            let recipient = Address::generate(&env);

            // Must not panic
            client.set_royalty(&1u64, &creator, &recipient, &fee);
            let info = client.get_royalty(&1u64).unwrap();
            prop_assert_eq!(info.fee, fee);
        }

        #[test]
        fn prop_royalty_fee_above_max_rejected(fee in 2501..=u32::MAX) {
            let env = Env::default();
            let (client, _) = setup(&env);
            let creator = Address::generate(&env);
            let recipient = Address::generate(&env);

            let result = client.try_set_royalty(&1u64, &creator, &recipient, &fee);
            prop_assert!(result.is_err(), "fee={fee} should exceed 25% cap");
        }

        // ── MP-3  platform fee ≤ 5000 bps ────────────────────────────────────

        #[test]
        fn prop_platform_fee_within_bounds(fee in 0..=5000u32) {
            let env = Env::default();
            let (client, admin) = setup(&env);

            client.set_platform_fee(&admin, &fee);
            prop_assert_eq!(client.get_platform_fee(), fee);
        }

        #[test]
        fn prop_platform_fee_above_max_rejected(fee in 5001..=u32::MAX) {
            let env = Env::default();
            let (client, admin) = setup(&env);

            let result = client.try_set_platform_fee(&admin, &fee);
            prop_assert!(result.is_err(), "fee={fee} should exceed 50% cap");
        }

        // ── MP-4  listing counter strictly increases ──────────────────────────

        #[test]
        fn prop_listing_counter_monotonically_increases(n in 1..20usize) {
            let env = Env::default();
            let (client, _) = setup(&env);

            let mut prev_id = 0u64;
            for i in 0..n {
                let id = make_listing(&env, &client, (i as u64) + 1, 100);
                prop_assert!(id > prev_id, "listing id must increase: {id} <= {prev_id}");
                prev_id = id;
            }
        }

        // ── MP-5  cancelled listing is never active ───────────────────────────

        #[test]
        fn prop_cancelled_listing_is_inactive(
            agent_id in 1..10_000u64,
            price in 1..1_000_000i128,
        ) {
            let env = Env::default();
            let (client, _) = setup(&env);
            let seller = Address::generate(&env);

            let listing_id = client.create_listing(&agent_id, &seller, &0u32, &price);
            client.cancel_listing(&listing_id, &seller);

            let listing = client.get_listing(&listing_id).unwrap();
            prop_assert!(!listing.active, "cancelled listing must be inactive");
        }

        // ── MP-6  royalty never exceeds 100 % (10 000 bps) ───────────────────
        // (enforced by the 2500 cap, but we verify the stored value is sane)

        #[test]
        fn prop_stored_royalty_never_exceeds_10000(fee in 0..=2500u32) {
            let env = Env::default();
            let (client, _) = setup(&env);
            let creator = Address::generate(&env);
            let recipient = Address::generate(&env);

            client.set_royalty(&42u64, &creator, &recipient, &fee);
            let info = client.get_royalty(&42u64).unwrap();
            prop_assert!(info.fee <= 10_000, "stored royalty fee must be ≤ 100%");
        }
    }
}
