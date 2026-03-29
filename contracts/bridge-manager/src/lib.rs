#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, Address, Env, IntoVal, Symbol, Vec,
};

// Interface for the AgentNFT contract
#[soroban_sdk::contractclient(name = "AgentNFTClient")]
pub trait AgentNFTInterface {
    fn transfer_agent(env: Env, agent_id: u64, from: Address, to: Address);
}

#[derive(Clone)]
#[contracttype]
pub enum DataKey {
    Admin,
    AgentContract,
    PaymentToken,
    OracleContract,
    SignerConfig,
    BridgeCounter,
    BridgeRequest(u64),
    LockedAgent(u64),
    WrappedToken(u128),
    LiquidityBalance,
    FeeBalance,
}

/// Configuration for M-of-N bridge signers (relayers / validators).
#[derive(Clone)]
#[contracttype]
pub struct SignerConfig {
    pub signers: Vec<Address>,
    pub m_required: u32,
}

/// Status of a bridge lifecycle.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[contracttype]
#[repr(u32)]
pub enum BridgeStatus {
    PendingOutbound = 0,
    OutboundCompleted = 1,
    PendingInbound = 2,
    InboundApproved = 3,
    Completed = 4,
    Cancelled = 5,
    Expired = 6,
}

/// Bridge request tracking a single agent's movement off Stellar and back.
#[derive(Clone)]
#[contracttype]
pub struct BridgeRequest {
    pub bridge_id: u64,
    pub agent_id: u64,
    pub owner: Address,
    pub metadata_cid: soroban_sdk::String,
    pub source_chain: u32,
    pub target_chain: u32,
    pub notional_value: i128,
    pub fee_paid: i128,
    pub wrapped_token_id: Option<u128>,
    pub status: BridgeStatus,
    pub initiated_at: u64,
    pub last_updated_at: u64,
    pub outbound_approvals: Vec<Address>,
    pub inbound_approvals: Vec<Address>,
}

/// 0.5% bridge fee expressed in basis points (50 / 10_000 = 0.5%).
pub const BRIDGE_FEE_BPS: u32 = 50;
/// Simple upper bound for chain identifiers to catch obvious bad inputs.
pub const MAX_CHAIN_ID: u32 = 1_000_000;
/// How long a bridge proof is considered valid (7 days).
pub const BRIDGE_EXPIRATION_SECONDS: u64 = 7 * 24 * 60 * 60;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum BridgeError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    InvalidAgentId = 4,
    AgentNotFound = 5,
    NotAgentOwner = 6,
    AgentAlreadyLocked = 7,
    AgentNotLocked = 8,
    InvalidChain = 9,
    InvalidAmount = 10,
    BridgeNotFound = 11,
    InvalidStatus = 12,
    Expired = 13,
    InsufficientApprovals = 14,
    DuplicateApproval = 15,
    SignerConfigMissing = 16,
    NotSigner = 17,
    WrappedTokenAlreadyMapped = 18,
    LiquidityUnderflow = 19,
}

#[contract]
pub struct BridgeManager;

