#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, Map, String,
    Symbol, Vec,
};
use stellai_lib::{
    admin, audit, validation, rbac, ComplianceFinding, ComplianceReport, ComplianceStatus,
    ComplianceType, CredentialType, ReputationReview, ReputationScore, RiskLevel,
};

// Contract errors
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Error {
    Unauthorized = 1,
    InvalidDID = 2,
    ReportNotFound = 3,
    InvalidCredential = 4,
    CredentialExpired = 5,
    CredentialRevoked = 6,
    ComplianceCheckFailed = 7,
    InvalidRiskLevel = 8,
    DuplicateReport = 9,
    UnauthorizedReviewer = 10,
    InvalidRating = 11,
    ReviewNotFound = 12,
    RateLimitExceeded = 13,
    AuditRequired = 14,
    // KYC state machine errors
    KycSubjectNotFound = 15,
    KycInvalidTransition = 16,
    KycTerminalState = 17,
    KycRequestExpired = 18,
    KycOperatorRequired = 19,
    KycSelfAssignment = 20,
    KycNotVerified = 21,
    KycOverrideNotScheduled = 22,
    KycOverrideNotReady = 23,
    KycOverrideAlreadyScheduled = 24,
}

// ── Compliance Types ────────────────────────────────────────────────────────

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
    pub rating: u32,
    pub category: String,
    pub comment: Option<String>,
    pub evidence: Vec<String>,
    pub created_at: u64,
    pub verified: bool,
}

// ── KYC State Machine (issues #174, #175, #176, #177) ──────────────────────

/// KYC lifecycle states.
/// Valid transitions: Pending → InReview → Verified
///                   Pending → InReview → Rejected
/// Terminal states (Verified, Rejected) are immutable except via governance override.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[contracttype]
#[repr(u32)]
pub enum KycStatus {
    Pending = 0,
    InReview = 1,
    Verified = 2,
    Rejected = 3,
}

#[derive(Clone, Debug)]
#[contracttype]
pub struct KycRecord {
    pub subject: Address,
    pub subject_did: String,
    pub status: KycStatus,
    pub updated_by: Address,
    pub updated_at: u64,
    /// Set when status reaches a terminal state (Verified / Rejected).
    pub finalized_at: Option<u64>,
    /// Timestamp when the KYC request was created (for Pending state).
    pub created_at: u64,
    /// Expiry timestamp for pending requests. None means no expiry.
    pub expires_at: Option<u64>,
}

const KYC_RECORDS: Symbol = symbol_short!("kyc_rec");
const KYC_OPERATORS: Symbol = symbol_short!("kyc_ops");
const KYC_OVERRIDES: Symbol = symbol_short!("kyc_ovr");
const ADMIN_KEY: Symbol = symbol_short!("admin");

#[derive(Clone, Debug)]
#[contracttype]
pub struct KycOverrideRequest {
    pub subject: Address,
    pub requested_by: Address,
    pub execute_after: u64,
    pub created_at: u64,
}

// Contract events
#[contracttype]
pub enum ComplianceEvent {
    ReportGenerated(ReportGeneratedEvent),
    ReportUpdated(ReportUpdatedEvent),
    CredentialVerified(CredentialVerifiedEvent),
    ReputationUpdated(ReputationUpdatedEvent),
    ReviewAdded(ReviewAddedEvent),
    RiskAssessmentCreated(RiskAssessmentCreatedEvent),
}

#[derive(Clone)]
#[contracttype]
pub struct ReportGeneratedEvent {
    pub report_id: u64,
    pub entity_did: String,
    pub compliance_type: ComplianceType,
    pub status: ComplianceStatus,
    pub risk_level: RiskLevel,
    pub generated_by: Address,
    pub timestamp: u64,
}

#[derive(Clone)]
#[contracttype]
pub struct ReportUpdatedEvent {
    pub report_id: u64,
    pub updated_by: Address,
    pub timestamp: u64,
}

#[derive(Clone)]
#[contracttype]
pub struct CredentialVerifiedEvent {
    pub credential_id: u64,
    pub entity_did: String,
    pub compliance_type: ComplianceType,
    pub verified_by: Address,
    pub timestamp: u64,
}

#[derive(Clone)]
#[contracttype]
pub struct ReputationUpdatedEvent {
    pub entity_did: String,
    pub new_score: u32,
    pub updated_by: Address,
    pub timestamp: u64,
}

#[derive(Clone)]
#[contracttype]
pub struct ReviewAddedEvent {
    pub review_id: u64,
    pub reviewer_did: String,
    pub subject_did: String,
    pub rating: u32,
    pub timestamp: u64,
}

#[derive(Clone)]
#[contracttype]
pub struct RiskAssessmentCreatedEvent {
    pub assessment_id: u64,
    pub entity_did: String,
    pub risk_level: RiskLevel,
    pub assessed_by: Address,
    pub timestamp: u64,
}

