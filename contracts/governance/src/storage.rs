use soroban_sdk::xdr::ToXdr;
use soroban_sdk::{contracttype, Address, Bytes, Env, String, Val, Vec};

use crate::types::{
    Delegation, DelegationSnapshot, MultisigApproval, MultisigConfig, ParameterRule, Proposal,
    StorageSnapshot, TimelockConfig, TimelockEntry, Vote, VoteEscrow, VotingMechanism,
    WaitlistProposal,
};

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Admin address
    Admin,
    /// Governance token address
    GovernanceToken,
    /// Proposal counter
    ProposalCounter,
    /// Proposal by ID
    Proposal(u64),
    /// Waitlist proposal counter
    WaitlistCounter,
    /// Waitlist proposal by ID
    WaitlistProposal(u64),
    /// Vote escrow for an address
    VoteEscrow(Address),
    /// Delegation from an address
    Delegation(Address),
    /// Delegators to an address (reverse index for efficient lookup)
    DelegatorsTo(Address),
    /// Delegation snapshot for secure voting
    DelegationSnapshot(u64), // proposal_id -> snapshot
    /// Voting mechanism (linear/quadratic)
    VotingMechanism,
    /// Vote record: (proposal_id, voter)
    Vote(u64, Address),
    /// Quorum threshold (basis points, default 3000 = 30%)
    QuorumThreshold,
    /// Approval threshold (basis points, default 6600 = 66%)
    ApprovalThreshold,
    /// Minimum voting period in seconds
    MinVotingPeriod,
    /// Maximum voting period in seconds
    MaxVotingPeriod,
    /// Minimum proposal deposit
    MinProposalDeposit,
    /// Circulating voting power (cached for efficiency)
    CirculatingVotingPower,
    /// Multisig approval for a proposal
    MultisigApproval(u64),
    /// Multisig configuration
    MultisigConfig,
    /// Timelock configuration
    TimelockConfig,
    /// Timelock entry counter
    TimelockCounter,
    /// Timelock entry by ID
    TimelockEntry(u64),
    /// Parameter validation rules
    ParameterRule(String), // parameter name -> rule
    /// Storage snapshot for integrity validation
    StorageSnapshot(Address, Bytes),
}

/* ---------------- ADMIN ---------------- */

pub fn set_admin(env: &Env, admin: &Address) {
    env.storage().instance().set(&DataKey::Admin, admin);
}

pub fn get_admin(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&DataKey::Admin)
        .expect("Contract not initialized")
}

pub fn require_admin(env: &Env, caller: &Address) {
    let admin = get_admin(env);
    if caller != &admin {
        panic!("Unauthorized: caller is not admin");
    }
}

/* ---------------- GOVERNANCE TOKEN ---------------- */

pub fn set_governance_token(env: &Env, token: &Address) {
    env.storage()
        .instance()
        .set(&DataKey::GovernanceToken, token);
}

pub fn get_governance_token(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&DataKey::GovernanceToken)
        .expect("Governance token not set")
}

/* ---------------- PROPOSAL COUNTER ---------------- */

pub fn get_proposal_counter(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::ProposalCounter)
        .unwrap_or(0)
}

pub fn increment_proposal_counter(env: &Env) -> u64 {
    let counter = get_proposal_counter(env) + 1;
    env.storage()
        .instance()
        .set(&DataKey::ProposalCounter, &counter);
    counter
}

/* ---------------- WAITLIST COUNTER ---------------- */

pub fn get_waitlist_counter(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::WaitlistCounter)
        .unwrap_or(0)
}

pub fn increment_waitlist_counter(env: &Env) -> u64 {
    let counter = get_waitlist_counter(env) + 1;
    env.storage()
        .instance()
        .set(&DataKey::WaitlistCounter, &counter);
    counter
}

/* ---------------- PROPOSALS ---------------- */

pub fn set_proposal(env: &Env, proposal: &Proposal) {
    env.storage()
        .instance()
        .set(&DataKey::Proposal(proposal.proposal_id), proposal);
}

pub fn get_proposal(env: &Env, proposal_id: u64) -> Option<Proposal> {
    env.storage()
        .instance()
        .get(&DataKey::Proposal(proposal_id))
}

/* ---------------- WAITLIST PROPOSALS ---------------- */

pub fn set_waitlist_proposal(env: &Env, proposal: &WaitlistProposal) {
    env.storage()
        .instance()
        .set(&DataKey::WaitlistProposal(proposal.waitlist_id), proposal);
}

pub fn get_waitlist_proposal(env: &Env, waitlist_id: u64) -> Option<WaitlistProposal> {
    env.storage()
        .instance()
        .get(&DataKey::WaitlistProposal(waitlist_id))
}

pub fn remove_waitlist_proposal(env: &Env, waitlist_id: u64) {
    env.storage()
        .instance()
        .remove(&DataKey::WaitlistProposal(waitlist_id));
}

