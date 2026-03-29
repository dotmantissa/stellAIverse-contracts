use crate::payment_types::PaymentRecord;
use soroban_sdk::{contracttype, Address, Env, String, Vec};

#[derive(Clone)]
#[contracttype]
pub enum DataKey {
    Admin,
    PaymentToken,
    PlatformFeeBps,
    RoyaltyBps,
    PreviousOwner(u64),
    PaymentCounter,
    PaymentRecord(u64),
    PaymentHistoryCount(u64),
    PaymentHistory(u64, u64), // (agent_id, history_index)
    Auction(u64),
    AuctionBid(u64, u64), // (auction_id, index)
    AuctionBidCount(u64),
    SealedCommit(u64, u64),
    SealedCommitCount(u64),
    SealedReveal(u64, u64),
    SealedRevealCount(u64),
    AuctionCounter,
    ApprovalConfig,
    ApprovalCounter,
    Approval(u64),
    ApprovalHistory(u64, u64), // (approval_id, history_index)
    FeeAdjustmentParams,
    CurrentFeeStructure,
    FeeAdjustmentHistory(u64), // fee_adjustment_id
    FeeAdjustmentCounter,
    OracleSubscriptions,
    LastOracleUpdate,
    FeeTransitionState,
    LeaseConfig,
    LeaseCounter,
    Lease(u64),
    ExtensionCounter,
    LeaseExtension(u64),
    LesseeLeases(Address, u64),
    LessorLeases(Address, u64),
    LeaseHistory(u64, u64),
}

/* ---------------- ADMIN ---------------- */

pub fn set_admin(env: &Env, admin: &Address) {
    env.storage().instance().set(&DataKey::Admin, admin);
}

#[allow(dead_code)]
pub fn require_admin(env: &Env) {
    let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
    admin.require_auth();
}

/* ---------------- PAYMENT TOKEN ---------------- */

pub fn set_payment_token(env: &Env, token: Address) {
    env.storage().instance().set(&DataKey::PaymentToken, &token);
}

pub fn get_payment_token(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&DataKey::PaymentToken)
        .unwrap()
}

pub fn set_platform_fee(env: &Env, bps: u32) {
    env.storage().instance().set(&DataKey::PlatformFeeBps, &bps);
}

pub fn get_platform_fee(env: &Env) -> u32 {
    if let Some(bps) = env
        .storage()
        .instance()
        .get::<_, u32>(&DataKey::PlatformFeeBps)
    {
        bps
    } else {
        250 // default 2.5%
    }
}

/* ---------------- ROYALTY ---------------- */

#[allow(dead_code)]
pub fn set_royalty_bps(env: &Env, bps: u32) {
    env.storage().instance().set(&DataKey::RoyaltyBps, &bps);
}

#[allow(dead_code)]
pub fn get_royalty_bps(env: &Env) -> u32 {
    env.storage().instance().get(&DataKey::RoyaltyBps).unwrap()
}

pub fn set_previous_owner(env: &Env, agent_id: u64, owner: &Address) {
    env.storage()
        .instance()
        .set(&DataKey::PreviousOwner(agent_id), owner);
}

pub fn get_previous_owner(env: &Env, agent_id: u64) -> Option<Address> {
    env.storage()
        .instance()
        .get(&DataKey::PreviousOwner(agent_id))
}

/* ---------------- PAYMENTS ---------------- */

pub fn increment_payment_counter(env: &Env) -> u64 {
    let counter: u64 = env
        .storage()
        .instance()
        .get(&DataKey::PaymentCounter)
        .unwrap_or(0);
    let updated = counter + 1;
    env.storage()
        .instance()
        .set(&DataKey::PaymentCounter, &updated);
    updated
}

pub fn set_payment_record(env: &Env, record: &PaymentRecord) {
    env.storage()
        .instance()
        .set(&DataKey::PaymentRecord(record.payment_id), record);
}

pub fn get_payment_record(env: &Env, payment_id: u64) -> Option<PaymentRecord> {
    env.storage()
        .instance()
        .get(&DataKey::PaymentRecord(payment_id))
}

