use soroban_sdk::{contracttype, Address, Bytes, Map, String, Symbol, Val, Vec};

/// Oracle data entry
#[derive(Clone, Debug)]
#[contracttype]
pub struct OracleData {
    pub key: Symbol,
    pub value: i128,
    pub timestamp: u64,
    pub provider: Address,
    pub signature: Option<String>,
    pub source: Option<String>,
}

/// Represents an agent's metadata and state
#[derive(Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[contracttype]
pub struct Agent {
    pub id: u64,
    pub owner: Address,
    pub name: String,
    pub model_hash: String,
    pub metadata_cid: String,
    pub capabilities: Vec<String>,
    pub evolution_level: u32,
    pub created_at: u64,
    pub updated_at: u64,
    pub nonce: u64,
    pub escrow_locked: bool,
    pub escrow_holder: Option<Address>,
}

/// Rate limiting window for security protection
#[derive(Clone, Copy)]
#[contracttype]
pub struct RateLimit {
    pub window_seconds: u64,
    pub max_operations: u32,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BehaviorProfile {
    pub agent_id: u64,
    pub operations_per_hour: Vec<u32>, // last 24 hours
    pub avg_execution_cost: i128,
    pub action_type_distribution: Vec<(String, u32)>,
    pub last_updated: u64,
    pub learning_count: u32,
    pub profile_frozen: bool,
}

#[contracttype]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThresholdKeyShare {
    pub agent_id: u64,
    pub share_holder: Address,
    pub share_index: u32,
    pub x_coordinate: u32,
    pub y_coordinate_encrypted: Bytes,
    pub commitment: Bytes,
    pub created_at: u64,
}

#[contracttype]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProposalStatus {
    Pending,
    Executed,
    Cancelled,
}

#[contracttype]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThresholdProposal {
    pub proposal_id: u64,
    pub agent_id: u64,
    pub action_data: Bytes,
    pub proposer: Address,
    pub threshold_m: u32,
    pub signers: Vec<Address>,
    pub status: ProposalStatus,
    pub created_at: u64,
}

#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum AnomalySeverity {
    Low = 0,
    Medium = 1,
    High = 2,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnomalyScore {
    pub score: i128, // basis points: 300 = 3.00
    pub anomaly_reason: String,
    pub severity: AnomalySeverity,
}

