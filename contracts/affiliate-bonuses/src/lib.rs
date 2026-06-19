#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, String, Symbol, Vec};
use stellai_lib::{errors::ContractError, ADMIN_KEY};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AffiliateInfo {
    pub affiliate: Address,
    pub referral_code: String,
    pub total_users_referred: u32,
    pub total_volume_generated: i128,
    pub total_commissions_earned: i128,
    pub created_at: u64,
    pub last_activity_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReferralRecord {
    pub referred_user: Address,
    pub affiliate: Address,
    pub referral_code: String,
    pub referred_at: u64,
    pub first_transaction_at: Option<u64>,
    pub total_volume: i128,
    pub commission_paid: i128,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[contracttype]
pub struct CommissionConfig {
    pub commission_rate_bps: u32, // Basis points (100 = 1%)
    pub min_volume_for_commission: i128,
    pub payout_threshold: i128,
    pub max_commission_per_user: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VolumeRecord {
    pub user: Address,
    pub affiliate: Address,
    pub volume: i128,
    pub timestamp: u64,
    pub transaction_id: String,
}

#[contract]
pub struct AffiliateBonuses;

#[contractimpl]
impl AffiliateBonuses {
    /// Initialize the affiliate bonuses contract.
    pub fn init_contract(
        env: Env,
        admin_addr: Address,
        config: CommissionConfig,
    ) -> Result<(), ContractError> {
        if env.storage().instance().has(&Symbol::new(&env, ADMIN_KEY)) {
            return Err(ContractError::AlreadyInitialized);
        }
        admin_addr.require_auth();

        // Validate configuration
        if config.commission_rate_bps > 5000 {
            // Max 50%
            return Err(ContractError::InvalidAgentId);
        }
        if config.min_volume_for_commission < 0
            || config.payout_threshold < 0
            || config.max_commission_per_user < 0
        {
            return Err(ContractError::InvalidAgentId);
        }

        env.storage()
            .instance()
            .set(&Symbol::new(&env, ADMIN_KEY), &admin_addr);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "config"), &config);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "affiliate_counter"), &0u64);

        Ok(())
    }

    /// Register as an affiliate with a unique referral code.
    pub fn register_affiliate(
        env: Env,
        affiliate: Address,
        referral_code: String,
    ) -> Result<(), ContractError> {
        affiliate.require_auth();

        // Validate referral code
        if referral_code.len() < 3 || referral_code.len() > 20 {
            return Err(ContractError::InvalidAgentId);
        }

        // Check if referral code already exists
        let code_key = (Symbol::new(&env, "code"), referral_code.clone());
        if env.storage().instance().has(&code_key) {
            return Err(ContractError::AlreadyInitialized);
        }

        // Check if already registered as affiliate
        let affiliate_key = (Symbol::new(&env, "affiliate"), affiliate.clone());
        if env.storage().instance().has(&affiliate_key) {
            return Err(ContractError::AlreadyInitialized);
        }

        let now = env.ledger().timestamp();
        let info = AffiliateInfo {
            affiliate: affiliate.clone(),
            referral_code: referral_code.clone(),
            total_users_referred: 0,
            total_volume_generated: 0,
            total_commissions_earned: 0,
            created_at: now,
            last_activity_at: now,
        };

        env.storage().instance().set(&code_key, &affiliate);
        env.storage().instance().set(&affiliate_key, &info);

        env.events().publish(
            (
                Symbol::new(&env, "affiliate"),
                Symbol::new(&env, "registered"),
            ),
            (affiliate, referral_code),
        );

        Ok(())
    }

    /// Record a new user referral using a referral code.
    pub fn record_referral(
        env: Env,
        referred_user: Address,
        referral_code: String,
    ) -> Result<(), ContractError> {
        referred_user.require_auth();

        // Find affiliate by referral code
        let code_key = (Symbol::new(&env, "code"), referral_code.clone());
        let affiliate: Address = env
            .storage()
            .instance()
            .get(&code_key)
            .ok_or(ContractError::InvalidAgentId)?; // Invalid referral code

        // Check if user already referred
        let user_referral_key = (Symbol::new(&env, "user_referral"), referred_user.clone());
        if env.storage().instance().has(&user_referral_key) {
            return Err(ContractError::AlreadyInitialized);
        }

        let now = env.ledger().timestamp();
        let record = ReferralRecord {
            referred_user: referred_user.clone(),
            affiliate: affiliate.clone(),
            referral_code: referral_code.clone(),
            referred_at: now,
            first_transaction_at: None,
            total_volume: 0,
            commission_paid: 0,
        };

        let referral_key = (Symbol::new(&env, "referral"), referred_user.clone());
        env.storage().instance().set(&referral_key, &record);
        env.storage().instance().set(&user_referral_key, &affiliate);

        // Update affiliate stats
        let affiliate_key = (Symbol::new(&env, "affiliate"), affiliate.clone());
        let mut info: AffiliateInfo = env.storage().instance().get(&affiliate_key).unwrap();
        info.total_users_referred += 1;
        info.last_activity_at = now;
        env.storage().instance().set(&affiliate_key, &info);

        env.events().publish(
            (Symbol::new(&env, "referral"), Symbol::new(&env, "recorded")),
            (referred_user, affiliate, referral_code),
        );

        Ok(())
    }

    /// Track transaction volume and calculate commissions.
    pub fn track_volume(
        env: Env,
        caller: Address,
        user: Address,
        volume: i128,
        transaction_id: String,
    ) -> Result<(), ContractError> {
        caller.require_auth();

        // ─── SNAPSHOT PHASE ───
        let user_referral_key = (Symbol::new(&env, "user_referral"), user.clone());
        let affiliate: Address = env
            .storage()
            .instance()
            .get(&user_referral_key)
            .ok_or(ContractError::InvalidAgentId)?; // User not referred

        let config: CommissionConfig = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, "config"))
            .ok_or(ContractError::NotInitialized)?;

        let volume_key = (Symbol::new(&env, "volume"), transaction_id.clone());
        let volume_exists = env.storage().instance().has(&volume_key);

        let referral_key = (Symbol::new(&env, "referral"), user.clone());
        let mut referral_record: ReferralRecord = env
            .storage()
            .instance()
            .get(&referral_key)
            .ok_or(ContractError::InvalidAgentId)?;

        let commission_key = (Symbol::new(&env, "commission"), affiliate.clone());
        let mut pending_commissions: i128 =
            env.storage().instance().get(&commission_key).unwrap_or(0);

        let user_commission_key = (Symbol::new(&env, "user_commission"), affiliate.clone());
        let mut user_total_commission: i128 = env
            .storage()
            .instance()
            .get(&user_commission_key)
            .unwrap_or(0);

        let affiliate_key = (Symbol::new(&env, "affiliate"), affiliate.clone());
        let mut affiliate_info: AffiliateInfo =
            env.storage().instance().get(&affiliate_key).unwrap();

        let now = env.ledger().timestamp();

        // ─── VALIDATION PHASE ───
        if volume <= 0 {
            return Err(ContractError::InvalidAgentId);
        }

        if volume_exists {
            return Err(ContractError::AlreadyInitialized); // Transaction already tracked
        }

        // ─── MUTATION PHASE ───

        // Record volume
        let volume_record = VolumeRecord {
            user: user.clone(),
            affiliate: affiliate.clone(),
            volume,
            timestamp: now,
            transaction_id: transaction_id.clone(),
        };
        env.storage().instance().set(&volume_key, &volume_record);

        // Update referral record
        if referral_record.first_transaction_at.is_none() {
            referral_record.first_transaction_at = Some(now);
        }
        referral_record.total_volume += volume;
        env.storage()
            .instance()
            .set(&referral_key, &referral_record);

        // Calculate and record commission if threshold met
        let commission_amount = (volume * config.commission_rate_bps as i128) / 10000;

        if commission_amount > 0
            && referral_record.total_volume >= config.min_volume_for_commission
            && user_total_commission + commission_amount <= config.max_commission_per_user
        {
            pending_commissions += commission_amount;
            user_total_commission += commission_amount;

            env.storage()
                .instance()
                .set(&commission_key, &pending_commissions);
            env.storage()
                .instance()
                .set(&user_commission_key, &user_total_commission);

            // Update affiliate stats
            affiliate_info.total_volume_generated += volume;
            affiliate_info.total_commissions_earned += commission_amount;
            affiliate_info.last_activity_at = now;
            env.storage()
                .instance()
                .set(&affiliate_key, &affiliate_info);

            env.events().publish(
                (Symbol::new(&env, "commission"), Symbol::new(&env, "earned")),
                (affiliate, user, commission_amount, volume),
            );
        }

        Ok(())
    }

    /// Claim pending commissions.
    pub fn claim_commissions(env: Env, affiliate: Address) -> Result<i128, ContractError> {
        affiliate.require_auth();

        let config: CommissionConfig = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, "config"))
            .ok_or(ContractError::NotInitialized)?;

        let commission_key = (Symbol::new(&env, "commission"), affiliate.clone());
        let pending_commissions: i128 = env.storage().instance().get(&commission_key).unwrap_or(0);

        if pending_commissions < config.payout_threshold {
            return Err(ContractError::InvalidAgentId); // Below threshold
        }

        if pending_commissions <= 0 {
            return Ok(0);
        }

        // Reset pending commissions
        env.storage().instance().set(&commission_key, &0i128);

        env.events().publish(
            (
                Symbol::new(&env, "commission"),
                Symbol::new(&env, "claimed"),
            ),
            (affiliate, pending_commissions),
        );

        Ok(pending_commissions)
    }

    /// Get affiliate information.
    pub fn get_affiliate_info(
        env: Env,
        affiliate: Address,
    ) -> Result<AffiliateInfo, ContractError> {
        let key = (Symbol::new(&env, "affiliate"), affiliate);
        env.storage()
            .instance()
            .get(&key)
            .ok_or(ContractError::InvalidAgentId)
    }

    /// Get referral record for a user.
    pub fn get_referral_record(env: Env, user: Address) -> Result<ReferralRecord, ContractError> {
        let key = (Symbol::new(&env, "referral"), user);
        env.storage()
            .instance()
            .get(&key)
            .ok_or(ContractError::InvalidAgentId)
    }

    /// Get pending commissions for an affiliate.
    pub fn get_pending_commissions(env: Env, affiliate: Address) -> i128 {
        let key = (Symbol::new(&env, "commission"), affiliate);
        env.storage().instance().get(&key).unwrap_or(0)
    }

    /// Get affiliate by referral code.
    pub fn get_affiliate_by_code(
        env: Env,
        referral_code: String,
    ) -> Result<Address, ContractError> {
        let key = (Symbol::new(&env, "code"), referral_code);
        env.storage()
            .instance()
            .get(&key)
            .ok_or(ContractError::InvalidAgentId)
    }

    /// Get commission configuration.
    pub fn get_commission_config(env: Env) -> Result<CommissionConfig, ContractError> {
        env.storage()
            .instance()
            .get(&Symbol::new(&env, "config"))
            .ok_or(ContractError::NotInitialized)
    }

    /// Get top affiliates by volume.
    pub fn get_top_affiliates(env: Env, limit: u32) -> Vec<AffiliateInfo> {
        let affiliate_counter: u64 = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, "affiliate_counter"))
            .unwrap_or(0);
        let affiliates = Vec::new(&env);

        // This is a simplified approach - in production, you'd want a more efficient ranking system
        // Note: For now, this is a placeholder that does not return real data to satisfy the compiler
        for _ in 1..=affiliate_counter {
            if affiliates.len() >= limit {
                break;
            }
            // In a real implementation, we would iterate over a list of registered affiliates
        }

        affiliates
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn test_affiliate_flow() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let affiliate = Address::generate(&env);
        let user = Address::generate(&env);

        let config = CommissionConfig {
            commission_rate_bps: 500, // 5%
            min_volume_for_commission: 1000,
            payout_threshold: 500,
            max_commission_per_user: 10000,
        };

        let contract_id = env.register_contract(None, AffiliateBonuses);
        let client = AffiliateBonusesClient::new(&env, &contract_id);

        client.init_contract(&admin, &config);

        // Register affiliate
        client.register_affiliate(&affiliate, &String::from_str(&env, "REF123"));

        // Record referral
        client.record_referral(&user, &String::from_str(&env, "REF123"));

        // Track volume
        client.track_volume(&user, &user, &2000, &String::from_str(&env, "tx1"));

        // Check affiliate info
        let info = client.get_affiliate_info(&affiliate);
        assert_eq!(info.total_users_referred, 1);
        assert_eq!(info.total_volume_generated, 2000);
        assert_eq!(info.total_commissions_earned, 100); // 5% of 2000

        // Check pending commissions
        let pending = client.get_pending_commissions(&affiliate);
        assert_eq!(pending, 100);

        // Claim commissions (above threshold of 500, so this should fail)
        let result = client.try_claim_commissions(&affiliate);
        assert!(result.is_err());

        // Track more volume to reach threshold
        client.track_volume(&user, &user, &10000, &String::from_str(&env, "tx2"));

        // Now claim should work
        let claimed = client.claim_commissions(&affiliate);
        assert!(claimed >= 500);
    }
}