pub fn add_payment_history(env: &Env, agent_id: u64, payment_id: u64) {
    let index = get_payment_history_count(env, agent_id);
    env.storage()
        .instance()
        .set(&DataKey::PaymentHistory(agent_id, index), &payment_id);
}

pub fn get_payment_history_count(env: &Env, agent_id: u64) -> u64 {
    let mut count = 0;
    while env
        .storage()
        .instance()
        .has(&DataKey::PaymentHistory(agent_id, count))
    {
        count += 1;
    }
    count
}

pub fn get_payment_history_entry(env: &Env, agent_id: u64, index: u64) -> Option<u64> {
    env.storage()
        .instance()
        .get(&DataKey::PaymentHistory(agent_id, index))
}

/* ---------------- AUCTION ---------------- */

pub fn set_auction_counter(env: &Env, counter: u64) {
    env.storage()
        .instance()
        .set(&DataKey::AuctionCounter, &counter);
}

pub fn get_auction_counter(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::AuctionCounter)
        .unwrap_or(0)
}

pub fn increment_auction_counter(env: &Env) -> u64 {
    let counter = get_auction_counter(env) + 1;
    set_auction_counter(env, counter);
    counter
}

pub fn set_auction(env: &Env, auction: &stellai_lib::Auction) {
    env.storage()
        .instance()
        .set(&DataKey::Auction(auction.auction_id), auction);
}

pub fn get_auction(env: &Env, auction_id: u64) -> Option<stellai_lib::Auction> {
    env.storage().instance().get(&DataKey::Auction(auction_id))
}

pub fn add_bid_history(env: &Env, auction_id: u64, bid: &stellai_lib::BidRecord) {
    let index = get_bid_history_count(env, auction_id);
    env.storage()
        .instance()
        .set(&DataKey::AuctionBid(auction_id, index), bid);
    env.storage()
        .instance()
        .set(&DataKey::AuctionBidCount(auction_id), &(index + 1));
}

pub fn get_bid_history_count(env: &Env, auction_id: u64) -> u64 {
    env.storage()
        .instance()
        .get::<_, u64>(&DataKey::AuctionBidCount(auction_id))
        .unwrap_or(0)
}

pub fn get_bid_history_entry(
    env: &Env,
    auction_id: u64,
    index: u64,
) -> Option<stellai_lib::BidRecord> {
    env.storage()
        .instance()
        .get(&DataKey::AuctionBid(auction_id, index))
}

pub fn add_sealed_commit(env: &Env, auction_id: u64, commit: &stellai_lib::SealedCommit) {
    let index = get_sealed_commit_count(env, auction_id);
    env.storage()
        .instance()
        .set(&DataKey::SealedCommit(auction_id, index), commit);
    env.storage()
        .instance()
        .set(&DataKey::SealedCommitCount(auction_id), &(index + 1));
}

pub fn get_sealed_commit_count(env: &Env, auction_id: u64) -> u64 {
    env.storage()
        .instance()
        .get::<_, u64>(&DataKey::SealedCommitCount(auction_id))
        .unwrap_or(0)
}

pub fn get_sealed_commit_entry(
    env: &Env,
    auction_id: u64,
    index: u64,
) -> Option<stellai_lib::SealedCommit> {
    env.storage()
        .instance()
        .get(&DataKey::SealedCommit(auction_id, index))
}

pub fn add_sealed_reveal(env: &Env, auction_id: u64, reveal: &stellai_lib::SealedReveal) {
    let index = get_sealed_reveal_count(env, auction_id);
    env.storage()
        .instance()
        .set(&DataKey::SealedReveal(auction_id, index), reveal);
    env.storage()
        .instance()
        .set(&DataKey::SealedRevealCount(auction_id), &(index + 1));
}

pub fn get_sealed_reveal_count(env: &Env, auction_id: u64) -> u64 {
    env.storage()
        .instance()
        .get::<_, u64>(&DataKey::SealedRevealCount(auction_id))
        .unwrap_or(0)
}

pub fn get_sealed_reveal_entry(
    env: &Env,
    auction_id: u64,
    index: u64,
) -> Option<stellai_lib::SealedReveal> {
    env.storage()
        .instance()
        .get(&DataKey::SealedReveal(auction_id, index))
}

