/// RBAC (Role-Based Access Control) helpers — Issue #152
///
/// All role checks read directly from on-chain storage on every call.
/// No caller context is implicitly trusted; every internal or indirect
/// call path must go through one of these functions.
use soroban_sdk::{Address, Env, Symbol, Vec};

use crate::{errors::ContractError, ADMIN_KEY, APPROVED_MINTERS_KEY};

// ── Admin ────────────────────────────────────────────────────────────────────

/// Return the stored admin address, or `Unauthorized` if not initialised.
pub fn get_admin(env: &Env) -> Result<Address, ContractError> {
    env.storage()
        .instance()
        .get::<_, Address>(&Symbol::new(env, ADMIN_KEY))
        .ok_or(ContractError::Unauthorized)
}

/// Assert that `caller` is the stored admin.
/// Always re-reads from storage — never trusts a passed-in reference.
pub fn require_admin(env: &Env, caller: &Address) -> Result<(), ContractError> {
    let admin = get_admin(env)?;
    if caller != &admin {
        return Err(ContractError::RoleEscalationAttempt);
    }
    Ok(())
}

// ── Minter ───────────────────────────────────────────────────────────────────

/// Assert that `caller` is the admin **or** an approved minter.
/// Always re-reads the approved-minters list from storage.
pub fn require_minter(env: &Env, caller: &Address) -> Result<(), ContractError> {
    // Admin is always allowed
    if let Ok(admin) = get_admin(env) {
        if caller == &admin {
            return Ok(());
        }
    }

    let minters: Vec<Address> = env
        .storage()
        .instance()
        .get(&Symbol::new(env, APPROVED_MINTERS_KEY))
        .unwrap_or_else(|| Vec::new(env));

    for m in minters.iter() {
        if &m == caller {
            return Ok(());
        }
    }

    Err(ContractError::RoleEscalationAttempt)
}

// ── Operator ─────────────────────────────────────────────────────────────────

/// Assert that `caller` is the owner of `agent_id` **or** an authorised,
/// non-expired operator for that agent.
///
/// `get_owner_fn`    – closure that returns the stored owner for `agent_id`.
/// `get_operator_fn` – closure that returns `Option<(operator, expires_at)>`.
pub fn require_owner_or_operator<FO, FP>(
    env: &Env,
    caller: &Address,
    agent_id: u64,
    get_owner_fn: FO,
    get_operator_fn: FP,
) -> Result<(), ContractError>
where
    FO: Fn(&Env, u64) -> Option<Address>,
    FP: Fn(&Env, u64) -> Option<(Address, u64)>,
{
    // Re-read owner from storage — never trust the caller's claim
    if let Some(owner) = get_owner_fn(env, agent_id) {
        if caller == &owner {
            return Ok(());
        }
    }

    // Check operator authorisation from storage
    if let Some((operator, expires_at)) = get_operator_fn(env, agent_id) {
        if caller == &operator {
            if env.ledger().timestamp() < expires_at {
                return Ok(());
            }
            // Operator exists but is expired — explicit escalation attempt
            return Err(ContractError::RoleEscalationAttempt);
        }
    }

    Err(ContractError::RoleEscalationAttempt)
}

// ── Oracle provider ──────────────────────────────────────────────────────────

