#![no_std]
use soroban_sdk::{contracttype, Address, String, Symbol, Val, Vec};

/// Vote types for proposals
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VoteType {
    For,
    Against,
    Abstain,
}

/// Types of proposals that can be created
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProposalType {
    /// Parameter change (e.g., fee adjustments)
    ParameterChange,
    /// Contract upgrade proposal
    ContractUpgrade,
    /// Emergency pause/unpause
    EmergencyPause,
}

/// Status of a proposal
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProposalStatus {
    /// Proposal created, waiting for voting to start
    Pending,
    /// Voting period is active
    Active,
    /// Voting period ended, proposal passed thresholds
    Passed,
    /// Voting period ended, proposal failed thresholds
    Failed,
    /// Proposal executed
    Executed,
    /// Proposal cancelled
    Cancelled,
}

/// Proposal structure
#[contracttype]
#[derive(Clone, Debug)]
pub struct Proposal {
    pub proposal_id: u64,
    pub title: String,
    pub description: String,
    pub proposer: Address,
    pub voting_starts: u64,
    pub voting_ends: u64,
    pub proposal_type: ProposalType,
    /// Parameters for proposal execution (workaround: using has_parameters flag instead of Option)
    /// For ParameterChange: contains the parameter name and new value
    /// For ContractUpgrade: contains the new contract address
    /// For EmergencyPause: contains pause/unpause flag
    pub has_parameters: bool,
    pub parameters: ProposalParameters,
    pub votes_for: u128,
    pub votes_against: u128,
    pub votes_abstain: u128,
    pub status: ProposalStatus,
    /// Target contract address for execution (if applicable)
    pub target_contract: Option<Address>,
    /// Function to call on target contract (if applicable)
    pub target_function: Option<Symbol>,
    /// Arguments for the target function (if applicable)
    pub target_args: Option<Vec<Val>>,
}

/// Parameters for proposal execution
#[contracttype]
#[derive(Clone, Debug)]
pub struct ProposalParameters {
    /// Parameter name (e.g., "fee_rate", "quorum_threshold")
    pub name: String,
    /// Parameter value (serialized as string, can be parsed by executor)
    pub value: String,
}

/// Vote escrow structure
#[contracttype]
#[derive(Clone, Debug)]
pub struct VoteEscrow {
    /// Amount of tokens locked
    pub amount: u128,
    /// Timestamp when lock ends
    pub lock_end: u64,
    /// Voting power multiplier (2x - 4x based on lock duration)
    pub multiplier: u32,
}

/// Delegation information
#[contracttype]
#[derive(Clone, Debug)]
pub struct Delegation {
    /// Address to which voting power is delegated
    pub delegatee: Address,
    /// Amount of voting power delegated
    pub amount: u128,
}

/// Vote record for a user on a proposal
#[contracttype]
#[derive(Clone, Debug)]
pub struct Vote {
    pub proposal_id: u64,
    pub voter: Address,
    pub vote_type: VoteType,
    pub weight: u128,
    pub timestamp: u64,
}

/// Waitlist for governance proposals
#[contracttype]
#[derive(Clone, Debug)]
pub struct WaitlistProposal {
    pub waitlist_id: u64,
    pub proposer: Address,
    pub title: String,
    pub description: String,
    pub proposal_type: ProposalType,
    pub parameters: ProposalParameters,
    pub deposit_amount: u128,
    pub submitted_at: u64,
}
