#[cfg(test)]
mod prop_tests {
    extern crate alloc;
    use super::*;
    use crate::{AgentMintData, AgentNFT, AgentNFTClient, ContractError};
    use alloc::string::ToString;
    use proptest::prelude::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env, String, Vec};
    use stellai_lib::types::OptionalRoyaltyInfo;

    fn setup_contract(env: &Env) -> (AgentNFTClient, Address) {
        let contract_id = env.register_contract(None, AgentNFT);
        let client = AgentNFTClient::new(env, &contract_id);
        let admin = Address::generate(env);
        env.mock_all_auths();
        client.init_contract(&admin);
        (client, admin)
    }

    fn mint_test_agent(
        env: &Env,
        client: &AgentNFTClient,
        owner: &Address,
        agent_id: u128,
        metadata_cid: &str,
        evolution_level: u32,
    ) {
        client.mint_agent(
            &agent_id,
            owner,
            &String::from_str(env, metadata_cid),
            &evolution_level,
            &None,
            &None,
        );
    }

    // Generates a random valid royalty fee (0 to 10,000)
    fn any_royalty_fee() -> impl Strategy<Value = u32> {
        0..=10000u32
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn prop_agent_counter_always_increases_correctly(num_mints in 1..20usize) {
            let env = Env::default();
            let (client, admin) = setup_contract(&env);
            client.add_approved_minter(&admin, &admin);

            // batch_mint uses the auto-incrementing counter
            let mut agents = soroban_sdk::Vec::new(&env);
            for i in 0..num_mints {
                agents.push_back(AgentMintData {
                    owner: Address::generate(&env),
                    name: String::from_str(&env, "A"),
                    model_hash: String::from_str(&env, "H"),
                    metadata_cid: String::from_str(&env, &alloc::format!("Qm{i}")),
                    capabilities: soroban_sdk::Vec::new(&env),
                    royalty: OptionalRoyaltyInfo::None,
                });
            }
            let ids = client.batch_mint(&admin, &agents);

            // INVARIANT: counter == number of minted agents
            prop_assert_eq!(client.total_agents(), num_mints as u64);
            // INVARIANT: returned IDs are sequential starting from 1
            for (i, id) in ids.iter().enumerate() {
                prop_assert_eq!(id, (i as u64) + 1);
            }
        }

        #[test]
        fn prop_royalty_fee_invariant(fee in 10001..u32::MAX) {
            let env = Env::default();
            let contract_id = env.register_contract(None, AgentNFT);
            let client = AgentNFTClient::new(&env, &contract_id);
            let admin = Address::generate(&env);

            env.mock_all_auths();
            client.init_contract(&admin);

            let owner = Address::generate(&env);
            let recipient = Address::generate(&env);
            client.add_approved_minter(&admin, &owner);

            // INVARIANT: Any fee > 10000 must return InvalidRoyaltyFee error
            let result = client.try_mint_agent(
                &1,
                &owner,
                &String::from_str(&env, "cid"),
                &1,
                &Some(recipient),
                &Some(fee)
            );

            match result {
                Err(Ok(ContractError::InvalidRoyaltyFee)) => {},
                _ => panic!("Should have failed with InvalidRoyaltyFee for value {}", fee),
            }
        }

        #[test]
        fn prop_transfer_auth_invariant(
            id in 1..100u64,
            random_user in proptest::option::of(proptest::strategy::Just(true))
        ) {
            let env = Env::default();
            env.mock_all_auths();
            let (client, admin) = setup_contract(&env);

            let owner = Address::generate(&env);
            let stranger = Address::generate(&env);
            let _ = client.add_approved_minter(&admin, &owner);

            mint_test_agent(&env, &client, &owner, id as u128, "cid", 1);

            // INVARIANT: Only owner can transfer. Stranger must fail.
            // We force the 'stranger' to be the one calling require_auth via mock_all_auths logic
            let result = client.try_transfer_agent(&id, &stranger, &Address::generate(&env));

            match result {
                Err(Ok(ContractError::NotOwner)) => {},
                _ => panic!("Non-owner was able to initiate transfer or got wrong error"),
            }
        }
    }

    // --- Standard Unit Tests (Moving from lib.rs and adding more) ---

    #[test]
    fn test_get_agent_metadata() {
        let env = Env::default();
        let (client, admin) = setup_contract(&env);

        let owner = Address::generate(&env);
        client.add_approved_minter(&admin, &owner);

        let metadata_cid = "QmTestMetadataCID456";
        env.mock_all_auths();
        mint_test_agent(&env, &client, &owner, 2, metadata_cid, 5);

        // Test get_agent_metadata returns correct CID
        let result = client.get_agent_metadata(&2);
        assert_eq!(result, String::from_str(&env, metadata_cid));
    }

    #[test]
    fn test_get_agent_evolution_level() {
        let env = Env::default();
        let (client, admin) = setup_contract(&env);

        let owner = Address::generate(&env);
        client.add_approved_minter(&admin, &owner);

        let evolution_level = 7u32;
        env.mock_all_auths();
        mint_test_agent(&env, &client, &owner, 3, "QmEvolutionTest", evolution_level);

        // Test get_agent_evolution_level returns correct level
        let result = client.get_agent_evolution_level(&3);
        assert_eq!(result, evolution_level);
    }

    #[test]
    fn test_query_non_existent_agent() {
        let env = Env::default();
        let (client, _admin) = setup_contract(&env);

        // Try to query a non-existent agent - should return AgentNotFound
        let result = client.try_get_agent_owner(&999);
        assert!(result.is_err());

        let result = client.try_get_agent_metadata(&999);
        assert!(result.is_err());

        let result = client.try_get_agent_evolution_level(&999);
        assert!(result.is_err());
    }

    #[test]
    fn test_query_zero_agent_id() {
        let env = Env::default();
        let (client, _admin) = setup_contract(&env);

        // Query with agent_id = 0 should return InvalidAgentId
        let result = client.try_get_agent_owner(&0);
        assert!(result.is_err());

        let result = client.try_get_agent_metadata(&0);
        assert!(result.is_err());

        let result = client.try_get_agent_evolution_level(&0);
        assert!(result.is_err());
    }

    #[test]
    fn test_capabilities_limit_error() {
        let env = Env::default();
        let (client, admin) = setup_contract(&env);
        let owner = Address::generate(&env);
        client.add_approved_minter(&admin, &owner);

        // Max capabilities is 32 (MAX_CAPABILITIES constant)
        let mut caps = Vec::new(&env);
        for _ in 0..33 {
            caps.push_back(String::from_str(&env, "cap"));
        }

        env.mock_all_auths();
        let result = client.try_mint_agent_legacy(
            &owner,
            &String::from_str(&env, "Name"),
            &String::from_str(&env, "Hash"),
            &caps,
            &None,
            &None,
        );

        match result {
            Err(Ok(ContractError::CapabilitiesExceeded)) => {}
            _ => panic!(
                "Should have failed with CapabilitiesExceeded, got {:?}",
                result
            ),
        }
    }

    // ── batch_mint tests (Issue #91) ─────────────────────────────────────────

    fn make_mint_data(env: &Env, cid_suffix: &str) -> AgentMintData {
        let owner = Address::generate(env);
        let mut cid = alloc::string::String::from("QmBatchCid");
        cid.push_str(cid_suffix);
        AgentMintData {
            owner,
            name: String::from_str(env, "BatchAgent"),
            model_hash: String::from_str(env, "hash"),
            metadata_cid: String::from_str(env, &cid),
            capabilities: Vec::new(env),
            royalty: stellai_lib::types::OptionalRoyaltyInfo::None,
        }
    }

    #[test]
    fn test_batch_mint_single_item() {
        let env = Env::default();
        let (client, admin) = setup_contract(&env);
        env.mock_all_auths();
        client.add_approved_minter(&admin, &admin);

        let mut agents = Vec::new(&env);
        agents.push_back(make_mint_data(&env, "0"));

        let ids = client.batch_mint(&admin, &agents);
        assert_eq!(ids.len(), 1);
        assert_eq!(ids.get(0).unwrap(), 1u64);
        assert_eq!(client.total_agents(), 1u64);
    }

    #[test]
    fn test_batch_mint_ten_agents() {
        let env = Env::default();
        let (client, admin) = setup_contract(&env);
        env.mock_all_auths();
        client.add_approved_minter(&admin, &admin);

        let suffixes = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
        let mut agents = Vec::new(&env);
        for s in &suffixes {
            agents.push_back(make_mint_data(&env, s));
        }

        let ids = client.batch_mint(&admin, &agents);
        assert_eq!(ids.len(), 10);
        for (i, id) in ids.iter().enumerate() {
            assert_eq!(id, (i as u64) + 1);
        }
        assert_eq!(client.total_agents(), 10u64);
    }

    #[test]
    fn test_batch_mint_fifty_agents() {
        let env = Env::default();
        let (client, admin) = setup_contract(&env);
        env.mock_all_auths();
        client.add_approved_minter(&admin, &admin);

        let mut agents = Vec::new(&env);
        for n in 0u32..50 {
            let s = n.to_string();
            agents.push_back(make_mint_data(&env, &s));
        }

        let ids = client.batch_mint(&admin, &agents);
        assert_eq!(ids.len(), 50);
        assert_eq!(client.total_agents(), 50u64);
    }

    #[test]
    fn test_batch_mint_empty_fails() {
        let env = Env::default();
        let (client, admin) = setup_contract(&env);
        env.mock_all_auths();
        client.add_approved_minter(&admin, &admin);

        let agents: Vec<AgentMintData> = Vec::new(&env);
        let result = client.try_batch_mint(&admin, &agents);
        match result {
            Err(Ok(ContractError::InvalidInput)) => {}
            _ => panic!("Expected InvalidInput for empty batch, got {:?}", result),
        }
    }

    #[test]
    fn test_batch_mint_exceeds_limit_fails() {
        let env = Env::default();
        let (client, admin) = setup_contract(&env);
        env.mock_all_auths();
        client.add_approved_minter(&admin, &admin);

        // 51 agents — one over the limit
        let mut agents = Vec::new(&env);
        for n in 0u32..51 {
            let s = n.to_string();
            agents.push_back(make_mint_data(&env, &s));
        }

        let result = client.try_batch_mint(&admin, &agents);
        match result {
            Err(Ok(ContractError::InvalidInput)) => {}
            _ => panic!(
                "Expected InvalidInput for oversized batch, got {:?}",
                result
            ),
        }
    }

    #[test]
    fn test_batch_mint_duplicate_cid_within_batch_fails() {
        let env = Env::default();
        let (client, admin) = setup_contract(&env);
        env.mock_all_auths();
        client.add_approved_minter(&admin, &admin);

        let mut agents = Vec::new(&env);
        // Two agents sharing the same metadata_cid
        let a1 = make_mint_data(&env, "dup");
        let mut a2 = make_mint_data(&env, "other");
        a2.metadata_cid = a1.metadata_cid.clone();
        agents.push_back(a1);
        agents.push_back(a2);

        let result = client.try_batch_mint(&admin, &agents);
        match result {
            Err(Ok(ContractError::InvalidInput)) => {}
            _ => panic!("Expected InvalidInput for duplicate CID, got {:?}", result),
        }
    }

    #[test]
    fn test_batch_mint_counter_continues_after_previous_mints() {
        let env = Env::default();
        let (client, admin) = setup_contract(&env);
        env.mock_all_auths();
        client.add_approved_minter(&admin, &admin);

        // Mint one agent via batch_mint first (increments counter)
        let owner = Address::generate(&env);
        client.add_approved_minter(&admin, &owner);
        let first = soroban_sdk::vec![&env, make_mint_data(&env, "individual")];
        client.batch_mint(&admin, &first);
        assert_eq!(client.total_agents(), 1u64);

        // Now batch-mint 3 more
        let mut agents = Vec::new(&env);
        for s in &["10", "11", "12"] {
            agents.push_back(make_mint_data(&env, s));
        }

        let ids = client.batch_mint(&admin, &agents);
        // Should start from 2 (counter was at 1)
        assert_eq!(ids.get(0).unwrap(), 2u64);
        assert_eq!(ids.get(2).unwrap(), 4u64);
        assert_eq!(client.total_agents(), 4u64);
    }

    #[test]
    fn test_batch_mint_non_admin_fails() {
        let env = Env::default();
        let (client, _admin) = setup_contract(&env);
        env.mock_all_auths();

        let stranger = Address::generate(&env);
        let mut agents = Vec::new(&env);
        agents.push_back(make_mint_data(&env, "stranger_cid"));

        // stranger is not in approved_minters
        let result = client.try_batch_mint(&stranger, &agents);
        assert!(
            result.is_err(),
            "Non-minter should not be able to batch_mint"
        );
    }

    #[test]
    fn test_mint_agent_empty_metadata_fails_with_invalid_metadata() {
        let env = Env::default();
        let (client, admin) = setup_contract(&env);
        let owner = Address::generate(&env);
        env.mock_all_auths();
        client.add_approved_minter(&admin, &owner);

        let result =
            client.try_mint_agent(&1, &owner, &String::from_str(&env, ""), &1, &None, &None);

        match result {
            Err(Ok(ContractError::InvalidMetadata)) => {}
            _ => panic!(
                "Expected InvalidMetadata for empty metadata, got {:?}",
                result
            ),
        }
    }

    #[test]
    fn test_mint_agent_legacy_empty_capability_fails_with_invalid_metadata() {
        let env = Env::default();
        let (client, admin) = setup_contract(&env);
        let owner = Address::generate(&env);
        env.mock_all_auths();
        client.add_approved_minter(&admin, &owner);

        let caps = Vec::from_array(&env, [String::from_str(&env, "")]);
        let result = client.try_mint_agent_legacy(
            &owner,
            &String::from_str(&env, "Name"),
            &String::from_str(&env, "Hash"),
            &caps,
            &None,
            &None,
        );

        match result {
            Err(Ok(ContractError::InvalidMetadata)) => {}
            _ => panic!(
                "Expected InvalidMetadata for empty capability, got {:?}",
                result
            ),
        }
    }

    #[test]
    fn test_batch_mint_is_atomic_when_validation_fails() {
        let env = Env::default();
        let (client, admin) = setup_contract(&env);
        env.mock_all_auths();
        client.add_approved_minter(&admin, &admin);

        let mut agents = Vec::new(&env);
        agents.push_back(make_mint_data(&env, "valid"));

        let mut invalid = make_mint_data(&env, "invalid");
        invalid.metadata_cid = String::from_str(&env, "");
        agents.push_back(invalid);

        let result = client.try_batch_mint(&admin, &agents);
        match result {
            Err(Ok(ContractError::InvalidMetadata)) => {}
            _ => panic!(
                "Expected InvalidMetadata for invalid batch item, got {:?}",
                result
            ),
        }

        assert_eq!(client.total_agents(), 0u64);
        assert!(client.try_get_agent(&1).is_err());
    }
}
