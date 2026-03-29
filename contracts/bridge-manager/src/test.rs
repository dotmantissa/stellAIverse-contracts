#![cfg(test)]

use super::*;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env, Vec};

use crate::BridgeManagerClient;

#[contract]
pub struct MockAgentNFT;

#[contractimpl]
impl MockAgentNFT {
    pub fn transfer_agent(_env: Env, _agent_id: u64, _from: Address, _to: Address) {
        // Mock transfer logic
    }
}

fn setup_env() -> Env {
    let env = Env::default();
    env.mock_all_auths();
    env
}

#[test]
fn test_init_and_config() {
    let env = setup_env();
    let admin = Address::generate(&env);
    let agent_contract = env.register_contract(None, MockAgentNFT);
    let payment_token = Address::generate(&env);

    let bridge_id = env.register_contract(None, BridgeManager);
    let bridge = BridgeManagerClient::new(&env, &bridge_id);

    let signer1 = Address::generate(&env);
    let signer2 = Address::generate(&env);
    let signers = Vec::from_array(&env, [signer1.clone(), signer2.clone()]);

    bridge.init_contract(
        &admin,
        &agent_contract,
        &payment_token,
        &signers,
        &1u32, // 1-of-2 for this test
    );

    let cfg = bridge.get_signer_config_view();
    assert!(cfg.is_some());
    let cfg = cfg.unwrap();
    assert_eq!(cfg.m_required, 1);
    assert_eq!(cfg.signers.len(), 2);
}

#[test]
fn test_lock_and_bridge_and_fee_calculation() {
    let env = setup_env();
    let admin = Address::generate(&env);
    let owner = Address::generate(&env);
    let agent_contract = env.register_contract(None, MockAgentNFT);
    let payment_token = Address::generate(&env);

    let bridge_id_addr = env.register_contract(None, BridgeManager);
    let bridge = BridgeManagerClient::new(&env, &bridge_id_addr);

    let signer = Address::generate(&env);
    let signers = Vec::from_array(&env, [signer.clone()]);

    bridge.init_contract(&admin, &agent_contract, &payment_token, &signers, &1u32);

    let notional_value: i128 = 100_000;
    let target_chain: u32 = 1;
    let metadata_cid = soroban_sdk::String::from_str(&env, "QmTestCID123");

    let bridge_id =
        bridge.lock_and_bridge(&1u64, &owner, &metadata_cid, &target_chain, &notional_value);

    assert_eq!(bridge_id, 1);

    let req = bridge.get_bridge_request(&bridge_id).unwrap();
    assert_eq!(req.agent_id, 1);
    assert_eq!(req.owner, owner);
    assert_eq!(req.metadata_cid, metadata_cid);
    assert_eq!(req.notional_value, notional_value);
    assert_eq!(req.status, BridgeStatus::PendingOutbound);

    // 0.5% of notional_value
    let expected_fee = (notional_value * BRIDGE_FEE_BPS as i128) / 10_000;
    assert_eq!(req.fee_paid, expected_fee);

    // Liquidity and fee balances should be updated
    let liquidity = bridge.get_liquidity_balance();
    let fees = bridge.get_fee_balance();
    assert_eq!(liquidity, notional_value);
    assert_eq!(fees, expected_fee);
}

#[test]
fn test_m_of_n_approvals_and_unwrap_flow() {
    let env = setup_env();
    let admin = Address::generate(&env);
    let owner = Address::generate(&env);
    let agent_contract = env.register_contract(None, MockAgentNFT);
    let payment_token = Address::generate(&env);

    let bridge_id_addr = env.register_contract(None, BridgeManager);
    let bridge = BridgeManagerClient::new(&env, &bridge_id_addr);

    let signer1 = Address::generate(&env);
    let signer2 = Address::generate(&env);
    let signer3 = Address::generate(&env);
    let signers = Vec::from_array(&env, [signer1.clone(), signer2.clone(), signer3.clone()]);

    // 2-of-3 M-of-N
    bridge.init_contract(&admin, &agent_contract, &payment_token, &signers, &2u32);

    let notional_value: i128 = 50_000;
    let target_chain: u32 = 2;
    let metadata_cid = soroban_sdk::String::from_str(&env, "QmTestCID123");
    let bridge_id =
        bridge.lock_and_bridge(&2u64, &owner, &metadata_cid, &target_chain, &notional_value);

    // Outbound approvals
    bridge.approve_outbound(&signer1, &bridge_id);
    let req = bridge.get_bridge_request(&bridge_id).unwrap();
    assert_eq!(req.status, BridgeStatus::PendingOutbound);

    bridge.approve_outbound(&signer2, &bridge_id);
    let req = bridge.get_bridge_request(&bridge_id).unwrap();
    assert_eq!(req.status, BridgeStatus::OutboundCompleted);

    // Inbound approvals with wrapped token id
    let wrapped_id: u128 = 777;
    bridge.approve_inbound(&signer1, &bridge_id, &wrapped_id);
    let req = bridge.get_bridge_request(&bridge_id).unwrap();
    assert_eq!(req.status, BridgeStatus::PendingInbound);

    bridge.approve_inbound(&signer2, &bridge_id, &wrapped_id);
    let req = bridge.get_bridge_request(&bridge_id).unwrap();
    assert_eq!(req.status, BridgeStatus::InboundApproved);

    // Unwrap and unlock: recipient gets liquidity back, agent is unlocked.
    let recipient = Address::generate(&env);

    bridge.unwrap_and_unlock(&wrapped_id, &recipient);

    let req = bridge.get_bridge_request(&bridge_id).unwrap();
    assert_eq!(req.status, BridgeStatus::Completed);

    // Second unwrap attempt should fail
    let res = bridge.try_unwrap_and_unlock(&wrapped_id, &recipient);
    assert!(res.is_err());
}

#[test]
fn test_bridge_expiration() {
    let env = setup_env();
    let admin = Address::generate(&env);
    let owner = Address::generate(&env);
    let agent_contract = env.register_contract(None, MockAgentNFT);
    let payment_token = Address::generate(&env);

    let bridge_id_addr = env.register_contract(None, BridgeManager);
    let bridge = BridgeManagerClient::new(&env, &bridge_id_addr);

    let signer = Address::generate(&env);
    let signers = Vec::from_array(&env, [signer.clone()]);

    bridge.init_contract(&admin, &agent_contract, &payment_token, &signers, &1u32);

    let notional_value: i128 = 10_000;
    let target_chain: u32 = 3;
    let metadata_cid = soroban_sdk::String::from_str(&env, "QmTestCID123");
    let bridge_id =
        bridge.lock_and_bridge(&3u64, &owner, &metadata_cid, &target_chain, &notional_value);

    // Move time forward beyond expiration
    let now = env.ledger().timestamp();
    env.ledger()
        .set_timestamp(now + BRIDGE_EXPIRATION_SECONDS + 1);

    // Further approval attempts should fail with Expired
    let res = bridge.try_approve_outbound(&signer, &bridge_id);
    assert!(res.is_err());
}
