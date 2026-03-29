#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, String, Symbol, Vec};
mod test;

// ============================================================================
// LIBRARY IMPORTS
// We import the shared types and errors from stellai_lib here.
// ============================================================================
use stellai_lib::{
    admin,
    audit::{create_audit_log, OperationType},
    errors::ContractError,
    storage_keys::{AGENT_COUNTER_KEY, APPROVED_MINTERS_KEY},
    types::{Agent, OptionalRoyaltyInfo, RoyaltyInfo},
    validation, ADMIN_KEY,
};

// ============================================================================
// Event types
// ============================================================================
#[contracttype]
#[derive(Clone)]
pub enum AgentEvent {
    AgentMinted,
    AgentUpdated,
    AgentTransferred,
    LeaseStarted,
    LeaseEnded,
    BatchMintCompleted,
}

// ============================================================================
// Batch Mint Data Structure
// ============================================================================
#[contracttype]
#[derive(Clone, Debug)]
pub struct AgentMintData {
    pub owner: Address,
    pub name: String,
    pub model_hash: String,
    pub metadata_cid: String,
    pub capabilities: Vec<String>,
    pub royalty: OptionalRoyaltyInfo,
}

#[contract]
pub struct AgentNFT;
#[contractimpl]
impl AgentNFT {
    /// Initialize contract with admin (one-time setup)
    pub fn init_contract(env: Env, admin: Address) -> Result<(), ContractError> {
        // Security: Verify this is first initialization
        let admin_data = env
            .storage()
            .instance()
            .get::<_, Address>(&Symbol::new(&env, ADMIN_KEY));
        if admin_data.is_some() {
            return Err(ContractError::AlreadyInitialized);
        }

        admin.require_auth();
        env.storage()
            .instance()
            .set(&Symbol::new(&env, ADMIN_KEY), &admin);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, AGENT_COUNTER_KEY), &0u64);

        // Initialize approved minters list (empty by default)
        let approved_minters: Vec<Address> = Vec::new(&env);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, APPROVED_MINTERS_KEY), &approved_minters);

        Ok(())
    }

    /// Add an approved minter (admin only)
    pub fn add_approved_minter(
        env: Env,
        admin: Address,
        minter: Address,
    ) -> Result<(), ContractError> {
        admin.require_auth();
        Self::verify_admin(&env, &admin)?;

        let mut approved_minters: Vec<Address> = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, APPROVED_MINTERS_KEY))
            .unwrap_or_else(|| Vec::new(&env));

        approved_minters.push_back(minter);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, APPROVED_MINTERS_KEY), &approved_minters);

        Ok(())
    }

    /// Helper to get storage key for an agent
    fn get_agent_key(env: &Env, agent_id: u64) -> (Symbol, u64) {
        (Symbol::new(env, "agent"), agent_id)
    }

    /// Helper to get storage key for agent lease status
    fn get_agent_lease_key(env: &Env, agent_id: u64) -> (Symbol, u64) {
        (Symbol::new(env, "lease"), agent_id)
    }

    /// Helper to get storage key for agent royalty info
    fn get_royalty_key(env: &Env, agent_id: u64) -> (Symbol, u64) {
        (Symbol::new(env, "royalty"), agent_id)
    }

    /// Validate royalty fee is within acceptable bounds
    fn validate_royalty_fee(fee: u32) -> Result<(), ContractError> {
        if fee > 2500 {
            return Err(ContractError::InvalidRoyaltyFee);
        }
        Ok(())
    }

    /// Verify caller is admin
    fn verify_admin(env: &Env, caller: &Address) -> Result<(), ContractError> {
        if admin::verify_admin(env, caller).is_err() {
            // Log the authorization failure
            let before_state = String::from_str(&env, "{}");
            let after_state = String::from_str(&env, "{}");
            let tx_hash = String::from_str(&env, "verify_admin_fail"); // Placeholder
            let description = Some(String::from_str(&env, "Admin verification failed."));

            let _ = create_audit_log(
                &env,
                caller.clone(),
                OperationType::AuthFailure,
                before_state,
                after_state,
                tx_hash,
                description,
            );
            return Err(ContractError::Unauthorized);
        }
        Ok(())
    }

    /// Verify caller is admin or approved minter
    fn verify_minter(env: &Env, caller: &Address) -> Result<(), ContractError> {
        // Check if admin
        if let Some(admin) = env
            .storage()
            .instance()
            .get::<_, Address>(&Symbol::new(env, ADMIN_KEY))
        {
            if caller == &admin {
                return Ok(());
            }
        }

        // Check if approved minter
        let approved_minters: Vec<Address> = env
            .storage()
            .instance()
            .get(&Symbol::new(env, APPROVED_MINTERS_KEY))
            .unwrap_or_else(|| Vec::new(env));

        for i in 0..approved_minters.len() {
            if let Some(minter) = approved_minters.get(i) {
                if &minter == caller {
                    return Ok(());
                }
            }
        }

        // If we reach here, no match was found.
        let before_state = String::from_str(&env, "{}");
        let after_state = String::from_str(&env, "{}");
        let tx_hash = String::from_str(&env, "verify_minter_fail"); // Placeholder
        let description = Some(String::from_str(&env, "Minter verification failed."));

        let _ = create_audit_log(
            &env,
            caller.clone(),
            OperationType::AuthFailure,
            before_state,
            after_state,
            tx_hash,
            description,
        );
        Err(ContractError::Unauthorized)
    }

    /// Safe addition with overflow checks
    fn safe_add(a: u64, b: u64) -> Result<u64, ContractError> {
        a.checked_add(b).ok_or(ContractError::OverflowError)
    }

    /// Check if agent is currently leased
    fn is_agent_leased(env: &Env, agent_id: u64) -> bool {
        let lease_key = Self::get_agent_lease_key(env, agent_id);
        env.storage()
            .instance()
            .get::<_, bool>(&lease_key)
            .unwrap_or(false)
    }

    /// Set agent lease status
    fn set_agent_lease_status(env: &Env, agent_id: u64, is_leased: bool) {
        let lease_key = Self::get_agent_lease_key(env, agent_id);
        env.storage().instance().set(&lease_key, &is_leased);
    }

    /// Check if agent ID already exists
    fn agent_exists(env: &Env, agent_id: u64) -> bool {
        let key = Self::get_agent_key(env, agent_id);
        env.storage().instance().has(&key)
    }

    /// Mint a new agent NFT - Implements requirement from issue
    ///
    /// # Arguments
    /// * `agent_id` - Unique identifier for the agent (u128 in spec, using u64 for storage efficiency)
    /// * `owner` - Address of the agent owner
    /// * `metadata_cid` - IPFS CID for agent metadata
    /// * `initial_evolution_level` - Starting evolution level
    /// * `royalty_recipient` - Optional address to receive royalty payments (basis points)
    /// * `royalty_fee` - Optional royalty fee in basis points (0-10000, where 10000 = 100%)
    ///
    /// # Returns
    /// Result<(), ContractError>
    ///
    /// # Errors
    /// - ContractError::Unauthorized if caller is not admin or approved minter
    /// - ContractError::DuplicateAgentId if agent_id already exists
    /// - ContractError::InvalidInput if validation fails
    /// - ContractError::InvalidRoyaltyFee if royalty fee exceeds maximum (10000)
    pub fn mint_agent(
        env: Env,
        agent_id: u128,
        owner: Address,
        metadata_cid: String,
        initial_evolution_level: u32,
        royalty_recipient: Option<Address>,
        royalty_fee: Option<u32>,
    ) -> Result<(), ContractError> {
        owner.require_auth();

        // Validate caller authorization (admin or approved minter)
        Self::verify_minter(&env, &owner)?;

        // Convert u128 to u64 for storage (validate it fits)
        let agent_id_u64 = agent_id
            .try_into()
            .map_err(|_| ContractError::InvalidInput)?;

        // Enforce uniqueness of agent_id
        if Self::agent_exists(&env, agent_id_u64) {
            return Err(ContractError::DuplicateAgentId);
        }

        // Input validation
        validation::validate_metadata(&metadata_cid)?;

        // Validate and store royalty info if provided
        if let (Some(recipient), Some(fee)) = (&royalty_recipient, royalty_fee) {
            Self::validate_royalty_fee(fee)?;
            let royalty_info = RoyaltyInfo {
                recipient: recipient.clone(),
                fee,
            };
            let royalty_key = Self::get_royalty_key(&env, agent_id_u64);
            env.storage().instance().set(&royalty_key, &royalty_info);
        } else if royalty_recipient.is_some() || royalty_fee.is_some() {
            // Both must be provided together or neither
            return Err(ContractError::InvalidInput);
        }

        // Create agent with metadata CID and evolution level
        let agent = Agent {
            id: agent_id_u64,
            owner: owner.clone(),
            name: String::from_str(&env, ""), // Can be set via update_agent
            model_hash: String::from_str(&env, ""), // Can be set via update_agent
            metadata_cid,
            capabilities: Vec::new(&env),
            evolution_level: initial_evolution_level,
            created_at: env.ledger().timestamp(),
            updated_at: env.ledger().timestamp(),
            nonce: 0,
            escrow_locked: false,
            escrow_holder: None,
        };

        // Persist agent data
        let key = Self::get_agent_key(&env, agent_id_u64);
        env.storage().instance().set(&key, &agent);

        // Initialize lease status to false (not leased)
        Self::set_agent_lease_status(&env, agent_id_u64, false);

        // Emit AgentMinted event
        env.events().publish(
            (Symbol::new(&env, "agent_nft"), AgentEvent::AgentMinted),
            (agent_id_u64, owner.clone(), initial_evolution_level),
        );

        // Log audit entry for admin mint operation
        let before_state = String::from_str(&env, "{}");
        let after_state = String::from_str(&env, "{\"created\":true}");
        let tx_hash = String::from_str(&env, "mint_agent");
        let description = Some(String::from_str(&env, "AgentNFT minted"));

        let _ = create_audit_log(
            &env,
            owner.clone(),
            OperationType::AdminMint,
            before_state,
            after_state,
            tx_hash,
            description,
        );

        Ok(())
    }

    /// Legacy mint function for backward compatibility
    ///
    /// # Arguments
    /// * `owner` - Address of the agent owner
    /// * `name` - Agent name
    /// * `model_hash` - Hash of the agent model
    /// * `capabilities` - List of agent capabilities
    /// * `royalty_recipient` - Optional address to receive royalty payments
    /// * `royalty_fee` - Optional royalty fee in basis points (0-10000, where 10000 = 100%)
    ///
    /// # Returns
    /// Result<u64, ContractError> - The minted agent ID
    ///
    /// # Errors
    /// - ContractError::Unauthorized if caller is not admin or approved minter
    /// - ContractError::InvalidInput if validation fails
    /// - ContractError::InvalidRoyaltyFee if royalty fee exceeds maximum (10000)
    pub fn mint_agent_legacy(
        env: Env,
        owner: Address,
        name: String,
        model_hash: String,
        capabilities: Vec<String>,
        royalty_recipient: Option<Address>,
        royalty_fee: Option<u32>,
    ) -> Result<u64, ContractError> {
        owner.require_auth();

        // Validate caller authorization
        Self::verify_minter(&env, &owner)?;

        // Input validation
        validation::validate_metadata(&name)?;
        validation::validate_metadata(&model_hash)?;
        validation::validate_capabilities(&capabilities)?;

        // Validate and store royalty info if provided
        if let (Some(recipient), Some(fee)) = (&royalty_recipient, royalty_fee) {
            Self::validate_royalty_fee(fee)?;
        } else if royalty_recipient.is_some() || royalty_fee.is_some() {
            // Both must be provided together or neither
            return Err(ContractError::InvalidInput);
        }

        // Increment agent counter safely
        let counter: u64 = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, AGENT_COUNTER_KEY))
            .unwrap_or(0);

        let agent_id = Self::safe_add(counter, 1)?;

        // Create agent
        let agent = Agent {
            id: agent_id,
            owner: owner.clone(),
            name,
            model_hash,
            metadata_cid: String::from_str(&env, ""),
            capabilities,
            evolution_level: 0,
            created_at: env.ledger().timestamp(),
            updated_at: env.ledger().timestamp(),
            nonce: 0,
            escrow_locked: false,
            escrow_holder: None,
        };

        // Store agent
        let key = Self::get_agent_key(&env, agent_id);
        env.storage().instance().set(&key, &agent);

        // Initialize lease status
        Self::set_agent_lease_status(&env, agent_id, false);

        // Store royalty info if provided
        if let (Some(recipient), Some(fee)) = (royalty_recipient, royalty_fee) {
            let royalty_info = RoyaltyInfo { recipient, fee };
            let royalty_key = Self::get_royalty_key(&env, agent_id);
            env.storage().instance().set(&royalty_key, &royalty_info);
        }

        // Update counter
        env.storage()
            .instance()
            .set(&Symbol::new(&env, AGENT_COUNTER_KEY), &agent_id);

        // Emit event
        env.events().publish(
            (Symbol::new(&env, "agent_nft"), AgentEvent::AgentMinted),
            (agent_id, owner.clone()),
        );

        Ok(agent_id)
    }

    /// Get agent metadata with bounds checking
    pub fn get_agent(env: Env, agent_id: u64) -> Result<Agent, ContractError> {
        if agent_id == 0 {
            return Err(ContractError::InvalidAgentId);
        }

        let key = Self::get_agent_key(&env, agent_id);
        env.storage()
            .instance()
            .get::<_, Agent>(&key)
            .ok_or(ContractError::AgentNotFound)
    }

    /// Update agent metadata with authorization check
    pub fn update_agent(
        env: Env,
        agent_id: u64,
        owner: Address,
        name: Option<String>,
        capabilities: Option<Vec<String>>,
    ) -> Result<(), ContractError> {
        owner.require_auth();

        if agent_id == 0 {
            return Err(ContractError::InvalidAgentId);
        }

        let key = Self::get_agent_key(&env, agent_id);
        let mut agent: Agent = env
            .storage()
            .instance()
            .get(&key)
            .ok_or(ContractError::AgentNotFound)?;

        // Authorization check: only owner can update
        if agent.owner != owner {
            // Log the ownership failure
            let before_state = String::from_str(&env, "{}");
            let after_state = String::from_str(&env, "{}");
            let tx_hash = String::from_str(&env, "update_agent_fail"); // Placeholder
            let description = Some(String::from_str(
                &env,
                "NotOwner check failed during update.",
            ));

            let _ = create_audit_log(
                &env,
                owner.clone(), // 'owner' is the caller here
                OperationType::UnauthorizedAttempt,
                before_state,
                after_state,
                tx_hash,
                description,
            );
            return Err(ContractError::NotOwner);
        }

        // Check if agent is leased
        if Self::is_agent_leased(&env, agent_id) {
            return Err(ContractError::AgentLeased);
        }

        // Update fields with validation
        if let Some(new_name) = name {
            validation::validate_metadata(&new_name)?;
            agent.name = new_name;
        }

        if let Some(new_capabilities) = capabilities {
            validation::validate_capabilities(&new_capabilities)?;
            agent.capabilities = new_capabilities;
        }

        // Increment nonce for replay protection
        agent.nonce = agent
            .nonce
            .checked_add(1)
            .ok_or(ContractError::OverflowError)?;
        agent.updated_at = env.ledger().timestamp();

        env.storage().instance().set(&key, &agent);

        env.events().publish(
            (Symbol::new(&env, "agent_nft"), AgentEvent::AgentUpdated),
            (agent_id, owner),
        );

        Ok(())
    }

    /// Get total agents minted
    pub fn total_agents(env: Env) -> Result<u64, ContractError> {
        Ok(env
            .storage()
            .instance()
            .get(&Symbol::new(&env, AGENT_COUNTER_KEY))
            .unwrap_or(0))
    }

    /// Get nonce for replay protection
    pub fn get_nonce(env: Env, agent_id: u64) -> Result<u64, ContractError> {
        if agent_id == 0 {
            return Err(ContractError::InvalidAgentId);
        }

        let key = Self::get_agent_key(&env, agent_id);
        env.storage()
            .instance()
            .get::<_, Agent>(&key)
            .map(|agent| agent.nonce)
            .ok_or(ContractError::AgentNotFound)
    }

    /// Transfer ownership of an Agent NFT
    pub fn transfer_agent(
        env: Env,
        agent_id: u64,
        from: Address,
        to: Address,
    ) -> Result<(), ContractError> {
        if agent_id == 0 {
            return Err(ContractError::InvalidAgentId);
        }

        from.require_auth();

        if from == to {
            return Err(ContractError::SameAddressTransfer);
        }

        let key = Self::get_agent_key(&env, agent_id);
        let mut agent: Agent = env
            .storage()
            .instance()
            .get(&key)
            .ok_or(ContractError::AgentNotFound)?;

        if agent.owner != from {
            return Err(ContractError::NotOwner);
        }

        if Self::is_agent_leased(&env, agent_id) {
            return Err(ContractError::AgentLeased);
        }

        let previous_owner = agent.owner.clone();
        agent.owner = to.clone();
        agent.nonce = agent
            .nonce
            .checked_add(1)
            .ok_or(ContractError::OverflowError)?;
        agent.updated_at = env.ledger().timestamp();

        env.storage().instance().set(&key, &agent);

        env.events().publish(
            (Symbol::new(&env, "agent_nft"), AgentEvent::AgentTransferred),
            (agent_id, previous_owner.clone(), to.clone()),
        );

        // Audit log for transfer
        let before_state = String::from_str(&env, "{\"transferred\":false}");
        let after_state = String::from_str(&env, "{\"transferred\":true}");
        let tx_hash = String::from_str(&env, "transfer_agent");
        let description = Some(String::from_str(&env, "Agent NFT transferred"));

        let _ = create_audit_log(
            &env,
            from,
            OperationType::AdminTransfer,
            before_state,
            after_state,
            tx_hash,
            description,
        );

        Ok(())
    }

    pub fn batch_mint(
        env: Env,
        admin: Address,
        agents: Vec<AgentMintData>,
    ) -> Result<Vec<u64>, ContractError> {
        // 1. Authorization: Only admin or approved minters
        admin.require_auth();
        Self::verify_minter(&env, &admin)?;

        // 2. Batch Size Validation
        let count = agents.len();
        if count == 0 {
            return Err(ContractError::InvalidInput);
        }
        if count > 50 {
            return Err(ContractError::InvalidInput);
        } // Limit to 50

        // 3. Duplicate Metadata Validation (Internal to Batch)
        // We use a temporary map to ensure no CID is repeated in this single call
        let mut seen_cids = Vec::new(&env);
        for i in 0..agents.len() {
            let agent = agents.get(i).ok_or(ContractError::InvalidInput)?;
            if seen_cids.contains(agent.metadata_cid.clone()) {
                return Err(ContractError::InvalidInput);
            }
            seen_cids.push_back(agent.metadata_cid.clone());

            Self::validate_agent_data(&env, &agent.name, &agent.metadata_cid, &agent.capabilities)?;

            if let OptionalRoyaltyInfo::Some(royalty) = agent.royalty {
                Self::validate_royalty_fee(royalty.fee)?;
            }
        }

        // 4. Execution Logic
        let mut minted_ids = Vec::new(&env);
        let mut current_counter: u64 = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, AGENT_COUNTER_KEY))
            .unwrap_or(0);

        for i in 0..agents.len() {
            let data = agents.get(i).ok_or(ContractError::InvalidInput)?;

            // Increment ID
            current_counter = Self::safe_add(current_counter, 1)?;
            let agent_id = current_counter;

            // Create Agent object
            let agent = Agent {
                id: agent_id,
                owner: data.owner.clone(),
                name: data.name,
                model_hash: data.model_hash,
                metadata_cid: data.metadata_cid,
                capabilities: data.capabilities,
                evolution_level: 0,
                created_at: env.ledger().timestamp(),
                updated_at: env.ledger().timestamp(),
                nonce: 0,
                escrow_locked: false,
                escrow_holder: None,
            };

            // Persist Agent
            let key = Self::get_agent_key(&env, agent_id);
            env.storage().instance().set(&key, &agent);
            Self::set_agent_lease_status(&env, agent_id, false);

            // Handle Royalty if present
            if let OptionalRoyaltyInfo::Some(royalty) = data.royalty {
                let royalty_key = Self::get_royalty_key(&env, agent_id);
                env.storage().instance().set(&royalty_key, &royalty);
            }

            // Emit Individual Event
            env.events().publish(
                (Symbol::new(&env, "agent_nft"), AgentEvent::AgentMinted),
                (agent_id, data.owner.clone()),
            );

            minted_ids.push_back(agent_id);
        }

        // 5. Update Global Counter
        env.storage()
            .instance()
            .set(&Symbol::new(&env, AGENT_COUNTER_KEY), &current_counter);

        // 6. Emit Batch Summary Event
        // We use the timestamp + admin address hash as a pseudo batch_id
        env.events().publish(
            (
                Symbol::new(&env, "agent_nft"),
                AgentEvent::BatchMintCompleted,
            ),
            (count, admin),
        );

        Ok(minted_ids)
    }

    fn validate_agent_data(
        env: &Env,
        name: &String,
        metadata_cid: &String,
        capabilities: &Vec<String>,
    ) -> Result<(), ContractError> {
        Self::validate_metadata(name)?;
        Self::validate_metadata(metadata_cid)?;
        Self::validate_capabilities(capabilities)?;
        Ok(())
    }

    /// Helper to validate metadata CID/string
    fn validate_metadata(metadata: &String) -> Result<(), ContractError> {
        validation::validate_metadata(metadata)
    }

    /// Helper to validate agent capabilities
    fn validate_capabilities(capabilities: &Vec<String>) -> Result<(), ContractError> {
        validation::validate_capabilities(capabilities)
    }
    /// Get current owner of an agent
    /// Read-only query function for off-chain consumers (Issue #6)
    pub fn get_agent_owner(env: Env, agent_id: u64) -> Result<Address, ContractError> {
        if agent_id == 0 {
            return Err(ContractError::InvalidAgentId);
        }

        let key = Self::get_agent_key(&env, agent_id);
        env.storage()
            .instance()
            .get::<_, Agent>(&key)
            .map(|agent| agent.owner)
            .ok_or(ContractError::AgentNotFound)
    }

    /// Get agent metadata CID (IPFS hash)
    /// Read-only query function for off-chain consumers (Issue #6)
    /// Returns the IPFS CID for the agent's metadata
    pub fn get_agent_metadata(env: Env, agent_id: u64) -> Result<String, ContractError> {
        if agent_id == 0 {
            return Err(ContractError::InvalidAgentId);
        }

        let key = Self::get_agent_key(&env, agent_id);
        env.storage()
            .instance()
            .get::<_, Agent>(&key)
            .map(|agent| agent.metadata_cid)
            .ok_or(ContractError::AgentNotFound)
    }

    /// Get agent evolution level
    /// Read-only query function for off-chain consumers (Issue #6)
    /// Returns the current evolution level of the agent
    pub fn get_agent_evolution_level(env: Env, agent_id: u64) -> Result<u32, ContractError> {
        if agent_id == 0 {
            return Err(ContractError::InvalidAgentId);
        }

        let key = Self::get_agent_key(&env, agent_id);
        env.storage()
            .instance()
            .get::<_, Agent>(&key)
            .map(|agent| agent.evolution_level)
            .ok_or(ContractError::AgentNotFound)
    }

    /// Check if agent can be transferred
    pub fn can_transfer_agent(
        env: Env,
        agent_id: u64,
        caller: Address,
    ) -> Result<bool, ContractError> {
        if agent_id == 0 {
            return Ok(false);
        }

        let key = Self::get_agent_key(&env, agent_id);
        let agent = match env.storage().instance().get::<_, Agent>(&key) {
            Some(agent) => agent,
            None => return Ok(false),
        };

        if agent.owner != caller {
            return Ok(false);
        }

        Ok(!Self::is_agent_leased(&env, agent_id))
    }

    /// Start leasing an agent
    pub fn start_lease(env: Env, agent_id: u64) -> Result<(), ContractError> {
        if agent_id == 0 {
            return Err(ContractError::InvalidAgentId);
        }

        Self::set_agent_lease_status(&env, agent_id, true);

        env.events().publish(
            (Symbol::new(&env, "agent_nft"), AgentEvent::LeaseStarted),
            (agent_id, env.ledger().timestamp()),
        );

        Ok(())
    }

    /// End leasing an agent
    pub fn end_lease(env: Env, agent_id: u64) -> Result<(), ContractError> {
        if agent_id == 0 {
            return Err(ContractError::InvalidAgentId);
        }

        Self::set_agent_lease_status(&env, agent_id, false);

        env.events().publish(
            (Symbol::new(&env, "agent_nft"), AgentEvent::LeaseEnded),
            (agent_id, env.ledger().timestamp()),
        );

        Ok(())
    }

    /// Check if agent is leased
    pub fn is_leased(env: Env, agent_id: u64) -> Result<bool, ContractError> {
        if agent_id == 0 {
            return Err(ContractError::InvalidAgentId);
        }
        Ok(Self::is_agent_leased(&env, agent_id))
    }

    /// Get royalty info for an agent
    ///
    /// # Arguments
    /// * `agent_id` - The agent ID to query
    ///
    /// # Returns
    /// Result<Option<RoyaltyInfo>, ContractError> - Royalty info if set, None if not set
    ///
    /// # Errors
    /// - ContractError::InvalidAgentId if agent_id is 0
    pub fn get_royalty(env: Env, agent_id: u64) -> Result<Option<RoyaltyInfo>, ContractError> {
        if agent_id == 0 {
            return Err(ContractError::InvalidAgentId);
        }

        let royalty_key = Self::get_royalty_key(&env, agent_id);
        Ok(env.storage().instance().get(&royalty_key))
    }
}

