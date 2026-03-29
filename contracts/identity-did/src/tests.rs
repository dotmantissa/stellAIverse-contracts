// note: avoid SDK-specific test macros here; use standard #[test] and assert!
use soroban_sdk::symbol_short;
use soroban_sdk::testutils::{Address as TestAddress, Ledger as TestLedger};
use soroban_sdk::Address;
use soroban_sdk::Bytes;
use soroban_sdk::Env;
use soroban_sdk::String;
use soroban_sdk::Vec;

use crate::{DIDContract, DIDDocument, DIDRecord, DIDStatus, Error, Service, VerificationMethod};

#[test]
fn test_create_did_success() {
    let env = Env::default();
    let contract_id = env.register_contract(None, DIDContract);
    env.as_contract(&contract_id, || {
        // call contract functions directly in tests

        // Setup admin
        let admin = Address::generate(&env);
        env.storage()
            .instance()
            .set(&symbol_short!("admin"), &admin);

        // Create test addresses
        let controller = Address::generate(&env);

        // Create verification methods
        let vm1 = VerificationMethod {
            id: String::from_str(&env, "key-1"),
            type_: String::from_str(&env, "Ed25519VerificationKey2018"),
            controller: String::from_str(&env, "did:stellar:test"),
            public_key: Bytes::from_array(&env, &[1u8; 32]),
            created: env.ledger().timestamp(),
        };

        let verification_methods = Vec::from_array(&env, [vm1]);

        // Create services
        let service1 = Service {
            id: String::from_str(&env, "service-1"),
            type_: String::from_str(&env, "AgentRegistry"),
            service_endpoint: String::from_str(&env, "https://api.stellai.verse/agents"),
            created: env.ledger().timestamp(),
        };

        let services = Vec::from_array(&env, [service1]);

        let did = DIDContract::create_did(
            env.clone(),
            controller.clone(),
            verification_methods.clone(),
            services.clone(),
        )
        .unwrap();

        // Verify DID was created
        assert!(did.len() > 0);

        // Verify DID document
        let document = DIDContract::get_did_document(env.clone(), did.clone()).unwrap();
        assert!(document.controller == controller);
        assert!(document.verification_methods.len() == 1);
        assert!(document.service.len() == 1);
        assert!(document.version_id == 1);

        // Verify DID record
        let record = DIDContract::get_did_record(env.clone(), did.clone()).unwrap();
        assert!(record.status == DIDStatus::Active);
        assert!(record.document.controller == controller);
    });
}

#[test]
fn test_create_did_already_exists() {
    let env = Env::default();
    let contract_id = env.register_contract(None, DIDContract);
    env.as_contract(&contract_id, || {
        // call contract functions directly in tests

        // Setup admin
        let admin = Address::generate(&env);
        env.storage()
            .instance()
            .set(&symbol_short!("admin"), &admin);

        // Create test addresses
        let controller = Address::generate(&env);

        // Create verification methods
        let vm1 = VerificationMethod {
            id: String::from_str(&env, "key-1"),
            type_: String::from_str(&env, "Ed25519VerificationKey2018"),
            controller: String::from_str(&env, "did:stellar:test"),
            public_key: Bytes::from_array(&env, &[1u8; 32]),
            created: env.ledger().timestamp(),
        };

        let verification_methods = Vec::from_array(&env, [vm1]);
        let services = Vec::new(&env);

        // Create first DID
        let did1 = DIDContract::create_did(
            env.clone(),
            controller.clone(),
            verification_methods.clone(),
            services.clone(),
        )
        .unwrap();

        // Try to create another DID for same controller - should fail
        let result = DIDContract::create_did(
            env.clone(),
            controller.clone(),
            verification_methods.clone(),
            services.clone(),
        );
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), Error::DIDAlreadyExists);
    });
}

#[test]
fn test_update_did_success() {
    let env = Env::default();
    let contract_id = env.register_contract(None, DIDContract);
    env.as_contract(&contract_id, || {
        // call contract functions directly in tests

        // Setup admin
        let admin = Address::generate(&env);
        env.storage()
            .instance()
            .set(&symbol_short!("admin"), &admin);

        // Create test addresses
        let controller = Address::generate(&env);

        // Create verification methods
        let vm1 = VerificationMethod {
            id: String::from_str(&env, "key-1"),
            type_: String::from_str(&env, "Ed25519VerificationKey2018"),
            controller: String::from_str(&env, "did:stellar:test"),
            public_key: Bytes::from_array(&env, &[1u8; 32]),
            created: env.ledger().timestamp(),
        };

        let verification_methods = Vec::from_array(&env, [vm1]);
        let services = Vec::new(&env);

        let did = DIDContract::create_did(
            env.clone(),
            controller.clone(),
            verification_methods.clone(),
            services.clone(),
        )
        .unwrap();

        // Create new verification method
        let vm2 = VerificationMethod {
            id: String::from_str(&env, "key-2"),
            type_: String::from_str(&env, "Ed25519VerificationKey2018"),
            controller: String::from_str(&env, "did:stellar:test"),
            public_key: Bytes::from_array(&env, &[2u8; 32]),
            created: env.ledger().timestamp(),
        };

        let new_verification_methods = Vec::from_array(&env, [vm2]);

        let new_version = DIDContract::update_did(
            env.clone(),
            did.clone(),
            controller.clone(),
            Some(new_verification_methods),
            None,
        )
        .unwrap();

        // Verify update
        assert!(new_version == 2);

        let document = DIDContract::get_did_document(env.clone(), did.clone()).unwrap();
        assert!(document.version_id == 2);
        assert!(document.verification_methods.len() == 1);
        assert!(
            document.verification_methods.get(0).unwrap().id == String::from_str(&env, "key-2")
        );
    });
}