// Storage keys
const COMPLIANCE_REPORTS: Symbol = symbol_short!("comp_rep");
const REPUTATION_SCORES: Symbol = symbol_short!("rep_score");
const REPUTATION_REVIEWS: Symbol = symbol_short!("rep_rev");
const RISK_ASSESSMENTS: Symbol = symbol_short!("risk_ass");
const CREDENTIAL_VERIFICATIONS: Symbol = symbol_short!("cred_ver");
const REPORT_COUNTER: Symbol = symbol_short!("rep_cnt");
const REVIEW_COUNTER: Symbol = symbol_short!("rev_cnt");
const ASSESSMENT_COUNTER: Symbol = symbol_short!("ass_cnt");

// Constants
#[allow(dead_code)]
const MAX_REVIEWS_PER_ENTITY: u32 = 1000;
const MAX_FINDINGS_PER_REPORT: u32 = 50;
#[allow(dead_code)]
const REPUTATION_DECAY_PERIOD: u64 = 30 * 24 * 60 * 60; // 30 days
const MIN_RATING: u32 = 1;
const MAX_RATING: u32 = 5;
/// Default expiry period for pending KYC requests (90 days)
const KYC_PENDING_EXPIRY_SECS: u64 = 90 * 24 * 60 * 60;
/// Governance timelock for terminal-state overrides (24 hours)
const KYC_OVERRIDE_DELAY_SECS: u64 = 24 * 60 * 60;

#[contract]
pub struct ComplianceIntegrationContract;

#[contractimpl]
impl ComplianceIntegrationContract {
    /// Generate a compliance report for an entity
    pub fn generate_compliance_report(
        env: Env,
        entity_did: String,
        compliance_type: ComplianceType,
        status: ComplianceStatus,
        score: u32,
        risk_level: RiskLevel,
        findings: Vec<ComplianceFinding>,
        issued_by: Address,
        validity_period: u64,
    ) -> Result<u64, Error> {
        // Validate inputs
        Self::validate_compliance_inputs(env.clone(), &entity_did, &findings)?;

        // Sensitive entry points are gated behind verified KYC and admin authorization.
        issued_by.require_auth();
        Self::require_verified_kyc(&env, &issued_by)?;
        Self::verify_admin(&env, &issued_by)?;

        // Generate report ID
        let report_id = Self::increment_counter(env.clone(), &REPORT_COUNTER);
        let now = env.ledger().timestamp();

        // Create compliance report
        let report = ComplianceReport {
            report_id,
            entity_did: entity_did.clone(),
            compliance_type,
            status,
            score,
            risk_level,
            findings: findings.clone(),
            issued_by: issued_by.clone(),
            issued_at: now,
            expires_at: now + validity_period,
        };

        // Store report
        env.storage()
            .instance()
            .set(&(COMPLIANCE_REPORTS, report_id), &report);

        // Update reputation score based on compliance
        Self::update_reputation_from_compliance(env.clone(), entity_did.clone(), score, risk_level);

        // Emit event
        env.events().publish(
            (Symbol::new(&env, "ReportGenerated"), &report_id),
            ReportGeneratedEvent {
                report_id,
                entity_did: entity_did.clone(),
                compliance_type,
                status,
                risk_level,
                generated_by: issued_by.clone(),
                timestamp: now,
            },
        );

        // Audit log
        Ok(report_id)
    }

    /// Update an existing compliance report
    pub fn update_compliance_report(
        env: Env,
        report_id: u64,
        status: ComplianceStatus,
        score: u32,
        risk_level: RiskLevel,
        findings: Vec<ComplianceFinding>,
        updated_by: Address,
    ) -> Result<(), Error> {
        // Get existing report
        let mut report = Self::get_compliance_report(env.clone(), report_id)?;

        updated_by.require_auth();
        Self::require_verified_kyc(&env, &updated_by)?;
        Self::verify_admin(&env, &updated_by)?;

        // Update report
        report.status = status;
        report.score = score;
        report.risk_level = risk_level;
        report.findings = findings.clone();

        // Store updated report
        env.storage()
            .instance()
            .set(&(COMPLIANCE_REPORTS, report_id), &report);

        // Update reputation score
        Self::update_reputation_from_compliance(
            env.clone(),
            report.entity_did.clone(),
            score,
            risk_level,
        );

        // Emit event
        env.events().publish(
            (Symbol::new(&env, "ReportUpdated"), &report_id),
            ReportUpdatedEvent {
                report_id,
                updated_by: updated_by.clone(),
                timestamp: env.ledger().timestamp(),
            },
        );

        // Audit log
        Ok(())
    }

    /// Verify credentials for compliance
    pub fn verify_creds_compliance(
        env: Env,
        entity_did: String,
        credential_ids: Vec<u64>,
        compliance_type: ComplianceType,
        verifier: Address,
    ) -> Result<bool, Error> {
        verifier.require_auth();
        Self::require_verified_kyc(&env, &verifier)?;
        Self::verify_admin(&env, &verifier)?;

        let all_valid = true;
        let now = env.ledger().timestamp();

        for credential_id in credential_ids {
            // In a real implementation, this would call the verifiable credentials contract
            // For now, we'll simulate the verification process

            // Store verification record
            let verification_key = (CREDENTIAL_VERIFICATIONS, credential_id, entity_did.clone());
            let verification_data = (verifier.clone(), now, compliance_type);
            env.storage()
                .instance()
                .set(&verification_key, &verification_data);

            // Emit event
            env.events().publish(
                (Symbol::new(&env, "CredentialVerified"), credential_id),
                CredentialVerifiedEvent {
                    credential_id,
                    entity_did: entity_did.clone(),
                    compliance_type,
                    verified_by: verifier.clone(),
                    timestamp: now,
                },
            );
        }

        Ok(all_valid)
    }

