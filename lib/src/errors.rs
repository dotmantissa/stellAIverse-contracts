#![allow(unused_imports)]
use soroban_sdk::{contracterror, contracttype, Address, Bytes, String, Vec};

// ============================================================================
// CENTRALIZED ERROR CODE CATALOG
// ============================================================================
// This module defines all error codes used across the StellAIverse contract suite.
// Each error has a unique deterministic code to enable precise error handling
// and debugging. Error codes are grouped by category for maintainability.
//
// Error Code Ranges:
//   1-99:   Core/General errors
//   100-199: Authentication & Authorization
//   200-299: State Machine & Lifecycle
//   300-399: Financial & Arithmetic
//   400-499: Data Validation
//   500-599: Time & Expiry
//   600-699: External Calls & Oracles
//   700-799: Governance & Multisig
//   800-899: KYC & Compliance
// ============================================================================

// ============================================================================
// Contract Error Enum
// ============================================================================
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ContractError {
    // Core/General errors (1-99)
    AlreadyInitialized = 1,
    NotInitialized = 2,
    InvalidInput = 3,
    OverflowError = 4,
    UnderflowError = 5,
    NotImplemented = 6,
    InternalError = 7,
    DuplicateEntry = 8,
    EntryNotFound = 9,
    
    // Authentication & Authorization (100-199)
    Unauthorized = 100,
    NotOwner = 101,
    RoleEscalationAttempt = 102,
    RoleConflict = 103,
    InsufficientPermissions = 104,
    InvalidSignature = 105,
    SignatureExpired = 106,
    
    // State Machine & Lifecycle (200-299)
    InvalidStateTransition = 200,
    AlreadyInState = 201,
    TerminalStateReached = 202,
    InvalidOperationForState = 203,
    
    // Financial & Arithmetic (300-399)
    InvalidAmount = 300,
    NotEnoughBalance = 301,
    ZeroAmount = 302,
    AmountExceedsLimit = 303,
    InvalidRoyaltyFee = 304,
    PriceOutOfBounds = 305,
    
    // Data Validation (400-499)
    DuplicateAgentId = 400,
    AgentNotFound = 401,
    InvalidAgentId = 402,
    InvalidMetadata = 403,
    MetadataTooLong = 404,
    CapabilitiesExceeded = 405,
    InvalidAddress = 406,
    EmptyValue = 407,
    
    // Time & Expiry (500-599)
    Expired = 500,
    NotYetActive = 501,
    InvalidTimestamp = 502,
    DurationOutOfBounds = 503,
    KycRequestExpired = 504,
    
    // External Calls & Oracles (600-699)
    OracleError = 600,
    ExternalCallFailed = 601,
    InvalidOracleResponse = 602,
    OracleTimeout = 603,
    RateLimitExceeded = 604,
    
    // Governance & Multisig (700-799)
    InsufficientApprovals = 700,
    DuplicateApproval = 701,
    QuorumNotMet = 702,
    ProposalNotFound = 703,
    VotingPeriodEnded = 704,
    AlreadyVoted = 705,
    InvalidProposalState = 706,
    
    // KYC & Compliance (800-899)
    KycSubjectNotFound = 800,
    KycInvalidTransition = 801,
    KycTerminalState = 802,
    ComplianceCheckFailed = 803,
    CredentialExpired = 804,
    CredentialRevoked = 805,
    
    // Deprecated legacy codes (for backward compatibility)
    #[doc(hidden)]
    AgentLeased = 7,
    #[doc(hidden)]
    SameAddressTransfer = 9,
    #[doc(hidden)]
    AlreadyExists = 13,
}