/* ---------------- HELPERS ---------------- */

#[allow(dead_code)]
pub fn calculate_royalty(price: i128, bps: u32) -> i128 {
    (price * (bps as i128)) / 10_000
}

/* ---------------- APPROVAL ---------------- */

pub fn set_approval_config(env: &Env, config: &stellai_lib::ApprovalConfig) {
    env.storage()
        .instance()
        .set(&DataKey::ApprovalConfig, config);
}

pub fn get_approval_config(env: &Env) -> stellai_lib::ApprovalConfig {
    env.storage()
        .instance()
        .get(&DataKey::ApprovalConfig)
        .unwrap_or_else(|| stellai_lib::ApprovalConfig {
            threshold: stellai_lib::DEFAULT_APPROVAL_THRESHOLD,
            approvers_required: stellai_lib::DEFAULT_APPROVERS_REQUIRED,
            total_approvers: stellai_lib::DEFAULT_TOTAL_APPROVERS,
            ttl_seconds: stellai_lib::DEFAULT_APPROVAL_TTL_SECONDS,
        })
}

pub fn get_approval_counter(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::ApprovalCounter)
        .unwrap_or(0)
}

pub fn set_approval_counter(env: &Env, counter: u64) {
    env.storage()
        .instance()
        .set(&DataKey::ApprovalCounter, &counter);
}

pub fn increment_approval_counter(env: &Env) -> u64 {
    let counter = get_approval_counter(env) + 1;
    set_approval_counter(env, counter);
    counter
}

pub fn set_approval(env: &Env, approval: &stellai_lib::Approval) {
    env.storage()
        .instance()
        .set(&DataKey::Approval(approval.approval_id), approval);
}

pub fn get_approval(env: &Env, approval_id: u64) -> Option<stellai_lib::Approval> {
    env.storage()
        .instance()
        .get(&DataKey::Approval(approval_id))
}

pub fn add_approval_history(env: &Env, approval_id: u64, history: &stellai_lib::ApprovalHistory) {
    let history_index = get_approval_history_count(env, approval_id);
    env.storage().instance().set(
        &DataKey::ApprovalHistory(approval_id, history_index),
        history,
    );
}

pub fn get_approval_history_count(env: &Env, approval_id: u64) -> u64 {
    let mut count = 0;
    while env
        .storage()
        .instance()
        .has(&DataKey::ApprovalHistory(approval_id, count))
    {
        count += 1;
    }
    count
}

pub fn get_approval_history(
    env: &Env,
    approval_id: u64,
    index: u64,
) -> Option<stellai_lib::ApprovalHistory> {
    env.storage()
        .instance()
        .get(&DataKey::ApprovalHistory(approval_id, index))
}

#[allow(dead_code)]
pub fn delete_approval(env: &Env, approval_id: u64) {
    env.storage()
        .instance()
        .remove(&DataKey::Approval(approval_id));

    // Clean up approval history
    let mut history_index = 0;
    while env
        .storage()
        .instance()
        .has(&DataKey::ApprovalHistory(approval_id, history_index))
    {
        env.storage()
            .instance()
            .remove(&DataKey::ApprovalHistory(approval_id, history_index));
        history_index += 1;
    }
}
/* ---------------- DYNAMIC FEE ADJUSTMENT ---------------- */

#[derive(Clone)]
#[contracttype]
pub struct FeeAdjustmentParams {
    pub base_marketplace_fee: u32, // basis points
    pub congestion_oracle_id: Address,
    pub utilization_oracle_id: Address,
    pub volatility_oracle_id: Address,
    pub min_fee_bps: u32,
    pub max_fee_bps: u32,
    pub adjustment_window: u64,
}

#[derive(Clone)]
#[contracttype]
pub struct FeeCalculationInput {
    pub network_congestion: i128,   // 0-100
    pub platform_utilization: i128, // 0-100
    pub market_volatility: i128,    // 0-100
}

#[derive(Clone)]
#[contracttype]
pub struct FeeStructure {
    pub marketplace_fee_bps: u32,
    pub calculated_at: u64,
    pub congestion_factor: i128,
    pub utilization_factor: i128,
    pub volatility_factor: i128,
}