    /// Add a reputation review
    pub fn add_reputation_review(
        env: Env,
        reviewer: Address,
        reviewer_did: String,
        subject_did: String,
        rating: u32,
        category: String,
        comment: Option<String>,
        evidence: Vec<String>,
    ) -> Result<u64, Error> {
        // Validate inputs
        Self::validate_review_inputs(&rating, &category, &evidence)?;
        reviewer.require_auth();
        Self::require_verified_kyc(&env, &reviewer)?;

        // Generate review ID
        let review_id = Self::increment_counter(env.clone(), &REVIEW_COUNTER);
        let now = env.ledger().timestamp();

        // Create review
        let review = ReputationReview {
            review_id,
            reviewer_did: reviewer_did.clone(),
            subject_did: subject_did.clone(),
            rating,
            category: category.clone(),
            comment: comment.clone(),
            evidence: evidence.clone(),
            created_at: now,
            verified: false, // Requires verification
        };

        // Store review
        env.storage()
            .instance()
            .set(&(REPUTATION_REVIEWS, review_id), &review);

        // Update reputation score
        Self::update_reputation_from_review(env.clone(), subject_did.clone(), rating, &category);

        // Emit event
        env.events().publish(
            (Symbol::new(&env, "ReviewAdded"), &review_id),
            ReviewAddedEvent {
                review_id,
                reviewer_did: reviewer_did.clone(),
                subject_did: subject_did.clone(),
                rating,
                timestamp: now,
            },
        );

        // Audit log
        Ok(review_id)
    }

    /// Get reputation score for an entity
    pub fn get_reputation_score(env: Env, entity_did: String) -> Result<ReputationScore, Error> {
        let score: Option<ReputationScore> = env
            .storage()
            .instance()
            .get(&(REPUTATION_SCORES, entity_did.clone()));

        match score {
            Some(s) => Ok(s),
            None => {
                // Create default reputation score
                let default_score = ReputationScore {
                    entity_did: entity_did.clone(),
                    overall_score: 50, // Neutral starting score
                    category_scores: Map::new(&env),
                    review_count: 0,
                    last_updated: env.ledger().timestamp(),
                    calculation_method: String::from_str(&env, "weighted_average"),
                };
                Ok(default_score)
            }
        }
    }

    /// Get compliance report
    pub fn get_compliance_report(env: Env, report_id: u64) -> Result<ComplianceReport, Error> {
        let report: Option<ComplianceReport> = env
            .storage()
            .instance()
            .get(&(COMPLIANCE_REPORTS, report_id));
        report.ok_or(Error::ReportNotFound)
    }

    /// Get reputation review
    pub fn get_reputation_review(env: Env, review_id: u64) -> Result<ReputationReview, Error> {
        let review: Option<ReputationReview> = env
            .storage()
            .instance()
            .get(&(REPUTATION_REVIEWS, review_id));
        review.ok_or(Error::ReviewNotFound)
    }

    /// Get reviews for an entity
    pub fn get_entity_reviews(
        env: Env,
        _entity_did: String,
        _limit: u32,
    ) -> Result<Vec<ReputationReview>, Error> {
        let reviews = Vec::new(&env);

        // In a real implementation, we'd have an index by entity_did
        // For now, return empty vector
        Ok(reviews)
    }

    /// Check if entity meets compliance requirements
    pub fn check_compliance_requirements(
        env: Env,
        entity_did: String,
        required_types: Vec<ComplianceType>,
        minimum_score: u32,
        _maximum_risk_level: RiskLevel,
    ) -> Result<bool, Error> {
        let mut meets_requirements = true;

        for _compliance_type in required_types {
            // Check if entity has a valid compliance report for this type
            let has_valid_report = false;

            // In a real implementation, we'd query reports by entity_did and type
            // For now, we'll simulate the check

            if !has_valid_report {
                meets_requirements = false;
                break;
            }
        }

        // Check reputation score
        let reputation = Self::get_reputation_score(env.clone(), entity_did.clone())?;
        if reputation.overall_score < minimum_score {
            meets_requirements = false;
        }

        Ok(meets_requirements)
    }

