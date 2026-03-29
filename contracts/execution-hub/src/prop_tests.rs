/// Property-based tests for the ExecutionHub contract.
///
/// Invariants verified:
///   EH-1  execution_id is strictly monotonically increasing
///   EH-2  nonce must always increase (replay protection)
///   EH-3  only owner or authorized operator can execute
///   EH-4  rate limit is enforced (ops per window)
///   EH-5  action count equals number of successful executions
///   EH-6  execution receipt is immutable after creation
#[cfg(test)]
mod prop_tests {
    use crate::{ExecutionHub, ExecutionHubClient};
    use proptest::prelude::*;
    use soroban_sdk::{
        contract, contractimpl, testutils::Address as _, Address, Bytes, Env, String,
    };

    // ── minimal mock AgentNFT ─────────────────────────────────────────────────

    #[contract]
    pub struct MockNFT;

    #[contractimpl]
    impl MockNFT {
        pub fn get_agent_owner(env: Env, agent_id: u64) -> Address {
            env.storage()
                .instance()
                .get(&agent_id)
                .expect("agent not found")
        }
        pub fn set_owner(env: Env, agent_id: u64, owner: Address) {
            env.storage().instance().set(&agent_id, &owner);
        }
    }

    // ── helpers ───────────────────────────────────────────────────────────────

    fn setup(env: &Env) -> (ExecutionHubClient, Address, MockNFTClient) {
        let hub_id = env.register_contract(None, ExecutionHub);
        let nft_id = env.register_contract(None, MockNFT);
        let client = ExecutionHubClient::new(env, &hub_id);
        let nft = MockNFTClient::new(env, &nft_id);
        let admin = Address::generate(env);
        env.mock_all_auths();
        client.initialize(&admin, &nft_id);
        (client, admin, nft)
    }

    fn exec(
        env: &Env,
        client: &ExecutionHubClient,
        agent_id: u64,
        executor: &Address,
        nonce: u64,
    ) -> u64 {
        let action = String::from_str(env, "act");
        let params = Bytes::from_array(env, &[1u8]);
        let hash = Bytes::from_array(env, &nonce.to_be_bytes());
        client.execute_action(&agent_id, executor, &action, &params, &nonce, &hash)
    }

    // ── EH-1  execution_id strictly increases ─────────────────────────────────

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_execution_id_strictly_increases(n in 1..10usize) {
            let env = Env::default();
            let (client, _, nft) = setup(&env);
            let executor = Address::generate(&env);
            nft.set_owner(&1u64, &executor);

            let mut prev = 0u64;
            for i in 1..=(n as u64) {
                let id = exec(&env, &client, 1, &executor, i);
                prop_assert!(id > prev, "id={id} must be > prev={prev}");
                prev = id;
            }
            prop_assert_eq!(client.get_execution_counter(), n as u64);
        }

        // ── EH-2  replay protection: same nonce rejected ──────────────────────

        #[test]
        fn prop_replay_nonce_rejected(nonce in 1u64..u64::MAX) {
            let env = Env::default();
            let (client, _, nft) = setup(&env);
            let executor = Address::generate(&env);
            nft.set_owner(&1u64, &executor);

            exec(&env, &client, 1, &executor, nonce);

            let action = String::from_str(&env, "act");
            let params = Bytes::from_array(&env, &[1u8]);
            let hash = Bytes::from_array(&env, &nonce.to_be_bytes());
            let result = client.try_execute_action(&1u64, &executor, &action, &params, &nonce, &hash);
            prop_assert!(result.is_err(), "duplicate nonce={nonce} must be rejected");
        }

        // ── EH-3  non-owner cannot execute ────────────────────────────────────

        #[test]
        fn prop_non_owner_cannot_execute(agent_id in 1..500u64) {
            let env = Env::default();
            let (client, _, nft) = setup(&env);
            let owner = Address::generate(&env);
            let stranger = Address::generate(&env);
            nft.set_owner(&agent_id, &owner);

            let action = String::from_str(&env, "act");
            let params = Bytes::from_array(&env, &[1u8]);
            let hash = Bytes::from_array(&env, &[0xaau8]);
            let result = client.try_execute_action(&agent_id, &stranger, &action, &params, &1, &hash);
            prop_assert!(result.is_err(), "stranger must not execute agent {agent_id}");
        }

        // ── EH-4  default rate limit (100 ops/60s) is enforced ───────────────
        // Tests that the default global rate limit blocks the 101st execution.
        // Case count is low because each case does 100 contract calls.
        #[test]
        #[cfg_attr(not(feature = "slow-tests"), ignore)]
        fn prop_rate_limit_default_blocks_over_100(extra in 1..5u32) {
            let env = Env::default();
            let (client, _, nft) = setup(&env);
            let executor = Address::generate(&env);
            nft.set_owner(&1u64, &executor);

            for i in 1..=100u64 {
                let id = exec(&env, &client, 1, &executor, i);
                prop_assert!(id > 0);
            }

            let action = String::from_str(&env, "act");
            let params = Bytes::from_array(&env, &[1u8]);
            let nonce = 100u64 + extra as u64;
            let hash = Bytes::from_array(&env, &nonce.to_be_bytes());
            let result = client.try_execute_action(&1u64, &executor, &action, &params, &nonce, &hash);
            prop_assert!(result.is_err(), "execution #{nonce} must be rate-limited");
        }

        // ── EH-5  action count matches executions ─────────────────────────────

        #[test]
        fn prop_action_count_matches_executions(n in 1..10usize) {
            let env = Env::default();
            let (client, _, nft) = setup(&env);
            let executor = Address::generate(&env);
            nft.set_owner(&1u64, &executor);

            for i in 1..=(n as u64) {
                exec(&env, &client, 1, &executor, i);
            }

            prop_assert_eq!(client.get_action_count(&1u64), n as u32);
        }

        // ── EH-6  receipt is immutable after creation ─────────────────────────

        #[test]
        fn prop_receipt_immutable(n in 2..6usize) {
            let env = Env::default();
            let (client, _, nft) = setup(&env);
            let executor = Address::generate(&env);
            nft.set_owner(&1u64, &executor);

            let first_id = exec(&env, &client, 1, &executor, 1);
            let receipt_before = client.get_execution_receipt(&first_id).unwrap();

            for i in 2..=(n as u64) {
                exec(&env, &client, 1, &executor, i);
            }

            let receipt_after = client.get_execution_receipt(&first_id).unwrap();
            prop_assert_eq!(receipt_before.execution_id, receipt_after.execution_id);
            prop_assert_eq!(receipt_before.execution_hash, receipt_after.execution_hash);
            prop_assert_eq!(receipt_before.timestamp, receipt_after.timestamp);
        }
    }

    // ── EH-4b  rate limit: 101st call in same window is blocked (fast) ───────
    #[test]
    #[should_panic(expected = "Rate limit exceeded")]
    fn prop_rate_limit_101st_blocked() {
        let env = Env::default();
        let (client, _, nft) = setup(&env);
        let executor = Address::generate(&env);
        nft.set_owner(&1u64, &executor);

        for i in 1..=100u64 {
            exec(&env, &client, 1, &executor, i);
        }

        let action = String::from_str(&env, "act");
        let params = Bytes::from_array(&env, &[1u8]);
        let hash = Bytes::from_array(&env, &101u64.to_be_bytes());
        client.execute_action(&1u64, &executor, &action, &params, &101u64, &hash);
    }
}