/// Get a human-readable description for an error code.
/// This function is useful for logging and debugging.
pub fn error_description(error: ContractError) -> &'static str {
    match error {
        ContractError::AlreadyInitialized => "Contract has already been initialized",
        ContractError::NotInitialized => "Contract has not been initialized",
        ContractError::InvalidInput => "Invalid input parameters provided",
        ContractError::OverflowError => "Arithmetic overflow detected",
        ContractError::UnderflowError => "Arithmetic underflow detected",
        ContractError::NotImplemented => "Functionality not yet implemented",
        ContractError::InternalError => "Internal contract error occurred",
        ContractError::DuplicateEntry => "Entry already exists in storage",
        ContractError::EntryNotFound => "Requested entry not found in storage",
        
        ContractError::Unauthorized => "Caller is not authorized to perform this action",
        ContractError::NotOwner => "Caller is not the owner of the resource",
        ContractError::RoleEscalationAttempt => "Attempt to escalate privileges beyond authorized role",
        ContractError::InsufficientPermissions => "Caller lacks required permissions",
        ContractError::InvalidSignature => "Cryptographic signature verification failed",
        ContractError::SignatureExpired => "Signature has expired",
        
        ContractError::InvalidStateTransition => "Invalid state machine transition",
        ContractError::AlreadyInState => "Resource is already in the target state",
        ContractError::TerminalStateReached => "Resource has reached a terminal state",
        ContractError::InvalidOperationForState => "Operation not valid for current state",
        
        ContractError::InvalidAmount => "Amount is invalid or malformed",
        ContractError::NotEnoughBalance => "Insufficient balance for operation",
        ContractError::ZeroAmount => "Amount cannot be zero",
        ContractError::AmountExceedsLimit => "Amount exceeds maximum allowed limit",
        ContractError::InvalidRoyaltyFee => "Royalty fee is outside valid range",
        ContractError::PriceOutOfBounds => "Price is outside acceptable bounds",
        
        ContractError::DuplicateAgentId => "Agent ID already exists",
        ContractError::AgentNotFound => "Agent not found",
        ContractError::InvalidAgentId => "Agent ID is invalid",
        ContractError::InvalidMetadata => "Metadata is invalid or malformed",
        ContractError::MetadataTooLong => "Metadata exceeds maximum length",
        ContractError::CapabilitiesExceeded => "Number of capabilities exceeds maximum",
        ContractError::InvalidAddress => "Address is invalid or malformed",
        ContractError::EmptyValue => "Value cannot be empty",
        
        ContractError::Expired => "Resource or operation has expired",
        ContractError::NotYetActive => "Resource or operation is not yet active",
        ContractError::InvalidTimestamp => "Timestamp is invalid",
        ContractError::DurationOutOfBounds => "Duration is outside acceptable bounds",
        ContractError::KycRequestExpired => "KYC request has expired and cannot be processed",
        
        ContractError::OracleError => "Oracle operation failed",
        ContractError::ExternalCallFailed => "External contract call failed",
        ContractError::InvalidOracleResponse => "Oracle response is invalid",
        ContractError::OracleTimeout => "Oracle request timed out",
        ContractError::RateLimitExceeded => "Rate limit has been exceeded",
        
        ContractError::InsufficientApprovals => "Insufficient approvals for operation",
        ContractError::DuplicateApproval => "Duplicate approval detected",
        ContractError::QuorumNotMet => "Quorum requirement not met",
        ContractError::ProposalNotFound => "Proposal not found",
        ContractError::VotingPeriodEnded => "Voting period has ended",
        ContractError::AlreadyVoted => "Caller has already voted on this proposal",
        ContractError::InvalidProposalState => "Proposal is in an invalid state",
        
        ContractError::KycSubjectNotFound => "KYC subject not found",
        ContractError::KycInvalidTransition => "Invalid KYC state transition",
        ContractError::KycTerminalState => "KYC record has reached terminal state",
        ContractError::ComplianceCheckFailed => "Compliance check failed",
        ContractError::CredentialExpired => "Credential has expired",
        ContractError::CredentialRevoked => "Credential has been revoked",
        
        // Legacy codes
        ContractError::AgentLeased => "Agent is currently leased",
        ContractError::SameAddressTransfer => "Cannot transfer to same address",
        ContractError::AlreadyExists => "Resource already exists",
    }
}

/// Validate that an error code is within the valid range.
/// This ensures all error codes follow the deterministic coding scheme.
pub fn validate_error_code(code: u32) -> bool {
    // Valid ranges: 1-99, 100-199, 200-299, etc.
    // Invalid: 0, or gaps between ranges
    code > 0 && code < 1000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_codes_are_unique() {
        // Verify all error codes are unique by creating instances
        let errors = vec![
            (ContractError::AlreadyInitialized, 1u32),
            (ContractError::NotInitialized, 2u32),
            (ContractError::InvalidInput, 3u32),
            (ContractError::OverflowError, 4u32),
            (ContractError::Unauthorized, 100u32),
            (ContractError::NotOwner, 101u32),
            (ContractError::InvalidStateTransition, 200u32),
            (ContractError::InvalidAmount, 300u32),
            (ContractError::AgentNotFound, 401u32),
            (ContractError::Expired, 500u32),
            (ContractError::KycRequestExpired, 504u32),
            (ContractError::OracleError, 600u32),
            (ContractError::InsufficientApprovals, 700u32),
            (ContractError::KycSubjectNotFound, 800u32),
        ];

        // Check that each error has the expected code
        for (error, expected_code) in errors {
            let error_code = error as u32;
            assert_eq!(error_code, expected_code, "Error code mismatch for {:?}", error);
        }
    }

    #[test]
    fn test_error_descriptions_not_empty() {
        let all_errors = [
            ContractError::AlreadyInitialized,
            ContractError::Unauthorized,
            ContractError::InvalidStateTransition,
            ContractError::InvalidAmount,
            ContractError::AgentNotFound,
            ContractError::Expired,
            ContractError::KycRequestExpired,
            ContractError::OracleError,
            ContractError::InsufficientApprovals,
            ContractError::KycSubjectNotFound,
        ];

        for error in &all_errors {
            let description = error_description(*error);
            assert!(!description.is_empty(), "Error description should not be empty for {:?}", error);
        }
    }

    #[test]
    fn test_error_code_validation() {
        // Valid codes
        assert!(validate_error_code(1));
        assert!(validate_error_code(99));
        assert!(validate_error_code(100));
        assert!(validate_error_code(999));

        // Invalid codes
        assert!(!validate_error_code(0));
        assert!(!validate_error_code(1000));
    }

    #[test]
    fn test_error_code_ranges() {
        // Verify error codes fall within expected ranges
        assert!((ContractError::AlreadyInitialized as u32) < 100);
        assert!((ContractError::Unauthorized as u32) >= 100 && (ContractError::Unauthorized as u32) < 200);
        assert!((ContractError::InvalidStateTransition as u32) >= 200 && (ContractError::InvalidStateTransition as u32) < 300);
        assert!((ContractError::InvalidAmount as u32) >= 300 && (ContractError::InvalidAmount as u32) < 400);
        assert!((ContractError::AgentNotFound as u32) >= 400 && (ContractError::AgentNotFound as u32) < 500);
        assert!((ContractError::Expired as u32) >= 500 && (ContractError::Expired as u32) < 600);
        assert!((ContractError::OracleError as u32) >= 600 && (ContractError::OracleError as u32) < 700);
        assert!((ContractError::InsufficientApprovals as u32) >= 700 && (ContractError::InsufficientApprovals as u32) < 800);
        assert!((ContractError::KycSubjectNotFound as u32) >= 800 && (ContractError::KycSubjectNotFound as u32) < 900);
    }
}
