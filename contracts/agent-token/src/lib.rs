//! Agent Token contract — fungible token representation of AI agent ownership.
//!
//! Refactored from contracts_backup/agent-token to use shared stellai_lib
//! types, validation, and admin helpers (Issue #88).

#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, String, Symbol, Vec};

use stellai_lib::{
    admin,
    errors::ContractError,
    storage_keys::{AGENT_COUNTER_KEY, APPROVED_MINTERS_KEY},
    types::{Agent, RoyaltyInfo},
    validation, ADMIN_KEY,
};

// ── Events ───────────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub enum AgentTokenEvent {
    TokenMinted,
    TokenTransferred,
    TokenBurned,
}

// ── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct AgentToken;

#[contractimpl]
impl AgentToken {
    /// One-time initialisation: set admin and zero out agent counter.
    pub fn init_contract(env: Env, admin_addr: Address) -> Result<(), ContractError> {
        if env
            .storage()
            .instance()
            .get::<_, Address>(&Symbol::new(&env, ADMIN_KEY))
            .is_some()
        {
            return Err(ContractError::AlreadyInitialized);
        }

        admin_addr.require_auth();
        env.storage()
            .instance()
            .set(&Symbol::new(&env, ADMIN_KEY), &admin_addr);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, AGENT_COUNTER_KEY), &0u64);

        let empty: Vec<Address> = Vec::new(&env);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, APPROVED_MINTERS_KEY), &empty);

        Ok(())
    }

    /// Admin: authorise a new minter address.
    pub fn add_approved_minter(
        env: Env,
        admin_addr: Address,
        minter: Address,
    ) -> Result<(), ContractError> {
        admin_addr.require_auth();
        admin::verify_admin(&env, &admin_addr)?;

        let mut minters: Vec<Address> = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, APPROVED_MINTERS_KEY))
            .unwrap_or_else(|| Vec::new(&env));
        minters.push_back(minter);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, APPROVED_MINTERS_KEY), &minters);
        Ok(())
    }

    /// Mint a new agent-token (approved minter or admin only).
    pub fn mint(
        env: Env,
        minter: Address,
        owner: Address,
        name: String,
        model_hash: String,
        metadata_cid: String,
        capabilities: Vec<String>,
        royalty: Option<RoyaltyInfo>,
    ) -> Result<u64, ContractError> {
        minter.require_auth();
        Self::verify_minter(&env, &minter)?;

        validation::validate_metadata(&name)?;
        validation::validate_metadata(&metadata_cid)?;
        validation::validate_capabilities(&capabilities)?;

        if let Some(ref r) = royalty {
            if r.fee > 2500 {
                return Err(ContractError::InvalidRoyaltyFee);
            }
        }

        let mut counter: u64 = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, AGENT_COUNTER_KEY))
            .unwrap_or(0);
        counter = counter.checked_add(1).ok_or(ContractError::OverflowError)?;

        let agent = Agent {
            id: counter,
            owner: owner.clone(),
            name,
            model_hash,
            metadata_cid,
            capabilities,
            evolution_level: 0,
            created_at: env.ledger().timestamp(),
            updated_at: env.ledger().timestamp(),
            nonce: 0,
            escrow_locked: false,
            escrow_holder: None,
        };

        let key = (Symbol::new(&env, "agent"), counter);
        env.storage().instance().set(&key, &agent);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, AGENT_COUNTER_KEY), &counter);

        if let Some(r) = royalty {
            let rkey = (Symbol::new(&env, "royalty"), counter);
            env.storage().instance().set(&rkey, &r);
        }

        env.events().publish(
            (
                Symbol::new(&env, "agent_token"),
                AgentTokenEvent::TokenMinted,
            ),
            (counter, owner),
        );

        Ok(counter)
    }

    /// Transfer ownership of an agent-token.
    pub fn transfer(
        env: Env,
        agent_id: u64,
        from: Address,
        to: Address,
    ) -> Result<(), ContractError> {
        if agent_id == 0 {
            return Err(ContractError::InvalidAgentId);
        }
        if from == to {
            return Err(ContractError::SameAddressTransfer);
        }

        from.require_auth();

        let key = (Symbol::new(&env, "agent"), agent_id);
        let mut agent: Agent = env
            .storage()
            .instance()
            .get(&key)
            .ok_or(ContractError::AgentNotFound)?;

        if agent.owner != from {
            return Err(ContractError::NotOwner);
        }

        agent.owner = to.clone();
        agent.updated_at = env.ledger().timestamp();
        env.storage().instance().set(&key, &agent);

        env.events().publish(
            (
                Symbol::new(&env, "agent_token"),
                AgentTokenEvent::TokenTransferred,
            ),
            (agent_id, from, to),
        );
        Ok(())
    }

    /// Burn (permanently destroy) an agent-token. Only the owner may burn.
    pub fn burn(env: Env, agent_id: u64, owner: Address) -> Result<(), ContractError> {
        if agent_id == 0 {
            return Err(ContractError::InvalidAgentId);
        }
        owner.require_auth();

        let key = (Symbol::new(&env, "agent"), agent_id);
        let agent: Agent = env
            .storage()
            .instance()
            .get(&key)
            .ok_or(ContractError::AgentNotFound)?;

        if agent.owner != owner {
            return Err(ContractError::NotOwner);
        }

        env.storage().instance().remove(&key);

        env.events().publish(
            (
                Symbol::new(&env, "agent_token"),
                AgentTokenEvent::TokenBurned,
            ),
            (agent_id, owner),
        );
        Ok(())
    }

    /// Query the owner of an agent-token.
    pub fn get_owner(env: Env, agent_id: u64) -> Result<Address, ContractError> {
        if agent_id == 0 {
            return Err(ContractError::InvalidAgentId);
        }
        let key = (Symbol::new(&env, "agent"), agent_id);
        let agent: Agent = env
            .storage()
            .instance()
            .get(&key)
            .ok_or(ContractError::AgentNotFound)?;
        Ok(agent.owner)
    }

    /// Total number of minted agent-tokens (not accounting for burns).
    pub fn total_supply(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&Symbol::new(&env, AGENT_COUNTER_KEY))
            .unwrap_or(0)
    }

    /// Transfer admin rights.
    pub fn transfer_admin(env: Env, current: Address, next: Address) -> Result<(), ContractError> {
        admin::transfer_admin(&env, &current, &next)
    }

    // ── helpers ──────────────────────────────────────────────────────────────

    fn verify_minter(env: &Env, caller: &Address) -> Result<(), ContractError> {
        if admin::verify_admin(env, caller).is_ok() {
            return Ok(());
        }
        let minters: Vec<Address> = env
            .storage()
            .instance()
            .get(&Symbol::new(env, APPROVED_MINTERS_KEY))
            .unwrap_or_else(|| Vec::new(env));
        if minters.contains(caller) {
            return Ok(());
        }
        Err(ContractError::Unauthorized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};

    fn setup(env: &Env) -> (AgentTokenClient, Address) {
        let id = env.register_contract(None, AgentToken);
        let client = AgentTokenClient::new(env, &id);
        let admin = Address::generate(env);
        env.mock_all_auths();
        client.init_contract(&admin);
        (client, admin)
    }

    #[test]
    fn test_mint_and_total_supply() {
        let env = Env::default();
        let (client, admin) = setup(&env);

        let owner = Address::generate(&env);
        let id = client.mint(
            &admin,
            &owner,
            &String::from_str(&env, "TestAgent"),
            &String::from_str(&env, "hash"),
            &String::from_str(&env, "QmCid"),
            &Vec::new(&env),
            &None,
        );

        assert_eq!(id, 1u64);
        assert_eq!(client.total_supply(), 1u64);
        assert_eq!(client.get_owner(&1u64), owner);
    }

    #[test]
    fn test_transfer_changes_owner() {
        let env = Env::default();
        let (client, admin) = setup(&env);
        let owner = Address::generate(&env);
        let next = Address::generate(&env);

        client.mint(
            &admin,
            &owner,
            &String::from_str(&env, "A"),
            &String::from_str(&env, "h"),
            &String::from_str(&env, "QmT"),
            &Vec::new(&env),
            &None,
        );

        client.transfer(&1u64, &owner, &next);
        assert_eq!(client.get_owner(&1u64), next);
    }

    #[test]
    fn test_burn_removes_token() {
        let env = Env::default();
        let (client, admin) = setup(&env);
        let owner = Address::generate(&env);

        client.mint(
            &admin,
            &owner,
            &String::from_str(&env, "B"),
            &String::from_str(&env, "h"),
            &String::from_str(&env, "QmB"),
            &Vec::new(&env),
            &None,
        );

        client.burn(&1u64, &owner);
        assert!(client.try_get_owner(&1u64).is_err());
    }

    #[test]
    fn test_non_minter_cannot_mint() {
        let env = Env::default();
        let (client, _admin) = setup(&env);
        let stranger = Address::generate(&env);

        let result = client.try_mint(
            &stranger,
            &stranger,
            &String::from_str(&env, "C"),
            &String::from_str(&env, "h"),
            &String::from_str(&env, "QmC"),
            &Vec::new(&env),
            &None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_double_init_fails() {
        let env = Env::default();
        let (client, admin) = setup(&env);
        let result = client.try_init_contract(&admin);
        match result {
            Err(Ok(ContractError::AlreadyInitialized)) => {}
            _ => panic!("Expected AlreadyInitialized"),
        }
    }
}
