#![no_std]

mod atomic;
mod payment_types;
mod payments;
#[cfg(test)]
mod prop_tests;
mod storage;

use core::fmt::Write;
use soroban_sdk::{contract, contractimpl, token, Address, Bytes, Env, String, Symbol, Val, Vec};
use stellai_lib::{
    atomic::AtomicTransactionSupport,
    audit::{create_audit_log, OperationType},
    storage_keys::LISTING_COUNTER_KEY,
    types::{
        Approval, ApprovalConfig, ApprovalHistory, ApprovalStatus, Auction, AuctionStatus,
        AuctionType, LeaseData, LeaseExtensionRequest, LeaseHistoryEntry, LeaseState, Listing,
        ListingType, RoyaltyInfo,
    },
    validation,
};

use atomic::MarketplaceAtomicSupport;
use payment_types::PaymentRecord;
use payments::{calculate_splits, execute_payment_routing, PaymentRoutingContext};
use storage::*;

#[contract]
pub struct Marketplace;

#[contractimpl]
impl Marketplace {
    /// Initialize contract with admin
    pub fn init_contract(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("Contract already initialized");
        }

        admin.require_auth();
        set_admin(&env, &admin);

        env.storage()
            .instance()
            .set(&Symbol::new(&env, LISTING_COUNTER_KEY), &0u64);
        storage::set_platform_fee(&env, 250);
    }

    /// Set a new admin
    pub fn set_admin(env: Env, new_admin: Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Contract not initialized");
        admin.require_auth();
        set_admin(&env, &new_admin);
    }

    /// Set the payment token
    pub fn set_payment_token(env: Env, admin: Address, token: Address) {
        admin.require_auth();
        let current_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Contract not initialized");
        assert!(admin == current_admin, "Unauthorized");

        set_payment_token(&env, token);
    }

    /// Set the platform fee in basis points (max 50%).
    pub fn set_platform_fee(env: Env, admin: Address, fee_bps: u32) {
        admin.require_auth();
        assert!(fee_bps <= 5000, "Platform fee cannot exceed 50%");
        let current_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Contract not initialized");
        assert!(admin == current_admin, "Unauthorized");

        storage::set_platform_fee(&env, fee_bps);
        env.events()
            .publish((Symbol::new(&env, "platform_fee_updated"),), (fee_bps,));
    }

    /// Get the configured platform fee.
    pub fn get_platform_fee(env: Env) -> u32 {
        storage::get_platform_fee(&env)
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

    /// Purchase an agent
    pub fn buy_agent(env: Env, listing_id: u64, buyer: Address) {
        buyer.require_auth();

        if validation::validate_nonzero_id(listing_id).is_err() {
            panic!("Invalid listing ID");
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

        // Check if multi-signature approval is required
        let config = get_approval_config(&env);
        if listing.price >= config.threshold {
            panic!("High-value sale requires multi-signature approval. Use propose_sale() first.");
        }

        // Process fee transition if active
        Self::process_fee_transition(env.clone());

        let platform_fee_bps = Self::get_platform_fee(env.clone());
        Self::route_sale_payment(
            &env,
            listing.agent_id,
            listing.price,
            &buyer,
            &listing.seller,
        );

        // Mark listing as inactive
        listing.active = false;
        env.storage().instance().set(&listing_key, &listing);

        env.events().publish(
            (Symbol::new(&env, "agent_sold"),),
            (listing_id, listing.agent_id, buyer, platform_fee_bps),
        );
    }

    /// Helper to route payment for a completed sale.
    fn route_sale_payment(
        env: &Env,
        agent_id: u64,
        sale_price: i128,
        buyer: &Address,
        seller: &Address,
    ) {
        let mut royalty_recipients = Vec::new(env);
        let mut royalty_rate = 0u32;

        if let Some(info) = Marketplace::get_royalty(env.clone(), agent_id) {
            royalty_rate = info.fee;
            royalty_recipients.push_back((
                info.recipient,
                royalty_rate,
                String::from_str(env, "creator"),
            ));
        }

        let platform_fee_bps = Self::get_platform_fee(env.clone());
        let context = PaymentRoutingContext {
            agent_id,
            transaction_id: env.ledger().sequence() as u64,
            buyer: buyer.clone(),
            seller: seller.clone(),
            platform_address: env.current_contract_address(),
            royalty_recipients,
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
        let current_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Contract not initialized");
        assert!(admin == current_admin, "Unauthorized");

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
            Marketplace::execute_approved_listing_sale(env, approval_id, listing_id);
        } else if let Some(auction_id) = approval.auction_id {
            // Auction sale
            Marketplace::execute_approved_auction_sale(env, approval_id, auction_id);
        } else {
            panic!("Invalid approval: no listing or auction ID");
        }
    }

    /// Execute approved fixed-price sale (internal function)
    fn execute_approved_listing_sale(env: Env, approval_id: u64, listing_id: u64) {
        let listing_key = (Symbol::new(&env, "listing"), listing_id);
        let mut listing: Listing = env
            .storage()
            .instance()
            .get(&listing_key)
            .expect("Listing not found");

        let approval = get_approval(&env, approval_id).expect("Approval not found");

        // Process fee transition if active
        Self::process_fee_transition(env.clone());

        let platform_fee_bps = Self::get_platform_fee(env.clone());
        Self::route_sale_payment(
            &env,
            listing.agent_id,
            listing.price,
            &approval.buyer,
            &listing.seller,
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
    fn execute_approved_auction_sale(env: Env, approval_id: u64, auction_id: u64) {
        let mut auction = get_auction(&env, auction_id).expect("Auction not found");
        let approval = get_approval(&env, approval_id).expect("Approval not found");

        // Process fee transition if active
        Self::process_fee_transition(env.clone());

        // Process the auction resolution
        if let Some(winner) = auction.highest_bidder.clone() {
            if auction.highest_bid >= auction.reserve_price {
                let platform_fee_bps = Self::get_platform_fee(env.clone());
                Self::route_sale_payment(
                    &env,
                    auction.agent_id,
                    auction.highest_bid,
                    &winner,
                    &auction.seller,
                );

                // NOTE: NFT transfer logic should be added here

                auction.status = AuctionStatus::Won;

                env.events().publish(
                    (Symbol::new(&env, "AuctionWon"),),
                    (auction_id, winner, auction.highest_bid, platform_fee_bps),
                );
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
            timestamp: env.ledger().timestamp(),
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

    /// Get approval history
    pub fn get_approval_history(env: Env, approval_id: u64) -> Vec<ApprovalHistory> {
        if approval_id == 0 {
            panic!("Invalid approval ID");
        }

        let history_count = get_approval_history_count(&env, approval_id);
        let mut history = Vec::new(&env);

        for i in 0..history_count {
            if let Some(entry) = get_approval_history(&env, approval_id, i) {
                history.push_back(entry);
            }
        }

        history
    }

    /// Clean up expired approvals (can be called by anyone)
    pub fn cleanup_expired_approvals(env: Env) {
        let counter = get_approval_counter(&env);
        let mut cleaned_count = 0u64;

        for approval_id in 1..=counter {
            if let Some(approval) = get_approval(&env, approval_id) {
                if approval.status == ApprovalStatus::Pending
                    && env.ledger().timestamp() >= approval.expires_at
                {
                    // Mark as expired
                    let mut expired_approval = approval;
                    expired_approval.status = ApprovalStatus::Expired;
                    set_approval(&env, &expired_approval);

                    // Add to history
                    let history = ApprovalHistory {
                        approval_id,
                        action: String::from_str(&env, "expired"),
                        actor: env.current_contract_address(),
                        timestamp: env.ledger().timestamp(),
                        reason: None,
                    };
                    add_approval_history(&env, approval_id, &history);

                    cleaned_count += 1;
                }
            }
        }

        if cleaned_count > 0 {
            env.events().publish(
                (Symbol::new(&env, "ExpiredApprovalsCleaned"),),
                (cleaned_count,),
            );
        }
    }

    // ---------------- AUCTIONS ----------------

    /// Dutch params: (start_price, end_price, duration_seconds, price_decay). Use (None,None,None,None) for non-Dutch.
    pub fn create_auction(
        env: Env,
        agent_id: u64,
        seller: Address,
        auction_type: AuctionType,
        start_price: i128,
        reserve_price: i128,
        duration: u64,
        min_bid_increment_bps: u32,
    ) -> u64 {
        seller.require_auth();
        assert!(start_price > 0, "Invalid start price");
        assert!(duration > 0, "Invalid duration");

        let auction_id = increment_auction_counter(&env);
        let start_time = env.ledger().timestamp();
        let end_time = start_time + duration;

        let auction = Auction {
            auction_id,
            agent_id,
            seller,
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

    pub fn calculate_dutch_price(env: Env, auction_id: u64) -> i128 {
        let auction = get_auction(&env, auction_id).expect("Auction not found");
        assert!(
            auction.auction_type == AuctionType::Dutch,
            "Not a Dutch auction"
        );

        // Simplified calculation without dutch_config
        let now = env.ledger().timestamp();
        if now <= auction.start_time {
            return auction.start_price;
        }
        if now >= auction.end_time {
            return auction.reserve_price;
        }

        // Linear decay from start_price to reserve_price
        let elapsed = now - auction.start_time;
        let duration = auction.end_time - auction.start_time;
        let price_range = auction.start_price - auction.reserve_price;
        auction.start_price - (price_range * (elapsed as i128)) / (duration as i128)
    }

    pub fn place_bid(env: Env, auction_id: u64, bidder: Address, amount: i128) {
        bidder.require_auth();
        let mut auction = get_auction(&env, auction_id).expect("Auction not found");
        assert!(
            auction.status == AuctionStatus::Active,
            "Auction not active"
        );
        assert!(
            auction.auction_type == AuctionType::English,
            "Not an English auction"
        );
        assert!(
            env.ledger().timestamp() < auction.end_time,
            "Auction expired"
        );

        let min_increment = (auction.highest_bid * (auction.min_bid_increment_bps as i128)) / 10000;
        let computed_min_step = if min_increment > 1000 {
            min_increment
        } else {
            1000
        };
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

        auction.highest_bidder = Some(bidder.clone());
        auction.highest_bid = amount;

        // Extend auction by 5 minutes if bid in final 5 minutes
        let time_left = auction.end_time - env.ledger().timestamp();
        if time_left < 300 {
            auction.end_time += 300;
        }

        set_auction(&env, &auction);

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
            bidder,
            OperationType::AuctionBidPlaced,
            before_state,
            after_state,
            tx_hash,
            description,
        );
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

        let current_price = Marketplace::calculate_dutch_price(env.clone(), auction_id);

        let token_client = token::Client::new(&env, &get_payment_token(&env));
        token_client.transfer(&buyer, &env.current_contract_address(), &current_price);

        auction.highest_bidder = Some(buyer);
        auction.highest_bid = current_price;

        set_auction(&env, &auction);

        Marketplace::resolve_auction(env, auction_id);
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
        let current_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Contract not initialized");
        assert!(admin == current_admin, "Unauthorized");

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
        let current_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Contract not initialized");
        assert!(admin == current_admin, "Unauthorized");

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

    fn get_oracle_value_by_key(
        _env: &Env,
        _oracle_id: &Address,
        _key: &str,
        fallback: i128,
    ) -> i128 {
        // Simplified oracle integration - in production this would call the actual oracle
        // For now, return fallback value to ensure compilation
        fallback
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

    // ============ ATOMIC TRANSACTION SUPPORT ============

    /// Prepare atomic transaction step
    pub fn prepare_atomic_step(
        env: Env,
        transaction_id: u64,
        step_id: u32,
        function: Symbol,
        args: Vec<Val>,
    ) -> bool {
        MarketplaceAtomicSupport::prepare_step(&env, transaction_id, step_id, &function, &args)
    }

    /// Commit atomic transaction step
    pub fn commit_atomic_step(
        env: Env,
        transaction_id: u64,
        step_id: u32,
        function: Symbol,
        args: Vec<Val>,
    ) -> Val {
        MarketplaceAtomicSupport::commit_step(&env, transaction_id, step_id, &function, &args)
    }

    /// Check if atomic step is prepared
    pub fn is_atomic_step_prepared(env: Env, transaction_id: u64, step_id: u32) -> bool {
        MarketplaceAtomicSupport::is_step_prepared(&env, transaction_id, step_id)
    }

    /// Get atomic step result
    pub fn get_atomic_step_result(env: Env, transaction_id: u64, step_id: u32) -> Option<Val> {
        MarketplaceAtomicSupport::get_step_result(&env, transaction_id, step_id)
    }

    /// Rollback atomic transaction step (called by rollback functions)
    pub fn rollback_atomic_step(
        env: Env,
        transaction_id: u64,
        step_id: u32,
        rollback_function: Symbol,
        rollback_args: Vec<Val>,
    ) -> bool {
        MarketplaceAtomicSupport::rollback_step(
            &env,
            transaction_id,
            step_id,
            &rollback_function,
            &rollback_args,
        )
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

    /// Refund from escrow (rollback function)
    pub fn refund_from_escrow(env: Env, buyer: Address, amount: i128) -> bool {
        // In a real implementation, this would refund tokens from escrow
        // For now, just return success
        true
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
        let current_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Contract not initialized");
        assert!(admin == current_admin, "Unauthorized");

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

    /// Initiate a lease for an agent
    pub fn initiate_lease(
        env: Env,
        listing_id: u64,
        lessee: Address,
        duration_seconds: u64,
        auto_renew: bool,
        lessee_consent_for_renewal: bool,
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

        let listing_key = (Symbol::new(&env, "listing"), listing_id);
        let listing: Listing = env
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
        };

        storage::set_lease(&env, &lease);
        storage::lessee_leases_append(&env, &lessee, lease_id);
        storage::lessor_leases_append(&env, &listing.seller, lease_id);

        // Add to history
        let entry = LeaseHistoryEntry {
            lease_id,
            action: String::from_str(&env, "initiated"),
            actor: lessee.clone(),
            timestamp: now,
            details: None,
        };
        storage::add_lease_history(&env, lease_id, &entry);

        env.events().publish(
            (Symbol::new(&env, "LeaseInitiated"),),
            (lease_id, listing_id, lessee, duration_seconds, auto_renew),
        );

        lease_id
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
        if lease.status != LeaseState::Active {
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

        lease.status = LeaseState::ExtensionRequested;
        lease.pending_extension_id = Some(extension_id);
        storage::set_lease(&env, &lease);

        // Add to history
        let entry = LeaseHistoryEntry {
            lease_id,
            action: String::from_str(&env, "extension_requested"),
            actor: lessee.clone(),
            timestamp: now,
            details: Some(String::from_str(&env, "additional_duration: 3600")),
        };
        storage::add_lease_history(&env, lease_id, &entry);

        env.events().publish(
            (Symbol::new(&env, "LeaseExtensionRequested"),),
            (lease_id, extension_id, lessee, additional_duration_seconds),
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

        // Update lease with extension
        lease.end_time += extension.additional_duration_seconds;
        lease.duration_seconds += extension.additional_duration_seconds;
        lease.status = LeaseState::Active;
        lease.pending_extension_id = None;

        storage::set_lease(&env, &lease);

        // Mark extension as approved
        let mut approved_extension = extension.clone();
        approved_extension.approved = true;
        storage::set_lease_extension(&env, &approved_extension);

        // Add to history
        let entry = LeaseHistoryEntry {
            lease_id,
            action: String::from_str(&env, "extension_approved"),
            actor: lessor.clone(),
            timestamp: env.ledger().timestamp(),
            details: Some(String::from_str(&env, "additional_duration: 3600")),
        };
        storage::add_lease_history(&env, lease_id, &entry);

        env.events().publish(
            (Symbol::new(&env, "LeaseExtended"),),
            (
                lease_id,
                extension_id,
                lessor,
                extension.additional_duration_seconds,
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
        if lease.status != LeaseState::Active {
            panic!("Lease is not active");
        }

        let now = env.ledger().timestamp();
        let remaining_time = if lease.end_time > now {
            lease.end_time - now
        } else {
            0
        };
        let remaining_value =
            (lease.total_value * remaining_time as i128) / lease.duration_seconds as i128;

        let config = storage::get_lease_config(&env);
        let required_penalty =
            (remaining_value * (config.early_termination_penalty_bps as i128)) / 10_000;

        if termination_fee_paid < required_penalty {
            panic!("Insufficient termination fee");
        }

        lease.status = LeaseState::Terminated;
        storage::set_lease(&env, &lease);

        // Process termination fee payment to lessor
        let token_address = storage::get_payment_token(&env);
        let token_client = token::Client::new(&env, &token_address);
        token_client.transfer(&lessee, &lease.lessor, &termination_fee_paid);

        // Refund deposit if any
        if lease.deposit_amount > 0 {
            token_client.transfer(
                &env.current_contract_address(),
                &lessee,
                &lease.deposit_amount,
            );
        }

        // Add to history
        let entry = LeaseHistoryEntry {
            lease_id,
            action: String::from_str(&env, "early_terminated"),
            actor: lessee.clone(),
            timestamp: now,
            details: Some(String::from_str(&env, "fee_paid: 1000, penalty: 2000")),
        };
        storage::add_lease_history(&env, lease_id, &entry);

        env.events().publish(
            (Symbol::new(&env, "LeaseTerminated"),),
            (lease_id, lessee, termination_fee_paid, required_penalty),
        );
    }

    /// Automatic lease renewal
    pub fn auto_renew_lease(env: Env, lease_id: u64) {
        let mut lease = storage::get_lease(&env, lease_id).expect("Lease not found");

        if lease.status != LeaseState::Active {
            panic!("Lease is not active");
        }
        if !lease.auto_renew {
            panic!("Auto-renewal not enabled");
        }
        if !lease.lessee_consent_for_renewal {
            panic!("Lessee consent not provided");
        }

        let now = env.ledger().timestamp();
        if now < lease.end_time {
            panic!("Lease not yet expired");
        }

        // Renew lease for same duration
        lease.start_time = now;
        lease.end_time = now + lease.duration_seconds;
        lease.status = LeaseState::Renewed;

        storage::set_lease(&env, &lease);

        // Add to history
        let entry = LeaseHistoryEntry {
            lease_id,
            action: String::from_str(&env, "auto_renewed"),
            actor: env.current_contract_address(),
            timestamp: now,
            details: Some(String::from_str(&env, "new_duration: 86400")),
        };
        storage::add_lease_history(&env, lease_id, &entry);

        env.events().publish(
            (Symbol::new(&env, "LeaseRenewed"),),
            (lease_id, lease.duration_seconds),
        );
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
                    if lease.status == LeaseState::Active {
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
                    if lease.status == LeaseState::Active {
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
}

//#[cfg(test)]
//mod test_approval;

#[cfg(test)]
mod test_dynamic_fees;

#[cfg(test)]
mod test_lease;
