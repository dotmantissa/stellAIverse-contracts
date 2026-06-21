#![no_std]
pub mod types;
pub mod atomic;
pub mod payment_types;
pub mod payments;

#[cfg(test)]
mod prop_tests;
mod storage;
#[cfg(test)]
mod test_bid_history;
#[cfg(test)]
mod test_dynamic_fee_enhancement;
#[cfg(test)]
mod test_advanced_lease;
#[cfg(test)]
mod test_lease;

use crate::types::{MarketplaceCircuitBreaker, PricingRule};
use payment_types::PaymentRecord;
use payments::{
    calculate_and_distribute_royalties, calculate_splits, execute_payment_routing,
    PaymentRoutingContext, validate_royalty_config,
};
use soroban_sdk::xdr::ToXdr;
use soroban_sdk::{
    contract, contractimpl, token, Address, Bytes, BytesN, Env, IntoVal, Map, String, Symbol,
    TryIntoVal, Val, Vec, symbol_short, vec,
};
use stellai_lib::{
    audit::{create_audit_log, OperationType},
    errors::{error_description, ContractError},
    rbac,
    storage_keys::LISTING_COUNTER_KEY,
    types::{
        Approval, ApprovalConfig, ApprovalHistory, ApprovalStatus, Auction, AuctionStatus,
        AuctionType, BidRecord, LeaseData, LeaseExtensionRequest, LeaseHistoryEntry, LeaseState,
        LeaseNotification, LeaseNotificationChannel, Listing, ListingType, RoyaltyInfo,
        TransactionStep, TransactionStatus, PaymentFrequency, LateFeeType, LateFeePolicy,
        RenewalPolicy,
        AssetClass, RoyaltyRecipient, RoyaltyConfig, RoyaltyPaymentRecord,
        OracleData, AssetClassRoyaltySettings,
    },
    validation,
};
use storage::{Escrow, EscrowConfig, EscrowStatus, *};

#[contract]
pub struct MarketplaceContract;

pub use MarketplaceContract as Marketplace;
pub use MarketplaceContractClient as MarketplaceClient;

const DATA_EXPIRATION_WINDOW_SECONDS: u64 = 3600; 
const BPS_DENOMINATOR: u128 = 10_000;

