#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Bytes, Env, Map,
    String, Symbol, Vec,
};
use stellai_lib::{
    admin, audit, validation, ComplianceFinding, ComplianceReport, ComplianceStatus,
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
const MAX_REVIEWS_PER_ENTITY: u32 = 1000;
const MAX_FINDINGS_PER_REPORT: u32 = 50;
const REPUTATION_DECAY_PERIOD: u64 = 30 * 24 * 60 * 60; // 30 days
const MIN_RATING: u32 = 1;
const MAX_RATING: u32 = 5;

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

        // Check authorization
        admin::verify_admin(&env, &issued_by).map_err(|_| Error::Unauthorized)?;

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

        // Check authorization
        admin::verify_admin(&env, &updated_by).map_err(|_| Error::Unauthorized)?;

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
        // Check authorization
        admin::verify_admin(&env, &verifier).map_err(|_| Error::Unauthorized)?;

        let mut all_valid = true;
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
        reviewer_did: String,
        subject_did: String,
        rating: u32,
        category: String,
        comment: Option<String>,
        evidence: Vec<String>,
    ) -> Result<u64, Error> {
        // Validate inputs
        Self::validate_review_inputs(&rating, &category, &evidence)?;

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
        entity_did: String,
        limit: u32,
    ) -> Result<Vec<ReputationReview>, Error> {
        let mut reviews = Vec::new(&env);

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
        maximum_risk_level: RiskLevel,
    ) -> Result<bool, Error> {
        let mut meets_requirements = true;
        let now = env.ledger().timestamp();

        for compliance_type in required_types {
            // Check if entity has a valid compliance report for this type
            let mut has_valid_report = false;

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
        // Check authorization
        admin::verify_admin(&env, &assessed_by).map_err(|_| Error::Unauthorized)?;

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
        env: Env,
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
        evidence: &Vec<String>,
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
        risk_level: RiskLevel,
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

    fn increment_counter(env: Env, counter_key: &Symbol) -> u64 {
        let count: u64 = env.storage().instance().get(counter_key).unwrap_or(0);
        let new_count = count + 1;
        env.storage().instance().set(counter_key, &new_count);
        new_count
    }
}
