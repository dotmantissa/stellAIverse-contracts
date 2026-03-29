#![no_std]
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Bytes, Env, Map,
    String, Symbol, Vec,
};
use stellai_lib::{admin, audit, validation};

// Verifiable Credential structure following W3C VC specification
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
    pub proof: OptionalProof,
    pub non_revoked: bool,
    pub created_at: u64,
    pub updated_at: u64,
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
pub struct Proof {
    pub type_: String,
    pub created: u64,
    pub proof_purpose: String,
    pub verification_method: String,
    pub challenge: Option<String>,
    pub domain: Option<String>,
    pub jws: Option<String>,
}

#[derive(Clone, Debug)]
#[contracttype]
pub enum OptionalProof {
    None,
    Some(Proof),
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
pub struct CredentialOffer {
    pub offer_id: u64,
    pub issuer: Address,
    pub subject_did: String,
    pub credential_type: Vec<String>,
    pub credential_schema: String,
    pub validity_period: u64,
    pub terms: Option<String>,
    pub created_at: u64,
    pub expires_at: u64,
    pub status: OfferStatus,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[contracttype]
#[repr(u32)]
pub enum OfferStatus {
    Pending = 0,
    Accepted = 1,
    Rejected = 2,
    Expired = 3,
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

#[derive(Clone, Debug)]
#[contracttype]
pub struct Presentation {
    pub id: String,
    pub presentation_id: u64,
    pub holder: String, // DID of the holder
    pub verifiable_credential: Vec<VerifiableCredential>,
    pub proof: OptionalProof,
    pub created_at: u64,
    pub expires_at: Option<u64>,
}

#[derive(Clone, Debug)]
#[contracttype]
pub struct CredentialVerification {
    pub verification_id: u64,
    pub credential_id: u64,
    pub verifier: Address,
    pub verification_date: u64,
    pub result: bool,
    pub reason: Option<String>,
    pub disclosed_fields: Vec<String>,
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

#[derive(Clone, Debug)]
#[contracttype]
pub struct CredentialRegistry {
    pub credential_id: u64,
    pub credential: VerifiableCredential,
    pub status: CredentialRegistryStatus,
    pub issuer_signature: Bytes,
    pub subject_signature: Option<Bytes>,
    pub audit_trail: Vec<AuditEntry>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[contracttype]
#[repr(u32)]
pub enum CredentialRegistryStatus {
    Issued = 0,
    Active = 1,
    Suspended = 2,
    Revoked = 3,
    Expired = 4,
}

#[derive(Clone, Debug)]
#[contracttype]
pub struct AuditEntry {
    pub action: String,
    pub actor: Address,
    pub timestamp: u64,
    pub details: Option<String>,
}

// Contract errors
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidCredentialFormat = 1,
    CredentialNotFound = 2,
    UnauthorizedIssuer = 3,
    UnauthorizedSubject = 4,
    CredentialExpired = 5,
    CredentialRevoked = 6,
    CredentialSuspended = 7,
    InvalidSchema = 8,
    InvalidSignature = 9,
    SelectiveDisclosureFailed = 10,
    PresentationExpired = 11,
    InvalidDID = 12,
    MaxFieldsExceeded = 13,
    UnsupportedCredentialType = 14,
    DuplicateCredential = 15,
    InvalidOffer = 16,
    OfferExpired = 17,
    RateLimitExceeded = 18,
    AuditRequired = 19,
    InvalidVerificationMethod = 20,
}

// Contract events
#[contracttype]
pub enum CredentialEvent {
    CredentialIssued(CredentialIssuedEvent),
    CredentialRevoked(CredentialRevokedEvent),
    CredentialSuspended(CredentialSuspendedEvent),
    CredentialVerified(CredentialVerifiedEvent),
    OfferCreated(OfferCreatedEvent),
    OfferAccepted(OfferAcceptedEvent),
    OfferRejected(OfferRejectedEvent),
    SelectiveDisclosureCreated(SelectiveDisclosureCreatedEvent),
    PresentationCreated(PresentationCreatedEvent),
    SchemaCreated(SchemaCreatedEvent),
}

#[derive(Clone)]
#[contracttype]
pub struct CredentialIssuedEvent {
    pub credential_id: u64,
    pub issuer: Address,
    pub subject: String,
    pub credential_type: Vec<String>,
    pub issuance_date: u64,
}

#[derive(Clone)]
#[contracttype]
pub struct CredentialRevokedEvent {
    pub credential_id: u64,
    pub revoked_by: Address,
    pub reason: String,
    pub revocation_date: u64,
}

#[derive(Clone)]
#[contracttype]
pub struct CredentialSuspendedEvent {
    pub credential_id: u64,
    pub suspended_by: Address,
    pub reason: String,
    pub suspension_date: u64,
}

#[derive(Clone)]
#[contracttype]
pub struct CredentialVerifiedEvent {
    pub credential_id: u64,
    pub verifier: Address,
    pub verification_date: u64,
    pub result: bool,
}

#[derive(Clone)]
#[contracttype]
pub struct OfferCreatedEvent {
    pub offer_id: u64,
    pub issuer: Address,
    pub subject_did: String,
    pub credential_type: Vec<String>,
    pub created_at: u64,
}

#[derive(Clone)]
#[contracttype]
pub struct OfferAcceptedEvent {
    pub offer_id: u64,
    pub accepted_by: Address,
    pub accepted_at: u64,
}

#[derive(Clone)]
#[contracttype]
pub struct OfferRejectedEvent {
    pub offer_id: u64,
    pub rejected_by: Address,
    pub rejected_at: u64,
}

#[derive(Clone)]
#[contracttype]
pub struct SelectiveDisclosureCreatedEvent {
    pub disclosure_id: u64,
    pub credential_id: u64,
    pub verifier: Address,
    pub created_at: u64,
}

#[derive(Clone)]
#[contracttype]
pub struct PresentationCreatedEvent {
    pub presentation_id: u64,
    pub holder: String,
    pub credential_count: u32,
    pub created_at: u64,
}

#[derive(Clone)]
#[contracttype]
pub struct SchemaCreatedEvent {
    pub schema_id: String,
    pub author: Address,
    pub name: String,
    pub created_at: u64,
}

// Storage keys
const CREDENTIAL_REGISTRY: Symbol = symbol_short!("cred_reg");
const CREDENTIAL_SCHEMAS: Symbol = symbol_short!("cred_sch");
const CREDENTIAL_OFFERS: Symbol = symbol_short!("cred_off");
const SELECTIVE_DISCLOSURES: Symbol = symbol_short!("sel_disc");
const PRESENTATIONS: Symbol = symbol_short!("present");
const VERIFICATION_RECORDS: Symbol = symbol_short!("ver_rec");
const ISSUER_REGISTRY: Symbol = symbol_short!("iss_reg");
const CREDENTIAL_COUNTER: Symbol = symbol_short!("cred_cnt");
const OFFER_COUNTER: Symbol = symbol_short!("offer_cnt");
const DISCLOSURE_COUNTER: Symbol = symbol_short!("disc_cnt");
const PRESENTATION_COUNTER: Symbol = symbol_short!("pres_cnt");

// Constants
const MAX_CREDENTIAL_FIELDS: u32 = 50;
const MAX_SELECTIVE_DISCLOSURES: u32 = 100;
const MAX_PRESENTATION_CREDENTIALS: u32 = 10;
const MAX_VERIFICATION_RECORDS: u32 = 1000;
const CREDENTIAL_VALIDITY_PERIOD: u64 = 365 * 24 * 60 * 60; // 1 year
const OFFER_VALIDITY_PERIOD: u64 = 7 * 24 * 60 * 60; // 7 days
const DISCLOSURE_VALIDITY_PERIOD: u64 = 24 * 60 * 60; // 24 hours

#[contract]
pub struct VerifiableCredentialsContract;

#[contractimpl]
impl VerifiableCredentialsContract {
    /// Register a new credential schema
    pub fn register_schema(
        env: Env,
        author: Address,
        name: String,
        version: String,
        fields: Vec<SchemaField>,
        required_fields: Vec<String>,
    ) -> Result<String, Error> {
        // Validate inputs
        Self::validate_schema_inputs(&fields, &required_fields)?;

        // Create schema
        let schema_id = Self::generate_schema_id(env.clone(), &author, &name, &version);
        let schema = CredentialSchema {
            id: schema_id.clone(),
            name: name.clone(),
            version: version.clone(),
            author: author.clone(),
            fields: fields.clone(),
            created_at: env.ledger().timestamp(),
            required_fields: required_fields.clone(),
        };

        // Store schema
        env.storage()
            .instance()
            .set(&(CREDENTIAL_SCHEMAS, schema_id.clone()), &schema);

        // Emit event
        env.events().publish(
            (Symbol::new(&env, "SchemaCreated"), schema_id.clone()),
            SchemaCreatedEvent {
                schema_id: schema_id.clone(),
                author: author.clone(),
                name: name.clone(),
                created_at: env.ledger().timestamp(),
            },
        );

        Ok(schema_id)
    }