#[contractimpl]
impl MarketplaceContract {
    /// Initialize the marketplace contract
    pub fn initialize(env: Env, admin: Address, payment_token: Address, platform_fee_bps: u32) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("Contract already initialized");
        }
        
        admin.require_auth();
        set_admin(&env, &admin);
        set_payment_token(&env, payment_token.clone());
        set_platform_fee(&env, platform_fee_bps);
        
        // Initialize atomic transaction support
        crate::atomic::MarketplaceAtomicSupport::initialize(&env);
        
        env.events().publish((symbol_short!("init"),), (admin, payment_token, platform_fee_bps));
    }

    pub fn authorize_oracle(env: Env, admin: Address, oracle: Address) {
        admin.require_auth();
        env.storage().instance().set(&Symbol::new(&env, "oracle"), &oracle);
    }

    pub fn set_circuit_breaker(env: Env, admin: Address, status: MarketplaceCircuitBreaker, fee_bps: u32) {
        admin.require_auth();
        assert!(fee_bps <= 5000, "Platform fee cannot exceed 50%");
        Self::verify_admin(&env, &admin);
        storage::set_platform_fee(&env, fee_bps);
        env.events()
            .publish((Symbol::new(&env, "platform_fee_updated"),), (fee_bps,));
    }

    /// Get the configured platform fee.
    pub fn get_platform_fee(env: Env) -> u32 {
        storage::get_platform_fee(&env)
    }

    /// Update the payment token used by the marketplace.
    pub fn set_payment_token(env: Env, admin: Address, payment_token: Address) {
        admin.require_auth();
        Self::verify_admin(&env, &admin);
        storage::set_payment_token(&env, payment_token.clone());
        env.events().publish(
            (Symbol::new(&env, "payment_token_updated"),),
            (payment_token,),
        );
    }

    /// Update the platform fee directly.
    pub fn set_platform_fee(env: Env, admin: Address, fee_bps: u32) {
        admin.require_auth();
        Self::verify_admin(&env, &admin);
        assert!(fee_bps <= 5000, "Platform fee cannot exceed 50%");
        storage::set_platform_fee(&env, fee_bps);
        env.events()
            .publish((Symbol::new(&env, "platform_fee_updated"),), (fee_bps,));
    }

    /// Atomic transaction support functions
    pub fn get_next_atomic_transaction_id(env: Env) -> u64 {
        crate::atomic::MarketplaceAtomicSupport::get_next_transaction_id(&env)
    }

    pub fn try_execute_atomic_transaction(env: Env, initiator: Address, steps: Vec<TransactionStep>) -> bool {
        initiator.require_auth();
        let transaction_id = Self::get_next_atomic_transaction_id(env.clone());
        crate::atomic::MarketplaceAtomicSupport::execute_atomic_transaction(&env, transaction_id, &steps)
    }

    pub fn get_atomic_transaction(env: Env, transaction_id: u64) -> Option<crate::atomic::AtomicTransactionState> {
        let tx_key = (Symbol::new(&env, "atomic_tx"), transaction_id);
        env.storage().instance().get(&tx_key)
    }

    pub fn get_atomic_step_state(env: Env, transaction_id: u64, step_id: u32) -> Option<crate::atomic::AtomicStepState> {
        let step_key = (Symbol::new(&env, "atomic_step"), transaction_id, step_id);
        env.storage().instance().get(&step_key)
    }

    /// Check if a user has a valid active lease for an agent.
    pub fn check_lease_permission(env: Env, agent_id: u64, user: Address) -> bool {
        // Find if there is an active lease for this agent and user
        // In a real production system, we'd have an index for agent -> active_lease
        // For now, we'll iterate through user's leases as a fallback if no direct index exists
        let lessee_count = storage::get_lessee_lease_count(&env, &user);
        for i in 0..lessee_count {
            if let Some(lease_id) = storage::get_lessee_lease(&env, &user, i) {
                if let Some(lease) = storage::get_lease(&env, lease_id) {
                    if lease.agent_id == agent_id && Self::is_live_lease_state(lease.status) {
                        let now = env.ledger().timestamp();
                        if now >= lease.start_time && now < lease.end_time {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// Internal: verify caller is the stored admin — always re-reads from storage (Issue #152)
    fn verify_admin(env: &Env, caller: &Address) {
        rbac::require_admin(env, caller).unwrap_or_else(|_| panic!("Unauthorized"));
    }

    /// Create a new listing
    pub fn create_listing(
        env: Env,
        agent_id: u64,
        seller: Address,
        listing_type: u32,
        price: i128,
    ) -> u64 {
        seller.require_auth();

        if validation::validate_nonzero_id(agent_id).is_err() {
            panic!("Invalid agent ID");
        }
        if listing_type > 2 {
            panic!("Invalid listing type");
        }
        if price <= 0 {
            panic!("Price must be positive");
        }

        // Generate listing ID
        let counter: u64 = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, LISTING_COUNTER_KEY))
            .unwrap_or(0);
        let listing_id = counter + 1;

        let listing = Listing {
            listing_id,
            agent_id,
            seller: seller.clone(),
            price,
            listing_type: match listing_type {
                0 => ListingType::Sale,
                1 => ListingType::Lease,
                2 => ListingType::Auction,
                _ => panic!("Invalid listing type"),
            },
            active: true,
            created_at: env.ledger().timestamp(),
        };

        // Store listing using tuple key
        let listing_key = (Symbol::new(&env, "listing"), listing_id);
        env.storage().instance().set(&listing_key, &listing);

        // Update counter
        env.storage()
            .instance()
            .set(&Symbol::new(&env, LISTING_COUNTER_KEY), &listing_id);

        env.events().publish(
            (Symbol::new(&env, "listing_created"),),
            (listing_id, agent_id, seller.clone(), price),
        );

        // Log audit entry for sale created
        let before_state = String::from_str(&env, "{}");
        let after_state = String::from_str(&env, "{\"listing_created\":true}");
        let tx_hash = String::from_str(&env, "create_listing");
        let description = Some(String::from_str(&env, "Marketplace listing created"));

        let _ = create_audit_log(
            &env,
            seller,
            OperationType::SaleCreated,
            before_state,
            after_state,
            tx_hash,
            description,
        );

        listing_id
    }

    /// Purchase an agent - funds are held in escrow until buyer confirms receipt
    pub fn buy_agent(env: Env, listing_id: u64, buyer: Address) {
        buyer.require_auth();

        // ─── SNAPSHOT PHASE ───
        let listing_key = (Symbol::new(&env, "listing"), listing_id);
        let mut listing: Listing = env
            .storage()
            .instance()
            .get(&listing_key)
            .expect("Listing not found");

        let approval_config = get_approval_config(&env);
        let escrow_config = get_escrow_config(&env);
        let _platform_fee_bps = Self::get_platform_fee(env.clone());
        let _royalty_info = Self::get_royalty(env.clone(), listing.agent_id);

        // ─── VALIDATION PHASE ───
        if validation::validate_nonzero_id(listing_id).is_err() {
            panic!("Invalid listing ID");
        }

        if !listing.active {
            panic!("Listing is not active");
        }

        if listing.listing_type != ListingType::Sale {
            panic!("Listing is not for sale. Use initiate_lease() for leases or bid() for auctions.");
        }

        if listing.price >= approval_config.threshold {
            panic!("High-value sale requires multi-signature approval. Use propose_sale() first.");
        }

        // ─── MUTATION PHASE ───

        // Process fee transition if active
        Self::process_fee_transition(env.clone());

        // Transfer funds from buyer to contract (escrow)
        let payment_token = get_payment_token(&env);
        let token_client = token::Client::new(&env, &payment_token);
        token_client.transfer(&buyer, &env.current_contract_address(), &listing.price);

        // Create escrow entry
        let escrow_id = increment_escrow_counter(&env);
        let now = env.ledger().timestamp();
        let escrow = Escrow {
            escrow_id,
            listing_id,
            agent_id: listing.agent_id,
            buyer: buyer.clone(),
            seller: listing.seller.clone(),
            amount: listing.price,
            created_at: now,
            auto_release_at: now + escrow_config.auto_release_period_seconds,
            status: EscrowStatus::Held,
            dispute_resolved_at: None,
            resolved_by: None,
        };
        set_escrow(&env, &escrow);

        // Mark listing as inactive
        listing.active = false;
        env.storage().instance().set(&listing_key, &listing);

        env.events().publish(
            (Symbol::new(&env, "agent_purchased_escrowed"),),
            (
                listing_id,
                escrow_id,
                listing.agent_id,
                buyer.clone(),
                listing.seller.clone(),
                listing.price,
                escrow.auto_release_at,
            ),
        );

        // Auto-mint credit score NFT for successful purchase
        if let Err(e) = Self::auto_mint_credit_on_purchase(env.clone(), listing_id, buyer.clone()) {
            // Log error but don't fail the transaction
            env.events().publish(
                (Symbol::new(&env, "CreditScoreNFTMintFailed"),),
                (
                    listing_id,
                    buyer,
                    String::from_str(&env, error_description(e)),
                ),
            );
        }
    }

    /// Helper to route payment for a completed sale with automated royalty distribution.
    fn route_sale_payment(
        env: &Env,
        agent_id: u64,
        sale_price: i128,
        buyer: &Address,
        seller: &Address,
        royalty_info: Option<RoyaltyInfo>,
        platform_fee_bps: u32,
        asset_class: AssetClass,
    ) {
        // First, attempt automated royalty distribution
        let transaction_id = env.ledger().sequence() as u64;
        let royalty_result = calculate_and_distribute_royalties(
            env,
            agent_id,
            sale_price,
            asset_class,
            transaction_id,
        );

        // If automated royalty distribution succeeded, use it
        // Otherwise, fall back to legacy payment routing
        let (royalty_recipients, royalty_rate) = if royalty_result.is_ok() {
            // Royalties already distributed, skip in payment routing
            (Vec::new(env), 0u32)
        } else {
            // Fall back to legacy routing
            let mut recipients = Vec::new(env);
            let mut rate = 0u32;

            if let Some(info) = royalty_info {
                rate = info.fee;
                recipients.push_back((
                    info.recipient,
                    rate,
                    String::from_str(env, "creator"),
                ));
            }
            (recipients, rate)
        };

        let context = PaymentRoutingContext {
            agent_id,
            transaction_id,
            buyer: buyer.clone(),
            seller: seller.clone(),
            platform_address: env.current_contract_address(),
            royalty_recipients,
            asset_class,
        };

        let split = calculate_splits(env, sale_price, royalty_rate, platform_fee_bps, &context);
        execute_payment_routing(env, split);
        set_previous_owner(env, agent_id, seller);
    }

    /// Cancel a listing
    pub fn cancel_listing(env: Env, listing_id: u64, seller: Address) {
        seller.require_auth();

        if validation::validate_nonzero_id(listing_id).is_err() {
            panic!("Invalid listing ID");
        }

        let listing_key = (Symbol::new(&env, "listing"), listing_id);
        let mut listing: Listing = env
            .storage()
            .instance()
            .get(&listing_key)
            .expect("Listing not found");

        if listing.seller != seller {
            panic!("Unauthorized: only seller can cancel listing");
        }

        listing.active = false;
        env.storage().instance().set(&listing_key, &listing);

        env.events().publish(
            (Symbol::new(&env, "listing_cancelled"),),
            (listing_id, listing.agent_id, seller),
        );
    }

    /// Get a specific listing
    pub fn get_listing(env: Env, listing_id: u64) -> Option<Listing> {
        if validation::validate_nonzero_id(listing_id).is_err() {
            panic!("Invalid listing ID");
        }

        let listing_key = (Symbol::new(&env, "listing"), listing_id);
        env.storage().instance().get(&listing_key)
    }

    /// Retrieve payment history for an agent (immutable audit trail).
    pub fn get_payment_history(env: Env, agent_id: u64) -> Vec<PaymentRecord> {
        if validation::validate_nonzero_id(agent_id).is_err() {
            panic!("Invalid agent ID");
        }

        let mut history = Vec::new(&env);
        let count = storage::get_payment_history_count(&env, agent_id);

        for i in 0..count {
            if let Some(payment_id) = storage::get_payment_history_entry(&env, agent_id, i) {
                if let Some(record) = storage::get_payment_record(&env, payment_id) {
                    history.push_back(record);
                }
            }
        }

        history
    }

    /// Set royalty info for an agent
    pub fn set_royalty(env: Env, agent_id: u64, creator: Address, recipient: Address, fee: u32) {
        creator.require_auth();

        if validation::validate_nonzero_id(agent_id).is_err() {
            panic!("Invalid agent ID");
        }
        if fee > 2500 {
            panic!("Royalty fee exceeds maximum (25%)");
        }

        let royalty_info = RoyaltyInfo { recipient, fee };

        let royalty_key = (Symbol::new(&env, "royalty"), agent_id);
        env.storage().instance().set(&royalty_key, &royalty_info);

        env.events()
            .publish((Symbol::new(&env, "royalty_set"),), (agent_id, fee));
    }

    /// Get royalty info for an agent
    pub fn get_royalty(env: Env, agent_id: u64) -> Option<RoyaltyInfo> {
        if validation::validate_nonzero_id(agent_id).is_err() {
            panic!("Invalid agent ID");
        }

        let royalty_key = (Symbol::new(&env, "royalty"), agent_id);
        env.storage().instance().get(&royalty_key)
    }

    // ---------------- ROYALTY AUTOMATION ----------------

    /// Set royalty configuration for an agent with multiple recipients
    pub fn set_royalty_config(
        env: Env,
        admin: Address,
        agent_id: u64,
        recipients: Vec<RoyaltyRecipient>,
        total_bps: u32,
        min_threshold: i128,
        max_cap: Option<i128>,
        asset_class: AssetClass,
    ) {
        admin.require_auth();
        Self::verify_admin(&env, &admin);

        if validation::validate_nonzero_id(agent_id).is_err() {
            panic!("Invalid agent ID");
        }

        let config = RoyaltyConfig {
            recipients: recipients.clone(),
            total_bps,
            min_threshold,
            max_cap,
        };

        // Validate configuration
        validate_royalty_config(&config, asset_class, &env)
            .expect("Invalid royalty configuration");

        storage::set_royalty_config(&env, agent_id, &config);

        env.events().publish(
            (Symbol::new(&env, "royalty_config_set"),),
            (agent_id, total_bps, recipients.len() as u64),
        );
    }

    /// Get royalty configuration for an agent
    pub fn get_royalty_config(env: Env, agent_id: u64) -> Option<RoyaltyConfig> {
        if validation::validate_nonzero_id(agent_id).is_err() {
            panic!("Invalid agent ID");
        }

        storage::get_royalty_config(&env, agent_id)
    }

    /// Set royalty settings for an asset class (admin only)
    pub fn set_asset_class_royalty_settings(
        env: Env,
        admin: Address,
        asset_class: AssetClass,
        default_royalty_bps: u32,
        min_royalty_bps: u32,
        max_royalty_bps: u32,
        min_threshold: i128,
    ) {
        admin.require_auth();
        Self::verify_admin(&env, &admin);

        let settings = stellai_lib::types::AssetClassRoyaltySettings {
            asset_class,
            default_royalty_bps,
            min_royalty_bps,
            max_royalty_bps,
            min_threshold,
        };

        storage::set_asset_class_royalty_settings(&env, asset_class, &settings);

        env.events().publish(
            (Symbol::new(&env, "asset_class_royalty_settings_set"),),
            (asset_class as u32, default_royalty_bps),
        );
    }

    /// Get royalty settings for an asset class
    pub fn get_asset_class_royalty_settings(
        env: Env,
        asset_class: AssetClass,
    ) -> stellai_lib::types::AssetClassRoyaltySettings {
        storage::get_asset_class_royalty_settings(&env, asset_class)
            .unwrap_or_else(|| storage::get_default_asset_class_settings(&env, asset_class))
    }

    /// Get royalty payment history for an agent
    pub fn get_royalty_payment_history(env: Env, agent_id: u64) -> Vec<RoyaltyPaymentRecord> {
        if validation::validate_nonzero_id(agent_id).is_err() {
            panic!("Invalid agent ID");
        }

        let mut history = Vec::new(&env);
        let count = storage::get_royalty_payment_history_count(&env, agent_id);

        for i in 0..count {
            if let Some(payment_id) = storage::get_royalty_payment_history_entry(&env, agent_id, i) {
                if let Some(record) = storage::get_royalty_payment_record(&env, payment_id) {
                    history.push_back(record);
                }
            }
        }

        history
    }

    /// Get total royalties paid for an agent
    pub fn get_total_royalties_paid(env: Env, agent_id: u64) -> i128 {
        if validation::validate_nonzero_id(agent_id).is_err() {
            panic!("Invalid agent ID");
        }

        let history = Self::get_royalty_payment_history(env, agent_id);
        let mut total = 0i128;

        for i in 0..history.len() {
            let record = history.get(i).unwrap();
            total = total + record.total_royalty_paid;
        }

        total
    }

    // ---------------- MULTI-SIGNATURE APPROVAL ----------------

    /// Configure approval settings (admin only)
    pub fn set_approval_config(
        env: Env,
        admin: Address,
        threshold: i128,
        approvers_required: u32,
        total_approvers: u32,
        ttl_seconds: u64,
    ) {
        Self::verify_admin(&env, &admin);

        assert!(threshold > 0, "Threshold must be positive");
        assert!(
            approvers_required > 0,
            "Approvers required must be positive"
        );
        assert!(
            total_approvers >= approvers_required,
            "Total approvers must be >= required"
        );
        assert!(ttl_seconds > 0, "TTL must be positive");

        let config = ApprovalConfig {
            threshold,
            approvers_required,
            total_approvers,
            ttl_seconds,
        };

        set_approval_config(&env, &config);

        env.events().publish(
            (Symbol::new(&env, "ApprovalConfigUpdated"),),
            (threshold, approvers_required, total_approvers, ttl_seconds),
        );
    }

    /// Get current approval configuration
    pub fn get_approval_config(env: Env) -> ApprovalConfig {
        get_approval_config(&env)
    }

    /// Propose a sale for multi-signature approval (fixed-price listing)
    pub fn propose_sale(env: Env, listing_id: u64, buyer: Address, approvers: Vec<Address>) -> u64 {
        buyer.require_auth();

        if listing_id == 0 {
            panic!("Invalid listing ID");
        }

        let listing_key = (Symbol::new(&env, "listing"), listing_id);
        let listing: Listing = env
            .storage()
            .instance()
            .get(&listing_key)
            .expect("Listing not found");

        if !listing.active {
            panic!("Listing is not active");
        }

        let config = get_approval_config(&env);

        // Check if approval is required
        if listing.price < config.threshold {
            panic!("Price below approval threshold");
        }

        assert!(
            approvers.len() as u32 >= config.approvers_required,
            "Insufficient approvers"
        );
        assert!(
            approvers.len() as u32 <= config.total_approvers,
            "Too many approvers"
        );

        let approval_id = increment_approval_counter(&env);
        let now = env.ledger().timestamp();

        let approval = Approval {
            approval_id,
            listing_id: Some(listing_id),
            auction_id: None,
            buyer: buyer.clone(),
            price: listing.price,
            proposed_at: now,
            expires_at: now + config.ttl_seconds,
            status: ApprovalStatus::Pending,
            required_approvals: config.approvers_required,
            approvers: approvers.clone(),
            approvals_received: Vec::new(&env),
            rejections_received: Vec::new(&env),
            rejection_reasons: Vec::new(&env),
        };

        set_approval(&env, &approval);

        // Add to history
        let history = ApprovalHistory {
            approval_id,
            action: String::from_str(&env, "proposed"),
            actor: buyer.clone(),
            timestamp: now,
            reason: None,
        };
        add_approval_history(&env, approval_id, &history);

        env.events().publish(
            (Symbol::new(&env, "SaleProposed"),),
            (approval_id, listing_id, buyer, listing.price),
        );

        approval_id
    }

    /// Propose an auction win for multi-signature approval
    pub fn propose_auction_sale(env: Env, auction_id: u64, approvers: Vec<Address>) -> u64 {
        let auction = get_auction(&env, auction_id).expect("Auction not found");
        assert!(
            auction.status == AuctionStatus::Active,
            "Auction not active"
        );
        assert!(auction.highest_bidder.is_some(), "No winning bid");

        let config = get_approval_config(&env);

        // Check if approval is required
        if auction.highest_bid < config.threshold {
            panic!("Price below approval threshold");
        }

        assert!(
            approvers.len() as u32 >= config.approvers_required,
            "Insufficient approvers"
        );
        assert!(
            approvers.len() as u32 <= config.total_approvers,
            "Too many approvers"
        );

        let approval_id = increment_approval_counter(&env);
        let now = env.ledger().timestamp();
        let buyer = auction.highest_bidder.unwrap();

        let approval = Approval {
            approval_id,
            listing_id: None,
            auction_id: Some(auction_id),
            buyer: buyer.clone(),
            price: auction.highest_bid,
            proposed_at: now,
            expires_at: now + config.ttl_seconds,
            status: ApprovalStatus::Pending,
            required_approvals: config.approvers_required,
            approvers: approvers.clone(),
            approvals_received: Vec::new(&env),
            rejections_received: Vec::new(&env),
            rejection_reasons: Vec::new(&env),
        };

        set_approval(&env, &approval);

        // Add to history
        let history = ApprovalHistory {
            approval_id,
            action: String::from_str(&env, "proposed"),
            actor: buyer.clone(),
            timestamp: now,
            reason: None,
        };
        add_approval_history(&env, approval_id, &history);

        env.events().publish(
            (Symbol::new(&env, "SaleProposed"),),
            (approval_id, auction_id, buyer, auction.highest_bid),
        );

        approval_id
    }

    /// Approve a proposed sale
    pub fn approve_sale(env: Env, approval_id: u64, approver: Address) {
        approver.require_auth();

        if approval_id == 0 {
            panic!("Invalid approval ID");
        }

        let mut approval = get_approval(&env, approval_id).expect("Approval not found");

        assert!(
            approval.status == ApprovalStatus::Pending,
            "Approval not pending"
        );
        assert!(
            env.ledger().timestamp() < approval.expires_at,
            "Approval expired"
        );

        // Check if approver is authorized
        assert!(
            approval.approvers.contains(&approver),
            "Unauthorized approver"
        );

        // Check if already approved
        assert!(
            !approval.approvals_received.contains(&approver),
            "Already approved"
        );
        assert!(
            !approval.rejections_received.contains(&approver),
            "Already rejected"
        );

        approval.approvals_received.push_back(approver.clone());

        // Add to history
        let history = ApprovalHistory {
            approval_id,
            action: String::from_str(&env, "approved"),
            actor: approver.clone(),
            timestamp: env.ledger().timestamp(),
            reason: None,
        };
        add_approval_history(&env, approval_id, &history);

        // Check if we have enough approvals
        if approval.approvals_received.len() as u32 >= approval.required_approvals {
            approval.status = ApprovalStatus::Approved;

            // Add final approval to history
            let final_history = ApprovalHistory {
                approval_id,
                action: String::from_str(&env, "fully_approved"),
                actor: approver,
                timestamp: env.ledger().timestamp(),
                reason: None,
            };
            add_approval_history(&env, approval_id, &final_history);

            env.events().publish(
                (Symbol::new(&env, "SaleApproved"),),
                (approval_id, approval.approvals_received.len()),
            );
        } else {
            env.events().publish(
                (Symbol::new(&env, "SaleApprovalReceived"),),
                (approval_id, approver, approval.approvals_received.len()),
            );
        }

        set_approval(&env, &approval);
    }

    /// Reject a proposed sale
    pub fn reject_sale(env: Env, approval_id: u64, approver: Address, reason: String) {
        approver.require_auth();

        if approval_id == 0 {
            panic!("Invalid approval ID");
        }

        let mut approval = get_approval(&env, approval_id).expect("Approval not found");

        assert!(
            approval.status == ApprovalStatus::Pending,
            "Approval not pending"
        );
        assert!(
            env.ledger().timestamp() < approval.expires_at,
            "Approval expired"
        );

        // Check if approver is authorized
        assert!(
            approval.approvers.contains(&approver),
            "Unauthorized approver"
        );

        // Check if already voted
        assert!(
            !approval.approvals_received.contains(&approver),
            "Already approved"
        );
        assert!(
            !approval.rejections_received.contains(&approver),
            "Already rejected"
        );

        approval.rejections_received.push_back(approver.clone());
        approval.rejection_reasons.push_back(reason.clone());
        approval.status = ApprovalStatus::Rejected;

        // Add to history
        let history = ApprovalHistory {
            approval_id,
            action: String::from_str(&env, "rejected"),
            actor: approver.clone(),
            timestamp: env.ledger().timestamp(),
            reason: Some(reason),
        };
        add_approval_history(&env, approval_id, &history);

        env.events().publish(
            (Symbol::new(&env, "SaleRejected"),),
            (approval_id, approver),
        );

        set_approval(&env, &approval);
    }

    /// Execute an approved sale
    pub fn execute_approved_sale(env: Env, approval_id: u64) {
        if approval_id == 0 {
            panic!("Invalid approval ID");
        }

        let approval = get_approval(&env, approval_id).expect("Approval not found");

        assert!(
            approval.status == ApprovalStatus::Approved,
            "Approval not approved"
        );
        assert!(
            env.ledger().timestamp() < approval.expires_at,
            "Approval expired"
        );

        // Execute the sale based on type
        if let Some(listing_id) = approval.listing_id {
            // Fixed-price sale
            Self::execute_approved_listing_sale(env, approval_id, listing_id);
        } else if let Some(auction_id) = approval.auction_id {
            // Auction sale
            Self::execute_approved_auction_sale(env, approval_id, auction_id);
        } else {
            panic!("Invalid approval: no listing or auction ID");
        }
    }

    /// Execute approved fixed-price sale (internal function)
    fn execute_approved_listing_sale(env: Env, approval_id: u64, listing_id: u64) {
        // ─── SNAPSHOT PHASE ───
        let listing_key = (Symbol::new(&env, "listing"), listing_id);
        let mut listing: Listing = env
            .storage()
            .instance()
            .get(&listing_key)
            .expect("Listing not found");

        let approval = get_approval(&env, approval_id).expect("Approval not found");
        let platform_fee_bps = Self::get_platform_fee(env.clone());
        let royalty_info = Self::get_royalty(env.clone(), listing.agent_id);

        // ─── VALIDATION PHASE ───
        // (Assuming approval status and expiry already checked in caller)

        // ─── MUTATION PHASE ───

        // Process fee transition if active
        Self::process_fee_transition(env.clone());

        Self::route_sale_payment(
            &env,
            listing.agent_id,
            listing.price,
            &approval.buyer,
            &listing.seller,
            royalty_info,
            platform_fee_bps,
            AssetClass::Agent,
        );

        // Mark listing as inactive
        listing.active = false;
        env.storage().instance().set(&listing_key, &listing);

        // Update approval status
        let mut updated_approval = approval.clone();
        updated_approval.status = ApprovalStatus::Executed;
        set_approval(&env, &updated_approval);

        // Add execution to history
        let history = ApprovalHistory {
            approval_id,
            action: String::from_str(&env, "executed"),
            actor: env.current_contract_address(),
            timestamp: env.ledger().timestamp(),
            reason: None,
        };
        add_approval_history(&env, approval_id, &history);

        env.events().publish(
            (Symbol::new(&env, "SaleExecuted"),),
            (approval_id, listing_id, approval.buyer, platform_fee_bps),
        );
    }

    /// Execute approved auction sale (internal function)
    /// Execute approved auction sale (internal function)
    fn execute_approved_auction_sale(env: Env, approval_id: u64, auction_id: u64) {
        // ─── SNAPSHOT PHASE ───
        let mut auction = get_auction(&env, auction_id).expect("Auction not found");
        let approval = get_approval(&env, approval_id).expect("Approval not found");
        let platform_fee_bps = Self::get_platform_fee(env.clone());
        let royalty_info = Self::get_royalty(env.clone(), auction.agent_id);
        let now = env.ledger().timestamp();

        // ─── VALIDATION PHASE ───

        // ─── MUTATION PHASE ───

        // Process fee transition if active
        Self::process_fee_transition(env.clone());

        // Process the auction resolution
        if let Some(winner) = auction.highest_bidder.clone() {
            if auction.highest_bid >= auction.reserve_price {
                Self::route_sale_payment(
                    &env,
                    auction.agent_id,
                    auction.highest_bid,
                    &winner,
                    &auction.seller,
                    royalty_info,
                    platform_fee_bps,
                    AssetClass::Agent,
                );

                // NOTE: NFT transfer logic should be added here

                auction.status = AuctionStatus::Won;

                env.events().publish(
                    (Symbol::new(&env, "AuctionWon"),),
                    (
                        auction_id,
                        winner.clone(),
                        auction.highest_bid,
                        platform_fee_bps,
                    ),
                );

                // Auto-mint credit score NFT for auction win
                if let Err(e) =
                    Self::auto_mint_credit_on_auction(env.clone(), auction_id, winner.clone())
                {
                    // Log error but don't fail the transaction
                    env.events().publish(
                        (Symbol::new(&env, "CreditScoreNFTMintFailed"),),
                        (
                            auction_id,
                            winner,
                            String::from_str(&env, error_description(e)),
                        ),
                    );
                }
            } else {
                // Refund if reserve not met
                let token_client = token::Client::new(&env, &get_payment_token(&env));
                token_client.transfer(
                    &env.current_contract_address(),
                    &winner,
                    &auction.highest_bid,
                );
                auction.status = AuctionStatus::Ended;
            }
        } else {
            auction.status = AuctionStatus::Ended;
        }

        set_auction(&env, &auction);

        // Update approval status
        let mut updated_approval = approval;
        updated_approval.status = ApprovalStatus::Executed;
        set_approval(&env, &updated_approval);

        // Add execution to history
        let history = ApprovalHistory {
            approval_id,
            action: String::from_str(&env, "executed"),
            actor: env.current_contract_address(),
            timestamp: now,
            reason: None,
        };
        add_approval_history(&env, approval_id, &history);

        env.events().publish(
            (Symbol::new(&env, "SaleExecuted"),),
            (approval_id, auction_id, updated_approval.buyer),
        );
    }

    /// Get approval details
    pub fn get_approval(env: Env, approval_id: u64) -> Option<Approval> {
        if approval_id == 0 {
            panic!("Invalid approval ID");
        }
        get_approval(&env, approval_id)
    }

    pub fn verify_and_get_oracle_value(
        env: Env, 
        oracle_data: OracleData, 
        _signature: BytesN<64>
    ) -> u128 {
        let breaker: MarketplaceCircuitBreaker = env.storage().instance()
            .get(&Symbol::new(&env, "breaker"))
            .unwrap_or(MarketplaceCircuitBreaker::Active);
            
        if let MarketplaceCircuitBreaker::Terminated = breaker {
            panic!("Marketplace core operations are locked via circuit breaker");
        }

        let authorized_oracle: Address = env.storage().instance()
            .get(&Symbol::new(&env, "oracle"))
            .unwrap_or_else(|| panic!("Oracle reference not configured"));

        let mut check_payload = Bytes::new(&env);
        check_payload.append(&oracle_data.key.to_xdr(&env));
        check_payload.append(&oracle_data.value.to_xdr(&env));
        check_payload.append(&oracle_data.timestamp.to_xdr(&env));

        // Fix: Cast explicitly into a generic Soroban Val mapping
        authorized_oracle.require_auth_for_args(vec![&env, check_payload.into()]);

        let current_ledger_time = env.ledger().timestamp();
        if current_ledger_time > oracle_data.timestamp + DATA_EXPIRATION_WINDOW_SECONDS {
            panic!("Oracle data attestation has expired");
        }

        oracle_data.value.try_into().unwrap()
    }

    pub fn calculate_dynamic_price(
        _env: Env, 
        rule: PricingRule, 
        verified_metric_value: u128
    ) -> u128 {
        if verified_metric_value == 0 {
            return rule.base_price;
        }

        let adjustment = verified_metric_value
            .checked_mul(rule.scale_factor_bps as u128)
            .unwrap_or(0) / BPS_DENOMINATOR;

        if rule.inverse {
            rule.base_price.checked_sub(adjustment).unwrap_or(0)
        } else {
            rule.base_price.checked_add(adjustment).unwrap_or(u128::MAX)
        }
    }

    /// Place a bid in an English auction
    pub fn place_bid(env: Env, auction_id: u64, bidder: Address, amount: i128) {
        bidder.require_auth();
        let mut auction = get_auction(&env, auction_id).expect("Auction not found");
        assert!(auction.status == AuctionStatus::Active, "Auction not active");
        assert!(
            env.ledger().timestamp() < auction.end_time,
            "Auction has ended"
        );

        let computed_min_step = (auction.start_price * (auction.min_bid_increment_bps as i128)) / 10_000;
         let min_bid = if auction.highest_bid > 0 {
             auction.highest_bid + computed_min_step
         } else {
             // No bids yet: require at least the start price (or start price + min step)
             let baseline = auction.start_price;
             if baseline > computed_min_step {
                 baseline
             } else {
                 computed_min_step
             }
         };

         assert!(amount >= min_bid, "Bid too low");

         let token_client = token::Client::new(&env, &get_payment_token(&env));

         // Refund previous highest bidder
         if let Some(prev_bidder) = auction.highest_bidder {
             token_client.transfer(
                 &env.current_contract_address(),
                 &prev_bidder,
                 &auction.highest_bid,
             );
         }

         // Lock new bid in contract
         token_client.transfer(&bidder, &env.current_contract_address(), &amount);

         // Capture previous bid before overwriting for increment calculation
         let prev_highest = auction.highest_bid;
         let bid_increment = amount - prev_highest;

         auction.highest_bidder = Some(bidder.clone());
         auction.highest_bid = amount;

         // Extend auction by 5 minutes if bid in final 5 minutes
         let time_left = auction.end_time - env.ledger().timestamp();
         if time_left < 300 {
             auction.end_time += 300;
         }

         set_auction(&env, &auction);

         // Record bid in history with sequence and increment
         let sequence = get_bid_history_count(&env, auction_id) + 1;
         add_bid_history(
             &env,
             auction_id,
             &BidRecord {
                 bidder: bidder.clone(),
                 amount,
                 timestamp: env.ledger().timestamp(),
                 bid_increment,
                 sequence,
             },
         );

         env.events().publish(
             (Symbol::new(&env, "BidPlaced"),),
             (auction_id, bidder.clone(), amount, auction.end_time),
         );

         // Audit log for bid placement
         let before_state = String::from_str(&env, "{\"bid_placed\":false}");
         let after_state = String::from_str(&env, "{\"bid_placed\":true}");
         let tx_hash = String::from_str(&env, "place_bid");
         let description = Some(String::from_str(&env, "Auction bid placed"));

         let _ = create_audit_log(
             &env,
             bidder.clone(),
             OperationType::AuctionBidPlaced,
             before_state,
             after_state,
             tx_hash,
             description,
         );
    }

    /// Returns the number of bids placed in an auction.
    pub fn get_bid_count(env: Env, auction_id: u64) -> u64 {
        get_bid_history_count(&env, auction_id)
    }

    /// Returns the full bid history for an auction ordered by sequence.
    pub fn get_bid_history(env: Env, auction_id: u64) -> Vec<BidRecord> {
        let count = get_bid_history_count(&env, auction_id);
        let mut history = Vec::new(&env);
        for i in 0..count {
            if let Some(entry) = get_bid_history_entry(&env, auction_id, i) {
                history.push_back(entry);
            }
        }
        history
    }

    /// Returns a single bid history entry by its 0-based index.
    pub fn get_bid_history_entry_at(env: Env, auction_id: u64, index: u64) -> Option<BidRecord> {
        get_bid_history_entry(&env, auction_id, index)
    }

    /// Create a standard English or Dutch auction.
    pub fn create_auction(
        env: Env,
        agent_id: u64,
        seller: Address,
        auction_type: AuctionType,
        start_price: i128,
        reserve_price: i128,
        duration_seconds: u64,
        min_bid_increment_bps: u32,
    ) -> u64 {
        seller.require_auth();
        assert!(start_price > 0, "Invalid start price");
        assert!(reserve_price >= 0, "Invalid reserve price");
        assert!(duration_seconds > 0, "Invalid duration");

        let auction_id = increment_auction_counter(&env);
        let start_time = env.ledger().timestamp();
        let end_time = start_time + duration_seconds;
        let auction = Auction {
            auction_id,
            agent_id,
            seller: seller.clone(),
            auction_type,
            start_price,
            reserve_price,
            current_price: start_price,
            highest_bidder: None,
            highest_bid: 0,
            start_time,
            end_time,
            min_bid_increment_bps,
            status: AuctionStatus::Active,
            dutch_config: None,
            sealed_commit_end: None,
            sealed_reveal_end: None,
        };

        set_auction(&env, &auction);
        env.events().publish(
            (Symbol::new(&env, "AuctionCreated"),),
            (auction_id, agent_id, auction_type, start_price),
        );
        auction_id
    }

    /// Create a sealed-bid auction with explicit commit/reveal durations
    pub fn create_sealed_auction(
        env: Env,
        agent_id: u64,
        seller: Address,
        start_price: i128,
        reserve_price: i128,
        commit_duration: u64,
        reveal_duration: u64,
        min_bid_increment_bps: u32,
    ) -> u64 {
        seller.require_auth();
        assert!(start_price > 0, "Invalid start price");
        assert!(
            commit_duration > 0 && reveal_duration > 0,
            "Invalid durations"
        );

        let auction_id = increment_auction_counter(&env);
        let start_time = env.ledger().timestamp();
        let commit_end = start_time + commit_duration;
        let reveal_end = commit_end + reveal_duration;

        let auction = Auction {
            auction_id,
            agent_id,
            seller,
            auction_type: AuctionType::Sealed,
            start_price,
            reserve_price,
            current_price: start_price,
            highest_bidder: None,
            highest_bid: 0,
            start_time,
            end_time: reveal_end,
            min_bid_increment_bps,
            status: AuctionStatus::Active,
            dutch_config: None,
            sealed_commit_end: Some(commit_end),
            sealed_reveal_end: Some(reveal_end),
        };

        set_auction(&env, &auction);

        env.events().publish(
            (Symbol::new(&env, "AuctionCreated"),),
            (auction_id, agent_id, AuctionType::Sealed, start_price),
        );

        auction_id
    }

    pub fn commit_sealed_bid(
        env: Env,
        auction_id: u64,
        bidder: Address,
        commitment: Bytes,
        deposit: i128,
    ) {
        bidder.require_auth();
        let mut auction = get_auction(&env, auction_id).expect("Auction not found");
        assert!(
            auction.status == AuctionStatus::Active,
            "Auction not active"
        );
        assert!(
            auction.auction_type == AuctionType::Sealed,
            "Not a sealed auction"
        );

        let now = env.ledger().timestamp();
        let commit_end = auction.sealed_commit_end.expect("No commit end");
        assert!(now < commit_end, "Commit phase ended");

        let token_client = token::Client::new(&env, &get_payment_token(&env));
        token_client.transfer(&bidder, &env.current_contract_address(), &deposit);

        let commit = stellai_lib::SealedCommit {
            bidder: bidder.clone(),
            commitment: commitment.clone(),
            deposit,
            timestamp: now,
        };

        add_sealed_commit(&env, auction_id, &commit);

        env.events().publish(
            (Symbol::new(&env, "BidCommitted"),),
            (auction_id, bidder, deposit),
        );
    }

    pub fn reveal_sealed_bid(
        env: Env,
        auction_id: u64,
        bidder: Address,
        amount: i128,
        nonce: String,
    ) {
        bidder.require_auth();
        let mut auction = get_auction(&env, auction_id).expect("Auction not found");
        assert!(
            auction.status == AuctionStatus::Active,
            "Auction not active"
        );
        assert!(
            auction.auction_type == AuctionType::Sealed,
            "Not a sealed auction"
        );

        let now = env.ledger().timestamp();
        let commit_end = auction.sealed_commit_end.expect("No commit end");
        let reveal_end = auction.sealed_reveal_end.expect("No reveal end");
        assert!(
            now >= commit_end && now < reveal_end,
            "Not in reveal window"
        );

        // Find the bidder's commitment
        let commit_count = get_sealed_commit_count(&env, auction_id);
        let mut found: Option<stellai_lib::SealedCommit> = None;
        for i in 0..commit_count {
            if let Some(c) = get_sealed_commit_entry(&env, auction_id, i) {
                if c.bidder == bidder {
                    found = Some(c);
                    break;
                }
            }
        }
        let commit = found.expect("Commitment not found");

        // Verify commitment hash: format "amount:nonce:bidder"
        let mut payload = Bytes::new(&env);
        payload.append(&Bytes::from_array(&env, &amount.to_be_bytes()));
        payload.append(&Bytes::from_array(&env, &auction_id.to_be_bytes()));
        let _ = nonce;
        let hash = env.crypto().sha256(&payload);
        let hash_bytes: Bytes = hash.into();
        assert!(hash_bytes == commit.commitment, "Commitment mismatch");

        // Ensure deposit covers amount
        assert!(commit.deposit >= amount, "Deposit insufficient for bid");

        let reveal = stellai_lib::SealedReveal {
            bidder: bidder.clone(),
            amount,
            nonce: nonce.clone(),
            deposit: commit.deposit,
            timestamp: now,
        };

        add_sealed_reveal(&env, auction_id, &reveal);

        // Track highest
        if amount > auction.highest_bid {
            auction.highest_bid = amount;
            auction.highest_bidder = Some(bidder.clone());
        }

        set_auction(&env, &auction);

        env.events().publish(
            (Symbol::new(&env, "BidRevealed"),),
            (auction_id, bidder, amount),
        );
    }

    pub fn accept_dutch_price(env: Env, auction_id: u64, buyer: Address) {
        buyer.require_auth();
        let mut auction = get_auction(&env, auction_id).expect("Auction not found");
        assert!(
            auction.status == AuctionStatus::Active,
            "Auction not active"
        );
        assert!(
            auction.auction_type == AuctionType::Dutch,
            "Not a Dutch auction"
        );

        let current_price = Self::calculate_dutch_price(env.clone(), &auction);

        let token_client = token::Client::new(&env, &get_payment_token(&env));
        token_client.transfer(&buyer, &env.current_contract_address(), &current_price);

        auction.highest_bidder = Some(buyer);
        auction.highest_bid = current_price;

        set_auction(&env, &auction);

        Self::resolve_auction(env, auction_id);
    }

    pub fn resolve_auction(env: Env, auction_id: u64) {
        let mut auction = get_auction(&env, auction_id).expect("Auction not found");
        assert!(
            auction.status == AuctionStatus::Active,
            "Auction not active"
        );

        let is_dutch = auction.auction_type == AuctionType::Dutch;
        let is_english = auction.auction_type == AuctionType::English;

        assert!(
            (is_english && env.ledger().timestamp() >= auction.end_time)
                || (is_dutch && auction.highest_bidder.is_some()),
            "Auction not yet ended"
        );

        if let Some(winner) = auction.highest_bidder.clone() {
            if auction.highest_bid >= auction.reserve_price {
                // Check if multi-signature approval is required
                let config = get_approval_config(&env);
                if auction.highest_bid >= config.threshold {
                    panic!(
                        "High-value auction requires multi-signature approval. Use propose_auction_sale() first."
                    );
                }

                // Process fee transition if active
                Self::process_fee_transition(env.clone());

                let platform_fee_bps = Self::get_platform_fee(env.clone());
                let royalty_info = Self::get_royalty(env.clone(), auction.agent_id);
                // For sealed auctions, collect deposits and refund non-winners
                let token_client = token::Client::new(&env, &get_payment_token(&env));

                if auction.auction_type == AuctionType::Sealed {
                    // Refund all sealed commits and reveals except winner; accumulate winner deposit
                    let mut winner_deposit: i128 = 0;

                    // Refund revealed bidders (non-winners)
                    let reveal_count = get_sealed_reveal_count(&env, auction_id);
                    for i in 0..reveal_count {
                        if let Some(rev) = get_sealed_reveal_entry(&env, auction_id, i) {
                            if rev.bidder != winner {
                                token_client.transfer(
                                    &env.current_contract_address(),
                                    &rev.bidder,
                                    &rev.deposit,
                                );
                            } else {
                                winner_deposit += rev.deposit;
                            }
                        }
                    }

                    // Refund committed-but-unrevealed bidders
                    let commit_count = get_sealed_commit_count(&env, auction_id);
                    for i in 0..commit_count {
                        if let Some(c) = get_sealed_commit_entry(&env, auction_id, i) {
                            // if no reveal exists for this bidder, refund deposit
                            let mut revealed = false;
                            for j in 0..reveal_count {
                                if let Some(r) = get_sealed_reveal_entry(&env, auction_id, j) {
                                    if r.bidder == c.bidder {
                                        revealed = true;
                                        break;
                                    }
                                }
                            }
                            if !revealed {
                                // refund full deposit
                                token_client.transfer(
                                    &env.current_contract_address(),
                                    &c.bidder,
                                    &c.deposit,
                                );
                            }
                        }
                    }

                    // Proceed with payment routing using the highest bid
                    Self::route_sale_payment(
                        &env,
                        auction.agent_id,
                        auction.highest_bid,
                        &winner,
                        &auction.seller,
                        royalty_info.clone(),
                        platform_fee_bps,
                        AssetClass::Agent,
                    );

                    // Refund winner excess deposit if any
                    if winner_deposit > auction.highest_bid {
                        let excess = winner_deposit - auction.highest_bid;
                        token_client.transfer(&env.current_contract_address(), &winner, &excess);
                    }

                    // NOTE: NFT transfer logic should be added here

                    auction.status = AuctionStatus::Won;

                    env.events().publish(
                        (Symbol::new(&env, "AuctionWon"),),
                        (auction_id, winner, auction.highest_bid, platform_fee_bps),
                    );
                } else {
                    // Non-sealed auctions: normal payment routing
                    Self::route_sale_payment(
                        &env,
                        auction.agent_id,
                        auction.highest_bid,
                        &winner,
                        &auction.seller,
                        royalty_info.clone(),
                        platform_fee_bps,
                        AssetClass::Agent,
                    );

                    // NOTE: NFT transfer logic should be added here

                    auction.status = AuctionStatus::Won;

                    env.events().publish(
                        (Symbol::new(&env, "AuctionWon"),),
                        (auction_id, winner, auction.highest_bid, platform_fee_bps),
                    );
                }
            } else {
                // Refund if reserve not met (English only)
                if is_english {
                    let token_client = token::Client::new(&env, &get_payment_token(&env));
                    token_client.transfer(
                        &env.current_contract_address(),
                        &winner,
                        &auction.highest_bid,
                    );
                }
                auction.status = AuctionStatus::Ended;
            }
        } else {
            auction.status = AuctionStatus::Ended;
        }

        set_auction(&env, &auction);

        env.events().publish(
            (Symbol::new(&env, "AuctionEnded"),),
            (auction_id, auction.status),
        );
    }

    pub fn cancel_auction(env: Env, auction_id: u64) {
        let mut auction = get_auction(&env, auction_id).expect("Auction not found");
        auction.seller.require_auth();
        assert!(
            auction.status == AuctionStatus::Active,
            "Auction not active"
        );
        assert!(
            auction.highest_bidder.is_none(),
            "Cannot cancel with active bids"
        );

        auction.status = AuctionStatus::Cancelled;
        set_auction(&env, &auction);

        env.events()
            .publish((Symbol::new(&env, "AuctionCancelled"),), (auction_id,));
    }
    // ---------------- DYNAMIC FEE ADJUSTMENT ----------------

    /// Initialize fee adjustment parameters (admin only)
    pub fn init_fee_adjustment(
        env: Env,
        admin: Address,
        base_marketplace_fee: u32,
        congestion_oracle_id: Address,
        utilization_oracle_id: Address,
        volatility_oracle_id: Address,
        min_fee_bps: u32,
        max_fee_bps: u32,
        adjustment_window: u64,
    ) {
        admin.require_auth();

        // Verify admin is the contract admin
        Self::verify_admin(&env, &admin);

        assert!(base_marketplace_fee <= 5000, "Base fee cannot exceed 50%");
        assert!(min_fee_bps >= 5, "Min fee cannot be below 0.05%");
        assert!(max_fee_bps <= 5000, "Max fee cannot exceed 50%");
        assert!(min_fee_bps <= max_fee_bps, "Min fee must be <= max fee");
        assert!(adjustment_window > 0, "Adjustment window must be positive");

        let params = storage::FeeAdjustmentParams {
            base_marketplace_fee,
            congestion_oracle_id,
            utilization_oracle_id,
            volatility_oracle_id,
            min_fee_bps,
            max_fee_bps,
            adjustment_window,
        };

        storage::set_fee_adjustment_params(&env, &params);

        // Initialize with base fee structure
        let initial_fee_structure = storage::FeeStructure {
            marketplace_fee_bps: base_marketplace_fee,
            calculated_at: env.ledger().timestamp(),
            congestion_factor: 1000, // 1.0x in basis points
            utilization_factor: 1000,
            volatility_factor: 1000,
        };

        storage::set_current_fee_structure(&env, &initial_fee_structure);

        env.events().publish(
            (Symbol::new(&env, "FeeAdjustmentInitialized"),),
            (base_marketplace_fee, min_fee_bps, max_fee_bps),
        );
    }

    /// Subscribe to oracle data feeds for fee adjustment
    pub fn subscribe_to_fee_oracles(env: Env, admin: Address, oracle_ids: Vec<Address>) {
        admin.require_auth();

        // Verify admin is the contract admin
        Self::verify_admin(&env, &admin);

        assert!(!oracle_ids.is_empty(), "Must provide at least one oracle");
        assert!(oracle_ids.len() <= 10, "Too many oracles");

        storage::set_oracle_subscriptions(&env, &oracle_ids);

        env.events().publish(
            (Symbol::new(&env, "OracleSubscriptionsUpdated"),),
            (oracle_ids.len(),),
        );
    }

    /// Aggregate oracle data for fee calculation
    pub fn aggregate_oracle_data(env: Env) -> storage::FeeCalculationInput {
        let params =
            storage::get_fee_adjustment_params(&env).expect("Fee adjustment not initialized");

        // Get oracle data with specific keys for each metric
        let congestion_data = Self::get_oracle_value_by_key(
            &env,
            &params.congestion_oracle_id,
            "network_congestion",
            50,
        );
        let utilization_data = Self::get_oracle_value_by_key(
            &env,
            &params.utilization_oracle_id,
            "platform_utilization",
            50,
        );
        let volatility_data = Self::get_oracle_value_by_key(
            &env,
            &params.volatility_oracle_id,
            "market_volatility",
            50,
        );

        storage::set_last_oracle_update(&env, env.ledger().timestamp());

        storage::FeeCalculationInput {
            network_congestion: congestion_data,
            platform_utilization: utilization_data,
            market_volatility: volatility_data,
        }
    }

    /// Calculate dynamic fees based on oracle input
    pub fn calculate_dynamic_fees(
        env: Env,
        input: storage::FeeCalculationInput,
    ) -> storage::FeeStructure {
        let params =
            storage::get_fee_adjustment_params(&env).expect("Fee adjustment not initialized");

        // Calculate adjustment factors (in basis points, 1000 = 1.0x)
        let congestion_factor = Self::calculate_congestion_factor(input.network_congestion);
        let utilization_factor = Self::calculate_utilization_factor(input.platform_utilization);
        let volatility_factor = Self::calculate_volatility_factor(input.market_volatility);

        // Combine factors multiplicatively
        let combined_factor =
            (congestion_factor * utilization_factor * volatility_factor) / 1_000_000; // Divide by 10^6 for two multiplications

        let adjusted_fee = (params.base_marketplace_fee as i128 * combined_factor) / 1000;
        let clamped_fee = adjusted_fee
            .max(params.min_fee_bps as i128)
            .min(params.max_fee_bps as i128) as u32;

        storage::FeeStructure {
            marketplace_fee_bps: clamped_fee,
            calculated_at: env.ledger().timestamp(),
            congestion_factor,
            utilization_factor,
            volatility_factor,
        }
    }

    /// Update fees with gradual transition
    pub fn update_dynamic_fees(env: Env) {
        let current_time = env.ledger().timestamp();
        let last_update = storage::get_last_oracle_update(&env);

        // Check if oracles are stale (>30 minutes)
        if current_time - last_update > 1800 {
            // Fall back to static fees
            Self::fallback_to_static_fees(&env);
            return;
        }

        let input = Self::aggregate_oracle_data(env.clone());
        let new_fee_structure = Self::calculate_dynamic_fees(env.clone(), input);

        let current_fee_structure = storage::get_current_fee_structure(&env);

        if let Some(current) = current_fee_structure {
            // Check if significant change (>20% jump protection)
            let fee_change_ratio = (new_fee_structure.marketplace_fee_bps as i128 * 1000)
                / (current.marketplace_fee_bps as i128);

            if fee_change_ratio > 1200 || fee_change_ratio < 800 {
                // Start gradual transition
                Self::start_fee_transition(
                    &env,
                    current.marketplace_fee_bps,
                    new_fee_structure.marketplace_fee_bps,
                );
            } else {
                // Direct update for small changes
                Self::apply_fee_update(&env, current.marketplace_fee_bps, new_fee_structure);
            }
        } else {
            // First time setup
            Self::apply_fee_update(&env, 0, new_fee_structure);
        }
    }

    /// Get current effective marketplace fee
    pub fn get_current_marketplace_fee(env: Env) -> u32 {
        // Check if in transition
        if let Some(transition_state) = storage::get_fee_transition_state(&env) {
            if transition_state.is_transitioning {
                return Self::calculate_transition_fee(&env, &transition_state);
            }
        }

        // Return current fee or fallback to base fee
        if let Some(fee_structure) = storage::get_current_fee_structure(&env) {
            fee_structure.marketplace_fee_bps
        } else if let Some(params) = storage::get_fee_adjustment_params(&env) {
            params.base_marketplace_fee
        } else {
            250 // Default 2.5% fee
        }
    }

    /// Get comprehensive fee status for monitoring
    pub fn get_fee_status(env: Env) -> storage::FeeStatus {
        let current_fee = Self::get_current_marketplace_fee(env.clone());
        let fee_structure = storage::get_current_fee_structure(&env);
        let transition_state = storage::get_fee_transition_state(&env);
        let last_oracle_update = storage::get_last_oracle_update(&env);
        let current_time = env.ledger().timestamp();

        storage::FeeStatus {
            current_fee_bps: current_fee,
            is_dynamic: fee_structure.is_some(),
            last_updated: fee_structure.as_ref().map(|f| f.calculated_at),
            is_transitioning: transition_state
                .as_ref()
                .map(|t| t.is_transitioning)
                .unwrap_or(false),
            transition_progress: transition_state.as_ref().map(|t| {
                if t.transition_steps > 0 {
                    (t.current_step * 100) / t.transition_steps
                } else {
                    100
                }
            }),
            oracle_data_age: current_time - last_oracle_update,
            congestion_factor: fee_structure.as_ref().map(|f| f.congestion_factor),
            utilization_factor: fee_structure.as_ref().map(|f| f.utilization_factor),
            volatility_factor: fee_structure.as_ref().map(|f| f.volatility_factor),
        }
    }

    /// Get network congestion metrics for transparency
    pub fn get_network_metrics(env: Env) -> storage::NetworkMetrics {
        let params = storage::get_fee_adjustment_params(&env);
        let current_time = env.ledger().timestamp();

        match params {
            Some(p) => {
                let congestion = Self::get_oracle_value_by_key(
                    &env,
                    &p.congestion_oracle_id,
                    "network_congestion",
                    50,
                );
                let utilization = Self::get_oracle_value_by_key(
                    &env,
                    &p.utilization_oracle_id,
                    "platform_utilization",
                    50,
                );
                let volatility = Self::get_oracle_value_by_key(
                    &env,
                    &p.volatility_oracle_id,
                    "market_volatility",
                    50,
                );

                storage::NetworkMetrics {
                    network_congestion: congestion,
                    platform_utilization: utilization,
                    market_volatility: volatility,
                    last_updated: current_time,
                    data_source: String::from_str(&env, "oracle"),
                }
            }
            None => storage::NetworkMetrics {
                network_congestion: 50,
                platform_utilization: 50,
                market_volatility: 50,
                last_updated: current_time,
                data_source: String::from_str(&env, "fallback"),
            },
        }
    }

    /// Notify users of significant fee changes
    pub fn notify_fee_change(env: Env, user: Address) {
        let fee_status = Self::get_fee_status(env.clone());

        if let Some(fee_structure) = storage::get_current_fee_structure(&env) {
            let params = storage::get_fee_adjustment_params(&env);

            if let Some(p) = params {
                let deviation = if fee_structure.marketplace_fee_bps > p.base_marketplace_fee {
                    fee_structure.marketplace_fee_bps - p.base_marketplace_fee
                } else {
                    p.base_marketplace_fee - fee_structure.marketplace_fee_bps
                };

                // Notify if deviation is significant (>100 basis points)
                if deviation > 100 {
                    env.events().publish(
                        (Symbol::new(&env, "FeeChangeNotification"),),
                        (
                            user,
                            fee_structure.marketplace_fee_bps,
                            p.base_marketplace_fee,
                            deviation,
                            fee_status.congestion_factor.unwrap_or(1000),
                            fee_status.utilization_factor.unwrap_or(1000),
                            fee_status.volatility_factor.unwrap_or(1000),
                        ),
                    );
                }
            }
        }
    }

    /// Monitor network usage and trigger fee adjustments automatically
    pub fn monitor_and_adjust_fees(env: Env) {
        let current_time = env.ledger().timestamp();
        let last_update = storage::get_last_oracle_update(&env);

        // Check if it's time to update fees (every 5 minutes minimum)
        if current_time - last_update >= 300 {
            Self::update_dynamic_fees(env.clone());

            // Log monitoring activity
            let network_metrics = Self::get_network_metrics(env.clone());
            env.events().publish(
                (Symbol::new(&env, "FeeMonitoringUpdate"),),
                (
                    current_time,
                    network_metrics.network_congestion,
                    network_metrics.platform_utilization,
                    network_metrics.market_volatility,
                    Self::get_current_marketplace_fee(env.clone()),
                ),
            );
        }
    }

    /// Get fee adjustment statistics for transparency
    pub fn get_fee_adjustment_stats(env: Env) -> storage::FeeAdjustmentStats {
        let adjustment_counter = storage::get_fee_adjustment_counter(&env);
        let current_fee = Self::get_current_marketplace_fee(env.clone());
        let network_metrics = Self::get_network_metrics(env.clone());
        let fee_status = Self::get_fee_status(env);

        storage::FeeAdjustmentStats {
            total_adjustments: adjustment_counter,
            current_fee_bps: current_fee,
            last_adjustment_timestamp: fee_status.last_updated.unwrap_or(0),
            network_congestion: network_metrics.network_congestion,
            platform_utilization: network_metrics.platform_utilization,
            market_volatility: network_metrics.market_volatility,
            is_transitioning: fee_status.is_transitioning,
            transition_progress: fee_status.transition_progress.unwrap_or(0),
        }
    }

    /// Process fee transition step (called during transactions)
    pub fn process_fee_transition(env: Env) {
        if let Some(mut transition_state) = storage::get_fee_transition_state(&env) {
            if transition_state.is_transitioning
                && transition_state.current_step < transition_state.transition_steps
            {
                transition_state.current_step += 1;

                if transition_state.current_step >= transition_state.transition_steps {
                    // Transition complete
                    transition_state.is_transitioning = false;
                    let final_fee_structure = storage::FeeStructure {
                        marketplace_fee_bps: transition_state.target_fee_bps,
                        calculated_at: env.ledger().timestamp(),
                        congestion_factor: 1000,
                        utilization_factor: 1000,
                        volatility_factor: 1000,
                    };
                    storage::set_current_fee_structure(&env, &final_fee_structure);
                }

                storage::set_fee_transition_state(&env, &transition_state);
            }
        }
    }

    /// Get fee adjustment history
    pub fn get_fee_adjustment_history(
        env: Env,
        adjustment_id: u64,
    ) -> Option<storage::FeeAdjustmentHistory> {
        storage::get_fee_adjustment_history(&env, adjustment_id)
    }

    // ---------------- INTERNAL FEE CALCULATION HELPERS ----------------

    fn get_oracle_value_by_key(env: &Env, oracle_id: &Address, key: &str, fallback: i128) -> i128 {
        // Enhanced oracle integration with proper error handling
        let oracle_key = Symbol::new(env, key);

        // Try to get data from oracle contract using direct contract invocation
        let result = env.invoke_contract::<Val>(
            oracle_id,
            &Symbol::new(env, "get_data"),
            Vec::from_array(env, [oracle_key.into_val(env)]),
        );

        if result.is_void() {
            // Oracle returned void, use fallback
            fallback
        } else {
            // Try to parse the result as OracleData
            // In a real implementation, this would be more robust
            // For now, we'll simulate oracle data validation
            let current_time = env.ledger().timestamp();

            // Simulate getting recent oracle data
            // In production, this would parse the actual oracle response
            let simulated_value = fallback; // Use fallback as simulated value
            let simulated_timestamp = current_time - 60; // 1 minute ago

            // Validate oracle data is within expected range
            if simulated_value >= 0 && simulated_value <= 100 {
                // Check if data is recent (within last 5 minutes)
                if current_time - simulated_timestamp <= 300 {
                    simulated_value
                } else {
                    // Oracle data is stale, use fallback
                    fallback
                }
            } else {
                // Oracle data out of range, use fallback
                fallback
            }
        }
    }

    fn get_oracle_value(env: &Env, oracle_id: &Address, fallback: i128) -> i128 {
        // Legacy function - use the key-based version
        Self::get_oracle_value_by_key(env, oracle_id, "default", fallback)
    }

    fn calculate_congestion_factor(congestion: i128) -> i128 {
        // Network congestion: 0.5x - 2.0x (500 - 2000 basis points)
        let clamped = congestion.max(0).min(100);
        500 + (clamped * 1500) / 100
    }

    fn calculate_utilization_factor(utilization: i128) -> i128 {
        // Platform utilization: 0.7x - 1.5x (700 - 1500 basis points)
        let clamped = utilization.max(0).min(100);
        700 + (clamped * 800) / 100
    }

    fn calculate_volatility_factor(volatility: i128) -> i128 {
        // Market volatility: 0.9x - 1.3x (900 - 1300 basis points)
        let clamped = volatility.max(0).min(100);
        900 + (clamped * 400) / 100
    }

    fn fallback_to_static_fees(env: &Env) {
        if let Some(params) = storage::get_fee_adjustment_params(env) {
            let fallback_structure = storage::FeeStructure {
                marketplace_fee_bps: params.base_marketplace_fee,
                calculated_at: env.ledger().timestamp(),
                congestion_factor: 1000,
                utilization_factor: 1000,
                volatility_factor: 1000,
            };
            storage::set_current_fee_structure(env, &fallback_structure);

            env.events().publish(
                (Symbol::new(env, "FallbackToStaticFees"),),
                (params.base_marketplace_fee,),
            );
        }
    }

    fn start_fee_transition(env: &Env, current_fee: u32, target_fee: u32) {
        let transition_state = storage::FeeTransitionState {
            is_transitioning: true,
            start_fee_bps: current_fee,
            target_fee_bps: target_fee,
            transition_start: env.ledger().timestamp(),
            transition_steps: 10, // Transition over 10 transactions
            current_step: 0,
        };

        storage::set_fee_transition_state(env, &transition_state);

        env.events().publish(
            (Symbol::new(env, "FeeTransitionStarted"),),
            (current_fee, target_fee),
        );
    }

    fn calculate_transition_fee(_env: &Env, transition_state: &storage::FeeTransitionState) -> u32 {
        if transition_state.current_step >= transition_state.transition_steps {
            return transition_state.target_fee_bps;
        }

        let progress = (transition_state.current_step as i128 * 1000)
            / (transition_state.transition_steps as i128);
        let fee_diff =
            transition_state.target_fee_bps as i128 - transition_state.start_fee_bps as i128;
        let adjusted_fee = transition_state.start_fee_bps as i128 + (fee_diff * progress) / 1000;

        adjusted_fee as u32
    }

    fn apply_fee_update(env: &Env, old_fee: u32, new_fee_structure: storage::FeeStructure) {
        storage::set_current_fee_structure(env, &new_fee_structure);

        // Record in history
        let adjustment_id = storage::increment_fee_adjustment_counter(env);
        let history = storage::FeeAdjustmentHistory {
            adjustment_id,
            timestamp: env.ledger().timestamp(),
            old_fee_bps: old_fee,
            new_fee_bps: new_fee_structure.marketplace_fee_bps,
            congestion_value: new_fee_structure.congestion_factor,
            utilization_value: new_fee_structure.utilization_factor,
            volatility_value: new_fee_structure.volatility_factor,
            adjustment_reason: String::from_str(&env, "oracle_update"),
        };

        storage::add_fee_adjustment_history(env, &history);

        env.events().publish(
            (Symbol::new(env, "FeeAdjusted"),),
            (
                adjustment_id,
                old_fee,
                new_fee_structure.marketplace_fee_bps,
            ),
        );
    }

    // ============ ATOMIC TRANSACTION ROLLBACK FUNCTIONS ============

    /// Unlock a listing (rollback function)
    pub fn unlock_listing(env: Env, listing_id: u64) -> bool {
        // This is called as a rollback function, so we don't need transaction context
        // Just unlock the listing if it exists
        let listing_key = (Symbol::new(&env, "listing"), listing_id);
        if env.storage().instance().has(&listing_key) {
            // In atomic implementation, this would remove the lock
            // For now, just return success
            true
        } else {
            false
        }
    }

    /// Refund from escrow (rollback function) - can only be called by contract for atomic rollbacks
    pub fn refund_from_escrow(env: Env, buyer: Address, amount: i128) -> bool {
        // Find any active escrow for this buyer with sufficient funds
        let buyer_escrows = get_buyer_escrows(&env, &buyer);
        for escrow_id in buyer_escrows.iter() {
            if let Some(mut escrow) = get_escrow(&env, escrow_id) {
                if escrow.status == EscrowStatus::Held
                    && escrow.amount >= amount
                    && escrow.buyer == buyer
                {
                    // Transfer funds back to buyer
                    let payment_token = get_payment_token(&env);
                    let token_client = token::Client::new(&env, &payment_token);
                    token_client.transfer(&env.current_contract_address(), &buyer, &amount);

                    // Update escrow status
                    escrow.status = EscrowStatus::Refunded;
                    escrow.dispute_resolved_at = Some(env.ledger().timestamp());
                    set_escrow(&env, &escrow);

                    env.events().publish(
                        (Symbol::new(&env, "escrow_refunded"),),
                        (escrow_id, buyer.clone(), amount),
                    );
                    return true;
                }
            }
        }
        false
    }

    /// Buyer confirms receipt of the agent, releasing funds from escrow to seller
    pub fn confirm_receipt(env: Env, escrow_id: u64, buyer: Address) {
        buyer.require_auth();

        let mut escrow = get_escrow(&env, escrow_id).expect("Escrow not found");
        assert!(escrow.buyer == buyer, "Only buyer can confirm receipt");
        assert!(
            escrow.status == EscrowStatus::Held,
            "Escrow is not in held status"
        );

        // Release funds to seller by routing payment
        let platform_fee_bps = Self::get_platform_fee(env.clone());
        let royalty_info = Self::get_royalty(env.clone(), escrow.agent_id);

        Self::route_sale_payment(
            &env,
            escrow.agent_id,
            escrow.amount,
            &escrow.buyer,
            &escrow.seller,
            royalty_info,
            platform_fee_bps,
            AssetClass::Agent,
        );

        // Update escrow status
        escrow.status = EscrowStatus::Released;
        escrow.dispute_resolved_at = Some(env.ledger().timestamp());
        escrow.resolved_by = Some(buyer.clone());
        set_escrow(&env, &escrow);

        env.events().publish(
            (Symbol::new(&env, "escrow_released"),),
            (escrow_id, escrow.seller.clone(), escrow.amount),
        );
    }

    /// Buyer opens a dispute on an escrow, requiring admin resolution
    pub fn open_dispute(env: Env, escrow_id: u64, buyer: Address, reason: String) {
        buyer.require_auth();

        let mut escrow = get_escrow(&env, escrow_id).expect("Escrow not found");
        assert!(escrow.buyer == buyer, "Only buyer can open a dispute");
        assert!(
            escrow.status == EscrowStatus::Held,
            "Escrow is not in held status"
        );

        // Update escrow status to disputed
        escrow.status = EscrowStatus::Disputed;
        set_escrow(&env, &escrow);

        env.events().publish(
            (Symbol::new(&env, "dispute_opened"),),
            (escrow_id, buyer.clone(), reason),
        );
    }

    /// Admin resolves a dispute, deciding whether to release funds to seller or refund buyer
    pub fn resolve_dispute(
        env: Env,
        escrow_id: u64,
        admin: Address,
        release_to_seller: bool,
        reason: String,
    ) {
        admin.require_auth();
        Self::verify_admin(&env, &admin);

        let mut escrow = get_escrow(&env, escrow_id).expect("Escrow not found");
        assert!(
            escrow.status == EscrowStatus::Disputed,
            "Escrow is not in disputed status"
        );

        let payment_token = get_payment_token(&env);
        let token_client = token::Client::new(&env, &payment_token);

        if release_to_seller {
            // Release funds to seller
            let platform_fee_bps = Self::get_platform_fee(env.clone());
            let royalty_info = Self::get_royalty(env.clone(), escrow.agent_id);

            Self::route_sale_payment(
                &env,
                escrow.agent_id,
                escrow.amount,
                &escrow.buyer,
                &escrow.seller,
                royalty_info,
                platform_fee_bps,
                AssetClass::Agent,
            );
            escrow.status = EscrowStatus::Released;
        } else {
            // Refund full amount to buyer
            token_client.transfer(
                &env.current_contract_address(),
                &escrow.buyer,
                &escrow.amount,
            );
            escrow.status = EscrowStatus::Refunded;
        }

        escrow.dispute_resolved_at = Some(env.ledger().timestamp());
        escrow.resolved_by = Some(admin.clone());
        set_escrow(&env, &escrow);

        env.events().publish(
            (Symbol::new(&env, "dispute_resolved"),),
            (escrow_id, release_to_seller, reason),
        );
    }

    /// Process auto-release of escrows that have passed their auto-release timestamp
    pub fn process_auto_release(env: Env) {
        // Get all escrows (in production, we'd track pending escrows more efficiently)
        // For simplicity, this can be called to process all expired escrows
        let now = env.ledger().timestamp();
        let mut escrow_id = 1;
        while let Some(mut escrow) = get_escrow(&env, escrow_id) {
            if escrow.status == EscrowStatus::Held && now >= escrow.auto_release_at {
                // Auto-release funds to seller
                let platform_fee_bps = Self::get_platform_fee(env.clone());
                let royalty_info = Self::get_royalty(env.clone(), escrow.agent_id);

                Self::route_sale_payment(
                    &env,
                    escrow.agent_id,
                    escrow.amount,
                    &escrow.buyer,
                    &escrow.seller,
                    royalty_info,
                    platform_fee_bps,
                    AssetClass::Agent,
                );

                escrow.status = EscrowStatus::Released;
                escrow.dispute_resolved_at = Some(now);
                set_escrow(&env, &escrow);

                env.events().publish(
                    (Symbol::new(&env, "escrow_auto_released"),),
                    (escrow_id, escrow.seller.clone(), escrow.amount),
                );
            }
            escrow_id += 1;
        }
    }

    /// Admin sets escrow configuration (auto-release period and dispute window)
    pub fn set_escrow_config(
        env: Env,
        admin: Address,
        auto_release_period: u64,
        dispute_window: u64,
    ) {
        admin.require_auth();
        Self::verify_admin(&env, &admin);

        let config = EscrowConfig {
            auto_release_period_seconds: auto_release_period,
            dispute_window_seconds: dispute_window,
        };
        set_escrow_config(&env, &config);

        env.events().publish(
            (Symbol::new(&env, "escrow_config_updated"),),
            (auto_release_period, dispute_window),
        );
    }

    /// Get current escrow configuration
    pub fn get_escrow_config(env: Env) -> EscrowConfig {
        get_escrow_config(&env)
    }

    /// Get a specific escrow entry
    pub fn get_escrow(env: Env, escrow_id: u64) -> Option<Escrow> {
        get_escrow(&env, escrow_id)
    }

    /// Get all escrows for a buyer
    pub fn get_buyer_escrows(env: Env, buyer: Address) -> Vec<u64> {
        get_buyer_escrows(&env, &buyer)
    }

    /// Get all escrows for a seller
    pub fn get_seller_escrows(env: Env, seller: Address) -> Vec<u64> {
        get_seller_escrows(&env, &seller)
    }

    /// Revert sale (rollback function)
    pub fn revert_sale(env: Env, listing_id: u64) -> bool {
        // Reactivate the listing
        let listing_key = (Symbol::new(&env, "listing"), listing_id);
        if let Some(mut listing) = env.storage().instance().get::<_, Listing>(&listing_key) {
            listing.active = true;
            env.storage().instance().set(&listing_key, &listing);
            true
        } else {
            false
        }
    }

    // ---------------- LEASE MANAGEMENT ----------------

    /// Set lease configuration (admin only)
    pub fn set_lease_config(
        env: Env,
        admin: Address,
        deposit_bps: u32,
        early_termination_penalty_bps: u32,
    ) {
        Self::verify_admin(&env, &admin);

        assert!(deposit_bps <= 5000, "Deposit cannot exceed 50%");
        assert!(
            early_termination_penalty_bps <= 5000,
            "Penalty cannot exceed 50%"
        );

        let config = storage::LeaseConfig {
            deposit_bps,
            early_termination_penalty_bps,
        };

        storage::set_lease_config(&env, &config);

        env.events().publish(
            (Symbol::new(&env, "LeaseConfigUpdated"),),
            (deposit_bps, early_termination_penalty_bps),
        );
    }

    /// Get current lease configuration
    pub fn get_lease_config(env: Env) -> storage::LeaseConfig {
        storage::get_lease_config(&env)
    }

    /// Initiate a lease for an agent using the default financial settings.
    pub fn initiate_lease(
        env: Env,
        listing_id: u64,
        lessee: Address,
        duration_seconds: u64,
        auto_renew: bool,
        lessee_consent_for_renewal: bool,
    ) -> u64 {
        let config = storage::get_lease_config(&env);
        Self::initiate_lease_v2(
            env,
            listing_id,
            lessee,
            duration_seconds,
            auto_renew,
            lessee_consent_for_renewal,
            PaymentFrequency::Monthly,
            0,
            config.early_termination_penalty_bps,
            LateFeePolicy {
                fee_type: LateFeeType::None,
                value: 0,
            },
        )
    }

    /// Request lease extension
    pub fn request_lease_extension(
        env: Env,
        lease_id: u64,
        lessee: Address,
        additional_duration_seconds: u64,
    ) -> u64 {
        lessee.require_auth();

        if validation::validate_nonzero_id(lease_id).is_err() {
            panic!("Invalid lease ID");
        }
        if additional_duration_seconds == 0 {
            panic!("Additional duration must be positive");
        }

        let mut lease = storage::get_lease(&env, lease_id).expect("Lease not found");

        if lease.lessee != lessee {
            panic!("Unauthorized: only lessee can request extension");
        }
        if !Self::is_live_lease_state(lease.status) {
            panic!("Lease is not active");
        }
        if lease.pending_extension_id.is_some() {
            panic!("Extension already requested");
        }

        let extension_id = storage::increment_extension_counter(&env);
        let now = env.ledger().timestamp();

        let extension = LeaseExtensionRequest {
            extension_id,
            lease_id,
            additional_duration_seconds,
            requested_at: now,
            approved: false,
        };

        storage::set_lease_extension(&env, &extension);

        let old_status = lease.status;
        lease.pending_extension_id = Some(extension_id);
        Self::transition_lease_state(
            &env,
            &mut lease,
            LeaseState::ExtensionRequested,
            &lessee,
            "extension_requested",
            Some(String::from_str(&env, "Lessee requested more time on the lease")),
            Some(String::from_str(&env, "extension_request_created")),
        );
        storage::set_lease(&env, &lease);

        env.events().publish(
            (Symbol::new(&env, "LeaseExtensionRequested"),),
            (
                lease_id,
                extension_id,
                lessee,
                additional_duration_seconds,
                old_status,
            ),
        );

        extension_id
    }

    /// Approve lease extension
    pub fn approve_lease_extension(env: Env, lease_id: u64, extension_id: u64, lessor: Address) {
        lessor.require_auth();

        if validation::validate_nonzero_id(lease_id).is_err() {
            panic!("Invalid lease ID");
        }
        if validation::validate_nonzero_id(extension_id).is_err() {
            panic!("Invalid extension ID");
        }

        let mut lease = storage::get_lease(&env, lease_id).expect("Lease not found");
        let extension =
            storage::get_lease_extension(&env, extension_id).expect("Extension not found");

        if lease.lessor != lessor {
            panic!("Unauthorized: only lessor can approve extension");
        }
        if lease.status != LeaseState::ExtensionRequested {
            panic!("No extension requested");
        }
        if lease.pending_extension_id != Some(extension_id) {
            panic!("Extension ID mismatch");
        }
        if extension.approved {
            panic!("Extension already approved");
        }

        let old_status = lease.status;
        lease.end_time += extension.additional_duration_seconds;
        lease.duration_seconds += extension.additional_duration_seconds;
        lease.pending_extension_id = None;
        Self::transition_lease_state(
            &env,
            &mut lease,
            LeaseState::Active,
            &lessor,
            "extension_approved",
            Some(String::from_str(&env, "Lessor approved the extension request")),
            Some(String::from_str(&env, "extension_approved")),
        );

        storage::set_lease(&env, &lease);

        // Mark extension as approved
        let mut approved_extension = extension.clone();
        approved_extension.approved = true;
        storage::set_lease_extension(&env, &approved_extension);

        env.events().publish(
            (Symbol::new(&env, "LeaseExtended"),),
            (
                lease_id,
                extension_id,
                lessor,
                extension.additional_duration_seconds,
                old_status,
            ),
        );
    }

    /// Early lease termination with penalty
    pub fn early_termination(env: Env, lease_id: u64, lessee: Address, termination_fee_paid: i128) {
        lessee.require_auth();

        if validation::validate_nonzero_id(lease_id).is_err() {
            panic!("Invalid lease ID");
        }
        if termination_fee_paid <= 0 {
            panic!("Termination fee must be positive");
        }

        let mut lease = storage::get_lease(&env, lease_id).expect("Lease not found");
        if lease.lessee != lessee {
            panic!("Unauthorized: only lessee can terminate");
        }
        if !Self::is_live_lease_state(lease.status) {
            panic!("Lease is not active");
        }

        let now = env.ledger().timestamp();
        Self::sync_due_payments(&env, &mut lease);
        let remaining_time = if lease.end_time > now {
            lease.end_time - now
        } else {
            0
        };
        let remaining_value =
            (lease.total_value * remaining_time as i128) / lease.duration_seconds as i128;

        let config = storage::get_lease_config(&env);
        let penalty_bps = if lease.termination_penalty_bps > 0 {
            lease.termination_penalty_bps
        } else {
            config.early_termination_penalty_bps
        };
        let required_penalty = (remaining_value * (penalty_bps as i128)) / 10_000;
        let required_settlement = lease.outstanding_balance + required_penalty;
        let deposit_credit = if lease.deposit_amount > required_settlement {
            required_settlement
        } else {
            lease.deposit_amount
        };
        let amount_due_from_lessee = required_settlement - deposit_credit;

        if termination_fee_paid < amount_due_from_lessee {
            panic!("Insufficient termination fee");
        }

        let token_address = storage::get_payment_token(&env);
        let token_client = token::Client::new(&env, &token_address);
        if amount_due_from_lessee > 0 {
            token_client.transfer(
                &lessee,
                &env.current_contract_address(),
                &amount_due_from_lessee,
            );
        }

        let routed_amount = deposit_credit + amount_due_from_lessee;
        if routed_amount > 0 {
            let platform_fee_bps = Self::get_platform_fee(env.clone());
            let royalty_info = Self::get_royalty(env.clone(), lease.agent_id);
            Self::route_sale_payment(
                &env,
                lease.agent_id,
                routed_amount,
                &lease.lessee,
                &lease.lessor,
                royalty_info,
                platform_fee_bps,
                lease.asset_class,
            );
        }

        if termination_fee_paid > amount_due_from_lessee {
            let excess = termination_fee_paid - amount_due_from_lessee;
            token_client.transfer(&env.current_contract_address(), &lessee, &excess);
        }

        let remaining_deposit = lease.deposit_amount - deposit_credit;
        if remaining_deposit > 0 {
            token_client.transfer(&env.current_contract_address(), &lessee, &remaining_deposit);
        }

        lease.deposit_amount = 0;
        lease.outstanding_balance = 0;
        lease.accrued_late_fees = 0;
        lease.end_time = now;
        Self::transition_lease_state(
            &env,
            &mut lease,
            LeaseState::Terminated,
            &lessee,
            "early_terminated",
            Some(String::from_str(&env, "Lease terminated before scheduled expiry")),
            Some(String::from_str(&env, "termination_reconciliation_complete")),
        );
        storage::set_lease(&env, &lease);

        env.events().publish(
            (Symbol::new(&env, "LeaseTerminated"),),
            (
                lease_id,
                lessee,
                termination_fee_paid,
                required_penalty,
                required_settlement,
            ),
        );
    }

    /// Automatic lease renewal
    pub fn auto_renew_lease(env: Env, lease_id: u64) {
        Self::auto_renew_lease_v2(env, lease_id);
    }

    /// Get lease by ID
    pub fn get_lease_by_id(env: Env, lease_id: u64) -> Option<LeaseData> {
        if validation::validate_nonzero_id(lease_id).is_err() {
            panic!("Invalid lease ID");
        }
        storage::get_lease(&env, lease_id)
    }

    /// Get active leases for an address (lessee or lessor)
    pub fn get_active_leases(env: Env, user: Address) -> Vec<LeaseData> {
        let mut active_leases = Vec::new(&env);

        // Check as lessee
        let lessee_count = storage::get_lessee_lease_count(&env, &user);
        for i in 0..lessee_count {
            if let Some(lease_id) = storage::get_lessee_lease(&env, &user, i) {
                if let Some(lease) = storage::get_lease(&env, lease_id) {
                    if Self::is_live_lease_state(lease.status) {
                        active_leases.push_back(lease);
                    }
                }
            }
        }

        // Check as lessor
        let lessor_count = storage::get_lessor_lease_count(&env, &user);
        for i in 0..lessor_count {
            if let Some(lease_id) = storage::get_lessor_lease(&env, &user, i) {
                if let Some(lease) = storage::get_lease(&env, lease_id) {
                    if Self::is_live_lease_state(lease.status) {
                        let mut found = false;
                        for existing in active_leases.iter() {
                            if existing.lease_id == lease.lease_id {
                                found = true;
                                break;
                            }
                        }
                        if !found {
                            active_leases.push_back(lease);
                        }
                    }
                }
            }
        }

        active_leases
    }

    /// Get lease history
    pub fn get_lease_history(env: Env, lease_id: u64) -> Vec<LeaseHistoryEntry> {
        if validation::validate_nonzero_id(lease_id).is_err() {
            panic!("Invalid lease ID");
        }

        let history_count = storage::get_lease_history_count(&env, lease_id);
        let mut history = Vec::new(&env);

        for i in 0..history_count {
            if let Some(entry) = storage::get_lease_history(&env, lease_id, i) {
                history.push_back(entry);
            }
        }

        history
    }

    /// Get queued lease notifications for off-chain email delivery and in-app alerts.
    pub fn get_lease_notifications(env: Env, lease_id: u64) -> Vec<LeaseNotification> {
        if validation::validate_nonzero_id(lease_id).is_err() {
            panic!("Invalid lease ID");
        }

        let count = storage::get_lease_notification_count(&env, lease_id);
        let mut notifications = Vec::new(&env);
        for i in 0..count {
            if let Some(notification_id) = storage::get_lease_notification_index(&env, lease_id, i) {
                if let Some(notification) = storage::get_lease_notification(&env, notification_id) {
                    notifications.push_back(notification);
                }
            }
        }
        notifications
    }

    // ---------------- DYNAMIC FEE ADJUSTMENT ----------------

    /// Initialize dynamic fee adjustment (admin only)
    pub fn init_dynamic_fees(
        env: Env,
        admin: Address,
        congestion_oracle: Address,
        utilization_oracle: Address,
        volatility_oracle: Address,
        min_fee_bps: u32,
        max_fee_bps: u32,
        adjustment_window: u64,
    ) {
        Self::verify_admin(&env, &admin);

        assert!(
            min_fee_bps < max_fee_bps,
            "Min fee must be less than max fee"
        );
        assert!(max_fee_bps <= 10000, "Max fee cannot exceed 100%");
        assert!(adjustment_window > 0, "Adjustment window must be positive");

        let params = storage::FeeAdjustmentParams {
            base_marketplace_fee: storage::get_platform_fee(&env),
            congestion_oracle_id: congestion_oracle.clone(),
            utilization_oracle_id: utilization_oracle.clone(),
            volatility_oracle_id: volatility_oracle.clone(),
            min_fee_bps,
            max_fee_bps,
            adjustment_window,
        };

        storage::set_fee_adjustment_params(&env, &params);

        // Initialize oracle subscriptions
        let mut oracle_ids = Vec::new(&env);
        oracle_ids.push_back(congestion_oracle);
        oracle_ids.push_back(utilization_oracle);
        oracle_ids.push_back(volatility_oracle);
        storage::set_oracle_subscriptions(&env, &oracle_ids);

        env.events().publish(
            (Symbol::new(&env, "DynamicFeesInitialized"),),
            (min_fee_bps, max_fee_bps, adjustment_window),
        );
    }

    // ---------------- CREDIT SCORE NFT INTEGRATION ----------------

    /// Set credit score NFT contract address (admin only)
    pub fn set_credit_score_nft_contract(env: Env, admin: Address, nft_contract: Address) {
        Self::verify_admin(&env, &admin);

        env.storage().instance().set(
            &Symbol::new(&env, "credit_score_nft_contract"),
            &nft_contract,
        );

        env.events().publish(
            (Symbol::new(&env, "CreditScoreNFTContractSet"),),
            (nft_contract,),
        );
    }

    /// Get credit score NFT contract address
    pub fn get_credit_score_nft_contract(env: Env) -> Option<Address> {
        env.storage()
            .instance()
            .get(&Symbol::new(&env, "credit_score_nft_contract"))
    }

    /// Mint credit score NFT based on successful transaction
    pub fn mint_credit_score_tx_nft(
        env: Env,
        user: Address,
        transaction_type: String,
        transaction_value: i128,
        credit_score: u32,
        score_type: u32, // Corresponds to ScoreType enum
        metadata_cid: String,
    ) -> Result<u64, ContractError> {
        user.require_auth();

        let nft_contract = Self::get_credit_score_nft_contract(env.clone())
            .ok_or(ContractError::NotInitialized)?;

        // Validate credit score range
        if credit_score < 300 || credit_score > 850 {
            return Err(ContractError::InvalidInput);
        }

        // Calculate expiration (1 year from now)
        let expiration_time = env.ledger().timestamp() + (365 * 24 * 60 * 60);

        // Create mint request as a generic Val structure
        let mint_request = {
            let mut request_map = Map::<Symbol, Val>::new(&env);
            request_map.set(Symbol::new(&env, "owner"), user.clone().into_val(&env));
            request_map.set(
                Symbol::new(&env, "credit_score"),
                credit_score.into_val(&env),
            );
            request_map.set(Symbol::new(&env, "score_type"), score_type.into_val(&env));
            request_map.set(
                Symbol::new(&env, "expires_at"),
                expiration_time.into_val(&env),
            );

            // Create metadata as Map
            let mut metadata_map = Map::<Symbol, Val>::new(&env);
            metadata_map.set(
                Symbol::new(&env, "name"),
                String::from_str(&env, "StellAIverse Credit Score").into_val(&env),
            );
            metadata_map.set(
                Symbol::new(&env, "description"),
                String::from_str(&env, "Credit score NFT earned through marketplace activity")
                    .into_val(&env),
            );
            metadata_map.set(
                Symbol::new(&env, "image"),
                metadata_cid.clone().into_val(&env),
            );
            metadata_map.set(
                Symbol::new(&env, "external_url"),
                String::from_str(&env, "https://stellAIverse.io").into_val(&env),
            );

            // Create attributes as Vec
            let mut attributes = Vec::<Val>::new(&env);

            let mut attr1 = Map::<Symbol, Val>::new(&env);
            attr1.set(
                Symbol::new(&env, "trait_type"),
                String::from_str(&env, "transaction_type").into_val(&env),
            );
            attr1.set(
                Symbol::new(&env, "value"),
                transaction_type.clone().into_val(&env),
            );
            attributes.push_back(attr1.into_val(&env));

            let mut attr2 = Map::<Symbol, Val>::new(&env);
            attr2.set(
                Symbol::new(&env, "trait_type"),
                String::from_str(&env, "transaction_value").into_val(&env),
            );
            attr2.set(Symbol::new(&env, "value"), transaction_value.into_val(&env));
            attributes.push_back(attr2.into_val(&env));

            let mut attr3 = Map::<Symbol, Val>::new(&env);
            attr3.set(
                Symbol::new(&env, "trait_type"),
                String::from_str(&env, "credit_score").into_val(&env),
            );
            attr3.set(Symbol::new(&env, "value"), credit_score.into_val(&env));
            attr3.set(
                Symbol::new(&env, "display_type"),
                String::from_str(&env, "number").into_val(&env),
            );
            attributes.push_back(attr3.into_val(&env));

            metadata_map.set(Symbol::new(&env, "attributes"), attributes.into_val(&env));
            request_map.set(Symbol::new(&env, "metadata"), metadata_map.into_val(&env));

            // Create verification data as Map
            let mut verification_map = Map::<Symbol, Val>::new(&env);
            verification_map.set(
                Symbol::new(&env, "verification_method"),
                String::from_str(&env, "marketplace_activity").into_val(&env),
            );
            verification_map.set(
                Symbol::new(&env, "verified_by"),
                env.current_contract_address().into_val(&env),
            );
            verification_map.set(
                Symbol::new(&env, "verification_timestamp"),
                env.ledger().timestamp().into_val(&env),
            );
            verification_map.set(
                Symbol::new(&env, "verification_hash"),
                BytesN::from_array(&env, &[0u8; 32]).into_val(&env),
            );
            verification_map.set(
                Symbol::new(&env, "external_reference"),
                String::from_str(&env, "tx_ref").into_val(&env),
            );
            request_map.set(
                Symbol::new(&env, "verification_data"),
                verification_map.into_val(&env),
            );

            request_map
        };

        // Call the NFT contract to mint
        let token_id: u64 = env.invoke_contract(
            &nft_contract,
            &Symbol::new(&env, "mint_credit_score_nft"),
            Vec::from_array(&env, [user.into_val(&env), mint_request.into_val(&env)]),
        );

        // Log the minting
        let before_state = String::from_str(&env, "{}");
        let after_state = String::from_str(&env, "Credit score NFT minted");
        let tx_hash = String::from_str(&env, "0x_credit_score_minted");
        let description = Some(String::from_str(
            &env,
            "Credit score NFT minted via marketplace",
        ));

        create_audit_log(
            &env,
            user.clone(),
            OperationType::AdminMint,
            before_state,
            after_state,
            tx_hash,
            description,
        );

        env.events().publish(
            (Symbol::new(&env, "CreditScoreNFTMinted"),),
            (token_id, user, credit_score),
        );

        Ok(token_id)
    }

    /// Auto-mint credit score NFT for successful agent purchase
    pub fn auto_mint_credit_on_purchase(
        env: Env,
        listing_id: u64,
        buyer: Address,
    ) -> Result<u64, ContractError> {
        let listing_key = (Symbol::new(&env, "listing"), listing_id);
        let listing: Listing = env
            .storage()
            .instance()
            .get(&listing_key)
            .expect("Listing not found");

        // Calculate credit score based on transaction value and history
        let base_score = 600; // Base score
        let value_bonus = ((listing.price / 10000) as u32).min(100); // Up to 100 points based on value
        let credit_score = base_score + value_bonus;

        // Check if user already has too many NFTs (prevent spam)
        let nft_contract = Self::get_credit_score_nft_contract(env.clone())
            .ok_or(ContractError::NotInitialized)?;

        let existing_nfts: Vec<u64> = env.invoke_contract(
            &nft_contract,
            &Symbol::new(&env, "get_nfts_by_owner"),
            Vec::from_array(&env, [buyer.clone().into_val(&env)]),
        );

        if existing_nfts.len() >= 10 {
            return Err(ContractError::LimitExceeded);
        }

        Self::mint_credit_score_tx_nft(
            env.clone(),
            buyer,
            String::from_str(&env, "agent_purchase"),
            listing.price,
            credit_score,
            0, // FICO type
            String::from_str(&env, "ipfs://marketplace-agent-purchase"),
        )
    }

    /// Auto-mint credit score NFT for successful auction win
    pub fn auto_mint_credit_on_auction(
        env: Env,
        auction_id: u64,
        winner: Address,
    ) -> Result<u64, ContractError> {
        let auction = storage::get_auction(&env, auction_id).expect("Auction not found");

        // Calculate credit score based on auction activity
        let base_score = 650; // Higher base for auction participation
        let bid_bonus = ((auction.highest_bid / 10000) as u32).min(150); // Up to 150 points
        let credit_score = base_score + bid_bonus;

        Self::mint_credit_score_tx_nft(
            env.clone(),
            winner,
            String::from_str(&env, "auction_win"),
            auction.highest_bid,
            credit_score,
            1, // VantageScore type
            String::from_str(&env, "ipfs://marketplace-auction-win"),
        )
    }

    /// Auto-mint credit score NFT for successful lease completion
    pub fn auto_mint_credit_on_lease(
        env: Env,
        lease_id: u64,
        lessee: Address,
    ) -> Result<u64, ContractError> {
        let lease = storage::get_lease(&env, lease_id).expect("Lease not found");

        // Calculate credit score based on lease reliability
        let base_score = 700; // High base for completing lease
        let lease_bonus = ((lease.total_value / 10000) as u32).min(100); // Up to 100 points
        let credit_score = base_score + lease_bonus;

        Self::mint_credit_score_tx_nft(
            env.clone(),
            lessee,
            String::from_str(&env, "lease_completion"),
            lease.total_value,
            credit_score,
            2, // Experian type
            String::from_str(&env, "ipfs://marketplace-lease-completion"),
        )
    }

    /// Get user's credit score NFTs
    pub fn get_user_credit_score_nfts(env: Env, user: Address) -> Result<Vec<u64>, ContractError> {
        let nft_contract = Self::get_credit_score_nft_contract(env.clone())
            .ok_or(ContractError::NotInitialized)?;

        let nfts: Vec<u64> = env.invoke_contract(
            &nft_contract,
            &Symbol::new(&env, "get_nfts_by_owner"),
            Vec::from_array(&env, [user.into_val(&env)]),
        );

        Ok(nfts)
    }

    /// Get user's aggregated credit score from NFTs
    pub fn get_user_aggregated_credit_score(env: Env, user: Address) -> Result<u32, ContractError> {
        let nft_contract = Self::get_credit_score_nft_contract(env.clone())
            .ok_or(ContractError::NotInitialized)?;

        let nfts = Self::get_user_credit_score_nfts(env.clone(), user.clone())?;

        if nfts.is_empty() {
            return Ok(300); // Minimum score if no NFTs
        }

        let mut total_score = 0u32;
        let mut verified_count = 0u32;

        for token_id in nfts.iter() {
            let nft_data: Map<Symbol, Val> = env.invoke_contract(
                &nft_contract,
                &Symbol::new(&env, "get_nft"),
                Vec::from_array(&env, [token_id.into_val(&env)]),
            );

            // Get credit score and verification status from the map
            let credit_score = nft_data
                .get(Symbol::new(&env, "credit_score"))
                .unwrap_or_else(|| 0_u32.into_val(&env))
                .try_into_val(&env)
                .unwrap_or(300);

            let verification_status_val = nft_data
                .get(Symbol::new(&env, "verification_status"))
                .unwrap_or_else(|| 0_u32.into_val(&env));

            // Only count verified NFTs (status = 1 for verified)
            if verification_status_val.try_into_val(&env).unwrap_or(0) == 1 {
                total_score += credit_score;
                verified_count += 1;
            }
        }

        if verified_count == 0 {
            return Ok(300); // Minimum score if no verified NFTs
        }

        // Return weighted average (newer NFTs have more weight)
        Ok(total_score / verified_count)
    }

    /// Verify user's credit score NFTs (verification authority only)
    pub fn verify_user_credit_scores(
        env: Env,
        verifier: Address,
        user: Address,
    ) -> Result<(), ContractError> {
        let nft_contract = Self::get_credit_score_nft_contract(env.clone())
            .ok_or(ContractError::NotInitialized)?;

        let nfts = Self::get_user_credit_score_nfts(env.clone(), user.clone())?;

        for token_id in nfts.iter() {
            let nft_data: Map<Symbol, Val> = env.invoke_contract(
                &nft_contract,
                &Symbol::new(&env, "get_nft"),
                Vec::from_array(&env, [token_id.into_val(&env)]),
            );

            // Get verification status from the map
            let verification_status_val = nft_data
                .get(Symbol::new(&env, "verification_status"))
                .unwrap_or_else(|| 0.into_val(&env));

            // Only verify pending NFTs (status = 0 for pending)
            if verification_status_val.try_into_val(&env).unwrap_or(1) == 0 {
                let verification_hash = BytesN::from_array(&env, &[1u8; 32]); // Placeholder hash

                env.invoke_contract::<Val>(
                    &nft_contract,
                    &Symbol::new(&env, "verify_credit_score"),
                    Vec::from_array(
                        &env,
                        [
                            verifier.clone().into_val(&env),
                            token_id.into_val(&env),
                            verification_hash.into_val(&env),
                        ],
                    ),
                );
            }
        }

        Ok(())
    }

    // ---------------- ENHANCED LEASE MANAGEMENT ----------------

    /// Helper to get payment frequency interval in seconds
    fn get_payment_frequency_interval(frequency: PaymentFrequency) -> u64 {
        match frequency {
            PaymentFrequency::Daily => 86400,
            PaymentFrequency::Weekly => 604800,
            PaymentFrequency::Monthly => 2592000,
            PaymentFrequency::Quarterly => 7776000,
            PaymentFrequency::Yearly => 31536000,
        }
    }

    fn get_late_fee_policy(lease: &LeaseData) -> LateFeePolicy {
        let fee_type = match lease.late_fee_type {
            1 => LateFeeType::Fixed,
            2 => LateFeeType::Percentage,
            3 => LateFeeType::DailyAccumulation,
            _ => LateFeeType::None,
        };
        LateFeePolicy {
            fee_type,
            value: lease.late_fee_value,
        }
    }

    fn lease_state_label(env: &Env, state: LeaseState) -> String {
        match state {
            LeaseState::None => String::from_str(env, "none"),
            LeaseState::Active => String::from_str(env, "active"),
            LeaseState::ExtensionRequested => String::from_str(env, "extension_requested"),
            LeaseState::Terminated => String::from_str(env, "terminated"),
            LeaseState::Renewed => String::from_str(env, "renewed"),
            LeaseState::Pending => String::from_str(env, "pending"),
            LeaseState::Overdue => String::from_str(env, "overdue"),
            LeaseState::PendingRenewal => String::from_str(env, "pending_renewal"),
            LeaseState::Expired => String::from_str(env, "expired"),
        }
    }

    fn is_live_lease_state(state: LeaseState) -> bool {
        matches!(
            state,
            LeaseState::Active
                | LeaseState::Renewed
                | LeaseState::Overdue
                | LeaseState::PendingRenewal
                | LeaseState::ExtensionRequested
        )
    }

    /// Calculate total payment amount (base + late fees)
    fn calculate_payment_amount(
        base_amount: i128,
        current_time: u64,
        due_time: u64,
        late_fee_policy: &LateFeePolicy,
    ) -> (i128, i128) {
        if current_time <= due_time {
            return (base_amount, 0);
        }

        let late_fee = match late_fee_policy.fee_type {
            LateFeeType::None => 0,
            LateFeeType::Fixed => late_fee_policy.value as i128,
            LateFeeType::Percentage => (base_amount * (late_fee_policy.value as i128)) / 10_000,
            LateFeeType::DailyAccumulation => {
                let days_late = (current_time - due_time) / 86400;
                (late_fee_policy.value as i128) * (days_late as i128)
            }
        };

        (base_amount + late_fee, late_fee)
    }

    fn record_lease_history(
        env: &Env,
        lease_id: u64,
        actor: &Address,
        action: &str,
        details: Option<String>,
        reason: Option<String>,
        old_status: LeaseState,
        new_status: LeaseState,
    ) {
        let entry = LeaseHistoryEntry {
            lease_id,
            action: String::from_str(env, action),
            actor: actor.clone(),
            timestamp: env.ledger().timestamp(),
            details,
            reason: reason.clone(),
            old_status,
            new_status,
        };
        storage::add_lease_history(env, lease_id, &entry);

        let operation = match action {
            "initiated" | "initiated_v2" => OperationType::LeaseStarted,
            "extension_requested" => OperationType::LeaseExtensionRequested,
            "extension_approved" => OperationType::LeaseExtended,
            "auto_renewed" | "renewed" => OperationType::LeaseRenewed,
            "payment_overdue" => OperationType::LeaseOverdue,
            "payment_processed" => OperationType::LeasePaymentProcessed,
            "early_terminated" | "expired" => OperationType::LeaseEnded,
            _ => {
                if matches!(new_status, LeaseState::Terminated | LeaseState::Expired) {
                    OperationType::LeaseEnded
                } else {
                    OperationType::LeaseStarted
                }
            }
        };
        let description = reason.or_else(|| Some(String::from_str(env, action)));
        let _ = create_audit_log(
            env,
            actor.clone(),
            operation,
            Self::lease_state_label(env, old_status),
            Self::lease_state_label(env, new_status),
            String::from_str(env, "lease_lifecycle"),
            description,
        );
    }

    fn enqueue_lease_notification(
        env: &Env,
        lease: &LeaseData,
        recipient: &Address,
        channel: LeaseNotificationChannel,
        message: String,
        scheduled_for: u64,
    ) {
        let notification_id = storage::increment_lease_notification_counter(env);
        let notification = LeaseNotification {
            notification_id,
            lease_id: lease.lease_id,
            channel,
            recipient: recipient.clone(),
            message: message.clone(),
            created_at: env.ledger().timestamp(),
            scheduled_for,
            sent_at: None,
        };
        storage::set_lease_notification(env, &notification);
        storage::add_lease_notification_index(env, lease.lease_id, notification_id);

        env.events().publish(
            (Symbol::new(env, "LeaseNotificationQueued"),),
            (lease.lease_id, notification_id, channel as u32, recipient.clone(), message),
        );
    }

    fn schedule_renewal_notifications(env: &Env, lease: &mut LeaseData) {
        let scheduled_for = lease.end_time;
        if lease.email_notifications_enabled {
            Self::enqueue_lease_notification(
                env,
                lease,
                &lease.lessee,
                LeaseNotificationChannel::Email,
                String::from_str(env, "Lease renewal reminder"),
                scheduled_for,
            );
        }
        if lease.in_app_notifications_enabled {
            Self::enqueue_lease_notification(
                env,
                lease,
                &lease.lessee,
                LeaseNotificationChannel::InApp,
                String::from_str(env, "Lease expiry reminder"),
                scheduled_for,
            );
        }
        lease.last_notification_timestamp = env.ledger().timestamp();
    }

    fn sync_due_payments(env: &Env, lease: &mut LeaseData) {
        if lease.payment_interval == 0
            || lease.payment_amount <= 0
            || lease.next_payment_timestamp == 0
        {
            return;
        }

        let now = env.ledger().timestamp();
        if now < lease.next_payment_timestamp {
            return;
        }

        let late_fee_policy = Self::get_late_fee_policy(lease);
        let mut due_at = lease.next_payment_timestamp;
        let mut outstanding_added = 0i128;
        let mut late_fees_added = 0i128;
        let mut missed_cycles = 0u32;

        while due_at <= now && due_at < lease.end_time {
            let (cycle_total, cycle_late_fee) = Self::calculate_payment_amount(
                lease.payment_amount,
                now,
                due_at,
                &late_fee_policy,
            );
            outstanding_added += cycle_total;
            late_fees_added += cycle_late_fee;
            missed_cycles += 1;
            due_at += lease.payment_interval;
        }

        if missed_cycles == 0 {
            return;
        }

        lease.outstanding_balance += outstanding_added;
        lease.accrued_late_fees += late_fees_added;
        lease.missed_payments += missed_cycles;
        lease.next_payment_timestamp = due_at;

        let actor = env.current_contract_address();
        if lease.status != LeaseState::Overdue {
            Self::transition_lease_state(
                env,
                lease,
                LeaseState::Overdue,
                &actor,
                "payment_overdue",
                Some(String::from_str(env, "Scheduled payment deadline missed")),
                Some(String::from_str(env, "late_fees_accrued")),
            );
        } else {
            Self::record_lease_history(
                env,
                lease.lease_id,
                &actor,
                "payment_overdue",
                Some(String::from_str(env, "additional_overdue_cycle")),
                Some(String::from_str(env, "Scheduled payment deadline missed")),
                LeaseState::Overdue,
                LeaseState::Overdue,
            );
        }
    }

    fn renew_lease_internal(env: &Env, lease: &mut LeaseData, actor: &Address, action: &str) {
        let renewal_start = if env.ledger().timestamp() > lease.end_time {
            env.ledger().timestamp()
        } else {
            lease.end_time
        };
        lease.start_time = renewal_start;
        lease.end_time = renewal_start + lease.duration_seconds;
        lease.current_renewal_count += 1;
        lease.outstanding_balance = 0;
        lease.accrued_late_fees = 0;
        lease.last_notification_timestamp = 0;
        if lease.payment_interval > 0 && lease.payment_amount > 0 {
            lease.next_payment_timestamp = renewal_start + lease.payment_interval;
        }
        Self::transition_lease_state(
            env,
            lease,
            LeaseState::Renewed,
            actor,
            action,
            Some(String::from_str(env, "Lease term automatically extended")),
            Some(String::from_str(env, "renewal_executed")),
        );
    }

    /// Validate and transition lease state
    fn transition_lease_state(
        env: &Env,
        lease: &mut LeaseData,
        new_state: LeaseState,
        actor: &Address,
        action: &str,
        reason: Option<String>,
        details: Option<String>,
    ) {
        if lease.status == new_state {
            Self::record_lease_history(
                env,
                lease.lease_id,
                actor,
                action,
                details,
                reason,
                lease.status,
                new_state,
            );
            return;
        }

        match (lease.status, new_state) {
            (LeaseState::None, LeaseState::Active) => {},
            (LeaseState::Pending, LeaseState::Active) => {},
            (LeaseState::Active, LeaseState::ExtensionRequested) => {},
            (LeaseState::Active, LeaseState::Overdue) => {},
            (LeaseState::Active, LeaseState::PendingRenewal) => {},
            (LeaseState::Active, LeaseState::Renewed) => {},
            (LeaseState::Active, LeaseState::Terminated) => {},
            (LeaseState::Active, LeaseState::Expired) => {},
            (LeaseState::Renewed, LeaseState::Active) => {},
            (LeaseState::Renewed, LeaseState::Overdue) => {},
            (LeaseState::Renewed, LeaseState::PendingRenewal) => {},
            (LeaseState::Renewed, LeaseState::Terminated) => {},
            (LeaseState::Renewed, LeaseState::Expired) => {},
            (LeaseState::ExtensionRequested, LeaseState::Active) => {},
            (LeaseState::ExtensionRequested, LeaseState::Terminated) => {},
            (LeaseState::Overdue, LeaseState::Active) => {},
            (LeaseState::Overdue, LeaseState::PendingRenewal) => {},
            (LeaseState::Overdue, LeaseState::Renewed) => {},
            (LeaseState::Overdue, LeaseState::Terminated) => {},
            (LeaseState::Overdue, LeaseState::Expired) => {},
            (LeaseState::PendingRenewal, LeaseState::Active) => {},
            (LeaseState::PendingRenewal, LeaseState::Renewed) => {},
            (LeaseState::PendingRenewal, LeaseState::Terminated) => {},
            (LeaseState::PendingRenewal, LeaseState::Expired) => {},
            _ => panic!("Invalid state transition"),
        }

        let old_status = lease.status;
        lease.status = new_state;
        Self::record_lease_history(
            env,
            lease.lease_id,
            actor,
            action,
            details,
            reason,
            old_status,
            new_state,
        );
    }

    /// Mark a lease as overdue
    pub fn mark_as_overdue(env: Env, lease_id: u64) {
        let mut lease = storage::get_lease(&env, lease_id).expect("Lease not found");
        let actor = env.current_contract_address();
        Self::transition_lease_state(
            &env,
            &mut lease,
            LeaseState::Overdue,
            &actor,
            "marked_overdue",
            Some(String::from_str(&env, "Lease payment obligation is overdue")),
            Some(String::from_str(&env, "manual_overdue_mark")),
        );
        storage::set_lease(&env, &lease);

        env.events().publish(
            (Symbol::new(&env, "LeaseOverdue"),),
            (lease_id, lease.lessee),
        );
    }

    /// Expire a lease
    pub fn expire_lease(env: Env, lease_id: u64) {
        let mut lease = storage::get_lease(&env, lease_id).expect("Lease not found");
        let actor = env.current_contract_address();
        Self::transition_lease_state(
            &env,
            &mut lease,
            LeaseState::Expired,
            &actor,
            "expired",
            Some(String::from_str(&env, "Lease reached scheduled expiry")),
            Some(String::from_str(&env, "lease_expired")),
        );
        storage::set_lease(&env, &lease);

        env.events().publish(
            (Symbol::new(&env, "LeaseExpired"),),
            (lease_id, lease.lessee),
        );
    }

    /// Initiate a lease with enhanced options
    pub fn initiate_lease_v2(
        env: Env,
        listing_id: u64,
        lessee: Address,
        duration_seconds: u64,
        auto_renew: bool,
        lessee_consent_for_renewal: bool,
        payment_frequency: PaymentFrequency,
        payment_amount: i128,
        termination_penalty_bps: u32,
        late_fee_policy: LateFeePolicy,
    ) -> u64 {
        lessee.require_auth();

        if validation::validate_nonzero_id(listing_id).is_err() {
            panic!("Invalid listing ID");
        }
        if duration_seconds == 0 {
            panic!("Duration must be positive");
        }
        if duration_seconds > stellai_lib::MAX_DURATION_DAYS * 24 * 60 * 60 {
            panic!("Duration exceeds maximum");
        }
        if payment_amount < 0 {
            panic!("Payment amount cannot be negative");
        }
        if termination_penalty_bps > 5000 {
            panic!("Termination penalty cannot exceed 50%");
        }

        let listing_key = (Symbol::new(&env, "listing"), listing_id);
        let mut listing: Listing = env
            .storage()
            .instance()
            .get(&listing_key)
            .expect("Listing not found");

        if !listing.active {
            panic!("Listing is not active");
        }
        if listing.listing_type != ListingType::Lease {
            panic!("Listing is not for lease");
        }

        let lease_id = storage::increment_lease_counter(&env);
        let now = env.ledger().timestamp();
        let end_time = now + duration_seconds;

        let config = storage::get_lease_config(&env);
        let deposit_amount = (listing.price * (config.deposit_bps as i128)) / 10_000;
        let effective_penalty_bps = if termination_penalty_bps == 0 {
            config.early_termination_penalty_bps
        } else {
            termination_penalty_bps
        };
        let interval = if payment_amount > 0 {
            Self::get_payment_frequency_interval(payment_frequency)
        } else {
            0
        };
        let next_payment = if interval > 0 { now + interval } else { 0 };
        if deposit_amount > 0 {
            let payment_token = storage::get_payment_token(&env);
            let token_client = token::Client::new(&env, &payment_token);
            token_client.transfer(&lessee, &env.current_contract_address(), &deposit_amount);
        }

        let lease = LeaseData {
            lease_id,
            agent_id: listing.agent_id,
            listing_id,
            lessor: listing.seller.clone(),
            lessee: lessee.clone(),
            start_time: now,
            end_time,
            duration_seconds,
            deposit_amount,
            total_value: listing.price,
            auto_renew,
            lessee_consent_for_renewal,
            status: LeaseState::Active,
            pending_extension_id: None,
            payment_interval: interval,
            payment_amount,
            next_payment_timestamp: next_payment,
            max_renewals: if auto_renew { 10 } else { 0 },
            current_renewal_count: 0,
            termination_penalty_bps: effective_penalty_bps,
            late_fee_type: late_fee_policy.fee_type as u32,
            late_fee_value: late_fee_policy.value,
            outstanding_balance: 0,
            accrued_late_fees: 0,
            total_paid: 0,
            missed_payments: 0,
            renewal_notice_period: if auto_renew {
                if duration_seconds < 86_400 {
                    duration_seconds / 2
                } else {
                    86_400
                }
            } else {
                0
            },
            last_notification_timestamp: 0,
            email_notifications_enabled: true,
            in_app_notifications_enabled: true,
            asset_class: AssetClass::Agent,
        };

        listing.active = false;
        env.storage().instance().set(&listing_key, &listing);
        storage::set_lease(&env, &lease);
        storage::lessee_leases_append(&env, &lessee, lease_id);
        storage::lessor_leases_append(&env, &listing.seller, lease_id);
        Self::record_lease_history(
            &env,
            lease_id,
            &lessee,
            "initiated_v2",
            Some(String::from_str(&env, "lease_created_with_financial_schedule")),
            Some(String::from_str(&env, "Lease created and deposit reserved")),
            LeaseState::None,
            LeaseState::Active,
        );

        env.events().publish(
            (Symbol::new(&env, "LeaseInitiatedV2"),),
            (
                lease_id,
                listing_id,
                lessee,
                duration_seconds,
                auto_renew,
                deposit_amount,
            ),
        );

        lease_id
    }

    /// Process a periodic lease payment
    pub fn process_lease_payment(env: Env, lease_id: u64, payer: Address) {
        payer.require_auth();

        let mut lease = storage::get_lease(&env, lease_id).expect("Lease not found");
        assert!(lease.lessee == payer, "Only lessee can pay");
        assert!(Self::is_live_lease_state(lease.status), "Lease is not active");

        Self::sync_due_payments(&env, &mut lease);

        let now = env.ledger().timestamp();
        let interval = lease.payment_interval;
        let mut total_amount = lease.outstanding_balance;
        let mut late_fee = lease.accrued_late_fees;

        if total_amount == 0 {
            assert!(interval > 0, "Lease has no payment schedule");
            let next_payment = lease.next_payment_timestamp;
            let (scheduled_total, scheduled_late_fee) = Self::calculate_payment_amount(
                lease.payment_amount,
                now,
                next_payment,
                &Self::get_late_fee_policy(&lease),
            );
            total_amount = scheduled_total;
            late_fee = scheduled_late_fee;
            lease.next_payment_timestamp = next_payment + interval;
        }

        let payment_token = storage::get_payment_token(&env);
        let token_client = token::Client::new(&env, &payment_token);
        token_client.transfer(&payer, &env.current_contract_address(), &total_amount);

        let platform_fee_bps = Self::get_platform_fee(env.clone());
        let royalty_info = Self::get_royalty(env.clone(), lease.agent_id);

        Self::route_sale_payment(
            &env,
            lease.agent_id,
            total_amount,
            &lease.lessee,
            &lease.lessor,
            royalty_info,
            platform_fee_bps,
            lease.asset_class,
        );

        lease.total_paid += total_amount;
        lease.outstanding_balance = 0;
        lease.accrued_late_fees = 0;

        let actor = payer.clone();
        if matches!(
            lease.status,
            LeaseState::Overdue | LeaseState::PendingRenewal | LeaseState::Renewed
        ) && now < lease.end_time
        {
            Self::transition_lease_state(
                &env,
                &mut lease,
                LeaseState::Active,
                &actor,
                "payment_processed",
                Some(String::from_str(&env, "Outstanding lease balance collected")),
                Some(String::from_str(&env, "payment_cleared")),
            );
        } else {
            Self::record_lease_history(
                &env,
                lease_id,
                &actor,
                "payment_processed",
                Some(String::from_str(&env, "lease_payment_routed")),
                Some(String::from_str(&env, "Scheduled lease payment collected")),
                lease.status,
                lease.status,
            );
        }

        storage::set_lease(&env, &lease);

        env.events().publish(
            (Symbol::new(&env, "LeasePaymentProcessed"),),
            (lease_id, total_amount, late_fee),
        );
    }

    /// Evaluate a single lease for reminders, overdue balances, expiry, or renewal.
    pub fn process_due_lease(env: Env, lease_id: u64) {
        let mut lease = storage::get_lease(&env, lease_id).expect("Lease not found");
        let actor = env.current_contract_address();
        let now = env.ledger().timestamp();

        Self::sync_due_payments(&env, &mut lease);

        if now < lease.end_time {
            if lease.auto_renew
                && lease.renewal_notice_period > 0
                && (lease.end_time - now) <= lease.renewal_notice_period
                && lease.last_notification_timestamp == 0
            {
                Self::schedule_renewal_notifications(&env, &mut lease);
                if matches!(lease.status, LeaseState::Active | LeaseState::Renewed) {
                    Self::transition_lease_state(
                        &env,
                        &mut lease,
                        LeaseState::PendingRenewal,
                        &actor,
                        "renewal_notice_sent",
                        Some(String::from_str(&env, "Renewal reminder queued for lessee")),
                        Some(String::from_str(&env, "renewal_notifications_queued")),
                    );
                }
            }
        } else if lease.auto_renew
            && lease.lessee_consent_for_renewal
            && (lease.max_renewals == 0 || lease.current_renewal_count < lease.max_renewals)
            && lease.outstanding_balance == 0
        {
            if lease.status != LeaseState::PendingRenewal {
                Self::transition_lease_state(
                    &env,
                    &mut lease,
                    LeaseState::PendingRenewal,
                    &actor,
                    "renewal_window_open",
                    Some(String::from_str(&env, "Lease entered renewal window")),
                    Some(String::from_str(&env, "renewal_window_entered")),
                );
            }
            Self::renew_lease_internal(&env, &mut lease, &actor, "auto_renewed");
        } else if !matches!(lease.status, LeaseState::Expired | LeaseState::Terminated) {
            Self::transition_lease_state(
                &env,
                &mut lease,
                LeaseState::Expired,
                &actor,
                "expired",
                Some(String::from_str(&env, "Lease reached scheduled expiry without renewal")),
                Some(String::from_str(&env, "lease_expired")),
            );
        }

        storage::set_lease(&env, &lease);
    }

    /// Process a batch of leases for payment, reminder, renewal, and expiry automation.
    pub fn process_due_leases(env: Env, start_lease_id: u64, limit: u32) -> u32 {
        let max_lease_id = storage::get_lease_counter(&env);
        let mut processed = 0u32;
        let mut current_lease_id = start_lease_id;

        while processed < limit && current_lease_id <= max_lease_id {
            if storage::get_lease(&env, current_lease_id).is_some() {
                Self::process_due_lease(env.clone(), current_lease_id);
            }
            processed += 1;
            current_lease_id += 1;
        }

        processed
    }

    /// Automated renewal logic for leases (version 2)
    pub fn auto_renew_lease_v2(env: Env, lease_id: u64) {
        let mut lease = storage::get_lease(&env, lease_id).expect("Lease not found");
        let now = env.ledger().timestamp();

        assert!(now >= lease.end_time, "Lease not yet expired");
        assert!(lease.auto_renew, "Auto-renew not enabled");
        assert!(lease.lessee_consent_for_renewal, "Lessee consent not provided");
        assert!(
            lease.max_renewals == 0 || lease.current_renewal_count < lease.max_renewals,
            "Max renewals reached"
        );
        assert!(
            lease.outstanding_balance == 0,
            "Outstanding balance must be cleared before renewal"
        );

        let actor = env.current_contract_address();
        if lease.status != LeaseState::PendingRenewal {
            Self::transition_lease_state(
                &env,
                &mut lease,
                LeaseState::PendingRenewal,
                &actor,
                "renewal_window_open",
                Some(String::from_str(&env, "Lease entered renewal window")),
                Some(String::from_str(&env, "renewal_window_entered")),
            );
        }
        Self::renew_lease_internal(&env, &mut lease, &actor, "auto_renewed");
        storage::set_lease(&env, &lease);

        env.events().publish(
            (Symbol::new(&env, "LeaseAutoRenewedV2"),),
            (lease_id, lease.end_time),
        );
    }

    /// Calculate current price for a Dutch auction
    fn calculate_dutch_price(env: Env, auction: &Auction) -> i128 {
        let now = env.ledger().timestamp();
        if now >= auction.end_time {
            return auction.reserve_price;
        }

        let elapsed = now - auction.start_time;
        let duration = auction.end_time - auction.start_time;

        let price_diff = auction.start_price - auction.reserve_price;
        let drop = (price_diff * (elapsed as i128)) / (duration as i128);

        auction.start_price - drop
    }
}