#[test]
fn test_update_did_unauthorized() {
    let env = Env::default();
    let contract_id = env.register_contract(None, DIDContract);
    env.as_contract(&contract_id, || {
        // call contract functions directly in tests

        // Setup admin
        let admin = Address::generate(&env);
        env.storage()
            .instance()
            .set(&symbol_short!("admin"), &admin);

        // Create test addresses
        let controller = Address::generate(&env);
        let unauthorized = Address::generate(&env);

        // Create verification methods
        let vm1 = VerificationMethod {
            id: String::from_str(&env, "key-1"),
            type_: String::from_str(&env, "Ed25519VerificationKey2018"),
            controller: String::from_str(&env, "did:stellar:test"),
            public_key: Bytes::from_array(&env, &[1u8; 32]),
            created: env.ledger().timestamp(),
        };

        let verification_methods = Vec::from_array(&env, [vm1]);
        let services = Vec::new(&env);

        let did = DIDContract::create_did(
            env.clone(),
            controller.clone(),
            verification_methods.clone(),
            services.clone(),
        )
        .unwrap();

        // Try to update DID with unauthorized address - should fail
        let result =
            DIDContract::update_did(env.clone(), did.clone(), unauthorized.clone(), None, None);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), Error::Unauthorized);
    });
}

#[test]
fn test_suspend_did_success() {
    let env = Env::default();
    let contract_id = env.register_contract(None, DIDContract);
    env.as_contract(&contract_id, || {
        // call contract functions directly in tests

        // Setup admin
        let admin = Address::generate(&env);
        env.storage()
            .instance()
            .set(&symbol_short!("admin"), &admin);

        // Create test addresses
        let controller = Address::generate(&env);

        // Create verification methods
        let vm1 = VerificationMethod {
            id: String::from_str(&env, "key-1"),
            type_: String::from_str(&env, "Ed25519VerificationKey2018"),
            controller: String::from_str(&env, "did:stellar:test"),
            public_key: Bytes::from_array(&env, &[1u8; 32]),
            created: env.ledger().timestamp(),
        };

        let verification_methods = Vec::from_array(&env, [vm1]);
        let services = Vec::new(&env);

        let did = DIDContract::create_did(
            env.clone(),
            controller.clone(),
            verification_methods.clone(),
            services.clone(),
        )
        .unwrap();

        // Suspend DID
        DIDContract::suspend_did(
            env.clone(),
            did.clone(),
            admin.clone(),
            String::from_str(&env, "Suspension for investigation"),
        )
        .unwrap();

        // Verify suspension
        let record = DIDContract::get_did_record(env.clone(), did.clone()).unwrap();
        assert!(record.status == DIDStatus::Suspended);
    });
}

#[test]
fn test_revoke_did_success() {
    let env = Env::default();
    let contract_id = env.register_contract(None, DIDContract);
    env.as_contract(&contract_id, || {
        // call contract functions directly in tests

        // Setup admin
        let admin = Address::generate(&env);
        env.storage()
            .instance()
            .set(&symbol_short!("admin"), &admin);

        // Create test addresses
        let controller = Address::generate(&env);

        // Create verification methods
        let vm1 = VerificationMethod {
            id: String::from_str(&env, "key-1"),
            type_: String::from_str(&env, "Ed25519VerificationKey2018"),
            controller: String::from_str(&env, "did:stellar:test"),
            public_key: Bytes::from_array(&env, &[1u8; 32]),
            created: env.ledger().timestamp(),
        };

        let verification_methods = Vec::from_array(&env, [vm1]);
        let services = Vec::new(&env);

        let did = DIDContract::create_did(
            env.clone(),
            controller.clone(),
            verification_methods.clone(),
            services.clone(),
        )
        .unwrap();

        // Revoke DID
        DIDContract::revoke_did(
            env.clone(),
            did.clone(),
            admin.clone(),
            String::from_str(&env, "Revocation for policy violation"),
        )
        .unwrap();

        // Verify revocation
        let record = DIDContract::get_did_record(env.clone(), did.clone()).unwrap();
        assert!(record.status == DIDStatus::Revoked);
    });
}