    /// Issue a new verifiable credential
    pub fn issue_credential(
        env: Env,
        issuer: Address,
        subject_did: String,
        credential_type: Vec<String>,
        credential_schema: String,
        credential_subject: Map<String, String>,
        validity_period: Option<u64>,
        issuer_signature: Bytes,
    ) -> Result<u64, Error> {
        // Validate inputs
        Self::validate_credential_inputs(
            env.clone(),
            &issuer,
            &subject_did,
            &credential_type,
            &credential_schema,
            &credential_subject,
        )?;

        // Get schema
        let schema = Self::get_schema(env.clone(), credential_schema.clone())?;

        // Validate credential against schema
        Self::validate_credential_against_schema(&credential_subject, &schema)?;

        // Generate credential ID
        let credential_id = Self::increment_counter(env.clone(), &CREDENTIAL_COUNTER);
        let credential_id_str = String::from_str(&env, "credential");

        // Create credential status
        let now = env.ledger().timestamp();
        let expiration_date = validity_period.map(|period| now + period);

        let status = CredentialStatus {
            id: String::from_str(&env, "status"),
            type_: String::from_str(&env, "StatusList2021"),
            status: String::from_str(&env, "valid"),
            revoked: false,
            suspended: false,
            revocation_reason: None,
            suspension_reason: None,
            effective_date: now,
        };

        // Create credential
        let credential = VerifiableCredential {
            id: credential_id_str.clone(),
            credential_id,
            issuer: issuer.clone(),
            subject: subject_did.clone(),
            credential_type: credential_type.clone(),
            credential_schema: credential_schema.clone(),
            credential_status: status,
            issuance_date: now,
            expiration_date,
            credential_subject: credential_subject.clone(),
            proof: OptionalProof::None,
            non_revoked: true,
            created_at: now,
            updated_at: now,
        };

        // Create registry entry
        let registry = CredentialRegistry {
            credential_id,
            credential: credential.clone(),
            status: CredentialRegistryStatus::Issued,
            issuer_signature: issuer_signature.clone(),
            subject_signature: None,
            audit_trail: Vec::new(&env),
        };

        // Store credential
        env.storage()
            .instance()
            .set(&(CREDENTIAL_REGISTRY, credential_id), &registry);

        // Emit event
        env.events().publish(
            (Symbol::new(&env, "CredentialIssued"), &credential_id),
            CredentialIssuedEvent {
                credential_id,
                issuer: issuer.clone(),
                subject: subject_did.clone(),
                credential_type: credential_type.clone(),
                issuance_date: now,
            },
        );

        Ok(credential_id)
    }