/// Represents a marketplace listing
#[derive(Clone)]
#[contracttype]
pub struct Listing {
    pub listing_id: u64,
    pub agent_id: u64,
    pub seller: Address,
    pub price: i128,
    pub listing_type: ListingType,
    pub active: bool,
    pub created_at: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[contracttype]
#[repr(u32)]
pub enum ListingType {
    Sale = 0,
    Lease = 1,
    Auction = 2,
}

/// Represents an evolution/upgrade request
#[derive(Clone)]
#[contracttype]
pub struct EvolutionRequest {
    pub request_id: u64,
    pub agent_id: u64,
    pub owner: Address,
    pub stake_amount: i128,
    pub status: EvolutionStatus,
    pub created_at: u64,
    pub completed_at: Option<u64>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[contracttype]
#[repr(u32)]
pub enum EvolutionStatus {
    Pending = 0,
    InProgress = 1,
    Completed = 2,
    Failed = 3,
}

/// Royalty information for marketplace transactions
#[derive(Clone, Debug)]
#[contracttype]
pub struct RoyaltyInfo {
    pub recipient: Address,
    pub fee: u32,
}

/// Wrapper enum so Option<RoyaltyInfo> works inside contracttype structs
#[derive(Clone, Debug)]
#[contracttype]
pub enum OptionalRoyaltyInfo {
    None,
    Some(RoyaltyInfo),
}

#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum AuctionType {
    English = 0,
    Dutch = 1,
    Sealed = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[contracttype]
#[repr(u32)]
pub enum AuctionStatus {
    Created = 0,
    Active = 1,
    Ended = 2,
    Cancelled = 3,
    Won = 4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[contracttype]
#[repr(u32)]
pub enum PriceDecay {
    Linear = 0,
    Exponential = 1,
}

#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DutchAuctionConfig {
    pub start_price: i128,
    pub reserve_price: i128,
    pub start_time: u64,
    pub end_time: u64,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Auction {
    pub auction_id: u64,
    pub agent_id: u64,
    pub seller: Address,
    pub auction_type: AuctionType,
    pub start_price: i128,
    pub reserve_price: i128,
    pub current_price: i128,
    pub highest_bidder: Option<Address>,
    pub highest_bid: i128,
    pub start_time: u64,
    pub end_time: u64,
    pub min_bid_increment_bps: u32,
    pub status: AuctionStatus,
    pub dutch_config: Option<Bytes>,
    pub sealed_commit_end: Option<u64>,
    pub sealed_reveal_end: Option<u64>,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SealedCommit {
    pub bidder: Address,
    pub commitment: Bytes,
    pub deposit: i128,
    pub timestamp: u64,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SealedReveal {
    pub bidder: Address,
    pub amount: i128,
    pub nonce: String,
    pub deposit: i128,
    pub timestamp: u64,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BidRecord {
    pub bidder: Address,
    pub amount: i128,
    pub timestamp: u64,
}

/// Multi-signature approval configuration for high-value sales
#[derive(Clone)]
#[contracttype]
pub struct ApprovalConfig {
    pub threshold: i128,
    pub approvers_required: u32,
    pub total_approvers: u32,
    pub ttl_seconds: u64,
}

/// Approval status for high-value transactions
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[contracttype]
#[repr(u32)]
pub enum ApprovalStatus {
    Pending = 0,
    Approved = 1,
    Rejected = 2,
    Expired = 3,
    Executed = 4,
}

/// Multi-signature approval for high-value agent sales
#[derive(Clone)]
#[contracttype]
pub struct Approval {
    pub approval_id: u64,
    pub listing_id: Option<u64>,
    pub auction_id: Option<u64>,
    pub buyer: Address,
    pub price: i128,
    pub proposed_at: u64,
    pub expires_at: u64,
    pub status: ApprovalStatus,
    pub required_approvals: u32,
    pub approvers: Vec<Address>,
    pub approvals_received: Vec<Address>,
    pub rejections_received: Vec<Address>,
    pub rejection_reasons: Vec<String>,
}

/// Approval history entry for audit trail
#[derive(Clone)]
#[contracttype]
pub struct ApprovalHistory {
    pub approval_id: u64,
    pub action: String,
    pub actor: Address,
    pub timestamp: u64,
    pub reason: Option<String>,
}

pub struct EvolutionAttestation {
    pub request_id: u64,
    pub agent_id: u64,
    pub oracle_provider: Address,
    pub new_model_hash: String,
    pub attestation_data: Bytes,
    pub signature: Bytes,
    pub timestamp: u64,
    pub nonce: u64,
}

/// State of a lease in its lifecycle.
#[derive(Clone, Copy, PartialEq, Eq)]
#[contracttype]
#[repr(u32)]
pub enum LeaseState {
    Active = 0,
    ExtensionRequested = 1,
    Terminated = 2,
    Renewed = 3,
}

/// Full lease record: duration, renewal terms, termination conditions, deposit.
#[derive(Clone)]
#[contracttype]
pub struct LeaseData {
    pub lease_id: u64,
    pub agent_id: u64,
    pub listing_id: u64,
    pub lessor: Address,
    pub lessee: Address,
    pub start_time: u64,
    pub end_time: u64,
    pub duration_seconds: u64,
    pub deposit_amount: i128,
    pub total_value: i128,
    pub auto_renew: bool,
    pub lessee_consent_for_renewal: bool,
    pub status: LeaseState,
    pub pending_extension_id: Option<u64>,
}

/// A request to extend an active lease by additional duration.
#[derive(Clone)]
#[contracttype]
pub struct LeaseExtensionRequest {
    pub extension_id: u64,
    pub lease_id: u64,
    pub additional_duration_seconds: u64,
    pub requested_at: u64,
    pub approved: bool,
}

/// Single entry in lease history (for lessee/lessor audit).
#[derive(Clone)]
#[contracttype]
pub struct LeaseHistoryEntry {
    pub lease_id: u64,
    pub action: String,
    pub actor: Address,
    pub timestamp: u64,
    pub details: Option<String>,
}

/// Transaction status in the two-phase commit protocol
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[contracttype]
#[repr(u32)]
pub enum TransactionStatus {
    Initiated = 0,
    Preparing = 1,
    Prepared = 2,
    Committing = 3,
    Committed = 4,
    RollingBack = 5,
    RolledBack = 6,
    Failed = 7,
    TimedOut = 8,
}

/// Individual step in an atomic transaction
#[derive(Clone)]
#[contracttype]
pub struct TransactionStep {
    pub step_id: u32,
    pub contract: Address,
    pub function: Symbol,
    pub args: Vec<Val>,
    pub depends_on: Option<u32>,
    pub rollback_contract: Option<Address>,
    pub rollback_function: Option<Symbol>,
    pub rollback_args: Option<Vec<Val>>,
    pub executed: bool,
    pub result: Option<String>,
}

/// Atomic transaction containing multiple coordinated steps
#[derive(Clone)]
#[contracttype]
pub struct AtomicTransaction {
    pub transaction_id: u64,
    pub initiator: Address,
    pub steps: Vec<TransactionStep>,
    pub status: TransactionStatus,
    pub created_at: u64,
    pub deadline: u64,
    pub prepared_steps: Vec<u32>,
    pub executed_steps: Vec<u32>,
    pub failure_reason: Option<String>,
}

/// Journal entry for transaction recovery and replay
#[derive(Clone)]
#[contracttype]
pub struct TransactionJournalEntry {
    pub transaction_id: u64,
    pub step_id: u32,
    pub action: String,
    pub timestamp: u64,
    pub success: bool,
    pub error_message: Option<String>,
    pub state_snapshot: Option<String>,
}

/// Transaction progress event for monitoring
#[derive(Clone)]
#[contracttype]
pub struct TransactionEvent {
    pub transaction_id: u64,
    pub event_type: String,
    pub step_id: Option<u32>,
    pub timestamp: u64,
    pub details: Option<String>,
}

/// DID Document structure following W3C DID specification
#[derive(Clone, Debug)]
#[contracttype]
pub struct DIDDocument {
    pub did: String,
    pub controller: Address,
    pub verification_methods: Vec<DIDVerificationMethod>,
    pub authentication: Vec<String>,
    pub assertion_method: Vec<String>,
    pub key_agreement: Vec<String>,
    pub capability_invocation: Vec<String>,
    pub capability_delegation: Vec<String>,
    pub service: Vec<DIDService>,
    pub created: u64,
    pub updated: u64,
    pub version_id: u64,
}

#[derive(Clone, Debug)]
#[contracttype]
pub struct DIDVerificationMethod {
    pub id: String,
    pub type_: String,
    pub controller: String,
    pub public_key: Bytes,
    pub created: u64,
}

#[derive(Clone, Debug)]
#[contracttype]
pub struct DIDService {
    pub id: String,
    pub type_: String,
    pub service_endpoint: String,
    pub created: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[contracttype]
#[repr(u32)]
pub enum DIDStatus {
    Active = 0,
    Suspended = 1,
    Revoked = 2,
}

#[derive(Clone)]
#[contracttype]
pub struct DIDRecord {
    pub document: DIDDocument,
    pub status: DIDStatus,
    pub nonce: u64,
    pub last_activity: u64,
}

/// Verifiable Credential structure following W3C VC specification
#[derive(Clone, Debug)]
#[contracttype]
pub struct VCProof {
    pub type_: String,
    pub created: u64,
    pub proof_purpose: String,
    pub verification_method: String,
    pub challenge: Option<String>,
    pub domain: Option<String>,
    pub jws: Option<String>,
}

/// Wrapper enum so Option<VCProof> works inside contracttype structs
#[derive(Clone, Debug)]
#[contracttype]
pub enum OptionalVCProof {
    None,
    Some(VCProof),
}

#[derive(Clone, Debug)]
#[contracttype]
pub struct CredentialStatus {
    pub id: String,
    pub type_: String,
    pub status: String,
    pub revoked: bool,
    pub suspended: bool,
    pub revocation_reason: Option<String>,
    pub suspension_reason: Option<String>,
    pub effective_date: u64,
}

#[derive(Clone, Debug)]
#[contracttype]
pub struct VerifiableCredential {
    pub id: String,
    pub credential_id: u64,
    pub issuer: Address,
    pub subject: String, // DID of the subject
    pub credential_type: Vec<String>,
    pub credential_schema: String,
    pub credential_status: CredentialStatus,
    pub issuance_date: u64,
    pub expiration_date: Option<u64>,
    pub credential_subject: Map<String, String>,
    pub proof: OptionalVCProof,
    pub non_revoked: bool,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Clone, Debug)]
#[contracttype]
pub struct CredentialSchema {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: Address,
    pub fields: Vec<SchemaField>,
    pub created_at: u64,
    pub required_fields: Vec<String>,
}

#[derive(Clone, Debug)]
#[contracttype]
pub struct SchemaField {
    pub name: String,
    pub type_: String,
    pub required: bool,
    pub description: Option<String>,
    pub validation: Option<String>,
}

#[derive(Clone, Debug)]
#[contracttype]
pub struct SelectiveDisclosure {
    pub disclosure_id: u64,
    pub credential_id: u64,
    pub verifier: Address,
    pub subject: String,
    pub disclosed_fields: Vec<String>,
    pub nonce: String,
    pub created_at: u64,
    pub expires_at: u64,
    pub presentation_hash: String,
    pub verified: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[contracttype]
#[repr(u32)]
pub enum CredentialType {
    KYC = 0,
    AML = 1,
    Accreditation = 2,
    Reputation = 3,
    License = 4,
    Education = 5,
    Employment = 6,
    Certification = 7,
    AgeVerification = 8,
    AddressVerification = 9,
    IdentityVerification = 10,
}

/// Compliance integration types
#[derive(Clone, Debug)]
#[contracttype]
pub struct ComplianceReport {
    pub report_id: u64,
    pub entity_did: String,
    pub compliance_type: ComplianceType,
    pub status: ComplianceStatus,
    pub score: u32,
    pub risk_level: RiskLevel,
    pub findings: Vec<ComplianceFinding>,
    pub issued_by: Address,
    pub issued_at: u64,
    pub expires_at: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[contracttype]
#[repr(u32)]
pub enum ComplianceType {
    KYC = 0,
    AML = 1,
    Sanctions = 2,
    TaxCompliance = 3,
    DataPrivacy = 4,
    FinancialRegulation = 5,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[contracttype]
#[repr(u32)]
pub enum ComplianceStatus {
    Compliant = 0,
    NonCompliant = 1,
    Pending = 2,
    UnderReview = 3,
    Exempt = 4,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[contracttype]
#[repr(u32)]
pub enum RiskLevel {
    Low = 0,
    Medium = 1,
    High = 2,
    Critical = 3,
}

#[derive(Clone, Debug)]
#[contracttype]
pub struct ComplianceFinding {
    pub category: String,
    pub severity: String,
    pub description: String,
    pub recommendation: Option<String>,
}

/// Reputation integration types
#[derive(Clone, Debug)]
#[contracttype]
pub struct ReputationScore {
    pub entity_did: String,
    pub overall_score: u32,
    pub category_scores: Map<String, u32>,
    pub review_count: u32,
    pub last_updated: u64,
    pub calculation_method: String,
}

#[derive(Clone, Debug)]
#[contracttype]
pub struct ReputationReview {
    pub review_id: u64,
    pub reviewer_did: String,
    pub subject_did: String,
    pub rating: u32, // 1-5 stars
    pub category: String,
    pub comment: Option<String>,
    pub evidence: Vec<String>, // Credential IDs as evidence
    pub created_at: u64,
    pub verified: bool,
}