#[derive(Clone)]
#[contracttype]
pub struct FeeAdjustmentHistory {
    pub adjustment_id: u64,
    pub timestamp: u64,
    pub old_fee_bps: u32,
    pub new_fee_bps: u32,
    pub congestion_value: i128,
    pub utilization_value: i128,
    pub volatility_value: i128,
    pub adjustment_reason: String,
}

#[derive(Clone)]
#[contracttype]
pub struct FeeTransitionState {
    pub is_transitioning: bool,
    pub start_fee_bps: u32,
    pub target_fee_bps: u32,
    pub transition_start: u64,
    pub transition_steps: u32,
    pub current_step: u32,
}

pub fn set_fee_adjustment_params(env: &Env, params: &FeeAdjustmentParams) {
    env.storage()
        .instance()
        .set(&DataKey::FeeAdjustmentParams, params);
}

pub fn get_fee_adjustment_params(env: &Env) -> Option<FeeAdjustmentParams> {
    env.storage().instance().get(&DataKey::FeeAdjustmentParams)
}

pub fn set_current_fee_structure(env: &Env, fee_structure: &FeeStructure) {
    env.storage()
        .instance()
        .set(&DataKey::CurrentFeeStructure, fee_structure);
}

pub fn get_current_fee_structure(env: &Env) -> Option<FeeStructure> {
    env.storage().instance().get(&DataKey::CurrentFeeStructure)
}

pub fn get_fee_adjustment_counter(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::FeeAdjustmentCounter)
        .unwrap_or(0)
}

pub fn increment_fee_adjustment_counter(env: &Env) -> u64 {
    let counter = get_fee_adjustment_counter(env) + 1;
    env.storage()
        .instance()
        .set(&DataKey::FeeAdjustmentCounter, &counter);
    counter
}

pub fn add_fee_adjustment_history(env: &Env, history: &FeeAdjustmentHistory) {
    env.storage().instance().set(
        &DataKey::FeeAdjustmentHistory(history.adjustment_id),
        history,
    );
}

pub fn get_fee_adjustment_history(env: &Env, adjustment_id: u64) -> Option<FeeAdjustmentHistory> {
    env.storage()
        .instance()
        .get(&DataKey::FeeAdjustmentHistory(adjustment_id))
}

pub fn set_oracle_subscriptions(env: &Env, oracle_ids: &Vec<Address>) {
    env.storage()
        .instance()
        .set(&DataKey::OracleSubscriptions, oracle_ids);
}

pub fn get_oracle_subscriptions(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&DataKey::OracleSubscriptions)
        .unwrap_or_else(|| Vec::new(env))
}

pub fn set_last_oracle_update(env: &Env, timestamp: u64) {
    env.storage()
        .instance()
        .set(&DataKey::LastOracleUpdate, &timestamp);
}

pub fn get_last_oracle_update(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::LastOracleUpdate)
        .unwrap_or(0)
}

pub fn set_fee_transition_state(env: &Env, state: &FeeTransitionState) {
    env.storage()
        .instance()
        .set(&DataKey::FeeTransitionState, state);
}

pub fn get_fee_transition_state(env: &Env) -> Option<FeeTransitionState> {
    env.storage().instance().get(&DataKey::FeeTransitionState)
}

/* ---------------- LEASE CONFIGURATION ---------------- */

#[derive(Clone)]
#[contracttype]
pub struct LeaseConfig {
    pub deposit_bps: u32,
    pub early_termination_penalty_bps: u32,
}

pub fn set_lease_config(env: &Env, config: &LeaseConfig) {
    env.storage().instance().set(&DataKey::LeaseConfig, config);
}

pub fn get_lease_config(env: &Env) -> LeaseConfig {
    env.storage()
        .instance()
        .get(&DataKey::LeaseConfig)
        .unwrap_or_else(|| LeaseConfig {
            deposit_bps: stellai_lib::DEFAULT_LEASE_DEPOSIT_BPS,
            early_termination_penalty_bps: stellai_lib::DEFAULT_EARLY_TERMINATION_PENALTY_BPS,
        })
}

/* ---------------- LEASE MANAGEMENT ---------------- */

