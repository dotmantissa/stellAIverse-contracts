#![no_std]

use soroban_sdk::{
    contract, contractimpl, symbol_short, Address, Bytes, Env, IntoVal, String, Symbol, Val, Vec,
};

use stellai_lib::{WorkflowStep, WorkflowStepStatus};

// ── Storage keys ──────────────────────────────────────────────────────────────

const ADMIN_KEY: &str = "mkt_admin";
const LISTING_CTR_KEY: &str = "lst_ctr";
const LISTING_PREFIX: &str = "lst_";
const ROYALTY_PREFIX: &str = "roy_";
const AGENT_NFT_KEY: &str = "agent_nft";
const HUB_KEY: &str = "exec_hub";
const PENDING_SALE_PREFIX: &str = "psale_";
const WF_LISTING_PREFIX: &str = "wf_lst_";

// ── Local types ───────────────────────────────────────────────────────────────

#[derive(Clone)]
#[soroban_sdk::contracttype]
pub struct PendingSale {
    pub listing_id: u64,
    pub buyer: Address,
    pub amount: i128,
    pub seller: Address,
    pub agent_id: u64,
    pub workflow_id: u64,
    pub created_at: u64,
}

// ── Contract ──────────────────────────────────────────────────────────────────

#[contract]
pub struct Marketplace;

#[contractimpl]
impl Marketplace {
    // =========================================================================
    // Initialisation
    // =========================================================================

