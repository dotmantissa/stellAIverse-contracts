use super::*;
use soroban_sdk::{testutils::Address as _, Address, Bytes, Env, Symbol, Vec};
use types::*;

// ─── Helpers ───────────────────────────────────────────────────────

fn setup_env() -> (Env, Address, LifecycleManagerClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(LifecycleManager, ());
    let client = LifecycleManagerClient::new(&env, &contract_id);

    client.init_contract(&admin);

    (env, admin, client)
}

// ─── Tests ─────────────────────────────────────────────────────────

#[test]
fn test_init_and_default_config() {
    let (_env, _admin, client) = setup_env();

    let config = client.get_ttl_config();
    assert_eq!(config.active_ttl, DEFAULT_ACTIVE_TTL);
    assert_eq!(config.historical_ttl, DEFAULT_HISTORICAL_TTL);
    assert_eq!(config.archived_ttl, DEFAULT_ARCHIVED_TTL);
}

#[test]
#[should_panic(expected = "Contract already initialized")]
fn test_double_init() {
    let (_env, admin, client) = setup_env();
    client.init_contract(&admin);
}

#[test]
fn test_set_ttl_config() {
    let (_env, admin, client) = setup_env();

    let new_config = TtlConfig {
        active_ttl: 100_000,
        historical_ttl: 50_000,
        archived_ttl: 10_000,
    };
    client.set_ttl_config(&admin, &new_config);

    let config = client.get_ttl_config();
    assert_eq!(config.active_ttl, 100_000);
    assert_eq!(config.historical_ttl, 50_000);
    assert_eq!(config.archived_ttl, 10_000);
}

#[test]
#[should_panic(expected = "Active TTL must exceed historical")]
fn test_set_ttl_config_invalid_ordering() {
    let (_env, admin, client) = setup_env();

    let bad_config = TtlConfig {
        active_ttl: 1000,
        historical_ttl: 2000, // greater than active: invalid
        archived_ttl: 500,
    };
    client.set_ttl_config(&admin, &bad_config);
}

#[test]
fn test_extend_ttl_active() {
    let (env, _admin, client) = setup_env();

    let key = Symbol::new(&env, "agent_1");
    // Place data in persistent storage so extend_ttl finds it.
    env.as_contract(&client.address, || {
        env.storage().persistent().set(&key, &1u32);
    });

    // Should not panic.
    client.extend_ttl(&key, &DataLifecycle::Active);
}

#[test]
fn test_extend_ttl_historical() {
    let (env, _admin, client) = setup_env();

    let key = Symbol::new(&env, "tx_42");
    env.as_contract(&client.address, || {
        env.storage().persistent().set(&key, &42u32);
    });

    client.extend_ttl(&key, &DataLifecycle::Historical);
}

#[test]
fn test_extend_ttl_archived() {
    let (env, _admin, client) = setup_env();

    let key = Symbol::new(&env, "old_data");
    env.as_contract(&client.address, || {
        env.storage().persistent().set(&key, &0u32);
    });

    client.extend_ttl(&key, &DataLifecycle::Archived);
}

#[test]
fn test_extend_ttl_nonexistent_key() {
    let (env, _admin, client) = setup_env();

    let key = Symbol::new(&env, "missing");
    // Should not panic even if key does not exist.
    client.extend_ttl(&key, &DataLifecycle::Active);
}

#[test]
fn test_archive_entry() {
    let (env, admin, client) = setup_env();

    let key = Symbol::new(&env, "listing_1");
    let archived_key = Symbol::new(&env, "listing_1_arc");
    let data = Bytes::from_array(&env, &[1, 2, 3, 4]);

    // Store data.
    env.as_contract(&client.address, || {
        env.storage().persistent().set(&key, &data);
    });

    client.archive_entry(&admin, &key, &archived_key);

    // Original key should be gone; archived key should exist.
    env.as_contract(&client.address, || {
        assert!(!env.storage().persistent().has(&key));
        assert!(env.storage().persistent().has(&archived_key));

        let stored: Bytes = env.storage().persistent().get(&archived_key).unwrap();
        assert_eq!(stored, data);
    });
}

#[test]
#[should_panic(expected = "Entry not found")]
fn test_archive_nonexistent_entry() {
    let (env, admin, client) = setup_env();

    let key = Symbol::new(&env, "missing");
    let archived_key = Symbol::new(&env, "missing_arc");

    client.archive_entry(&admin, &key, &archived_key);
}

#[test]
fn test_cleanup_expired() {
    let (env, admin, client) = setup_env();

    // key1 exists, key2 does not (simulating expiration).
    let key1 = Symbol::new(&env, "alive");
    let key2 = Symbol::new(&env, "expired");

    env.as_contract(&client.address, || {
        env.storage().persistent().set(&key1, &1u32);
        // key2 is NOT stored, simulating an already-expired entry.
    });

    let mut keys = Vec::new(&env);
    keys.push_back(key1.clone());
    keys.push_back(key2.clone());

    // Should not panic.
    client.cleanup_expired(&admin, &keys);
}

#[test]
fn test_batch_extend() {
    let (env, _admin, client) = setup_env();

    let key1 = Symbol::new(&env, "k1");
    let key2 = Symbol::new(&env, "k2");

    env.as_contract(&client.address, || {
        env.storage().persistent().set(&key1, &1u32);
        env.storage().persistent().set(&key2, &2u32);
    });

    let mut keys = Vec::new(&env);
    keys.push_back(key1);
    keys.push_back(key2);

    client.batch_extend(&keys, &DataLifecycle::Active);
}

#[test]
fn test_get_entry_state_default() {
    let (env, _admin, client) = setup_env();

    let key = Symbol::new(&env, "unknown");
    let state = client.get_entry_state(&key);
    assert_eq!(state, DataLifecycle::Active);
}

#[test]
fn test_lifecycle_transition_via_archive() {
    let (env, admin, client) = setup_env();

    let key = Symbol::new(&env, "entry_1");
    let archived_key = Symbol::new(&env, "entry_1_arc");
    let data = Bytes::from_array(&env, &[10, 20, 30]);

    env.as_contract(&client.address, || {
        env.storage().persistent().set(&key, &data);
    });

    // Archive the entry.
    client.archive_entry(&admin, &key, &archived_key);

    // The archived key should have Archived state tracked.
    let state = client.get_entry_state(&archived_key);
    assert_eq!(state, DataLifecycle::Archived);
}

#[test]
fn test_ttl_config_values_match_spec() {
    // Verify that default constants match the spec: Active~1yr, Historical~6mo, Archived~1mo.
    assert_eq!(DEFAULT_ACTIVE_TTL, 52560);
    assert_eq!(DEFAULT_HISTORICAL_TTL, 26280);
    assert_eq!(DEFAULT_ARCHIVED_TTL, 5256);
    #[allow(clippy::assertions_on_constants)]
    {
        assert!(DEFAULT_ACTIVE_TTL > DEFAULT_HISTORICAL_TTL);
        assert!(DEFAULT_HISTORICAL_TTL > DEFAULT_ARCHIVED_TTL);
    }
}
