//! Tests for advanced lease management: periodic payments, late fees, auto-renewal V2.

#![cfg(test)]

use soroban_sdk::testutils::Address as _;
use soroban_sdk::testutils::Ledger;
use soroban_sdk::{Address, Env, Symbol, token};
use stellai_lib::types::{
    LeaseState, Listing, ListingType, PaymentFrequency, LateFeeType, LateFeePolicy, 
};
use stellai_lib::storage_keys::LISTING_COUNTER_KEY;

use crate::{MarketplaceContract, MarketplaceContractClient};

fn setup_token<'a>(env: &'a Env, admin: &Address) -> (Address, token::StellarAssetClient<'a>) {
    let token_address = env.register_stellar_asset_contract(admin.clone());
    let token_client = token::StellarAssetClient::new(env, &token_address);
    (token_address, token_client)
}

#[test]
fn test_initiate_lease_v2() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, MarketplaceContract);
    let client = MarketplaceContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let (token_address, _token_client) = setup_token(&env, &admin);
    client.initialize(&admin, &token_address, &500);

    let lessor = Address::generate(&env);
    let lessee = Address::generate(&env);
    let token_client = token::StellarAssetClient::new(&env, &token_address);
    token_client.mint(&lessee, &10_000);
    
    // Create a listing
    let listing_id = 1u64;
    env.as_contract(&contract_id, || {
        let listing = Listing {
            listing_id,
            agent_id: 10,
            seller: lessor.clone(),
            price: 1000,
            listing_type: ListingType::Lease,
            active: true,
            created_at: env.ledger().timestamp(),
        };
        env.storage().instance().set(&(Symbol::new(&env, "listing"), listing_id), &listing);
        env.storage().instance().set(&Symbol::new(&env, LISTING_COUNTER_KEY), &listing_id);
    });

    let late_fee_policy = LateFeePolicy {
        fee_type: LateFeeType::Percentage,
        value: 500, // 5%
    };

    let lease_id = client.initiate_lease_v2(
        &listing_id,
        &lessee,
        &864000, // 10 days
        &true,
        &true,
        &PaymentFrequency::Daily,
        &100,
        &2000,
        &late_fee_policy
    );

    let lease = client.get_lease_by_id(&lease_id).unwrap();
    assert_eq!(lease.lease_id, lease_id);
    assert_eq!(lease.payment_interval, 86400); // Daily
    assert_eq!(lease.payment_amount, 100);
    assert_eq!(lease.late_fee_value, 500);
}

#[test]
fn test_process_lease_payment_with_late_fees() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, MarketplaceContract);
    let client = MarketplaceContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let (token_address, token_client) = setup_token(&env, &admin);
    client.initialize(&admin, &token_address, &500);

    let lessor = Address::generate(&env);
    let lessee = Address::generate(&env);
    
    token_client.mint(&lessee, &10000);

    // Create listing and lease
    let listing_id = 1u64;
    env.as_contract(&contract_id, || {
        let listing = Listing {
            listing_id,
            agent_id: 10,
            seller: lessor.clone(),
            price: 1000,
            listing_type: ListingType::Lease,
            active: true,
            created_at: env.ledger().timestamp(),
        };
        env.storage().instance().set(&(Symbol::new(&env, "listing"), listing_id), &listing);
        env.storage().instance().set(&Symbol::new(&env, LISTING_COUNTER_KEY), &listing_id);
    });

    let late_fee_policy = LateFeePolicy {
        fee_type: LateFeeType::Fixed,
        value: 20,
    };

    let lease_id = client.initiate_lease_v2(
        &listing_id,
        &lessee,
        &864000,
        &true,
        &true,
        &PaymentFrequency::Daily,
        &100,
        &2000,
        &late_fee_policy
    );

    // Advance time to 1 hour after due date
    let lease = client.get_lease_by_id(&lease_id).unwrap();
    let due_date = lease.next_payment_timestamp;
    env.ledger().set_timestamp(due_date + 3600); 

    client.process_lease_payment(&lease_id, &lessee);

    // Total should be 100 (base) + 20 (fixed late fee) = 120
    let lessor_balance = token::Client::new(&env, &token_address).balance(&lessor);
    assert_eq!(lessor_balance, 114);

    let lease_after = client.get_lease_by_id(&lease_id).unwrap();
    assert_eq!(lease_after.next_payment_timestamp, due_date + 86400);
}

#[test]
fn test_auto_renew_lease_v2() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, MarketplaceContract);
    let client = MarketplaceContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let (token_address, token_client) = setup_token(&env, &admin);
    client.initialize(&admin, &token_address, &500);

    let lessor = Address::generate(&env);
    let lessee = Address::generate(&env);
    token_client.mint(&lessee, &10_000);
    
    let listing_id = 1u64;
    env.as_contract(&contract_id, || {
        let listing = Listing {
            listing_id,
            agent_id: 10,
            seller: lessor.clone(),
            price: 1000,
            listing_type: ListingType::Lease,
            active: true,
            created_at: env.ledger().timestamp(),
        };
        env.storage().instance().set(&(Symbol::new(&env, "listing"), listing_id), &listing);
        env.storage().instance().set(&Symbol::new(&env, LISTING_COUNTER_KEY), &listing_id);
    });

    let lease_id = client.initiate_lease_v2(
        &listing_id,
        &lessee,
        &86400, // 1 day
        &true,
        &true,
        &PaymentFrequency::Daily,
        &100,
        &2000,
        &LateFeePolicy { fee_type: LateFeeType::None, value: 0 }
    );

    let lease = client.get_lease_by_id(&lease_id).unwrap();
    let end_time = lease.end_time;
    
    env.ledger().set_timestamp(end_time);
    
    client.auto_renew_lease_v2(&lease_id);

    let renewed_lease = client.get_lease_by_id(&lease_id).unwrap();
    assert_eq!(renewed_lease.status, LeaseState::Renewed);
    assert_eq!(renewed_lease.end_time, end_time + 86400);
    assert_eq!(renewed_lease.current_renewal_count, 1);
}