    /// Create risk assessment
    pub fn create_risk_assessment(
        env: Env,
        entity_did: String,
        risk_level: RiskLevel,
        factors: Vec<String>,
        mitigation: Option<String>,
        assessed_by: Address,
    ) -> Result<u64, Error> {
        assessed_by.require_auth();
        Self::require_verified_kyc(&env, &assessed_by)?;
        Self::verify_admin(&env, &assessed_by)?;

        // Generate assessment ID
        let assessment_id = Self::increment_counter(env.clone(), &ASSESSMENT_COUNTER);
        let now = env.ledger().timestamp();

        // Create assessment (simplified structure)
        let assessment = (
            assessment_id,
            entity_did.clone(),
            risk_level,
            factors.clone(),
            mitigation.clone(),
            assessed_by.clone(),
            now,
        );

        // Store assessment
        env.storage()
            .instance()
            .set(&(RISK_ASSESSMENTS, assessment_id), &assessment);

        // Emit event
        env.events().publish(
            (Symbol::new(&env, "RiskAssessmentCreated"), &assessment_id),
            RiskAssessmentCreatedEvent {
                assessment_id,
                entity_did: entity_did.clone(),
                risk_level,
                assessed_by: assessed_by.clone(),
                timestamp: now,
            },
        );

        // Audit log
        Ok(assessment_id)
    }

    // Helper functions
    fn validate_compliance_inputs(
        _env: Env,
        entity_did: &String,
        findings: &Vec<ComplianceFinding>,
    ) -> Result<(), Error> {
        // Validate DID format
        if entity_did.is_empty() {
            return Err(Error::InvalidDID);
        }

        // Validate findings
        if findings.len() > MAX_FINDINGS_PER_REPORT {
            return Err(Error::ComplianceCheckFailed);
        }

        Ok(())
    }

    fn validate_review_inputs(
        rating: &u32,
        category: &String,
        _evidence: &Vec<String>,
    ) -> Result<(), Error> {
        // Validate rating range
        if *rating < MIN_RATING || *rating > MAX_RATING {
            return Err(Error::InvalidRating);
        }

        // Validate category
        if category.is_empty() {
            return Err(Error::InvalidRating);
        }

        Ok(())
    }

    fn update_reputation_from_compliance(
        env: Env,
        entity_did: String,
        compliance_score: u32,
        _risk_level: RiskLevel,
    ) {
        let mut reputation = Self::get_reputation_score(env.clone(), entity_did.clone()).unwrap();

        // Update overall score based on compliance
        let compliance_weight = 30; // 30% weight for compliance
        let current_weight = 70; // 70% weight for current score
        let new_score = (reputation.overall_score * current_weight
            + compliance_score * compliance_weight)
            / 100;

        reputation.overall_score = new_score;
        reputation.last_updated = env.ledger().timestamp();

        // Store updated reputation
        env.storage()
            .instance()
            .set(&(REPUTATION_SCORES, entity_did.clone()), &reputation);

        // Emit event
        env.events().publish(
            (Symbol::new(&env, "ReputationUpdated"), entity_did.clone()),
            ReputationUpdatedEvent {
                entity_did: entity_did.clone(),
                new_score,
                updated_by: env.current_contract_address(),
                timestamp: env.ledger().timestamp(),
            },
        );
    }

    fn update_reputation_from_review(env: Env, entity_did: String, rating: u32, category: &String) {
        let mut reputation = Self::get_reputation_score(env.clone(), entity_did.clone()).unwrap();

        // Update category score
        let current_category_score = reputation
            .category_scores
            .get(category.clone())
            .unwrap_or(50);
        let review_count = reputation.review_count + 1;

        // Calculate new category score (weighted average)
        let new_category_score =
            (current_category_score * (review_count - 1) + rating * 20) / review_count; // Rating 1-5 -> 20-100 scale
        reputation
            .category_scores
            .set(category.clone(), new_category_score);

        // Update overall score
        let mut total_category_score = 0;
        let mut category_count = 0;
        for (_, score) in reputation.category_scores.iter() {
            total_category_score += score;
            category_count += 1;
        }

        if category_count > 0 {
            reputation.overall_score = total_category_score / category_count;
        }

        reputation.review_count = review_count;
        reputation.last_updated = env.ledger().timestamp();

        // Store updated reputation
        env.storage()
            .instance()
            .set(&(REPUTATION_SCORES, entity_did.clone()), &reputation);

        // Emit event
        env.events().publish(
            (Symbol::new(&env, "ReputationUpdated"), entity_did.clone()),
            ReputationUpdatedEvent {
                entity_did: entity_did.clone(),
                new_score: reputation.overall_score,
                updated_by: env.current_contract_address(),
                timestamp: env.ledger().timestamp(),
            },
        );
    }