    /// Revoke a credential
    pub fn revoke_credential(
        env: Env,
        credential_id: u64,
        issuer: Address,
        reason: String,
    ) -> Result<(), Error> {
        // Get credential registry
        let mut registry = Self::get_credential_registry(env.clone(), credential_id)?;

        // Validate authorization
        if registry.credential.issuer != issuer {
            return Err(Error::UnauthorizedIssuer);
        }

        // Check if credential can be revoked
        if registry.credential.credential_status.revoked {
            return Err(Error::CredentialRevoked);
        }

        // Update status
        let now = env.ledger().timestamp();
        registry.credential.credential_status.revoked = true;
        registry.credential.credential_status.revocation_reason = Some(reason.clone());
        registry.credential.credential_status.effective_date = now;
        registry.credential.non_revoked = false;
        registry.status = CredentialRegistryStatus::Revoked;

        // Add audit entry
        let audit_entry = AuditEntry {
            action: String::from_str(&env, "revoked"),
            actor: issuer.clone(),
            timestamp: now,
            details: Some(reason.clone()),
        };
        registry.audit_trail.push_back(audit_entry);

        // Store updated registry
        env.storage()
            .instance()
            .set(&(CREDENTIAL_REGISTRY, credential_id), &registry);

        // Emit event
        env.events().publish(
            (Symbol::new(&env, "CredentialRevoked"), &credential_id),
            CredentialRevokedEvent {
                credential_id,
                revoked_by: issuer.clone(),
                reason: reason.clone(),
                revocation_date: now,
            },
        );

        Ok(())
    }

