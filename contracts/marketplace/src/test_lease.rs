#![cfg(test)]

use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{token, Address, Env, String, Symbol};
use stellai_lib::storage_keys::LISTING_COUNTER_KEY;
use stellai_lib::types::{LateFeePolicy, LateFeeType, LeaseState, Listing, ListingType, PaymentFrequency};

use crate::{MarketplaceContract, MarketplaceContractClient};

fn setup_marketplace(env: &Env) -> (Address, Address) {
    let admin = Address::generate(env);
    let token_address = env.register_stellar_asset_contract(admin.clone());
    let contract_id = env.register_contract(None, MarketplaceContract);
    let client = MarketplaceContractClient::new(env, &contract_id);
    client.initialize(&admin, &token_address, &500);
    (contract_id, token_address)
}

fn seed_listing(env: &Env, contract_id: &Address, lessor: &Address, listing_id: u64, price: i128) {
    env.as_contract(contract_id, || {
        let listing = Listing {
            listing_id,
            agent_id: 10,
            seller: lessor.clone(),
            price,
            listing_type: ListingType::Lease,
            active: true,
            created_at: env.ledger().timestamp(),
        };
        env.storage().instance().set(&(Symbol::new(env, "listing"), listing_id), &listing);
        env.storage()
            .instance()
            .set(&Symbol::new(env, LISTING_COUNTER_KEY), &listing_id);
    });
}

fn create_standard_lease(
    env: &Env,
    auto_renew: bool,
) -> (MarketplaceContractClient<'_>, Address, Address, Address, Address, u64) {
    let (contract_id, token_address) = setup_marketplace(env);
    let client = MarketplaceContractClient::new(env, &contract_id);
    let lessor = Address::generate(env);
    let lessee = Address::generate(env);
    let token_admin = Address::generate(env);
    let token_client = token::StellarAssetClient::new(env, &token_address);
    token_client.mint(&lessee, &20_000);
    let _ = token_admin;

    let listing_id = 1u64;
    seed_listing(env, &contract_id, &lessor, listing_id, 1_000);

    let lease_id = client.initiate_lease_v2(
        &listing_id,
        &lessee,
        &259_200,
        &auto_renew,
        &auto_renew,
        &PaymentFrequency::Daily,
        &200,
        &2_000,
        &LateFeePolicy {
            fee_type: LateFeeType::Fixed,
            value: 20,
        },
    );

    (client, contract_id, token_address, lessor, lessee, lease_id)
}

#[test]
fn test_initiate_lease_tracks_history_and_deposit() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, contract_id, token_address, lessor, lessee, lease_id) =
        create_standard_lease(&env, true);

    let lease = client.get_lease_by_id(&lease_id).unwrap();
    assert_eq!(lease.lessor, lessor);
    assert_eq!(lease.lessee, lessee);
    assert_eq!(lease.deposit_amount, 100);
    assert_eq!(lease.outstanding_balance, 0);
    assert_eq!(lease.asset_class as u32, 0);

    let contract_balance = token::Client::new(&env, &token_address).balance(&contract_id);
    assert_eq!(contract_balance, 100);

    let history = client.get_lease_history(&lease_id);
    assert_eq!(history.len(), 1);
    assert_eq!(history.get(0).unwrap().action, String::from_str(&env, "initiated_v2"));
}

#[test]
fn test_scheduler_queues_notifications_and_auto_renews() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _contract_id, _token_address, _lessor, _lessee, lease_id) =
        create_standard_lease(&env, true);

    let lease = client.get_lease_by_id(&lease_id).unwrap();
    env.ledger().set_timestamp(lease.end_time - 3_600);
    client.process_due_lease(&lease_id);

    // Pay outstanding balance to allow renewal
    client.process_lease_payment(&lease_id, &_lessee);

    let notifications = client.get_lease_notifications(&lease_id);
    assert_eq!(notifications.len(), 2);
    assert_eq!(notifications.get(0).unwrap().channel as u32, 0);
    assert_eq!(notifications.get(1).unwrap().channel as u32, 1);

    env.ledger().set_timestamp(lease.end_time + 1);
    client.process_due_lease(&lease_id);

    let renewed = client.get_lease_by_id(&lease_id).unwrap();
    assert_eq!(renewed.status, LeaseState::Renewed);
    assert_eq!(renewed.current_renewal_count, 1);
    assert!(renewed.end_time > lease.end_time);
}