    pub fn init_contract(env: Env, admin: Address) {
        let key = Symbol::new(&env, ADMIN_KEY);
        if env.storage().instance().has(&key) {
            panic!("Already initialized");
        }
        admin.require_auth();
        env.storage().instance().set(&key, &admin);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, LISTING_CTR_KEY), &0u64);
    }

    pub fn set_agent_nft_contract(env: Env, admin: Address, agent_nft: Address) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, AGENT_NFT_KEY), &agent_nft);
        env.events().publish((symbol_short!("nft_set"),), agent_nft);
    }

    pub fn set_execution_hub(env: Env, admin: Address, hub: Address) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, HUB_KEY), &hub);
        env.events().publish((symbol_short!("hub_set"),), hub);
    }

    // =========================================================================
    // Listings
    // =========================================================================

    pub fn create_listing(
        env: Env,
        agent_id: u64,
        seller: Address,
        listing_type: u32,
        price: i128,
        duration_days: Option<u64>,
    ) -> u64 {
        seller.require_auth();
        if agent_id == 0 {
            panic!("Invalid agent ID");
        }
        if listing_type > 2 {
            panic!("Invalid listing type");
        }
        if price < stellai_lib::PRICE_LOWER_BOUND || price > stellai_lib::PRICE_UPPER_BOUND {
            panic!("Price out of valid range");
        }
        if listing_type == 1 {
            let dur = duration_days.expect("Duration required for leases");
            if dur == 0 || dur > stellai_lib::MAX_DURATION_DAYS {
                panic!("Lease duration out of valid range");
            }
        }

        let agent = Self::load_agent(&env, agent_id);
        if agent.owner != seller {
            panic!("Only agent owner can create listings");
        }
        if agent.escrow_locked {
            panic!("Agent already locked in escrow");
        }

        let listing_id = Self::next_listing_id(&env);
        let marketplace = env.current_contract_address();

        let listing = stellai_lib::Listing {
            listing_id,
            asset_id: agent_id,
            asset_type: stellai_lib::AssetType::Agent,
            seller: seller.clone(),
            price,
            listing_type: match listing_type {
                0 => stellai_lib::ListingType::Sale,
                1 => stellai_lib::ListingType::Lease,
                2 => stellai_lib::ListingType::Auction,
                _ => panic!("Invalid listing type"),
            },
            active: true,
            created_at: env.ledger().timestamp(),
        };

        let lk = Self::listing_key(&env, listing_id);
        env.storage().instance().set(&lk, &listing);

        let mut updated_agent = agent;
        updated_agent.escrow_locked = true;
        updated_agent.escrow_holder = Some(marketplace.clone());
        updated_agent.updated_at = env.ledger().timestamp();
        Self::save_agent(&env, agent_id, &updated_agent);

        env.events().publish(
            (symbol_short!("lst_creat"),),
            (listing_id, agent_id, seller.clone(), price),
        );
        env.events().publish(
            (symbol_short!("esc_lock"),),
            (agent_id, seller, marketplace),
        );

        listing_id
    }

    // =========================================================================
    // Execution-hub-orchestrated sale
    // =========================================================================

    /// Purchase an agent via an execution-hub workflow.
    ///
    /// Registers a three-step workflow in the hub, stores a pending-sale
    /// record, then drives step 0 immediately.  Remaining steps are driven by
    /// subsequent `execute_workflow_step` calls on the hub.
    ///
    /// Returns `(listing_id, workflow_id)`.
    pub fn buy_agent(env: Env, listing_id: u64, buyer: Address, amount: i128) -> (u64, u64) {
        buyer.require_auth();

        if listing_id == 0 {
            panic!("Invalid listing ID");
        }

        let listing = Self::load_listing(&env, listing_id);
        if !listing.active {
            panic!("Listing is not active");
        }
        if amount < listing.price {
            panic!("Insufficient payment");
        }
        if amount > stellai_lib::PRICE_UPPER_BOUND {
            panic!("Payment exceeds safe maximum");
        }

        let marketplace = env.current_contract_address();
        let agent = Self::load_agent(&env, listing.asset_id);
        if !agent.escrow_locked {
            panic!("Agent not in escrow");
        }
        match &agent.escrow_holder {
            Some(h) if h == &marketplace => {}
            _ => panic!("Agent locked by a different contract"),
        }

        // Persist pending sale (workflow_id filled in after the hub call)
        let pending = PendingSale {
            listing_id,
            buyer: buyer.clone(),
            amount,
            seller: listing.seller.clone(),
            agent_id: listing.asset_id,
            workflow_id: 0,
            created_at: env.ledger().timestamp(),
        };
        let psk = Self::pending_sale_key(&env, listing_id);
        env.storage().instance().set(&psk, &pending);

        let hub = Self::get_hub(&env);
        let steps = Self::build_sale_steps(&env, &marketplace, listing_id);
        let context_tag: Option<String> = Some(String::from_str(&env, "agent_sale"));
        let none_u64: Option<u64> = None;
        let cb_contract: Option<Address> = Some(marketplace.clone());

        // Build args for create_workflow
        let mut cw_args = Vec::<Val>::new(&env);
        cw_args.push_back(marketplace.clone().into_val(&env));
        cw_args.push_back(String::from_str(&env, "agent_sale").into_val(&env));
        cw_args.push_back(steps.into_val(&env));
        cw_args.push_back(none_u64.into_val(&env));
        cw_args.push_back(context_tag.into_val(&env));
        cw_args.push_back(cb_contract.into_val(&env));

        let workflow_id: u64 = env.invoke_contract(
            &hub,
            &Symbol::new(&env, "create_workflow"),
            cw_args,
        );

        // Back-fill workflow_id
        let mut updated_pending: PendingSale =
            env.storage().instance().get(&psk).expect("Pending sale disappeared");
        updated_pending.workflow_id = workflow_id;
        env.storage().instance().set(&psk, &updated_pending);

        // Store workflow→listing mapping for callback reconciliation
        let wlk = Self::wf_listing_key(&env, workflow_id);
        env.storage().instance().set(&wlk, &listing_id);

        env.events().publish(
            (symbol_short!("sale_init"),),
            (listing_id, buyer, workflow_id, env.ledger().timestamp()),
        );

        // Drive step 0
        let mut ews_args = Vec::<Val>::new(&env);
        ews_args.push_back(workflow_id.into_val(&env));
        let _: WorkflowStepStatus = env.invoke_contract(
            &hub,
            &Symbol::new(&env, "execute_workflow_step"),
            ews_args,
        );

        (listing_id, workflow_id)
    }

    // =========================================================================
    // Workflow step functions (called by the execution hub)
    // =========================================================================

    /// Step 0 — verify the listing and escrow are still valid.
    /// `encoded_args`: 8 bytes big-endian listing_id.
    pub fn verify_sale(env: Env, encoded_args: Bytes) {
        let listing_id = Self::decode_u64(&encoded_args);
        let listing = Self::load_listing(&env, listing_id);
        if !listing.active {
            panic!("Listing no longer active");
        }
        let psk = Self::pending_sale_key(&env, listing_id);
        if !env.storage().instance().has(&psk) {
            panic!("No pending sale for this listing");
        }
        let marketplace = env.current_contract_address();
        let agent = Self::load_agent(&env, listing.asset_id);
        if !agent.escrow_locked {
            panic!("Agent not in escrow at verify time");
        }
        match &agent.escrow_holder {
            Some(h) if h == &marketplace => {}
            _ => panic!("Escrow holder mismatch at verify time"),
        }
        env.events()
            .publish((symbol_short!("sale_vfy"),), (listing_id, env.ledger().timestamp()));
    }

    /// Step 1 — transfer ownership to the buyer.
    /// `encoded_args`: 8 bytes big-endian listing_id.
    pub fn transfer_ownership(env: Env, encoded_args: Bytes) {
        let listing_id = Self::decode_u64(&encoded_args);
        let listing = Self::load_listing(&env, listing_id);

        let psk = Self::pending_sale_key(&env, listing_id);
        let pending: PendingSale =
            env.storage().instance().get(&psk).expect("No pending sale");

        let mut agent = Self::load_agent(&env, listing.asset_id);
        agent.owner = pending.buyer.clone();
        agent.nonce = agent.nonce.checked_add(1).expect("Agent nonce overflow");
        agent.updated_at = env.ledger().timestamp();
        Self::save_agent(&env, listing.asset_id, &agent);

        env.events().publish(
            (symbol_short!("own_xfer"),),
            (listing.asset_id, listing.seller, pending.buyer, env.ledger().timestamp()),
        );
    }

    /// Step 2 — release escrow, deactivate listing, emit sale record.
    /// `encoded_args`: 8 bytes big-endian listing_id.
    pub fn record_sale(env: Env, encoded_args: Bytes) {
        let listing_id = Self::decode_u64(&encoded_args);
        let mut listing = Self::load_listing(&env, listing_id);

        let psk = Self::pending_sale_key(&env, listing_id);
        let pending: PendingSale =
            env.storage().instance().get(&psk).expect("No pending sale");

        let royalty_key = Self::royalty_key(&env, listing.asset_id);
        let royalty_info: Option<stellai_lib::RoyaltyInfo> =
            env.storage().instance().get(&royalty_key);

        let royalty_amount: i128 = if let Some(ref r) = royalty_info {
            if r.fee > stellai_lib::MAX_ROYALTY_PERCENTAGE {
                panic!("Invalid royalty percentage");
            }
            pending
                .amount
                .checked_mul(r.fee as i128)
                .expect("Royalty overflow")
                .checked_div(10_000)
                .expect("Royalty division")
        } else {
            0
        };

        let seller_amount = pending
            .amount
            .checked_sub(royalty_amount)
            .expect("Seller amount underflow");

        let mut agent = Self::load_agent(&env, listing.asset_id);
        agent.escrow_locked = false;
        agent.escrow_holder = None;
        agent.updated_at = env.ledger().timestamp();
        Self::save_agent(&env, listing.asset_id, &agent);

        listing.active = false;
        let lk = Self::listing_key(&env, listing_id);
        env.storage().instance().set(&lk, &listing);

        env.storage().instance().remove(&psk);

        env.events().publish(
            (symbol_short!("agnt_sold"),),
            (listing_id, listing.asset_id, pending.buyer.clone(), seller_amount, royalty_amount),
        );
        env.events().publish(
            (symbol_short!("esc_rel"),),
            (listing.asset_id, pending.buyer, env.current_contract_address()),
        );
    }

    // =========================================================================
    // Rollback (called by hub on failure)
    // =========================================================================

    /// Compensating action for the sale steps.
    /// Restores agent ownership to seller and releases escrow if needed.
    /// `encoded_args`: 8 bytes big-endian listing_id.
    pub fn rollback(env: Env, encoded_args: Bytes) {
        if encoded_args.is_empty() {
            return;
        }
        let listing_id = Self::decode_u64(&encoded_args);
        let psk = Self::pending_sale_key(&env, listing_id);
        let pending_opt: Option<PendingSale> = env.storage().instance().get(&psk);

        let pending = match pending_opt {
            Some(p) => p,
            None => return, // nothing to roll back
        };

        let listing_opt = Self::try_load_listing(&env, listing_id);
        if let Ok(listing) = listing_opt {
            if let Ok(mut agent) = Self::try_load_agent(&env, listing.asset_id) {
                // Restore ownership if it was transferred
                if agent.owner == pending.buyer {
                    agent.owner = pending.seller.clone();
                    agent.nonce = agent.nonce.checked_add(1).expect("Nonce overflow");
                    agent.updated_at = env.ledger().timestamp();
                    env.events().publish(
                        (symbol_short!("rb_own"),),
                        (listing.asset_id, pending.buyer.clone(), pending.seller.clone(), env.ledger().timestamp()),
                    );
                }
                // Release escrow
                if agent.escrow_locked {
                    agent.escrow_locked = false;
                    agent.escrow_holder = None;
                    agent.updated_at = env.ledger().timestamp();
                    env.events().publish(
                        (symbol_short!("rb_esc"),),
                        (listing.asset_id, env.ledger().timestamp()),
                    );
                }
                Self::save_agent(&env, listing.asset_id, &agent);
            }
        }

        env.storage().instance().remove(&psk);
    }

    // =========================================================================
    // Standard execution-hub step interface
    // =========================================================================

    /// Entry point called by the execution hub for every workflow step.
    /// Dispatches to the correct step function based on step_index.
    pub fn exec_step(env: Env, step_index: u32, encoded_args: Bytes) {
        match step_index {
            0 => Self::verify_sale(env, encoded_args),
            1 => Self::transfer_ownership(env, encoded_args),
            2 => Self::record_sale(env, encoded_args),
            _ => panic!("Unknown step index"),
        }
    }

    // =========================================================================
    // Workflow completion callback (called by hub)
    // =========================================================================

    /// `status`: 2=Completed, 3=RolledBack, 4=Failed, 5=Cancelled
    pub fn wf_done(env: Env, workflow_id: u64, status: u32) {
        let wlk = Self::wf_listing_key(&env, workflow_id);
        let listing_id: Option<u64> = env.storage().instance().get(&wlk);

        let lid = match listing_id {
            Some(id) => id,
            None => return,
        };

        let psk = Self::pending_sale_key(&env, lid);

        match status {
            2 => {
                // Completed — remove cross-reference
                env.storage().instance().remove(&wlk);
                env.events().publish(
                    (symbol_short!("cb_ok"),),
                    (workflow_id, lid, env.ledger().timestamp()),
                );
            }
            3 | 4 | 5 => {
                // RolledBack / Failed / Cancelled — ensure listing stays active
                if let Ok(mut listing) = Self::try_load_listing(&env, lid) {
                    if !listing.active {
                        listing.active = true;
                        let lk = Self::listing_key(&env, lid);
                        env.storage().instance().set(&lk, &listing);
                    }
                }
                if env.storage().instance().has(&psk) {
                    env.storage().instance().remove(&psk);
                }
                env.storage().instance().remove(&wlk);
                env.events().publish(
                    (symbol_short!("cb_fail"),),
                    (workflow_id, lid, status, env.ledger().timestamp()),
                );
            }
            _ => {}
        }
    }

    // =========================================================================
    // Cancel listing
    // =========================================================================

    pub fn cancel_listing(env: Env, listing_id: u64, seller: Address) {
        seller.require_auth();
        if listing_id == 0 {
            panic!("Invalid listing ID");
        }
        let mut listing = Self::load_listing(&env, listing_id);
        if listing.seller != seller {
            panic!("Only seller can cancel listing");
        }
        if !listing.active {
            panic!("Listing is not active");
        }

        let marketplace = env.current_contract_address();
        let mut agent = Self::load_agent(&env, listing.asset_id);
        if agent.escrow_locked {
            match &agent.escrow_holder {
                Some(h) if h == &marketplace => {
                    agent.escrow_locked = false;
                    agent.escrow_holder = None;
                    agent.updated_at = env.ledger().timestamp();
                    agent.nonce = agent.nonce.checked_add(1).expect("Nonce overflow");
                    Self::save_agent(&env, listing.asset_id, &agent);
                }
                _ => panic!("Agent locked by a different contract"),
            }
        }

        listing.active = false;
        let lk = Self::listing_key(&env, listing_id);
        env.storage().instance().set(&lk, &listing);

        env.events().publish(
            (symbol_short!("lst_cncl"),),
            (listing_id, listing.asset_id, seller),
        );
    }

    // =========================================================================
    // Royalties
    // =========================================================================

    pub fn set_royalty(
        env: Env,
        agent_id: u64,
        creator: Address,
        recipient: Address,
        percentage: u32,
    ) {
        creator.require_auth();
        if agent_id == 0 {
            panic!("Invalid agent ID");
        }
        if percentage > stellai_lib::MAX_ROYALTY_PERCENTAGE {
            panic!("Royalty exceeds maximum");
        }
        let agent = Self::load_agent(&env, agent_id);
        if agent.owner != creator {
            panic!("Only agent owner can set royalty");
        }
        let rk = Self::royalty_key(&env, agent_id);
        env.storage()
            .instance()
            .set(&rk, &stellai_lib::RoyaltyInfo { recipient, fee: percentage });
        env.events()
            .publish((symbol_short!("roy_set"),), (agent_id, percentage));
    }

    pub fn get_royalty(env: Env, agent_id: u64) -> Option<stellai_lib::RoyaltyInfo> {
        if agent_id == 0 {
            panic!("Invalid agent ID");
        }
        env.storage().instance().get(&Self::royalty_key(&env, agent_id))
    }

    // =========================================================================
    // Queries
    // =========================================================================

    pub fn get_listing(env: Env, listing_id: u64) -> stellai_lib::Listing {
        Self::load_listing(&env, listing_id)
    }

    pub fn get_pending_sale(env: Env, listing_id: u64) -> Option<PendingSale> {
        env.storage().instance().get(&Self::pending_sale_key(&env, listing_id))
    }

    pub fn get_admin(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&Symbol::new(&env, ADMIN_KEY))
            .expect("Not initialized")
    }

    pub fn get_execution_hub(env: Env) -> Address {
        Self::get_hub(&env)
    }

    // =========================================================================
    // Private helpers
    // =========================================================================

    fn listing_key(env: &Env, listing_id: u64) -> (String, u64) {
        (String::from_str(env, LISTING_PREFIX), listing_id)
    }

    fn royalty_key(env: &Env, agent_id: u64) -> (String, u64) {
        (String::from_str(env, ROYALTY_PREFIX), agent_id)
    }

    fn pending_sale_key(env: &Env, listing_id: u64) -> (String, u64) {
        (String::from_str(env, PENDING_SALE_PREFIX), listing_id)
    }

    fn wf_listing_key(env: &Env, workflow_id: u64) -> (String, u64) {
        (String::from_str(env, WF_LISTING_PREFIX), workflow_id)
    }

    fn agent_key(env: &Env, agent_id: u64) -> (String, u64) {
        (String::from_str(env, stellai_lib::AGENT_KEY_PREFIX), agent_id)
    }

    fn load_agent(env: &Env, agent_id: u64) -> stellai_lib::Agent {
        env.storage()
            .instance()
            .get(&Self::agent_key(env, agent_id))
            .expect("Agent not found")
    }

    fn try_load_agent(env: &Env, agent_id: u64) -> Result<stellai_lib::Agent, ()> {
        env.storage().instance().get(&Self::agent_key(env, agent_id)).ok_or(())
    }

    fn save_agent(env: &Env, agent_id: u64, agent: &stellai_lib::Agent) {
        env.storage().instance().set(&Self::agent_key(env, agent_id), agent);
    }

    fn load_listing(env: &Env, listing_id: u64) -> stellai_lib::Listing {
        env.storage()
            .instance()
            .get(&Self::listing_key(env, listing_id))
            .expect("Listing not found")
    }

    fn try_load_listing(env: &Env, listing_id: u64) -> Result<stellai_lib::Listing, ()> {
        env.storage().instance().get(&Self::listing_key(env, listing_id)).ok_or(())
    }

    fn get_hub(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&Symbol::new(env, HUB_KEY))
            .expect("Execution hub not set")
    }

    fn next_listing_id(env: &Env) -> u64 {
        let key = Symbol::new(env, LISTING_CTR_KEY);
        let current: u64 = env.storage().instance().get(&key).unwrap_or(0);
        let next = current.checked_add(1).expect("Listing ID overflow");
        env.storage().instance().set(&key, &next);
        next
    }

    fn assert_admin(env: &Env, caller: &Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&Symbol::new(env, ADMIN_KEY))
            .expect("Not initialized");
        if caller != &admin {
            panic!("Unauthorized");
        }
    }

    fn build_sale_steps(env: &Env, marketplace: &Address, listing_id: u64) -> Vec<WorkflowStep> {
        let encoded = Self::encode_u64(env, listing_id);

        let step0 = WorkflowStep {
            step_index: 0,
            name: String::from_str(env, "verify_sale"),
            target_contract: marketplace.clone(),
            function_name: String::from_str(env, "verify_sale"),
            encoded_args: encoded.clone(),
            required: true,
            max_retries: 0,
            retry_count: 0,
            status: WorkflowStepStatus::Pending,
            result: None,
            error: None,
            updated_at: 0,
        };

        let step1 = WorkflowStep {
            step_index: 1,
            name: String::from_str(env, "transfer_ownership"),
            target_contract: marketplace.clone(),
            function_name: String::from_str(env, "transfer_ownership"),
            encoded_args: encoded.clone(),
            required: true,
            max_retries: 1,
            retry_count: 0,
            status: WorkflowStepStatus::Pending,
            result: None,
            error: None,
            updated_at: 0,
        };

        let step2 = WorkflowStep {
            step_index: 2,
            name: String::from_str(env, "record_sale"),
            target_contract: marketplace.clone(),
            function_name: String::from_str(env, "record_sale"),
            encoded_args: encoded,
            required: true,
            max_retries: 0,
            retry_count: 0,
            status: WorkflowStepStatus::Pending,
            result: None,
            error: None,
            updated_at: 0,
        };

        let mut steps = Vec::new(env);
        steps.push_back(step0);
        steps.push_back(step1);
        steps.push_back(step2);
        steps
    }

    fn encode_u64(env: &Env, value: u64) -> Bytes {
        Bytes::from_array(env, &value.to_be_bytes())
    }

    fn decode_u64(data: &Bytes) -> u64 {
        if data.len() < 8 {
            panic!("Encoded args too short");
        }
        let mut arr = [0u8; 8];
        for i in 0..8 {
            arr[i] = data.get(i as u32).expect("byte missing");
        }
        u64::from_be_bytes(arr)
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};

    fn setup_marketplace(env: &Env) -> (Address, Address) {
        let contract_id = env.register(Marketplace, ());
        let admin = Address::generate(env);
        MarketplaceClient::new(env, &contract_id).init_contract(&admin);
        (contract_id, admin)
    }

    fn seed_agent(env: &Env, contract_id: &Address, agent_id: u64, owner: &Address) {
        env.as_contract(contract_id, || {
            let key = (String::from_str(env, stellai_lib::AGENT_KEY_PREFIX), agent_id);
            env.storage().instance().set(
                &key,
                &stellai_lib::Agent {
                    id: agent_id,
                    owner: owner.clone(),
                    name: String::from_str(env, "Bot"),
                    model_hash: String::from_str(env, "h"),
                    metadata_cid: String::from_str(env, "c"),
                    capabilities: Vec::new(env),
                    evolution_level: 0,
                    created_at: 0,
                    updated_at: 0,
                    nonce: 0,
                    escrow_locked: false,
                    escrow_holder: None,
                },
            );
        });
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Initialisation
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_init() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract_id, admin) = setup_marketplace(&env);
        assert_eq!(MarketplaceClient::new(&env, &contract_id).get_admin(), admin);
    }

    #[test]
    #[should_panic(expected = "Already initialized")]
    fn test_double_init() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract_id, admin) = setup_marketplace(&env);
        MarketplaceClient::new(&env, &contract_id).init_contract(&admin);
    }

    #[test]
    fn test_set_execution_hub() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract_id, admin) = setup_marketplace(&env);
        let hub = Address::generate(&env);
        let client = MarketplaceClient::new(&env, &contract_id);
        client.set_execution_hub(&admin, &hub);
        assert_eq!(client.get_execution_hub(), hub);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Listings
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_create_listing() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract_id, _) = setup_marketplace(&env);
        let seller = Address::generate(&env);
        seed_agent(&env, &contract_id, 1, &seller);

        let client = MarketplaceClient::new(&env, &contract_id);
        let listing_id = client.create_listing(&1u64, &seller, &0u32, &1_000_000i128, &None);
        assert_eq!(listing_id, 1u64);
        let listing = client.get_listing(&listing_id);
        assert!(listing.active);
        assert_eq!(listing.seller, seller);
    }

    #[test]
    #[should_panic(expected = "Agent already locked in escrow")]
    fn test_create_listing_already_locked() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract_id, _) = setup_marketplace(&env);
        let seller = Address::generate(&env);
        let holder = Address::generate(&env);
        env.as_contract(&contract_id, || {
            let key = (String::from_str(&env, stellai_lib::AGENT_KEY_PREFIX), 2u64);
            env.storage().instance().set(
                &key,
                &stellai_lib::Agent {
                    id: 2,
                    owner: seller.clone(),
                    name: String::from_str(&env, "B"),
                    model_hash: String::from_str(&env, "h"),
                    metadata_cid: String::from_str(&env, "c"),
                    capabilities: Vec::new(&env),
                    evolution_level: 0,
                    created_at: 0,
                    updated_at: 0,
                    nonce: 0,
                    escrow_locked: true,
                    escrow_holder: Some(holder),
                },
            );
        });
        MarketplaceClient::new(&env, &contract_id)
            .create_listing(&2u64, &seller, &0u32, &500i128, &None);
    }

    #[test]
    #[should_panic(expected = "Price out of valid range")]
    fn test_negative_price_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract_id, _) = setup_marketplace(&env);
        let seller = Address::generate(&env);
        seed_agent(&env, &contract_id, 3, &seller);
        MarketplaceClient::new(&env, &contract_id)
            .create_listing(&3u64, &seller, &0u32, &-1i128, &None);
    }

    #[test]
    fn test_cancel_listing() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract_id, _) = setup_marketplace(&env);
        let seller = Address::generate(&env);
        seed_agent(&env, &contract_id, 4, &seller);
        let client = MarketplaceClient::new(&env, &contract_id);
        let lid = client.create_listing(&4u64, &seller, &0u32, &2_000i128, &None);
        assert!(client.get_listing(&lid).active);
        client.cancel_listing(&lid, &seller);
        assert!(!client.get_listing(&lid).active);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Royalties
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_set_and_get_royalty() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract_id, _) = setup_marketplace(&env);
        let creator = Address::generate(&env);
        let recipient = Address::generate(&env);
        seed_agent(&env, &contract_id, 5, &creator);
        let client = MarketplaceClient::new(&env, &contract_id);
        client.set_royalty(&5u64, &creator, &recipient, &500u32);
        let info = client.get_royalty(&5u64).unwrap();
        assert_eq!(info.fee, 500u32);
    }

    #[test]
    #[should_panic(expected = "Royalty exceeds maximum")]
    fn test_royalty_cap_enforced() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract_id, _) = setup_marketplace(&env);
        let creator = Address::generate(&env);
        let recipient = Address::generate(&env);
        seed_agent(&env, &contract_id, 6, &creator);
        MarketplaceClient::new(&env, &contract_id)
            .set_royalty(&6u64, &creator, &recipient, &20_000u32);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Step functions (direct invocation)
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_verify_sale_step() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract_id, _) = setup_marketplace(&env);
        let seller = Address::generate(&env);
        let buyer = Address::generate(&env);

        env.as_contract(&contract_id, || {
            let mp = contract_id.clone();
            let ak = (String::from_str(&env, stellai_lib::AGENT_KEY_PREFIX), 10u64);
            env.storage().instance().set(&ak, &stellai_lib::Agent {
                id: 10, owner: seller.clone(),
                name: String::from_str(&env, "V"), model_hash: String::from_str(&env, "h"),
                metadata_cid: String::from_str(&env, "c"), capabilities: Vec::new(&env),
                evolution_level: 0, created_at: 0, updated_at: 0, nonce: 0,
                escrow_locked: true, escrow_holder: Some(mp),
            });
            let lk = (String::from_str(&env, LISTING_PREFIX), 1u64);
            env.storage().instance().set(&lk, &stellai_lib::Listing {
                listing_id: 1, asset_id: 10, asset_type: stellai_lib::AssetType::Agent,
                seller: seller.clone(), price: 100,
                listing_type: stellai_lib::ListingType::Sale, active: true, created_at: 0,
            });
            let psk = (String::from_str(&env, PENDING_SALE_PREFIX), 1u64);
            env.storage().instance().set(&psk, &PendingSale {
                listing_id: 1, buyer: buyer.clone(), amount: 200,
                seller: seller.clone(), agent_id: 10, workflow_id: 1, created_at: 0,
            });
        });

        let client = MarketplaceClient::new(&env, &contract_id);
        client.verify_sale(&Bytes::from_array(&env, &1u64.to_be_bytes()));
    }

    #[test]
    fn test_transfer_ownership_step() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract_id, _) = setup_marketplace(&env);
        let seller = Address::generate(&env);
        let buyer = Address::generate(&env);

        env.as_contract(&contract_id, || {
            let mp = contract_id.clone();
            let ak = (String::from_str(&env, stellai_lib::AGENT_KEY_PREFIX), 11u64);
            env.storage().instance().set(&ak, &stellai_lib::Agent {
                id: 11, owner: seller.clone(),
                name: String::from_str(&env, "T"), model_hash: String::from_str(&env, "h"),
                metadata_cid: String::from_str(&env, "c"), capabilities: Vec::new(&env),
                evolution_level: 0, created_at: 0, updated_at: 0, nonce: 0,
                escrow_locked: true, escrow_holder: Some(mp),
            });
            let lk = (String::from_str(&env, LISTING_PREFIX), 2u64);
            env.storage().instance().set(&lk, &stellai_lib::Listing {
                listing_id: 2, asset_id: 11, asset_type: stellai_lib::AssetType::Agent,
                seller: seller.clone(), price: 100,
                listing_type: stellai_lib::ListingType::Sale, active: true, created_at: 0,
            });
            let psk = (String::from_str(&env, PENDING_SALE_PREFIX), 2u64);
            env.storage().instance().set(&psk, &PendingSale {
                listing_id: 2, buyer: buyer.clone(), amount: 200,
                seller: seller.clone(), agent_id: 11, workflow_id: 2, created_at: 0,
            });
        });

        MarketplaceClient::new(&env, &contract_id)
            .transfer_ownership(&Bytes::from_array(&env, &2u64.to_be_bytes()));

        env.as_contract(&contract_id, || {
            let ak = (String::from_str(&env, stellai_lib::AGENT_KEY_PREFIX), 11u64);
            let agent: stellai_lib::Agent = env.storage().instance().get(&ak).unwrap();
            assert_eq!(agent.owner, buyer);
        });
    }

    #[test]
    fn test_record_sale_step() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract_id, _) = setup_marketplace(&env);
        let seller = Address::generate(&env);
        let buyer = Address::generate(&env);

        env.as_contract(&contract_id, || {
            let mp = contract_id.clone();
            let ak = (String::from_str(&env, stellai_lib::AGENT_KEY_PREFIX), 12u64);
            env.storage().instance().set(&ak, &stellai_lib::Agent {
                id: 12, owner: buyer.clone(),
                name: String::from_str(&env, "R"), model_hash: String::from_str(&env, "h"),
                metadata_cid: String::from_str(&env, "c"), capabilities: Vec::new(&env),
                evolution_level: 0, created_at: 0, updated_at: 0, nonce: 1,
                escrow_locked: true, escrow_holder: Some(mp),
            });
            let lk = (String::from_str(&env, LISTING_PREFIX), 3u64);
            env.storage().instance().set(&lk, &stellai_lib::Listing {
                listing_id: 3, asset_id: 12, asset_type: stellai_lib::AssetType::Agent,
                seller: seller.clone(), price: 100,
                listing_type: stellai_lib::ListingType::Sale, active: true, created_at: 0,
            });
            let psk = (String::from_str(&env, PENDING_SALE_PREFIX), 3u64);
            env.storage().instance().set(&psk, &PendingSale {
                listing_id: 3, buyer: buyer.clone(), amount: 200,
                seller: seller.clone(), agent_id: 12, workflow_id: 3, created_at: 0,
            });
        });

        MarketplaceClient::new(&env, &contract_id)
            .record_sale(&Bytes::from_array(&env, &3u64.to_be_bytes()));

        env.as_contract(&contract_id, || {
            let lk = (String::from_str(&env, LISTING_PREFIX), 3u64);
            let listing: stellai_lib::Listing = env.storage().instance().get(&lk).unwrap();
            assert!(!listing.active);

            let ak = (String::from_str(&env, stellai_lib::AGENT_KEY_PREFIX), 12u64);
            let agent: stellai_lib::Agent = env.storage().instance().get(&ak).unwrap();
            assert!(!agent.escrow_locked);
            assert!(agent.escrow_holder.is_none());

            let psk = (String::from_str(&env, PENDING_SALE_PREFIX), 3u64);
            assert!(!env.storage().instance().has(&psk));
        });
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Rollback
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_rollback_restores_seller() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract_id, _) = setup_marketplace(&env);
        let seller = Address::generate(&env);
        let buyer = Address::generate(&env);

        env.as_contract(&contract_id, || {
            let mp = contract_id.clone();
            let ak = (String::from_str(&env, stellai_lib::AGENT_KEY_PREFIX), 20u64);
            env.storage().instance().set(&ak, &stellai_lib::Agent {
                id: 20, owner: buyer.clone(), // ownership already xferred
                name: String::from_str(&env, "Rb"), model_hash: String::from_str(&env, "rb"),
                metadata_cid: String::from_str(&env, "rbc"), capabilities: Vec::new(&env),
                evolution_level: 0, created_at: 0, updated_at: 0, nonce: 1,
                escrow_locked: true, escrow_holder: Some(mp),
            });
            let lk = (String::from_str(&env, LISTING_PREFIX), 10u64);
            env.storage().instance().set(&lk, &stellai_lib::Listing {
                listing_id: 10, asset_id: 20, asset_type: stellai_lib::AssetType::Agent,
                seller: seller.clone(), price: 300,
                listing_type: stellai_lib::ListingType::Sale, active: true, created_at: 0,
            });
            let psk = (String::from_str(&env, PENDING_SALE_PREFIX), 10u64);
            env.storage().instance().set(&psk, &PendingSale {
                listing_id: 10, buyer: buyer.clone(), amount: 300,
                seller: seller.clone(), agent_id: 20, workflow_id: 99, created_at: 0,
            });
        });

        MarketplaceClient::new(&env, &contract_id)
            .rollback(&Bytes::from_array(&env, &10u64.to_be_bytes()));

        env.as_contract(&contract_id, || {
            let ak = (String::from_str(&env, stellai_lib::AGENT_KEY_PREFIX), 20u64);
            let agent: stellai_lib::Agent = env.storage().instance().get(&ak).unwrap();
            assert_eq!(agent.owner, seller);
            assert!(!agent.escrow_locked);
        });
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Callback
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_callback_success_cleans_up() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract_id, _) = setup_marketplace(&env);

        env.as_contract(&contract_id, || {
            let wlk = (String::from_str(&env, WF_LISTING_PREFIX), 7u64);
            env.storage().instance().set(&wlk, &5u64);
            let lk = (String::from_str(&env, LISTING_PREFIX), 5u64);
            env.storage().instance().set(&lk, &stellai_lib::Listing {
                listing_id: 5, asset_id: 99, asset_type: stellai_lib::AssetType::Agent,
                seller: Address::generate(&env),
                price: 100, listing_type: stellai_lib::ListingType::Sale,
                active: false, created_at: 0,
            });
        });

        MarketplaceClient::new(&env, &contract_id).wf_done(&7u64, &2u32);

        env.as_contract(&contract_id, || {
            let wlk = (String::from_str(&env, WF_LISTING_PREFIX), 7u64);
            assert!(!env.storage().instance().has(&wlk));
        });
    }

    #[test]
    fn test_callback_failure_reactivates_listing() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract_id, _) = setup_marketplace(&env);

        env.as_contract(&contract_id, || {
            let wlk = (String::from_str(&env, WF_LISTING_PREFIX), 8u64);
            env.storage().instance().set(&wlk, &6u64);
            let lk = (String::from_str(&env, LISTING_PREFIX), 6u64);
            env.storage().instance().set(&lk, &stellai_lib::Listing {
                listing_id: 6, asset_id: 50, asset_type: stellai_lib::AssetType::Agent,
                seller: Address::generate(&env),
                price: 100, listing_type: stellai_lib::ListingType::Sale,
                active: false, created_at: 0,
            });
        });

        MarketplaceClient::new(&env, &contract_id).wf_done(&8u64, &4u32);

        env.as_contract(&contract_id, || {
            let lk = (String::from_str(&env, LISTING_PREFIX), 6u64);
            let listing: stellai_lib::Listing = env.storage().instance().get(&lk).unwrap();
            assert!(listing.active);
        });
    }
}