/* ---------------- VOTE ESCROW ---------------- */

pub fn set_vote_escrow(env: &Env, address: &Address, escrow: &VoteEscrow) {
    env.storage()
        .instance()
        .set(&DataKey::VoteEscrow(address.clone()), escrow);
}

pub fn get_vote_escrow(env: &Env, address: &Address) -> Option<VoteEscrow> {
    env.storage()
        .instance()
        .get(&DataKey::VoteEscrow(address.clone()))
}

/* ---------------- DELEGATIONS ---------------- */

pub fn set_delegation(env: &Env, delegator: &Address, delegation: &Delegation) {
    // Remove from old delegatee's list if exists
    if let Some(old_delegation) = get_delegation(env, delegator) {
        if old_delegation.delegatee != delegation.delegatee {
            remove_delegator_from_list(env, &old_delegation.delegatee, delegator);
        }
    }

    env.storage()
        .instance()
        .set(&DataKey::Delegation(delegator.clone()), delegation);

    // Add to new delegatee's list
    add_delegator_to_list(env, &delegation.delegatee, delegator);
}

pub fn get_delegation(env: &Env, delegator: &Address) -> Option<Delegation> {
    env.storage()
        .instance()
        .get(&DataKey::Delegation(delegator.clone()))
}

pub fn get_delegators_to(env: &Env, delegatee: &Address) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&DataKey::DelegatorsTo(delegatee.clone()))
        .unwrap_or_else(|| Vec::new(env))
}

pub fn set_delegation_snapshot(env: &Env, proposal_id: u64, snapshot: &DelegationSnapshot) {
    env.storage()
        .instance()
        .set(&DataKey::DelegationSnapshot(proposal_id), snapshot);
}

pub fn get_delegation_snapshot(env: &Env, proposal_id: u64) -> Option<DelegationSnapshot> {
    env.storage()
        .instance()
        .get(&DataKey::DelegationSnapshot(proposal_id))
}

pub fn set_voting_mechanism(env: &Env, mechanism: &VotingMechanism) {
    env.storage()
        .instance()
        .set(&DataKey::VotingMechanism, mechanism);
}

pub fn get_voting_mechanism(env: &Env) -> VotingMechanism {
    env.storage()
        .instance()
        .get(&DataKey::VotingMechanism)
        .unwrap_or(VotingMechanism::Linear) // Default to linear voting
}

pub(crate) fn add_delegator_to_list(env: &Env, delegatee: &Address, delegator: &Address) {
    let mut delegators = get_delegators_to(env, delegatee);

    // Use contains() for more efficient lookup instead of manual iteration
    if !delegators.contains(delegator) {
        delegators.push_back(delegator.clone());
        env.storage()
            .instance()
            .set(&DataKey::DelegatorsTo(delegatee.clone()), &delegators);
    }
}

pub(crate) fn remove_delegator_from_list(env: &Env, delegatee: &Address, delegator: &Address) {
    let delegators = get_delegators_to(env, delegatee);

    // Only proceed if delegator is actually in the list
    if delegators.contains(delegator) {
        let mut new_delegators = Vec::new(env);
        for i in 0..delegators.len() {
            let addr = delegators.get(i).unwrap();
            if addr != *delegator {
                new_delegators.push_back(addr);
            }
        }

        if !new_delegators.is_empty() {
            env.storage()
                .instance()
                .set(&DataKey::DelegatorsTo(delegatee.clone()), &new_delegators);
        } else {
            env.storage()
                .instance()
                .remove(&DataKey::DelegatorsTo(delegatee.clone()));
        }
    }
}

/* ---------------- VOTES ---------------- */

pub fn set_vote(env: &Env, proposal_id: u64, voter: &Address, vote: &Vote) {
    env.storage()
        .instance()
        .set(&DataKey::Vote(proposal_id, voter.clone()), vote);
}

pub fn get_vote(env: &Env, proposal_id: u64, voter: &Address) -> Option<Vote> {
    env.storage()
        .instance()
        .get(&DataKey::Vote(proposal_id, voter.clone()))
}

/* ---------------- CONFIGURATION ---------------- */

pub fn set_quorum_threshold(env: &Env, threshold: u32) {
    env.storage()
        .instance()
        .set(&DataKey::QuorumThreshold, &threshold);
}

pub fn get_quorum_threshold(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::QuorumThreshold)
        .unwrap_or(3000) // Default 30%
}

pub fn set_approval_threshold(env: &Env, threshold: u32) {
    env.storage()
        .instance()
        .set(&DataKey::ApprovalThreshold, &threshold);
}

pub fn get_approval_threshold(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::ApprovalThreshold)
        .unwrap_or(6600) // Default 66%
}

pub fn set_min_voting_period(env: &Env, period: u64) {
    env.storage()
        .instance()
        .set(&DataKey::MinVotingPeriod, &period);
}