    /// Create a credential offer
    pub fn create_offer(
        env: Env,
        issuer: Address,
        subject_did: String,
        credential_type: Vec<String>,
        credential_schema: String,
        validity_period: u64,
        terms: Option<String>,
    ) -> Result<u64, Error> {
        // Validate inputs
        Self::validate_offer_inputs(
            env.clone(),
            &issuer,
            &subject_did,
            &credential_type,
            &credential_schema,
        )?;

        // Generate offer ID
        let offer_id = Self::increment_counter(env.clone(), &OFFER_COUNTER);
        let now = env.ledger().timestamp();

        // Create offer
        let offer = CredentialOffer {
            offer_id,
            issuer: issuer.clone(),
            subject_did: subject_did.clone(),
            credential_type: credential_type.clone(),
            credential_schema: credential_schema.clone(),
            validity_period,
            terms: terms.clone(),
            created_at: now,
            expires_at: now + OFFER_VALIDITY_PERIOD,
            status: OfferStatus::Pending,
        };

        // Store offer
        env.storage()
            .instance()
            .set(&(CREDENTIAL_OFFERS, offer_id), &offer);

        // Emit event
        env.events().publish(
            (Symbol::new(&env, "OfferCreated"), &offer_id),
            OfferCreatedEvent {
                offer_id,
                issuer: issuer.clone(),
                subject_did: subject_did.clone(),
                credential_type: credential_type.clone(),
                created_at: now,
            },
        );

        Ok(offer_id)
    }

    /// Accept a credential offer
    pub fn accept_offer(env: Env, offer_id: u64, subject: Address) -> Result<(), Error> {
        // Get offer
        let mut offer = Self::get_offer(env.clone(), offer_id)?;

        // Validate offer
        if offer.status != OfferStatus::Pending {
            return Err(Error::InvalidOffer);
        }

        if env.ledger().timestamp() > offer.expires_at {
            return Err(Error::OfferExpired);
        }

        // Update offer status
        offer.status = OfferStatus::Accepted;
        env.storage()
            .instance()
            .set(&(CREDENTIAL_OFFERS, offer_id), &offer);

        // Emit event
        env.events().publish(
            (Symbol::new(&env, "OfferAccepted"), &offer_id),
            OfferAcceptedEvent {
                offer_id,
                accepted_by: subject.clone(),
                accepted_at: env.ledger().timestamp(),
            },
        );

        Ok(())
    }