/// Assert that `caller` is in the registered oracle-provider list.
/// Always re-reads from storage.
pub fn require_oracle_provider(
    env: &Env,
    caller: &Address,
    provider_list_key: &str,
) -> Result<(), ContractError> {
    let providers: Vec<Address> = env
        .storage()
        .instance()
        .get(&Symbol::new(env, provider_list_key))
        .unwrap_or_else(|| Vec::new(env));

    for p in providers.iter() {
        if &p == caller {
            return Ok(());
        }
    }

    Err(ContractError::RoleEscalationAttempt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger as _};
    use soroban_sdk::{contract, contractimpl, Env};

    #[contract]
    struct RbacHarness;
    #[contractimpl]
    impl RbacHarness {}

    fn setup() -> (Env, soroban_sdk::Address) {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(RbacHarness, ());
        (env, contract_id)
    }

    // ── require_admin ────────────────────────────────────────────────────────

    #[test]
    fn require_admin_passes_for_stored_admin() {
        let (env, cid) = setup();
        let admin = Address::generate(&env);
        env.as_contract(&cid, || {
            env.storage()
                .instance()
                .set(&Symbol::new(&env, ADMIN_KEY), &admin);
            assert!(require_admin(&env, &admin).is_ok());
        });
    }

    #[test]
    fn require_admin_rejects_non_admin() {
        let (env, cid) = setup();
        let admin = Address::generate(&env);
        let attacker = Address::generate(&env);
        env.as_contract(&cid, || {
            env.storage()
                .instance()
                .set(&Symbol::new(&env, ADMIN_KEY), &admin);
            let err = require_admin(&env, &attacker).unwrap_err();
            assert_eq!(err, ContractError::RoleEscalationAttempt);
        });
    }

    /// Indirect escalation: attacker passes admin address as argument but is
    /// not the stored admin — must be rejected.
    #[test]
    fn require_admin_indirect_escalation_rejected() {
        let (env, cid) = setup();
        let real_admin = Address::generate(&env);
        let attacker = Address::generate(&env);
        env.as_contract(&cid, || {
            env.storage()
                .instance()
                .set(&Symbol::new(&env, ADMIN_KEY), &real_admin);
            // Attacker passes real_admin's address but is not that address
            let err = require_admin(&env, &attacker).unwrap_err();
            assert_eq!(err, ContractError::RoleEscalationAttempt);
        });
    }

    // ── require_minter ───────────────────────────────────────────────────────

    #[test]
    fn require_minter_passes_for_admin() {
        let (env, cid) = setup();
        let admin = Address::generate(&env);
        env.as_contract(&cid, || {
            env.storage()
                .instance()
                .set(&Symbol::new(&env, ADMIN_KEY), &admin);
            assert!(require_minter(&env, &admin).is_ok());
        });
    }

    #[test]
    fn require_minter_passes_for_approved_minter() {
        let (env, cid) = setup();
        let admin = Address::generate(&env);
        let minter = Address::generate(&env);
        env.as_contract(&cid, || {
            env.storage()
                .instance()
                .set(&Symbol::new(&env, ADMIN_KEY), &admin);
            let mut list: Vec<Address> = Vec::new(&env);
            list.push_back(minter.clone());
            env.storage()
                .instance()
                .set(&Symbol::new(&env, APPROVED_MINTERS_KEY), &list);
            assert!(require_minter(&env, &minter).is_ok());
        });
    }

    #[test]
    fn require_minter_rejects_unapproved_caller() {
        let (env, cid) = setup();
        let admin = Address::generate(&env);
        let attacker = Address::generate(&env);
        env.as_contract(&cid, || {
            env.storage()
                .instance()
                .set(&Symbol::new(&env, ADMIN_KEY), &admin);
            let list: Vec<Address> = Vec::new(&env);
            env.storage()
                .instance()
                .set(&Symbol::new(&env, APPROVED_MINTERS_KEY), &list);
            let err = require_minter(&env, &attacker).unwrap_err();
            assert_eq!(err, ContractError::RoleEscalationAttempt);
        });
    }

    // ── require_owner_or_operator ────────────────────────────────────────────

    #[test]
    fn require_owner_or_operator_passes_for_owner() {
        let (env, cid) = setup();
        let owner = Address::generate(&env);
        env.as_contract(&cid, || {
            let result = require_owner_or_operator(
                &env,
                &owner,
                1u64,
                |_e, _id| Some(owner.clone()),
                |_e, _id| None,
            );
            assert!(result.is_ok());
        });
    }

    #[test]
    fn require_owner_or_operator_passes_for_valid_operator() {
        let (env, cid) = setup();
        let owner = Address::generate(&env);
        let operator = Address::generate(&env);
        env.ledger().set_timestamp(500);
        let expires_at = 1000u64; // strictly after current timestamp
        env.as_contract(&cid, || {
            let result = require_owner_or_operator(
                &env,
                &operator,
                1u64,
                |_e, _id| Some(owner.clone()),
                |_e, _id| Some((operator.clone(), expires_at)),
            );
            assert!(result.is_ok());
        });
    }

    #[test]
    fn require_owner_or_operator_rejects_expired_operator() {
        let (env, cid) = setup();
        let owner = Address::generate(&env);
        let operator = Address::generate(&env);
        // Set ledger time ahead so expires_at is clearly in the past
        env.ledger().set_timestamp(1000);
        let expires_at = 500u64; // strictly before current timestamp
        env.as_contract(&cid, || {
            let result = require_owner_or_operator(
                &env,
                &operator,
                1u64,
                |_e, _id| Some(owner.clone()),
                |_e, _id| Some((operator.clone(), expires_at)),
            );
            assert_eq!(result.unwrap_err(), ContractError::RoleEscalationAttempt);
        });
    }

    /// Indirect escalation: attacker claims to be operator but is not stored.
    #[test]
    fn require_owner_or_operator_rejects_indirect_escalation() {
        let (env, cid) = setup();
        let owner = Address::generate(&env);
        let real_operator = Address::generate(&env);
        let attacker = Address::generate(&env);
        let expires_at = env.ledger().timestamp() + 1000;
        env.as_contract(&cid, || {
            // Storage has real_operator, attacker tries to act as operator
            let result = require_owner_or_operator(
                &env,
                &attacker,
                1u64,
                |_e, _id| Some(owner.clone()),
                |_e, _id| Some((real_operator.clone(), expires_at)),
            );
            assert_eq!(result.unwrap_err(), ContractError::RoleEscalationAttempt);
        });
    }

    // ── require_oracle_provider ──────────────────────────────────────────────

    #[test]
    fn require_oracle_provider_passes_for_registered() {
        let (env, cid) = setup();
        let provider = Address::generate(&env);
        env.as_contract(&cid, || {
            let mut list: Vec<Address> = Vec::new(&env);
            list.push_back(provider.clone());
            env.storage()
                .instance()
                .set(&Symbol::new(&env, "providers"), &list);
            assert!(require_oracle_provider(&env, &provider, "providers").is_ok());
        });
    }

    #[test]
    fn require_oracle_provider_rejects_unregistered() {
        let (env, cid) = setup();
        let attacker = Address::generate(&env);
        env.as_contract(&cid, || {
            let list: Vec<Address> = Vec::new(&env);
            env.storage()
                .instance()
                .set(&Symbol::new(&env, "providers"), &list);
            let err = require_oracle_provider(&env, &attacker, "providers").unwrap_err();
            assert_eq!(err, ContractError::RoleEscalationAttempt);
        });
    }
}
