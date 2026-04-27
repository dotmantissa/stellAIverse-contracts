#![allow(unused_imports)]
use soroban_sdk::{contracterror, contracttype, Address, Bytes, String, Vec};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ContractError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    InvalidInput = 3,
    Unauthorized = 100,
    RoleEscalationAttempt = 102,
    RoleConflict = 103,
    InvalidMetadata = 403,
    MetadataTooLong = 404,
    CapabilitiesExceeded = 405,
    KycRequestExpired = 504,
    KycSubjectNotFound = 800,
    KycInvalidTransition = 801,
    KycTerminalState = 802,
    ComplianceCheckFailed = 803,
}

pub fn error_description(error: ContractError) -> &'static str {
    match error {
        ContractError::AlreadyInitialized => "Already initialized",
        ContractError::NotInitialized => "Not initialized",
        ContractError::InvalidInput => "Invalid input",
        ContractError::Unauthorized => "Unauthorized",
        ContractError::RoleEscalationAttempt => "Role escalation",
        ContractError::RoleConflict => "Role conflict",
        ContractError::InvalidMetadata => "Invalid metadata",
        ContractError::MetadataTooLong => "Metadata too long",
        ContractError::CapabilitiesExceeded => "Capabilities exceeded",
        ContractError::KycRequestExpired => "KYC expired",
        ContractError::KycSubjectNotFound => "KYC subject not found",
        ContractError::KycInvalidTransition => "Invalid KYC transition",
        ContractError::KycTerminalState => "KYC terminal state",
        ContractError::ComplianceCheckFailed => "Compliance check failed",
    }
}