    /// Create selective disclosure
    pub fn create_selective_disclosure(
        env: Env,
        credential_id: u64,
        verifier: Address,
        disclosed_fields: Vec<String>,
        nonce: String,
    ) -> Result<u64, Error> {
        // Get credential
        let registry = Self::get_credential_registry(env.clone(), credential_id)?;

        // Validate credential
        if registry.credential.credential_status.revoked {
            return Err(Error::CredentialRevoked);
        }

        if registry.credential.credential_status.suspended {
            return Err(Error::CredentialSuspended);
        }

        // Check expiration
        if let Some(expiration) = registry.credential.expiration_date {
            if env.ledger().timestamp() > expiration {
                return Err(Error::CredentialExpired);
            }
        }

        // Validate disclosed fields
        Self::validate_disclosed_fields(
            &registry.credential.credential_subject,
            &disclosed_fields,
        )?;

        // Generate disclosure ID
        let disclosure_id = Self::increment_counter(env.clone(), &DISCLOSURE_COUNTER);
        let now = env.ledger().timestamp();

        // Create disclosure
        let disclosure = SelectiveDisclosure {
            disclosure_id,
            credential_id,
            verifier: verifier.clone(),
            subject: registry.credential.subject.clone(),
            disclosed_fields: disclosed_fields.clone(),
            nonce: nonce.clone(),
            created_at: now,
            expires_at: now + DISCLOSURE_VALIDITY_PERIOD,
            presentation_hash: Self::generate_presentation_hash(
                env.clone(),
                credential_id,
                &disclosed_fields,
                &nonce,
            ),
            verified: false,
        };

        // Store disclosure
        env.storage()
            .instance()
            .set(&(SELECTIVE_DISCLOSURES, disclosure_id), &disclosure);

        // Emit event
        env.events().publish(
            (
                Symbol::new(&env, "SelectiveDisclosureCreated"),
                &disclosure_id,
            ),
            SelectiveDisclosureCreatedEvent {
                disclosure_id,
                credential_id,
                verifier: verifier.clone(),
                created_at: now,
            },
        );

        Ok(disclosure_id)
    }

    /// Verify a credential
    pub fn verify_credential(
        env: Env,
        credential_id: u64,
        verifier: Address,
        disclosed_fields: Option<Vec<String>>,
    ) -> Result<bool, Error> {
        // Get credential
        let registry = Self::get_credential_registry(env.clone(), credential_id)?;

        let now = env.ledger().timestamp();
        let mut result = true;
        let mut reason = None;

        // Check revocation status
        if registry.credential.credential_status.revoked {
            result = false;
            reason = Some(String::from_str(&env, "Credential revoked"));
        }

        // Check suspension status
        if registry.credential.credential_status.suspended {
            result = false;
            reason = Some(String::from_str(&env, "Credential suspended"));
        }

        // Check expiration
        if let Some(expiration) = registry.credential.expiration_date {
            if now > expiration {
                result = false;
                reason = Some(String::from_str(&env, "Credential expired"));
            }
        }

        // Create verification record
        let verification_id = Self::increment_counter(env.clone(), &VERIFICATION_RECORDS);
        let verification = CredentialVerification {
            verification_id,
            credential_id,
            verifier: verifier.clone(),
            verification_date: now,
            result,
            reason: reason.clone(),
            disclosed_fields: disclosed_fields.unwrap_or(Vec::new(&env)),
        };

        // Store verification record
        env.storage()
            .instance()
            .set(&(VERIFICATION_RECORDS, verification_id), &verification);

        // Emit event
        env.events().publish(
            (Symbol::new(&env, "CredentialVerified"), &credential_id),
            CredentialVerifiedEvent {
                credential_id,
                verifier: verifier.clone(),
                verification_date: now,
                result,
            },
        );

        Ok(result)
    }

    /// Get credential by ID
    pub fn get_credential(env: Env, credential_id: u64) -> Result<VerifiableCredential, Error> {
        let registry = Self::get_credential_registry(env, credential_id)?;
        Ok(registry.credential)
    }

    /// Get credential registry with full details
    pub fn get_credential_registry(
        env: Env,
        credential_id: u64,
    ) -> Result<CredentialRegistry, Error> {
        let registry: Option<CredentialRegistry> = env
            .storage()
            .instance()
            .get(&(CREDENTIAL_REGISTRY, credential_id));
        registry.ok_or(Error::CredentialNotFound)
    }

    /// Get schema by ID
    pub fn get_schema(env: Env, schema_id: String) -> Result<CredentialSchema, Error> {
        let schema: Option<CredentialSchema> = env
            .storage()
            .instance()
            .get(&(CREDENTIAL_SCHEMAS, schema_id));
        schema.ok_or(Error::InvalidSchema)
    }

    /// Get offer by ID
    pub fn get_offer(env: Env, offer_id: u64) -> Result<CredentialOffer, Error> {
        let offer: Option<CredentialOffer> =
            env.storage().instance().get(&(CREDENTIAL_OFFERS, offer_id));
        offer.ok_or(Error::InvalidOffer)
    }

