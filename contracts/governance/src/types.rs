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

/// Voting mechanism types
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VotingMechanism {
    /// Linear voting (1 token = 1 vote)
    Linear,
    /// Quadratic voting (sqrt of tokens)
    Quadratic,
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
    /// Timestamp when delegation was created
    pub created_at: u64,
    /// Optional expiry timestamp for delegation
    pub expires_at: Option<u64>,
    /// Whether delegation is currently active
    pub active: bool,
}

/// Delegation snapshot for secure delegation
#[contracttype]
#[derive(Clone, Debug)]
pub struct DelegationSnapshot {
    /// Block number when snapshot was taken
    pub block_number: u64,
    /// Total delegated power at snapshot time
    pub total_delegated_power: u128,
    /// Delegators and their amounts at snapshot time
    pub delegator_powers: Vec<(Address, u128)>,
}

/// Vote record for a user on a proposal
#[contracttype]
#[derive(Clone, Debug)]
pub struct Vote {
    pub proposal_id: u64,
    pub voter: Address,
    pub vote_type: VoteType,
    pub weight: u128,
    pub voting_power_used: u128, // Raw voting power before quadratic calculation
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

/// Multi-signature approval for high-risk governance actions
#[contracttype]
#[derive(Clone, Debug)]
pub struct MultisigApproval {
    /// Proposal ID being approved
    pub proposal_id: u64,
    /// List of approvers who have signed
    pub approvers: Vec<Address>,
    /// Required number of approvals (threshold)
    pub required_approvals: u32,
    /// Timestamp when approval was created
    pub created_at: u64,
    /// Timestamp when approval expires
    pub expires_at: u64,
    /// Whether the approval has been executed
    pub executed: bool,
}

/// Configuration for multisig governance
#[contracttype]
#[derive(Clone, Debug)]
pub struct MultisigConfig {
    /// Minimum number of signatures required
    pub threshold: u32,
    /// List of authorized signers
    pub authorized_signers: Vec<Address>,
    /// Approval validity period in seconds
    pub approval_validity_secs: u64,
    /// Whether multisig is enabled for this contract
    pub enabled: bool,
}

/// Timelock queue entry for governance parameter updates
#[contracttype]
#[derive(Clone, Debug)]
pub struct TimelockEntry {
    /// Unique identifier for the timelock entry
    pub entry_id: u64,
    /// Associated proposal ID
    pub proposal_id: u64,
    /// Target contract address
    pub target_contract: Address,
    /// Target function name
    pub target_function: Symbol,
    /// Arguments for the target function
    pub target_args: Vec<Val>,
    /// When the entry was queued (timestamp)
    pub queued_at: u64,
    /// When the entry becomes executable (timestamp)
    pub executable_at: u64,
    /// Whether the entry has been executed
    pub executed: bool,
    /// Whether the entry was cancelled
    pub cancelled: bool,
    /// Who queued this entry
    pub queued_by: Address,
}

/// Timelock configuration
#[contracttype]
#[derive(Clone, Debug)]
pub struct TimelockConfig {
    /// Minimum delay before execution (in seconds)
    pub min_delay: u64,
    /// Maximum delay before execution (in seconds)
    pub max_delay: u64,
    /// Default delay if not specified (in seconds)
    pub default_delay: u64,
    /// Whether timelock is enabled
    pub enabled: bool,
}

/// Parameter validation rules
#[contracttype]
#[derive(Clone, Debug)]
pub struct ParameterRule {
    /// Parameter name
    pub name: String,
    /// Minimum allowed value (as string)
    pub min_value: Option<String>,
    /// Maximum allowed value (as string)
    pub max_value: Option<String>,
    /// Allowed values (enum-like)
    pub allowed_values: Option<Vec<String>>,
    /// Whether this parameter requires timelock
    pub requires_timelock: bool,
    /// Parameter type for validation
    pub param_type: ParameterType,
}

/// Parameter types for validation
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParameterType {
    /// Unsigned integer
    U64,
    /// Unsigned 128-bit integer
    U128,
    /// Signed integer
    I64,
    /// Boolean
    Bool,
    /// String
    String,
    /// Address
    Address,
    /// Symbol
    Symbol,
}

/// Storage snapshot for integrity validation
#[contracttype]
#[derive(Clone, Debug)]
pub struct StorageSnapshot {
    /// Contract address
    pub contract_address: Address,
    /// Storage key
    pub storage_key: Val,
    /// Value before change
    pub before_value: Option<Val>,
    /// Value after change
    pub after_value: Option<Val>,
    /// Timestamp of snapshot
    pub timestamp: u64,
}