#[test]
fn test_scheduler_marks_overdue_and_payment_clears_balance() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _contract_id, token_address, lessor, lessee, lease_id) =
        create_standard_lease(&env, false);

    let lease = client.get_lease_by_id(&lease_id).unwrap();
    env.ledger().set_timestamp(lease.next_payment_timestamp + 3_600);
    client.process_due_lease(&lease_id);

    let overdue = client.get_lease_by_id(&lease_id).unwrap();
    assert_eq!(overdue.status, LeaseState::Overdue);
    assert_eq!(overdue.outstanding_balance, 220);
    assert_eq!(overdue.accrued_late_fees, 20);

    client.process_lease_payment(&lease_id, &lessee);

    let cleared = client.get_lease_by_id(&lease_id).unwrap();
    assert_eq!(cleared.status, LeaseState::Active);
    assert_eq!(cleared.outstanding_balance, 0);
    assert_eq!(cleared.accrued_late_fees, 0);

    let lessor_balance = token::Client::new(&env, &token_address).balance(&lessor);
    assert_eq!(lessor_balance, 209);
}

#[test]
fn test_lease_expires_without_auto_renew() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _contract_id, _token_address, _lessor, _lessee, lease_id) =
        create_standard_lease(&env, false);

    let lease = client.get_lease_by_id(&lease_id).unwrap();
    env.ledger().set_timestamp(lease.end_time + 1);
    client.process_due_lease(&lease_id);

    let expired = client.get_lease_by_id(&lease_id).unwrap();
    assert_eq!(expired.status, LeaseState::Expired);
}

#[test]
#[should_panic(expected = "Listing is not for sale")]
fn test_buy_agent_fails_on_lease_listing() {
    let env = Env::default();
    env.mock_all_auths();
    let (contract_id, token_address) = setup_marketplace(&env);
    let client = MarketplaceContractClient::new(&env, &contract_id);
    let lessor = Address::generate(&env);
    let lessee = Address::generate(&env);
    
    // Create an ACTIVE lease listing
    seed_listing(&env, &contract_id, &lessor, 1, 1000);
    
    // Try to buy it
    client.buy_agent(&1, &lessee);
}

#[test]
fn test_early_termination_reconciles_penalty_and_deposit() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, contract_id, token_address, _lessor, lessee, lease_id) =
        create_standard_lease(&env, false);

    let lease = client.get_lease_by_id(&lease_id).unwrap();
    env.ledger().set_timestamp(lease.next_payment_timestamp + 3_600);
    client.process_due_lease(&lease_id);

    let overdue = client.get_lease_by_id(&lease_id).unwrap();
    let now = env.ledger().timestamp();
    let remaining_time = overdue.end_time - now;
    let remaining_value = (overdue.total_value * remaining_time as i128) / overdue.duration_seconds as i128;
    let penalty = (remaining_value * overdue.termination_penalty_bps as i128) / 10_000;
    let required_settlement = overdue.outstanding_balance + penalty;
    let deposit_credit = if overdue.deposit_amount > required_settlement {
        required_settlement
    } else {
        overdue.deposit_amount
    };
    let amount_due = required_settlement - deposit_credit;

    client.early_termination(&lease_id, &lessee, &amount_due);

    let terminated = client.get_lease_by_id(&lease_id).unwrap();
    assert_eq!(terminated.status, LeaseState::Terminated);
    assert_eq!(terminated.deposit_amount, 0);
    assert_eq!(terminated.outstanding_balance, 0);

    let history = client.get_lease_history(&lease_id);
    assert_eq!(history.get(history.len() - 1).unwrap().action, String::from_str(&env, "early_terminated"));

    let client_balance = token::Client::new(&env, &token_address).balance(&contract_id);
    assert!(client_balance > 0, "Contract should hold platform fees");
}