#[contractimpl]
impl BridgeManager {
    /// Initialize bridge manager: admin, agent NFT contract, payment token and signer set.
    pub fn init_contract(
        env: Env,
        admin: Address,
        agent_contract: Address,
        payment_token: Address,
        signers: Vec<Address>,
        m_required: u32,
    ) -> Result<(), BridgeError> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(BridgeError::AlreadyInitialized);
        }

        admin.require_auth();

        if signers.len() == 0 {
            return Err(BridgeError::SignerConfigMissing);
        }
        if m_required == 0 || m_required > signers.len() {
            return Err(BridgeError::InvalidAmount);
        }

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::AgentContract, &agent_contract);
        env.storage()
            .instance()
            .set(&DataKey::PaymentToken, &payment_token);

        let cfg = SignerConfig {
            signers,
            m_required,
        };
        env.storage().instance().set(&DataKey::SignerConfig, &cfg);

        env.storage().instance().set(&DataKey::BridgeCounter, &0u64);
        env.storage()
            .instance()
            .set(&DataKey::LiquidityBalance, &0i128);
        env.storage().instance().set(&DataKey::FeeBalance, &0i128);

        Ok(())
    }

    /// Update signer set and M-of-N threshold (admin only).
    pub fn update_signer_config(
        env: Env,
        admin: Address,
        signers: Vec<Address>,
        m_required: u32,
    ) -> Result<(), BridgeError> {
        Self::require_admin(&env, &admin)?;

        if signers.len() == 0 {
            return Err(BridgeError::SignerConfigMissing);
        }
        if m_required == 0 || m_required > signers.len() {
            return Err(BridgeError::InvalidAmount);
        }

        let cfg = SignerConfig {
            signers,
            m_required,
        };
        env.storage().instance().set(&DataKey::SignerConfig, &cfg);
        Ok(())
    }

    /// Optionally link an oracle / relayer contract for off-chain message passing.
    pub fn set_oracle_contract(
        env: Env,
        admin: Address,
        oracle_contract: Address,
    ) -> Result<(), BridgeError> {
        Self::require_admin(&env, &admin)?;
        env.storage()
            .instance()
            .set(&DataKey::OracleContract, &oracle_contract);
        Ok(())
    }

    /// Lock an on-Stellar agent and initiate outbound bridge to `target_chain`.
    /// Returns a `bridge_id` used for approvals and eventual unwrapping.
    pub fn lock_and_bridge(
        env: Env,
        agent_id: u64,
        owner: Address,
        metadata_cid: soroban_sdk::String,
        target_chain: u32,
        notional_value: i128,
    ) -> Result<u64, BridgeError> {
        if agent_id == 0 {
            return Err(BridgeError::InvalidAgentId);
        }
        if target_chain == 0 || target_chain > MAX_CHAIN_ID {
            return Err(BridgeError::InvalidChain);
        }
        if notional_value <= 0 {
            return Err(BridgeError::InvalidAmount);
        }

        owner.require_auth();

        // 1. Lock the Agent NFT in the bridge contract
        let agent_contract: Address = env
            .storage()
            .instance()
            .get(&DataKey::AgentContract)
            .ok_or(BridgeError::NotInitialized)?;
        let agent_client = AgentNFTClient::new(&env, &agent_contract);
        agent_client.transfer_agent(&agent_id, &owner, &env.current_contract_address());

        // Ensure we are not double-bridging the same agent.
        let locked: bool = env
            .storage()
            .instance()
            .get(&DataKey::LockedAgent(agent_id))
            .unwrap_or(false);
        if locked {
            return Err(BridgeError::AgentAlreadyLocked);
        }

        // Charge bridge fee (0.5%) + deposit notional into virtual liquidity pool.
        let fee = (notional_value
            .checked_mul(BRIDGE_FEE_BPS as i128)
            .ok_or(BridgeError::InvalidAmount)?)
            / 10_000;
        if fee <= 0 {
            return Err(BridgeError::InvalidAmount);
        }

        let _total = notional_value
            .checked_add(fee)
            .ok_or(BridgeError::InvalidAmount)?;

        // Update liquidity and fee balances.
        let mut liquidity: i128 = env
            .storage()
            .instance()
            .get(&DataKey::LiquidityBalance)
            .unwrap_or(0);
        let mut fees: i128 = env
            .storage()
            .instance()
            .get(&DataKey::FeeBalance)
            .unwrap_or(0);

        liquidity = liquidity
            .checked_add(notional_value)
            .ok_or(BridgeError::InvalidAmount)?;
        fees = fees.checked_add(fee).ok_or(BridgeError::InvalidAmount)?;

        env.storage()
            .instance()
            .set(&DataKey::LiquidityBalance, &liquidity);
        env.storage().instance().set(&DataKey::FeeBalance, &fees);

        // Allocate bridge id.
        let counter: u64 = env
            .storage()
            .instance()
            .get(&DataKey::BridgeCounter)
            .unwrap_or(0);
        let bridge_id = counter.checked_add(1).ok_or(BridgeError::InvalidAmount)?;
        env.storage()
            .instance()
            .set(&DataKey::BridgeCounter, &bridge_id);

        let now = env.ledger().timestamp();

        let request = BridgeRequest {
            bridge_id,
            agent_id,
            owner: owner.clone(),
            metadata_cid: metadata_cid.clone(),
            source_chain: 0, // Stellar canonical source
            target_chain,
            notional_value,
            fee_paid: fee,
            wrapped_token_id: None,
            status: BridgeStatus::PendingOutbound,
            initiated_at: now,
            last_updated_at: now,
            outbound_approvals: Vec::new(&env),
            inbound_approvals: Vec::new(&env),
        };

        env.storage()
            .instance()
            .set(&DataKey::BridgeRequest(bridge_id), &request);
        env.storage()
            .instance()
            .set(&DataKey::LockedAgent(agent_id), &true);

        env.events().publish(
            (Symbol::new(&env, "BridgeOutboundInitiated"),),
            (
                bridge_id,
                agent_id,
                owner,
                metadata_cid,
                target_chain,
                notional_value,
                fee,
            ),
        );

        Ok(bridge_id)
    }

    /// Approve outbound bridge (M-of-N signers). When threshold reached, marks
    /// the bridge as `OutboundCompleted`.
    pub fn approve_outbound(env: Env, signer: Address, bridge_id: u64) -> Result<(), BridgeError> {
        signer.require_auth();
        Self::require_signer(&env, &signer)?;

        let mut req: BridgeRequest = env
            .storage()
            .instance()
            .get(&DataKey::BridgeRequest(bridge_id))
            .ok_or(BridgeError::BridgeNotFound)?;

        if !Self::is_active(&env, &req) {
            return Err(BridgeError::Expired);
        }

        if req.status != BridgeStatus::PendingOutbound {
            return Err(BridgeError::InvalidStatus);
        }

        if Self::contains_address(&env, &req.outbound_approvals, &signer) {
            return Err(BridgeError::DuplicateApproval);
        }

        req.outbound_approvals.push_back(signer.clone());
        req.last_updated_at = env.ledger().timestamp();

        let cfg = Self::get_signer_config(&env)?;
        let approvals = req.outbound_approvals.len() as u32;

        if approvals >= cfg.m_required {
            req.status = BridgeStatus::OutboundCompleted;
            env.events().publish(
                (Symbol::new(&env, "BridgeOutboundFinalized"),),
                (bridge_id, approvals),
            );
        } else {
            env.events().publish(
                (Symbol::new(&env, "BridgeOutboundApprovalReceived"),),
                (bridge_id, signer, approvals),
            );
        }

        env.storage()
            .instance()
            .set(&DataKey::BridgeRequest(bridge_id), &req);

        Ok(())
    }

    /// Approve inbound bridge after wrapped token burn on target chain.
    /// The first call binds a `wrapped_token_id` for this bridge.
    pub fn approve_inbound(
        env: Env,
        signer: Address,
        bridge_id: u64,
        wrapped_token_id: u128,
    ) -> Result<(), BridgeError> {
        signer.require_auth();
        Self::require_signer(&env, &signer)?;

        let mut req: BridgeRequest = env
            .storage()
            .instance()
            .get(&DataKey::BridgeRequest(bridge_id))
            .ok_or(BridgeError::BridgeNotFound)?;

        if !Self::is_active(&env, &req) {
            return Err(BridgeError::Expired);
        }

        if req.status != BridgeStatus::OutboundCompleted
            && req.status != BridgeStatus::PendingInbound
        {
            return Err(BridgeError::InvalidStatus);
        }

        if Self::contains_address(&env, &req.inbound_approvals, &signer) {
            return Err(BridgeError::DuplicateApproval);
        }

        match req.wrapped_token_id {
            None => {
                // First inbound approval binds wrapped_token_id and mapping.
                if env
                    .storage()
                    .instance()
                    .has(&DataKey::WrappedToken(wrapped_token_id))
                {
                    return Err(BridgeError::WrappedTokenAlreadyMapped);
                }
                req.wrapped_token_id = Some(wrapped_token_id);
                env.storage()
                    .instance()
                    .set(&DataKey::WrappedToken(wrapped_token_id), &bridge_id);
            }
            Some(existing) => {
                if existing != wrapped_token_id {
                    return Err(BridgeError::InvalidStatus);
                }
            }
        }

        req.status = BridgeStatus::PendingInbound;
        req.inbound_approvals.push_back(signer.clone());
        req.last_updated_at = env.ledger().timestamp();

        let cfg = Self::get_signer_config(&env)?;
        let approvals = req.inbound_approvals.len() as u32;

        if approvals >= cfg.m_required {
            req.status = BridgeStatus::InboundApproved;
            env.events().publish(
                (Symbol::new(&env, "BridgeInboundFinalized"),),
                (bridge_id, wrapped_token_id, approvals),
            );
        } else {
            env.events().publish(
                (Symbol::new(&env, "BridgeInboundApprovalReceived"),),
                (bridge_id, signer, approvals),
            );
        }

        env.storage()
            .instance()
            .set(&DataKey::BridgeRequest(bridge_id), &req);

        Ok(())
    }

    /// Final step: user (or relayer) presents `wrapped_token_id` and receives
    /// the liquidity back while the canonical agent is unlocked on Stellar.
    pub fn unwrap_and_unlock(
        env: Env,
        wrapped_token_id: u128,
        recipient: Address,
    ) -> Result<(), BridgeError> {
        recipient.require_auth();

        let bridge_id: u64 = env
            .storage()
            .instance()
            .get(&DataKey::WrappedToken(wrapped_token_id))
            .ok_or(BridgeError::BridgeNotFound)?;

        let mut req: BridgeRequest = env
            .storage()
            .instance()
            .get(&DataKey::BridgeRequest(bridge_id))
            .ok_or(BridgeError::BridgeNotFound)?;

        if !Self::is_active(&env, &req) {
            return Err(BridgeError::Expired);
        }

        if req.status != BridgeStatus::InboundApproved {
            return Err(BridgeError::InvalidStatus);
        }

        let locked: bool = env
            .storage()
            .instance()
            .get(&DataKey::LockedAgent(req.agent_id))
            .unwrap_or(false);
        if !locked {
            return Err(BridgeError::AgentNotLocked);
        }

        let mut liquidity: i128 = env
            .storage()
            .instance()
            .get(&DataKey::LiquidityBalance)
            .unwrap_or(0);
        if liquidity < req.notional_value {
            return Err(BridgeError::LiquidityUnderflow);
        }
        liquidity -= req.notional_value;
        env.storage()
            .instance()
            .set(&DataKey::LiquidityBalance, &liquidity);

        // 2. Unlock the Agent NFT and transfer it to the recipient
        let agent_contract: Address = env
            .storage()
            .instance()
            .get(&DataKey::AgentContract)
            .ok_or(BridgeError::NotInitialized)?;
        let agent_client = AgentNFTClient::new(&env, &agent_contract);
        agent_client.transfer_agent(&req.agent_id, &env.current_contract_address(), &recipient);

        env.storage()
            .instance()
            .set(&DataKey::LockedAgent(req.agent_id), &false);

        req.status = BridgeStatus::Completed;
        req.last_updated_at = env.ledger().timestamp();
        env.storage()
            .instance()
            .set(&DataKey::BridgeRequest(bridge_id), &req);

        env.events().publish(
            (Symbol::new(&env, "BridgeUnwrapped"),),
            (
                bridge_id,
                req.agent_id,
                wrapped_token_id,
                recipient,
                req.notional_value,
                req.fee_paid,
            ),
        );

        Ok(())
    }

    /// Read-only helpers for tests / off-chain indexers.
    pub fn get_bridge_request(env: Env, bridge_id: u64) -> Option<BridgeRequest> {
        env.storage()
            .instance()
            .get(&DataKey::BridgeRequest(bridge_id))
    }

    pub fn get_liquidity_balance(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::LiquidityBalance)
            .unwrap_or(0)
    }

    pub fn get_fee_balance(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::FeeBalance)
            .unwrap_or(0)
    }

    pub fn get_signer_config_view(env: Env) -> Option<SignerConfig> {
        env.storage().instance().get(&DataKey::SignerConfig)
    }

    // ---------------- internal helpers ----------------

    fn require_admin(env: &Env, caller: &Address) -> Result<(), BridgeError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(BridgeError::NotInitialized)?;
        if &admin != caller {
            return Err(BridgeError::Unauthorized);
        }
        Ok(())
    }

    fn get_signer_config(env: &Env) -> Result<SignerConfig, BridgeError> {
        env.storage()
            .instance()
            .get(&DataKey::SignerConfig)
            .ok_or(BridgeError::SignerConfigMissing)
    }

    fn require_signer(env: &Env, signer: &Address) -> Result<(), BridgeError> {
        let cfg = Self::get_signer_config(env)?;
        if !Self::contains_address(env, &cfg.signers, signer) {
            return Err(BridgeError::NotSigner);
        }
        Ok(())
    }

    fn contains_address(_env: &Env, list: &Vec<Address>, needle: &Address) -> bool {
        for i in 0..list.len() {
            if let Some(a) = list.get(i) {
                if &a == needle {
                    return true;
                }
            }
        }
        false
    }

    fn is_active(env: &Env, req: &BridgeRequest) -> bool {
        let now = env.ledger().timestamp();
        now <= req.initiated_at + BRIDGE_EXPIRATION_SECONDS
            && (req.status == BridgeStatus::PendingOutbound
                || req.status == BridgeStatus::OutboundCompleted
                || req.status == BridgeStatus::PendingInbound
                || req.status == BridgeStatus::InboundApproved)
    }
}

#[cfg(test)]
mod test;