// ============================================================================
// Tests for Issue #6: Read-only agent query functions
// ============================================================================
#[cfg(test)]
pub mod tests {
    extern crate alloc;
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};

    pub fn setup_contract(env: &Env) -> (AgentNFTClient, Address) {
        let contract_id = env.register_contract(None, AgentNFT);
        let client = AgentNFTClient::new(env, &contract_id);
        let admin = Address::generate(env);

        env.mock_all_auths();
        client.init_contract(&admin);

        (client, admin)
    }

    pub fn mint_test_agent(
        env: &Env,
        client: &AgentNFTClient,
        owner: &Address,
        agent_id: u128,
        metadata_cid: &str,
        evolution_level: u32,
    ) {
        let metadata = String::from_str(&env, metadata_cid);
        // Added 'None' for royalty_recipient and 'None' for royalty_fee
        client.mint_agent(
            &agent_id,
            owner,
            &metadata,
            &evolution_level,
            &None, // royalty_recipient
            &None, // royalty_fee
        );
    }

    #[test]
    fn test_get_agent_owner() {
        let env = Env::default();
        let (client, admin) = setup_contract(&env);

        let owner = Address::generate(&env);
        client.add_approved_minter(&admin, &owner);

        env.mock_all_auths();
        mint_test_agent(&env, &client, &owner, 1, "QmTestCID123", 1);

        // Test get_agent_owner returns correct owner
        let result = client.get_agent_owner(&1);
        assert_eq!(result, owner);
    }

    #[test]
    fn test_total_agents_returns_result() {
        let env = Env::default();
        let (client, _admin) = setup_contract(&env);

        // Initial count should be 0
        assert_eq!(client.total_agents(), 0);
    }

    #[test]
    fn test_metadata_error_variants() {
        let env = Env::default();
        let (client, admin) = setup_contract(&env);
        let owner = Address::generate(&env);
        client.add_approved_minter(&admin, &owner);

        // Mock too long string (over 256)
        let mut long_str = alloc::string::String::new();
        for _ in 0..300 {
            long_str.push('a');
        }
        let metadata = String::from_str(&env, &long_str);

        env.mock_all_auths();
        let result = client.try_mint_agent(&5, &owner, &metadata, &1, &None, &None);

        match result {
            Err(Ok(ContractError::MetadataTooLong)) => {}
            _ => panic!("Should have failed with MetadataTooLong, got {:?}", result),
        }
    }
}