    fn verify_admin(env: &Env, caller: &Address) -> Result<(), Error> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&ADMIN_KEY)
            .ok_or(Error::Unauthorized)?;
        if &admin != caller {
            return Err(Error::Unauthorized);
        }
        Ok(())
    }

    fn increment_counter(env: Env, counter_key: &Symbol) -> u64 {
        let count: u64 = env.storage().instance().get(counter_key).unwrap_or(0);
        let new_count = count + 1;
        env.storage().instance().set(counter_key, &new_count);
        new_count
    }

    // ── KYC State Machine ────────────────────────────────────────────────────

    /// Initialise a KYC record for a subject DID (starts in Pending).
    pub fn kyc_init(env: Env, operator: Address, subject_did: String) -> Result<(), Error> {
        rbac::require_kyc_operator_role(&env, &operator).map_err(|_| Error::Unauthorized)?;
        let key = (KYC_RECORDS, subject_did.clone());
        if env.storage().instance().has(&key) {
            return Err(Error::DuplicateReport);
        }

        let now = env.ledger().timestamp();
        let record = KycRecord {
            subject: subject.clone(),
            subject_did: subject_did.clone(),
            status: KycStatus::Pending,
            updated_by: operator.clone(),
            updated_at: now,
            finalized_at: None,
            created_at: now,
            expires_at: Some(now + KYC_PENDING_EXPIRY_SECS),
        };
        Self::put_kyc_record(&env, &subject, &record);
        env.events().publish(
            (Symbol::new(&env, "kyc"), Symbol::new(&env, "init")),
            (subject, subject_did, operator, KycStatus::Pending),
        );
        Ok(())
    }

    /// Advance the KYC state following the strict transition table.
    /// Allowed: Pending→InReview, InReview→Verified, InReview→Rejected.
    /// Terminal states (Verified, Rejected) are immutable without governance override.
    /// Pending requests that have expired cannot be transitioned.
    pub fn kyc_transition(
        env: Env,
        operator: Address,
        subject: Address,
        new_status: KycStatus,
    ) -> Result<(), Error> {
        rbac::require_kyc_operator_role(&env, &operator).map_err(|_| Error::Unauthorized)?;
        let key = (KYC_RECORDS, subject_did.clone());
        let mut record: KycRecord = env
            .storage()
            .instance()
            .get(&key)
            .ok_or(Error::KycSubjectNotFound)?;

        let mut record = Self::get_kyc_record(&env, &subject)?;
        if record.status == KycStatus::Pending {
            if let Some(expires_at) = record.expires_at {
                let now = env.ledger().timestamp();
                if now >= expires_at {
                    env.events().publish(
                        (Symbol::new(&env, "kyc"), Symbol::new(&env, "expired")),
                        (subject.clone(), record.subject_did.clone(), expires_at),
                    );
                    return Err(Error::KycRequestExpired);
                }
            }
        }

        if Self::is_terminal_kyc_status(record.status) {
            return Err(Error::KycTerminalState);
        }
        if !Self::is_valid_kyc_transition(record.status, new_status) {
            return Err(Error::KycInvalidTransition);
        }

        let previous_status = record.status;
        let now = env.ledger().timestamp();
        record.status = new_status;
        record.updated_by = operator.clone();
        record.updated_at = now;
        record.expires_at = None;
        record.finalized_at = if Self::is_terminal_kyc_status(new_status) {
            Some(now)
        } else {
            None
        };
        Self::put_kyc_record(&env, &subject, &record);
        env.events().publish(
            (Symbol::new(&env, "kyc"), Symbol::new(&env, "transition")),
            (
                subject,
                record.subject_did,
                operator,
                previous_status,
                new_status,
            ),
        );
        Ok(())
    }

    /// Governance-approved override to reset a terminal KYC state.
    /// Requires governance role and resets to Pending.
    pub fn kyc_governance_override(
        env: Env,
        governance: Address,
        subject: Address,
    ) -> Result<u64, Error> {
        governance.require_auth();
        Self::verify_admin(&env, &governance)?;
        let record = Self::get_kyc_record(&env, &subject)?;
        if !Self::is_terminal_kyc_status(record.status) {
            return Err(Error::KycInvalidTransition);
        }
        if env
            .storage()
            .instance()
            .has(&(KYC_OVERRIDES, subject.clone()))
        {
            return Err(Error::KycOverrideAlreadyScheduled);
        }

        let now = env.ledger().timestamp();
        let execute_after = now + KYC_OVERRIDE_DELAY_SECS;
        let request = KycOverrideRequest {
            subject: subject.clone(),
            requested_by: governance.clone(),
            execute_after,
            created_at: now,
        };
        env.storage()
            .instance()
            .set(&(KYC_OVERRIDES, subject.clone()), &request);
        env.events().publish(
            (Symbol::new(&env, "kyc"), Symbol::new(&env, "schedule")),
            (subject, governance, execute_after),
        );
        Ok(execute_after)
    }

    /// Governance executes a previously scheduled override and resets the subject to Pending.
    pub fn kyc_execute_override(
        env: Env,
        governance: Address,
        subject: Address,
    ) -> Result<(), Error> {
        rbac::require_governance_role(&env, &governance).map_err(|_| Error::Unauthorized)?;
        let key = (KYC_RECORDS, subject_did.clone());
        let mut record: KycRecord = env
            .storage()
            .instance()
            .get(&(KYC_OVERRIDES, subject.clone()))
            .ok_or(Error::KycOverrideNotScheduled)?;
        let now = env.ledger().timestamp();
        if now < request.execute_after {
            return Err(Error::KycOverrideNotReady);
        }

        let mut record = Self::get_kyc_record(&env, &subject)?;
        if !Self::is_terminal_kyc_status(record.status) {
            return Err(Error::KycInvalidTransition);
        }

        record.status = KycStatus::Pending;
        record.updated_by = governance.clone();
        record.updated_at = now;
        record.created_at = now;
        record.finalized_at = None;
        record.expires_at = Some(now + KYC_PENDING_EXPIRY_SECS);
        Self::put_kyc_record(&env, &subject, &record);
        env.storage()
            .instance()
            .remove(&(KYC_OVERRIDES, subject.clone()));
        env.events().publish(
            (Symbol::new(&env, "kyc"), Symbol::new(&env, "override")),
            (subject, record.subject_did, governance, KycStatus::Pending),
        );
        Ok(())
    }

    /// Read the current KYC record for a subject.
    pub fn kyc_get(env: Env, subject: Address) -> Result<KycRecord, Error> {
        Self::get_kyc_record(&env, &subject)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        symbol_short,
        testutils::{Address as _, Ledger as _},
        Address, Env, String, Vec,
    };

    fn setup(env: &Env) -> (Address, Address, Address, Address) {
        let contract_id = env.register(ComplianceIntegrationContract, ());
        let admin = Address::generate(env);
        let operator_one = Address::generate(env);
        let operator_two = Address::generate(env);
        env.as_contract(&contract_id, || {
            env.storage()
                .instance()
                .set(&symbol_short!("admin"), &admin);
        });
        env.as_contract(&contract_id, || {
            ComplianceIntegrationContract::kyc_set_operator(
                env.clone(),
                admin.clone(),
                operator_one.clone(),
                true,
            )
            .unwrap()
        });
        env.as_contract(&contract_id, || {
            ComplianceIntegrationContract::kyc_set_operator(
                env.clone(),
                admin.clone(),
                operator_two.clone(),
                true,
            )
            .unwrap()
        });
        (contract_id, admin, operator_one, operator_two)
    }

    fn init_subject(
        env: &Env,
        contract_id: &Address,
        operator: &Address,
        subject: &Address,
        did: &str,
    ) {
        env.as_contract(contract_id, || {
            ComplianceIntegrationContract::kyc_init(
                env.clone(),
                operator.clone(),
                subject.clone(),
                String::from_str(env, did),
            )
            .unwrap();
        });
    }

    fn transition_subject(
        env: &Env,
        contract_id: &Address,
        operator: &Address,
        subject: &Address,
        status: KycStatus,
    ) -> Result<(), Error> {
        env.as_contract(contract_id, || {
            ComplianceIntegrationContract::kyc_transition(
                env.clone(),
                operator.clone(),
                subject.clone(),
                status,
            )
        })
    }

    fn verify_subject(
        env: &Env,
        contract_id: &Address,
        operator: &Address,
        subject: &Address,
        did: &str,
    ) {
        init_subject(env, contract_id, operator, subject, did);
        transition_subject(env, contract_id, operator, subject, KycStatus::InReview).unwrap();
        transition_subject(env, contract_id, operator, subject, KycStatus::Verified).unwrap();
    }

    fn sample_findings(env: &Env) -> Vec<ComplianceFinding> {
        let mut findings = Vec::new(env);
        findings.push_back(ComplianceFinding {
            category: String::from_str(env, "kyc"),
            severity: String::from_str(env, "medium"),
            description: String::from_str(env, "manual review complete"),
            recommendation: Some(String::from_str(env, "approve")),
        });
        findings
    }

    fn sample_string_vec(env: &Env, value: &str) -> Vec<String> {
        let mut items = Vec::new(env);
        items.push_back(String::from_str(env, value));
        items
    }

    #[test]
    fn test_kyc_valid_transitions() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract_id, _admin, operator_one, _operator_two) = setup(&env);
        let subject = Address::generate(&env);

        init_subject(
            &env,
            &contract_id,
            &operator_one,
            &subject,
            "did:stellar:subject1",
        );
        let rec = env.as_contract(&contract_id, || {
            ComplianceIntegrationContract::kyc_get(env.clone(), subject.clone()).unwrap()
        });
        assert_eq!(rec.status, KycStatus::Pending);
        assert_eq!(rec.subject, subject);

        transition_subject(
            &env,
            &contract_id,
            &operator_one,
            &subject,
            KycStatus::InReview,
        )
        .unwrap();
        transition_subject(
            &env,
            &contract_id,
            &operator_one,
            &subject,
            KycStatus::Verified,
        )
        .unwrap();

        let rec = env.as_contract(&contract_id, || {
            ComplianceIntegrationContract::kyc_get(env.clone(), subject.clone()).unwrap()
        });
        assert_eq!(rec.status, KycStatus::Verified);
        assert!(rec.finalized_at.is_some());
        assert!(env.as_contract(&contract_id, || {
            ComplianceIntegrationContract::kyc_is_verified(env.clone(), subject.clone())
        }));
    }

    #[test]
    fn test_invalid_transition_sequences_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract_id, _admin, operator_one, _operator_two) = setup(&env);
        let subject = Address::generate(&env);

        init_subject(
            &env,
            &contract_id,
            &operator_one,
            &subject,
            "did:stellar:subject2",
        );

        assert_eq!(
            transition_subject(
                &env,
                &contract_id,
                &operator_one,
                &subject,
                KycStatus::Pending
            )
            .unwrap_err(),
            Error::KycInvalidTransition
        );
        assert_eq!(
            transition_subject(
                &env,
                &contract_id,
                &operator_one,
                &subject,
                KycStatus::Verified
            )
            .unwrap_err(),
            Error::KycInvalidTransition
        );
        assert_eq!(
            transition_subject(
                &env,
                &contract_id,
                &operator_one,
                &subject,
                KycStatus::Rejected
            )
            .unwrap_err(),
            Error::KycInvalidTransition
        );

        transition_subject(
            &env,
            &contract_id,
            &operator_one,
            &subject,
            KycStatus::InReview,
        )
        .unwrap();
        assert_eq!(
            transition_subject(
                &env,
                &contract_id,
                &operator_one,
                &subject,
                KycStatus::Pending
            )
            .unwrap_err(),
            Error::KycInvalidTransition
        );
        assert_eq!(
            transition_subject(
                &env,
                &contract_id,
                &operator_one,
                &subject,
                KycStatus::InReview
            )
            .unwrap_err(),
            Error::KycInvalidTransition
        );
    }

    #[test]
    fn test_terminal_states_are_immutable_without_override() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract_id, _admin, operator_one, _operator_two) = setup(&env);
        let verified_subject = Address::generate(&env);
        verify_subject(
            &env,
            &contract_id,
            &operator_one,
            &verified_subject,
            "did:stellar:subject3",
        );
        assert_eq!(
            transition_subject(
                &env,
                &contract_id,
                &operator_one,
                &verified_subject,
                KycStatus::Rejected,
            )
            .unwrap_err(),
            Error::KycTerminalState
        );
        assert_eq!(
            transition_subject(
                &env,
                &contract_id,
                &operator_one,
                &verified_subject,
                KycStatus::Pending,
            )
            .unwrap_err(),
            Error::KycTerminalState
        );

        let rejected_subject = Address::generate(&env);
        init_subject(
            &env,
            &contract_id,
            &operator_one,
            &rejected_subject,
            "did:stellar:subject4",
        );
        transition_subject(
            &env,
            &contract_id,
            &operator_one,
            &rejected_subject,
            KycStatus::InReview,
        )
        .unwrap();
        transition_subject(
            &env,
            &contract_id,
            &operator_one,
            &rejected_subject,
            KycStatus::Rejected,
        )
        .unwrap();
        assert_eq!(
            transition_subject(
                &env,
                &contract_id,
                &operator_one,
                &rejected_subject,
                KycStatus::Verified,
            )
            .unwrap_err(),
            Error::KycTerminalState
        );
    }

    #[test]
    fn test_only_kyc_operators_can_assign_status() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract_id, _admin, operator_one, _operator_two) = setup(&env);
        let outsider = Address::generate(&env);
        let subject = Address::generate(&env);

        let init_err = env.as_contract(&contract_id, || {
            ComplianceIntegrationContract::kyc_init(
                env.clone(),
                outsider.clone(),
                subject.clone(),
                String::from_str(&env, "did:stellar:subject5"),
            )
            .unwrap_err()
        });
        assert_eq!(init_err, Error::KycOperatorRequired);

        init_subject(
            &env,
            &contract_id,
            &operator_one,
            &subject,
            "did:stellar:subject5",
        );
        assert_eq!(
            transition_subject(&env, &contract_id, &outsider, &subject, KycStatus::InReview)
                .unwrap_err(),
            Error::KycOperatorRequired
        );
    }

    #[test]
    fn test_operator_cannot_self_assign_kyc_status() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract_id, _admin, operator_one, operator_two) = setup(&env);

        let init_err = env.as_contract(&contract_id, || {
            ComplianceIntegrationContract::kyc_init(
                env.clone(),
                operator_one.clone(),
                operator_one.clone(),
                String::from_str(&env, "did:stellar:self"),
            )
            .unwrap_err()
        });
        assert_eq!(init_err, Error::KycSelfAssignment);

        init_subject(
            &env,
            &contract_id,
            &operator_two,
            &operator_one,
            "did:stellar:operator-one",
        );
        assert_eq!(
            transition_subject(
                &env,
                &contract_id,
                &operator_one,
                &operator_one,
                KycStatus::InReview,
            )
            .unwrap_err(),
            Error::KycSelfAssignment
        );
    }

    #[test]
    fn test_governance_override_requires_timelock_delay() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract_id, admin, operator_one, _operator_two) = setup(&env);
        let subject = Address::generate(&env);
        verify_subject(
            &env,
            &contract_id,
            &operator_one,
            &subject,
            "did:stellar:subject6",
        );

        let execute_after = env.as_contract(&contract_id, || {
            ComplianceIntegrationContract::kyc_schedule_override(
                env.clone(),
                admin.clone(),
                subject.clone(),
            )
            .unwrap()
        });
        let early_err = env.as_contract(&contract_id, || {
            ComplianceIntegrationContract::kyc_execute_override(
                env.clone(),
                admin.clone(),
                subject.clone(),
            )
            .unwrap_err()
        });
        assert_eq!(early_err, Error::KycOverrideNotReady);

        env.ledger().set_timestamp(execute_after);
        env.as_contract(&contract_id, || {
            ComplianceIntegrationContract::kyc_execute_override(
                env.clone(),
                admin.clone(),
                subject.clone(),
            )
            .unwrap();
        });

        let rec = env.as_contract(&contract_id, || {
            ComplianceIntegrationContract::kyc_get(env.clone(), subject.clone()).unwrap()
        });
        assert_eq!(rec.status, KycStatus::Pending);
        assert!(rec.finalized_at.is_none());
        assert_eq!(rec.created_at, execute_after);
        assert_eq!(
            rec.expires_at,
            Some(execute_after + KYC_PENDING_EXPIRY_SECS)
        );
    }

    #[test]
    fn test_kyc_expiry_boundary_enforced() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract_id, _admin, operator_one, _operator_two) = setup(&env);
        let subject = Address::generate(&env);

        init_subject(
            &env,
            &contract_id,
            &operator_one,
            &subject,
            "did:stellar:subject7",
        );
        env.ledger()
            .set_timestamp(env.ledger().timestamp() + KYC_PENDING_EXPIRY_SECS - 1);
        transition_subject(
            &env,
            &contract_id,
            &operator_one,
            &subject,
            KycStatus::InReview,
        )
        .unwrap();

        let second_subject = Address::generate(&env);
        init_subject(
            &env,
            &contract_id,
            &operator_one,
            &second_subject,
            "did:stellar:subject8",
        );
        env.ledger()
            .set_timestamp(env.ledger().timestamp() + KYC_PENDING_EXPIRY_SECS);
        assert_eq!(
            transition_subject(
                &env,
                &contract_id,
                &operator_one,
                &second_subject,
                KycStatus::InReview,
            )
            .unwrap_err(),
            Error::KycRequestExpired
        );
    }

    #[test]
    fn test_sensitive_admin_entry_points_require_verified_kyc() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract_id, admin, operator_one, _operator_two) = setup(&env);
        let findings = sample_findings(&env);

        let report_err = env.as_contract(&contract_id, || {
            ComplianceIntegrationContract::generate_compliance_report(
                env.clone(),
                String::from_str(&env, "did:stellar:entity1"),
                ComplianceType::KYC,
                ComplianceStatus::Compliant,
                95,
                RiskLevel::Low,
                findings.clone(),
                admin.clone(),
                30,
            )
            .unwrap_err()
        });
        assert_eq!(report_err, Error::KycNotVerified);

        verify_subject(
            &env,
            &contract_id,
            &operator_one,
            &admin,
            "did:stellar:admin",
        );

        let report_id = env.as_contract(&contract_id, || {
            ComplianceIntegrationContract::generate_compliance_report(
                env.clone(),
                String::from_str(&env, "did:stellar:entity1"),
                ComplianceType::KYC,
                ComplianceStatus::Compliant,
                95,
                RiskLevel::Low,
                findings.clone(),
                admin.clone(),
                30,
            )
            .unwrap()
        });
        assert_eq!(report_id, 1);

        let mut credential_ids = Vec::new(&env);
        credential_ids.push_back(7u64);
        assert!(env.as_contract(&contract_id, || {
            ComplianceIntegrationContract::verify_creds_compliance(
                env.clone(),
                String::from_str(&env, "did:stellar:entity1"),
                credential_ids.clone(),
                ComplianceType::AML,
                admin.clone(),
            )
            .unwrap()
        }));

        let assessment_id = env.as_contract(&contract_id, || {
            ComplianceIntegrationContract::create_risk_assessment(
                env.clone(),
                String::from_str(&env, "did:stellar:entity1"),
                RiskLevel::Medium,
                sample_string_vec(&env, "jurisdiction"),
                Some(String::from_str(&env, "enhanced monitoring")),
                admin.clone(),
            )
            .unwrap()
        });
        assert_eq!(assessment_id, 1);
    }

    #[test]
    fn test_reputation_reviews_require_verified_kyc() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract_id, _admin, operator_one, _operator_two) = setup(&env);
        let reviewer = Address::generate(&env);

        let review_err = env.as_contract(&contract_id, || {
            ComplianceIntegrationContract::add_reputation_review(
                env.clone(),
                reviewer.clone(),
                String::from_str(&env, "did:stellar:reviewer"),
                String::from_str(&env, "did:stellar:subject9"),
                5,
                String::from_str(&env, "delivery"),
                Some(String::from_str(&env, "complete and timely")),
                sample_string_vec(&env, "cred-1"),
            )
            .unwrap_err()
        });
        assert_eq!(review_err, Error::KycNotVerified);

        verify_subject(
            &env,
            &contract_id,
            &operator_one,
            &reviewer,
            "did:stellar:reviewer",
        );
        let review_id = env.as_contract(&contract_id, || {
            ComplianceIntegrationContract::add_reputation_review(
                env.clone(),
                reviewer.clone(),
                String::from_str(&env, "did:stellar:reviewer"),
                String::from_str(&env, "did:stellar:subject9"),
                5,
                String::from_str(&env, "delivery"),
                Some(String::from_str(&env, "complete and timely")),
                sample_string_vec(&env, "cred-1"),
            )
            .unwrap()
        });
        assert_eq!(review_id, 1);
    }
}