    /// Get selective disclosure by ID
    pub fn get_selective_disclosure(
        env: Env,
        disclosure_id: u64,
    ) -> Result<SelectiveDisclosure, Error> {
        let disclosure: Option<SelectiveDisclosure> = env
            .storage()
            .instance()
            .get(&(SELECTIVE_DISCLOSURES, disclosure_id));
        disclosure.ok_or(Error::SelectiveDisclosureFailed)
    }

    /// Get verification records for a credential
    pub fn get_verification_records(
        env: Env,
        credential_id: u64,
        limit: u32,
    ) -> Result<Vec<CredentialVerification>, Error> {
        let records = Vec::new(&env);
        let _counter_key = (VERIFICATION_RECORDS, credential_id);

        // In a real implementation, we'd store records by credential ID
        // For now, return empty vector
        Ok(records)
    }

    // Helper functions
    fn validate_schema_inputs(
        fields: &Vec<SchemaField>,
        required_fields: &Vec<String>,
    ) -> Result<(), Error> {
        if fields.len() > MAX_CREDENTIAL_FIELDS {
            return Err(Error::MaxFieldsExceeded);
        }

        // Check if all required fields exist in the schema
        for required_field in required_fields {
            if !fields.iter().any(|f| f.name == required_field) {
                return Err(Error::InvalidSchema);
            }
        }

        Ok(())
    }

    fn validate_credential_inputs(
        env: Env,
        _issuer: &Address,
        subject_did: &String,
        _credential_type: &Vec<String>,
        credential_schema: &String,
        credential_subject: &Map<String, String>,
    ) -> Result<(), Error> {
        // Validate DID format
        if subject_did.is_empty() {
            return Err(Error::InvalidDID);
        }

        // Validate credential subject fields
        if credential_subject.len() > MAX_CREDENTIAL_FIELDS {
            return Err(Error::MaxFieldsExceeded);
        }

        // Check if schema exists
        Self::get_schema(env, credential_schema.clone())?;

        Ok(())
    }

    fn validate_credential_against_schema(
        credential_subject: &Map<String, String>,
        schema: &CredentialSchema,
    ) -> Result<(), Error> {
        // Check all required fields are present
        for required_field in &schema.required_fields {
            if !credential_subject.contains_key(required_field) {
                return Err(Error::InvalidSchema);
            }
        }

        // Check all fields are defined in schema
        for (field_name, _) in credential_subject.iter() {
            if !schema.fields.iter().any(|f| f.name == field_name) {
                return Err(Error::InvalidSchema);
            }
        }

        Ok(())
    }

    fn validate_offer_inputs(
        env: Env,
        _issuer: &Address,
        subject_did: &String,
        _credential_type: &Vec<String>,
        credential_schema: &String,
    ) -> Result<(), Error> {
        // Validate DID format
        if subject_did.is_empty() {
            return Err(Error::InvalidDID);
        }

        // Check if schema exists
        Self::get_schema(env, credential_schema.clone())?;

        Ok(())
    }

    fn validate_disclosed_fields(
        credential_subject: &Map<String, String>,
        disclosed_fields: &Vec<String>,
    ) -> Result<(), Error> {
        // Check if all disclosed fields exist in the credential
        for field in disclosed_fields {
            if !credential_subject.contains_key(field) {
                return Err(Error::SelectiveDisclosureFailed);
            }
        }

        Ok(())
    }

    fn generate_schema_id(env: Env, author: &Address, name: &String, version: &String) -> String {
        let _ = (author, name, version);
        let _ = (author, name, version);
        String::from_str(&env, "schema")
    }

    fn generate_presentation_hash(
        env: Env,
        credential_id: u64,
        disclosed_fields: &Vec<String>,
        nonce: &String,
    ) -> String {
        let _ = (credential_id, disclosed_fields, nonce);
        String::from_str(&env, "presentation")
    }

    fn increment_counter(env: Env, counter_key: &Symbol) -> u64 {
        let count: u64 = env.storage().instance().get(counter_key).unwrap_or(0);
        let new_count = count + 1;
        env.storage().instance().set(counter_key, &new_count);
        new_count
    }
}