pub fn increment_lease_counter(env: &Env) -> u64 {
    let counter: u64 = env
        .storage()
        .instance()
        .get(&DataKey::LeaseCounter)
        .unwrap_or(0);
    let updated = counter + 1;
    env.storage()
        .instance()
        .set(&DataKey::LeaseCounter, &updated);
    updated
}

pub fn get_lease_counter(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::LeaseCounter)
        .unwrap_or(0)
}

pub fn set_lease(env: &Env, lease: &stellai_lib::LeaseData) {
    env.storage()
        .instance()
        .set(&DataKey::Lease(lease.lease_id), lease);
}

pub fn get_lease(env: &Env, lease_id: u64) -> Option<stellai_lib::LeaseData> {
    env.storage().instance().get(&DataKey::Lease(lease_id))
}

pub fn increment_extension_counter(env: &Env) -> u64 {
    let counter: u64 = env
        .storage()
        .instance()
        .get(&DataKey::ExtensionCounter)
        .unwrap_or(0);
    let updated = counter + 1;
    env.storage()
        .instance()
        .set(&DataKey::ExtensionCounter, &updated);
    updated
}

pub fn set_lease_extension(env: &Env, extension: &stellai_lib::LeaseExtensionRequest) {
    env.storage()
        .instance()
        .set(&DataKey::LeaseExtension(extension.extension_id), extension);
}

pub fn get_lease_extension(
    env: &Env,
    extension_id: u64,
) -> Option<stellai_lib::LeaseExtensionRequest> {
    env.storage()
        .instance()
        .get(&DataKey::LeaseExtension(extension_id))
}

pub fn lessee_leases_append(env: &Env, lessee: &Address, lease_id: u64) {
    let mut count = 0;
    while env
        .storage()
        .instance()
        .has(&DataKey::LesseeLeases(lessee.clone(), count))
    {
        count += 1;
    }
    env.storage()
        .instance()
        .set(&DataKey::LesseeLeases(lessee.clone(), count), &lease_id);
}

pub fn lessor_leases_append(env: &Env, lessor: &Address, lease_id: u64) {
    let mut count = 0;
    while env
        .storage()
        .instance()
        .has(&DataKey::LessorLeases(lessor.clone(), count))
    {
        count += 1;
    }
    env.storage()
        .instance()
        .set(&DataKey::LessorLeases(lessor.clone(), count), &lease_id);
}

pub fn get_lessee_lease_count(env: &Env, lessee: &Address) -> u64 {
    let mut count = 0;
    while env
        .storage()
        .instance()
        .has(&DataKey::LesseeLeases(lessee.clone(), count))
    {
        count += 1;
    }
    count
}

pub fn get_lessor_lease_count(env: &Env, lessor: &Address) -> u64 {
    let mut count = 0;
    while env
        .storage()
        .instance()
        .has(&DataKey::LessorLeases(lessor.clone(), count))
    {
        count += 1;
    }
    count
}

pub fn get_lessee_lease(env: &Env, lessee: &Address, index: u64) -> Option<u64> {
    env.storage()
        .instance()
        .get(&DataKey::LesseeLeases(lessee.clone(), index))
}

pub fn get_lessor_lease(env: &Env, lessor: &Address, index: u64) -> Option<u64> {
    env.storage()
        .instance()
        .get(&DataKey::LessorLeases(lessor.clone(), index))
}

pub fn add_lease_history(env: &Env, lease_id: u64, history: &stellai_lib::LeaseHistoryEntry) {
    let history_index = get_lease_history_count(env, lease_id);
    env.storage()
        .instance()
        .set(&DataKey::LeaseHistory(lease_id, history_index), history);
}

pub fn get_lease_history_count(env: &Env, lease_id: u64) -> u64 {
    let mut count = 0;
    while env
        .storage()
        .instance()
        .has(&DataKey::LeaseHistory(lease_id, count))
    {
        count += 1;
    }
    count
}

pub fn get_lease_history(
    env: &Env,
    lease_id: u64,
    index: u64,
) -> Option<stellai_lib::LeaseHistoryEntry> {
    env.storage()
        .instance()
        .get(&DataKey::LeaseHistory(lease_id, index))
}