pub fn get_min_voting_period(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::MinVotingPeriod)
        .unwrap_or(7 * 24 * 60 * 60) // Default 7 days
}

pub fn set_max_voting_period(env: &Env, period: u64) {
    env.storage()
        .instance()
        .set(&DataKey::MaxVotingPeriod, &period);
}

pub fn get_max_voting_period(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::MaxVotingPeriod)
        .unwrap_or(14 * 24 * 60 * 60) // Default 14 days
}

pub fn set_min_proposal_deposit(env: &Env, deposit: u128) {
    env.storage()
        .instance()
        .set(&DataKey::MinProposalDeposit, &deposit);
}

pub fn get_min_proposal_deposit(env: &Env) -> u128 {
    env.storage()
        .instance()
        .get(&DataKey::MinProposalDeposit)
        .unwrap_or(1000u128)
}

/* ---------------- CIRCULATING VOTING POWER (CACHED) ---------------- */

pub fn set_circulating_voting_power(env: &Env, power: u128) {
    env.storage()
        .instance()
        .set(&DataKey::CirculatingVotingPower, &power);
}

pub fn get_circulating_voting_power(env: &Env) -> Option<u128> {
    env.storage()
        .instance()
        .get(&DataKey::CirculatingVotingPower)
}

/* ---------------- MULTISIG GOVERNANCE ---------------- */

pub fn set_multisig_config(env: &Env, config: &MultisigConfig) {
    env.storage()
        .instance()
        .set(&DataKey::MultisigConfig, config);
}

pub fn get_multisig_config(env: &Env) -> Option<MultisigConfig> {
    env.storage().instance().get(&DataKey::MultisigConfig)
}

pub fn set_multisig_approval(env: &Env, proposal_id: u64, approval: &MultisigApproval) {
    env.storage()
        .instance()
        .set(&DataKey::MultisigApproval(proposal_id), approval);
}

pub fn get_multisig_approval(env: &Env, proposal_id: u64) -> Option<MultisigApproval> {
    env.storage()
        .instance()
        .get(&DataKey::MultisigApproval(proposal_id))
}

/// Check if an address is an authorized multisig signer
pub fn is_authorized_signer(env: &Env, signer: &Address) -> bool {
    if let Some(config) = get_multisig_config(env) {
        if !config.enabled {
            return false;
        }

        for i in 0..config.authorized_signers.len() {
            if config.authorized_signers.get(i).unwrap() == *signer {
                return true;
            }
        }
    }
    false
}

/// Check if multisig approval has reached threshold
pub fn has_reached_threshold(approval: &MultisigApproval) -> bool {
    approval.approvers.len() >= approval.required_approvals
}

/* ---------------- TIMELOCK ---------------- */

pub fn set_timelock_config(env: &Env, config: &TimelockConfig) {
    env.storage()
        .instance()
        .set(&DataKey::TimelockConfig, config);
}

pub fn get_timelock_config(env: &Env) -> Option<TimelockConfig> {
    env.storage().instance().get(&DataKey::TimelockConfig)
}

pub fn get_timelock_counter(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::TimelockCounter)
        .unwrap_or(0)
}

pub fn increment_timelock_counter(env: &Env) -> u64 {
    let counter = get_timelock_counter(env) + 1;
    env.storage()
        .instance()
        .set(&DataKey::TimelockCounter, &counter);
    counter
}

pub fn set_timelock_entry(env: &Env, entry: &TimelockEntry) {
    env.storage()
        .instance()
        .set(&DataKey::TimelockEntry(entry.entry_id), entry);
}

pub fn get_timelock_entry(env: &Env, entry_id: u64) -> Option<TimelockEntry> {
    env.storage()
        .instance()
        .get(&DataKey::TimelockEntry(entry_id))
}

/* ---------------- PARAMETER VALIDATION ---------------- */

pub fn set_parameter_rule(env: &Env, rule: &ParameterRule) {
    env.storage()
        .instance()
        .set(&DataKey::ParameterRule(rule.name.clone()), rule);
}

pub fn get_parameter_rule(env: &Env, parameter_name: String) -> Option<ParameterRule> {
    env.storage()
        .instance()
        .get(&DataKey::ParameterRule(parameter_name))
}

/* ---------------- STORAGE SNAPSHOTS ---------------- */

pub fn set_storage_snapshot(env: &Env, snapshot: &StorageSnapshot) {
    env.storage().instance().set(
        &DataKey::StorageSnapshot(
            snapshot.contract_address.clone(),
            snapshot.storage_key.clone(),
        ),
        snapshot,
    );
}

pub fn get_storage_snapshot(
    env: &Env,
    contract_address: &Address,
    storage_key: &Val,
) -> Option<StorageSnapshot> {
    env.storage().instance().get(&DataKey::StorageSnapshot(
        contract_address.clone(),
        storage_key.to_xdr(env),
    ))
}
