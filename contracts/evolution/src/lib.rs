#![no_std]

mod evolution_history;

use evolution_history::{
    append_evolution, get_evolution_at_index, get_evolution_count, get_evolution_history,
    get_latest_evolution, EvolutionRecord,
};
use soroban_sdk::{contract, contractimpl, Address, Env, String, Symbol, Vec};
use stellai_lib::{
    admin,
    audit::{create_audit_log, OperationType},
    rbac,
    storage_keys::REQUEST_COUNTER_KEY,
    types::{EvolutionRequest, EvolutionStatus},
    ADMIN_KEY,
};

#[contract]
pub struct Evolution;

#[contractimpl]
impl Evolution {
    /// Initialize contract with admin
    pub fn init_contract(env: Env, admin: Address) {
        let admin_data = env
            .storage()
            .instance()
            .get::<_, Address>(&Symbol::new(&env, ADMIN_KEY));
        if admin_data.is_some() {
            panic!("Contract already initialized");
        }

        admin.require_auth();
        env.storage()
            .instance()
            .set(&Symbol::new(&env, ADMIN_KEY), &admin);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, REQUEST_COUNTER_KEY), &0u64);
    }

    /// Create an evolution request
    pub fn create_request(env: Env, agent_id: u64, owner: Address, stake_amount: i128) -> u64 {
        owner.require_auth();

        if agent_id == 0 {
            panic!("Invalid agent ID");
        }
        if stake_amount <= 0 {
            panic!("Stake amount must be positive");
        }

        let counter: u64 = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, REQUEST_COUNTER_KEY))
            .unwrap_or(0);
        let request_id = counter + 1;

        let request = EvolutionRequest {
            request_id,
            agent_id,
            owner: owner.clone(),
            stake_amount,
            status: EvolutionStatus::Pending,
            created_at: env.ledger().timestamp(),
            completed_at: None,
        };

        // Use tuple as key (prefix, request_id)
        let request_key = (Symbol::new(&env, "request"), request_id);
        env.storage().instance().set(&request_key, &request);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, REQUEST_COUNTER_KEY), &request_id);

        env.events().publish(
            (Symbol::new(&env, "request_created"),),
            (request_id, agent_id, owner.clone(), stake_amount),
        );

        // Audit log for evolution request creation
        let before_state = String::from_str(&env, "{}");
        let after_state = String::from_str(&env, "{\"request_created\":true}");
        let tx_hash = String::from_str(&env, "create_request");
        let description = Some(String::from_str(&env, "Evolution request created"));

        let _ = create_audit_log(
            &env,
            owner,
            OperationType::ConfigurationChange,
            before_state,
            after_state,
            tx_hash,
            description,
        );

        request_id
    }

    /// Get an evolution request
    pub fn get_request(env: Env, request_id: u64) -> Option<EvolutionRequest> {
        if request_id == 0 {
            panic!("Invalid request ID");
        }

        let request_key = (Symbol::new(&env, "request"), request_id);
        env.storage().instance().get(&request_key)
    }

    /// Execute an evolution request (Admin only)
    /// This approves the request and records the history.
    pub fn execute_evolution(env: Env, request_id: u64, from_stage: u32, to_stage: u32) {
        // 1. Verify Admin Auth — re-reads from storage, no implicit trust (Issue #152)
        let admin: Address = admin::get_admin(&env).unwrap();
        admin.require_auth();
        rbac::require_admin(&env, &admin)
            .unwrap_or_else(|_| panic!("Unauthorized: caller is not admin"));

        // 2. Get the request
        let request_key = (Symbol::new(&env, "request"), request_id);
        let mut request: EvolutionRequest = env
            .storage()
            .instance()
            .get(&request_key)
            .expect("Request not found");

        // 3. Verify status
        if request.status != EvolutionStatus::Pending {
            panic!("Request is not pending");
        }

        // 4. Update status
        request.status = EvolutionStatus::Completed;
        request.completed_at = Some(env.ledger().timestamp());
        env.storage().instance().set(&request_key, &request);

        // 5. Record evolution in immutable history (Step 3)
        // Note: We use the owner's address to track the history
        append_evolution(
            &env,
            &request.owner,
            from_stage,
            to_stage,
            Symbol::new(&env, "admin_exe"),
        );

        // 6. Emit event
        env.events().publish(
            (Symbol::new(&env, "evolution_executed"),),
            (request_id, request.agent_id, to_stage),
        );

        // 7. Audit log for evolution execution
        let before_state = String::from_str(&env, "{\"status\":\"pending\"}");
        let after_state = String::from_str(&env, "{\"status\":\"completed\"}");
        let tx_hash = String::from_str(&env, "execute_evolution");
        let description = Some(String::from_str(&env, "Evolution executed"));

        let _ = create_audit_log(
            &env,
            admin,
            OperationType::AdminSettingsChange,
            before_state,
            after_state,
            tx_hash,
            description,
        );
    }

    // Step 4: Add public getter functions

    /// Get full evolution history for an agent
    pub fn get_agent_evolution_history(env: Env, agent_id: Address) -> Vec<EvolutionRecord> {
        get_evolution_history(&env, &agent_id)
    }

    /// Get total number of evolutions for an agent
    pub fn get_agent_evolution_count(env: Env, agent_id: Address) -> u32 {
        get_evolution_count(&env, &agent_id)
    }

    /// Get evolution record at specific index
    pub fn get_agent_evolution_at(
        env: Env,
        agent_id: Address,
        index: u32,
    ) -> Option<EvolutionRecord> {
        get_evolution_at_index(&env, &agent_id, index)
    }

    /// Get most recent evolution
    pub fn get_agent_latest_evolution(env: Env, agent_id: Address) -> Option<EvolutionRecord> {
        get_latest_evolution(&env, &agent_id)
    }
} // <--- End of impl Evolution

// Test module must be outside the impl block
#[cfg(test)]
mod test;