#[test]
fn test_reactivate_did_success() {
    let env = Env::default();
    let contract_id = env.register_contract(None, DIDContract);
    env.as_contract(&contract_id, || {
        // Setup admin
        let admin = Address::generate(&env);
        env.storage()
            .instance()
            .set(&symbol_short!("admin"), &admin);

        // Create test addresses
        let controller = Address::generate(&env);

        // Create verification methods
        let vm1 = VerificationMethod {
            id: String::from_str(&env, "key-1"),
            type_: String::from_str(&env, "Ed25519VerificationKey2018"),
            controller: String::from_str(&env, "did:stellar:test"),
            public_key: Bytes::from_array(&env, &[1u8; 32]),
            created: env.ledger().timestamp(),
        };

        let verification_methods = Vec::from_array(&env, [vm1]);
        let services = Vec::new(&env);

        let did = DIDContract::create_did(
            env.clone(),
            controller.clone(),
            verification_methods.clone(),
            services.clone(),
        )
        .unwrap();

        DIDContract::suspend_did(
            env.clone(),
            did.clone(),
            admin.clone(),
            String::from_str(&env, "Suspension for investigation"),
        )
        .unwrap();
        DIDContract::reactivate_did(env.clone(), did.clone(), admin.clone()).unwrap();

        // Verify reactivation
        let record = DIDContract::get_did_record(env.clone(), did.clone()).unwrap();
        assert!(record.status == DIDStatus::Active);
    });
}

#[test]
fn test_get_did_by_controller() {
    let env = Env::default();
    let contract_id = env.register_contract(None, DIDContract);
    env.as_contract(&contract_id, || {
        // Setup admin
        let admin = Address::generate(&env);
        env.storage()
            .instance()
            .set(&symbol_short!("admin"), &admin);

        // Create test addresses
        let controller = Address::generate(&env);

        // Create verification methods
        let vm1 = VerificationMethod {
            id: String::from_str(&env, "key-1"),
            type_: String::from_str(&env, "Ed25519VerificationKey2018"),
            controller: String::from_str(&env, "did:stellar:test"),
            public_key: Bytes::from_array(&env, &[1u8; 32]),
            created: env.ledger().timestamp(),
        };

        let verification_methods = Vec::from_array(&env, [vm1]);
        let services = Vec::new(&env);

        let did = DIDContract::create_did(
            env.clone(),
            controller.clone(),
            verification_methods.clone(),
            services.clone(),
        )
        .unwrap();

        // Get DID by controller
        let retrieved_did =
            DIDContract::get_did_by_controller(env.clone(), controller.clone()).unwrap();
        assert!(retrieved_did == did);
    });
}

#[test]
fn test_is_valid_did() {
    let env = Env::default();
    let contract_id = env.register_contract(None, DIDContract);
    env.as_contract(&contract_id, || {
        // Setup admin
        let admin = Address::generate(&env);
        env.storage()
            .instance()
            .set(&symbol_short!("admin"), &admin);

        // Create test addresses
        let controller = Address::generate(&env);

        // Create verification methods
        let vm1 = VerificationMethod {
            id: String::from_str(&env, "key-1"),
            type_: String::from_str(&env, "Ed25519VerificationKey2018"),
            controller: String::from_str(&env, "did:stellar:test"),
            public_key: Bytes::from_array(&env, &[1u8; 32]),
            created: env.ledger().timestamp(),
        };

        let verification_methods = Vec::from_array(&env, [vm1]);
        let services = Vec::new(&env);

        let did = DIDContract::create_did(
            env.clone(),
            controller.clone(),
            verification_methods.clone(),
            services.clone(),
        )
        .unwrap();

        // Check if DID is valid
        let is_valid = DIDContract::is_valid_did(env.clone(), did.clone()).unwrap();
        assert!(is_valid);

        DIDContract::suspend_did(
            env.clone(),
            did.clone(),
            admin.clone(),
            String::from_str(&env, "Test suspension"),
        )
        .unwrap();

        let is_valid_after_suspension =
            DIDContract::is_valid_did(env.clone(), did.clone()).unwrap();
        assert!(!is_valid_after_suspension);
    });
}

#[test]
fn test_get_did_history() {
    let env = Env::default();
    let contract_id = env.register_contract(None, DIDContract);
    env.as_contract(&contract_id, || {
        // Setup admin
        let admin = Address::generate(&env);
        env.storage()
            .instance()
            .set(&symbol_short!("admin"), &admin);

        // Create test addresses
        let controller = Address::generate(&env);

        // Create verification methods
        let vm1 = VerificationMethod {
            id: String::from_str(&env, "key-1"),
            type_: String::from_str(&env, "Ed25519VerificationKey2018"),
            controller: String::from_str(&env, "did:stellar:test"),
            public_key: Bytes::from_array(&env, &[1u8; 32]),
            created: env.ledger().timestamp(),
        };

        let verification_methods = Vec::from_array(&env, [vm1]);
        let services = Vec::new(&env);

        let did = DIDContract::create_did(
            env.clone(),
            controller.clone(),
            verification_methods.clone(),
            services.clone(),
        )
        .unwrap();

        // Get history
        let history = DIDContract::get_did_history(env.clone(), did.clone(), 10).unwrap();
        assert!(history.len() == 1);
        assert!(history.get(0).unwrap().action == String::from_str(&env, "created"));
    });
}
