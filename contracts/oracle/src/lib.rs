#![no_std]

#[cfg(test)]
mod tests;
#[cfg(any(test, feature = "testutils"))]
mod testutils;
mod types;

use soroban_sdk::{contract, contractimpl, Address, Bytes, BytesN, Env, String, Symbol, Val, Vec};
use stellai_lib::{
    admin,
    audit::{create_audit_log, OperationType},
    storage_keys::PROVIDER_LIST_KEY,
    types::OracleData,
    ADMIN_KEY,
};

pub use types::*;

#[contract]
pub struct Oracle;

#[contractimpl]
impl Oracle {
    pub fn init_contract(env: Env, admin: Address) {
        let admin_data: Option<Address> =
            env.storage().instance().get(&Symbol::new(&env, ADMIN_KEY));
        if admin_data.is_some() {
            panic!("Contract already initialized");
        }

        admin.require_auth();
        env.storage()
            .instance()
            .set(&Symbol::new(&env, ADMIN_KEY), &admin);

        let providers: Vec<Address> = Vec::new(&env);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, PROVIDER_LIST_KEY), &providers);
    }

    fn verify_admin(env: &Env, caller: &Address) {
        if admin::verify_admin(env, caller).is_err() {
            panic!("Caller is not admin");
        }
    }

    fn is_authorized_provider(env: &Env, provider: &Address) -> bool {
        let providers: Vec<Address> = env
            .storage()
            .instance()
            .get(&Symbol::new(env, PROVIDER_LIST_KEY))
            .unwrap_or_else(|| Vec::new(env));

        for p in providers.iter() {
            if &p == provider {
                return true;
            }
        }
        false
    }

    pub fn register_provider(env: Env, admin: Address, provider: Address) {
        admin.require_auth();
        Self::verify_admin(&env, &admin);

        let mut providers: Vec<Address> = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, PROVIDER_LIST_KEY))
            .unwrap_or_else(|| Vec::new(&env));

        for p in providers.iter() {
            if p == provider {
                panic!("Provider already registered");
            }
        }

        providers.push_back(provider.clone());
        env.storage()
            .instance()
            .set(&Symbol::new(&env, PROVIDER_LIST_KEY), &providers);

        env.events().publish(
            (Symbol::new(&env, "provider_registered"),),
            (admin, provider),
        );
    }

    pub fn submit_data(env: Env, provider: Address, key: Symbol, value: i128) {
        provider.require_auth();

        if !Self::is_authorized_provider(&env, &provider) {
            panic!("Unauthorized: provider not registered");
        }

        let timestamp = env.ledger().timestamp();

        let oracle_data = OracleData {
            key: key.clone(),
            value,
            timestamp,
            provider: provider.clone(),
            signature: None,
            source: None,
        };

        env.storage().instance().set(&key, &oracle_data);

        env.events().publish(
            (Symbol::new(&env, "data_submitted"),),
            (key.clone(), timestamp, provider.clone()),
        );

        // Log audit entry for oracle data submission
        let before_state = String::from_str(&env, "{}"); // No specific 'before' state for new data
                                                         // A simple after state, could be more detailed in a real scenario
        let after_state = String::from_str(&env, "{\"status\":\"submitted\"}");
        // In a real scenario, this would be the actual transaction hash
        let tx_hash = String::from_str(&env, "0x_placeholder_tx_hash");
        let description = Some(String::from_str(&env, "Oracle data submitted."));

        create_audit_log(
            &env,
            provider.clone(),
            OperationType::ConfigurationChange, // Using a general type as no specific one exists
            before_state,
            after_state,
            tx_hash,
            description,
        );
    }

    pub fn get_data(env: Env, key: Symbol) -> Option<OracleData> {
        env.storage().instance().get(&key)
    }

    pub fn deregister_provider(env: Env, admin: Address, provider: Address) {
        admin.require_auth();
        Self::verify_admin(&env, &admin);

        let providers: Vec<Address> = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, PROVIDER_LIST_KEY))
            .unwrap_or_else(|| Vec::new(&env));

        let mut updated_providers = Vec::new(&env);
        let mut found = false;

        for p in providers.iter() {
            if p != provider {
                updated_providers.push_back(p.clone());
            } else {
                found = true;
            }
        }

        if !found {
            panic!("Provider not found");
        }

        env.storage()
            .instance()
            .set(&Symbol::new(&env, PROVIDER_LIST_KEY), &updated_providers);

        env.events().publish(
            (Symbol::new(&env, "provider_deregistered"),),
            (admin, provider),
        );
    }

    fn is_approved_oracle_key(env: &Env, oracle_pubkey: &BytesN<32>) -> bool {
        env.storage()
            .instance()
            .get::<_, bool>(&DataKey::Oracle(oracle_pubkey.clone()))
            .unwrap_or(false)
    }

    pub fn register_oracle_key(env: Env, admin: Address, oracle_pubkey: BytesN<32>) {
        admin.require_auth();
        Self::verify_admin(&env, &admin);

        if Self::is_approved_oracle_key(&env, &oracle_pubkey) {
            panic!("Oracle key already registered");
        }

        env.storage()
            .instance()
            .set(&DataKey::Oracle(oracle_pubkey.clone()), &true);

        env.events().publish(
            (Symbol::new(&env, "oracle_key_registered"),),
            (admin, oracle_pubkey),
        );
    }

    pub fn deregister_oracle_key(env: Env, admin: Address, oracle_pubkey: BytesN<32>) {
        admin.require_auth();
        Self::verify_admin(&env, &admin);

        if !Self::is_approved_oracle_key(&env, &oracle_pubkey) {
            panic!("Oracle key not found");
        }

        env.storage()
            .instance()
            .remove(&DataKey::Oracle(oracle_pubkey.clone()));
        env.storage()
            .instance()
            .remove(&DataKey::OracleNonce(oracle_pubkey.clone()));

        env.events().publish(
            (Symbol::new(&env, "oracle_key_deregistered"),),
            (admin, oracle_pubkey),
        );
    }

    pub fn is_registered_oracle_key(env: Env, oracle_pubkey: BytesN<32>) -> bool {
        Self::is_approved_oracle_key(&env, &oracle_pubkey)
    }

    fn get_oracle_nonce(env: &Env, oracle_pubkey: &BytesN<32>) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::OracleNonce(oracle_pubkey.clone()))
            .unwrap_or(0u64)
    }

    fn set_oracle_nonce(env: &Env, oracle_pubkey: &BytesN<32>, nonce: u64) {
        env.storage()
            .instance()
            .set(&DataKey::OracleNonce(oracle_pubkey.clone()), &nonce);
    }

    fn build_relay_message(env: &Env, req: &RelayRequest) -> Bytes {
        // Simplified implementation - just create a hash from the deadline and nonce
        let deadline_bytes = req.deadline.to_be_bytes();
        let nonce_bytes = req.nonce.to_be_bytes();
        let mut combined = [0u8; 16];
        combined[..8].copy_from_slice(&deadline_bytes);
        combined[8..].copy_from_slice(&nonce_bytes);
        let data_bytes = Bytes::from_array(env, &combined);
        let hash = env.crypto().sha256(&data_bytes);
        Bytes::from_array(env, &hash.to_array())
    }

    pub fn relay_signed(
        env: Env,
        oracle_pubkey: BytesN<32>,
        target_contract: Address,
        function: Symbol,
        args: Vec<Val>,
        nonce: u64,
        deadline: u64,
        signature: BytesN<64>,
    ) -> Val {
        if !Self::is_approved_oracle_key(&env, &oracle_pubkey) {
            panic!("Oracle not approved");
        }

        if env.ledger().timestamp() > deadline {
            panic!("Signature expired");
        }

        let stored_nonce = Self::get_oracle_nonce(&env, &oracle_pubkey);
        if nonce <= stored_nonce {
            panic!("Invalid nonce: replay protection triggered");
        }

        let req = RelayRequest {
            relay_contract: env.current_contract_address(),
            oracle_pubkey: oracle_pubkey.clone(),
            target_contract: target_contract.clone(),
            function: function.clone(),
            args: args.clone(),
            nonce,
            deadline,
        };

        let message = Self::build_relay_message(&env, &req);
        env.crypto()
            .ed25519_verify(&oracle_pubkey, &message, &signature);

        Self::set_oracle_nonce(&env, &oracle_pubkey, nonce);

        let result: Val = env.invoke_contract(&target_contract, &function, args);

        env.events().publish(
            (Symbol::new(&env, "payload_relayed"),),
            (oracle_pubkey, target_contract, function, nonce),
        );

        result
    }
}
