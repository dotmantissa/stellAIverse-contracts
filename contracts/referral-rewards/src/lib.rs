#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol, Vec};
use stellai_lib::{admin, errors::ContractError, ADMIN_KEY};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReferralInfo {
    pub referrer: Address,
    pub referred_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReferralStats {
    pub direct_referrals: u32,
    pub indirect_referrals: u32,
    pub total_earnings: i128,
    pub pending_rewards: i128,
}

#[contract]
pub struct ReferralRewards;

const TIER1_VOLUME: i128 = 1000;
const TIER2_VOLUME: i128 = 10000;
const TIER1_RATE: i128 = 5; // 5%
const TIER2_RATE: i128 = 7;
const TIER3_RATE: i128 = 10;
const INDIRECT_MULTIPLIER: i128 = 50; // 50% of direct

#[contractimpl]
impl ReferralRewards {
    /// Initialize the contract with an admin address.
    pub fn init_contract(env: Env, admin_addr: Address) -> Result<(), ContractError> {
        if env.storage().instance().has(&Symbol::new(&env, ADMIN_KEY)) {
            return Err(ContractError::AlreadyInitialized);
        }
        admin_addr.require_auth();
        env.storage()
            .instance()
            .set(&Symbol::new(&env, ADMIN_KEY), &admin_addr);
        Ok(())
    }

    /// Register a new referral.
    pub fn register_referral(
        env: Env,
        referred: Address,
        referrer: Address,
    ) -> Result<(), ContractError> {
        // Check for circular referral
        let referrer_referrer_key = (Symbol::new(&env, "ref"), referrer.clone());
        if let Some(referrer_info) = env
            .storage()
            .instance()
            .get::<_, ReferralInfo>(&referrer_referrer_key)
        {
            if referrer_info.referrer == referred {
                return Err(ContractError::InvalidAgentId); // Circular: referred is referrer of referrer
            }
        }

        let referred_referrals_key = (Symbol::new(&env, "referrals"), referred.clone());
        if let Some(referred_referrals) = env
            .storage()
            .instance()
            .get::<_, Vec<Address>>(&referred_referrals_key)
        {
            for i in 0..referred_referrals.len() {
                if let Some(r) = referred_referrals.get(i) {
                    if r == referrer {
                        return Err(ContractError::InvalidAgentId); // Circular: referrer is referred by referred
                    }
                }
            }
        }

        let key = (Symbol::new(&env, "ref"), referred.clone());
        if env.storage().instance().has(&key) {
            return Err(ContractError::AlreadyInitialized); // Already referred
        }

        let info = ReferralInfo {
            referrer: referrer.clone(),
            referred_at: env.ledger().timestamp(),
        };

        env.storage().instance().set(&key, &info);

        // Update referrer's count
        let count_key = (Symbol::new(&env, "count"), referrer.clone());
        let mut count: u32 = env.storage().instance().get(&count_key).unwrap_or(0);
        count += 1;
        env.storage().instance().set(&count_key, &count);

        // Add to referrer's referrals list
        let referrals_key = (Symbol::new(&env, "referrals"), referrer.clone());
        let mut referrals: Vec<Address> = env
            .storage()
            .instance()
            .get(&referrals_key)
            .unwrap_or(Vec::new(&env));
        referrals.push_back(referred.clone());
        env.storage().instance().set(&referrals_key, &referrals);

        env.events().publish(
            (
                Symbol::new(&env, "referral"),
                Symbol::new(&env, "registered"),
            ),
            (referred, referrer),
        );

        Ok(())
    }

    /// Add rewards to a referrer (called by an authorized contract/admin).
    pub fn add_reward(
        env: Env,
        caller: Address,
        referrer: Address,
        amount: i128,
    ) -> Result<(), ContractError> {
        caller.require_auth();
        // In a real scenario, we might verify if caller is an authorized module
        // For now, let's allow admin or a specific role.
        admin::verify_admin(&env, &caller)?;

        if amount <= 0 {
            return Err(ContractError::InvalidAgentId); // Use appropriate error for invalid amount
        }

        let reward_key = (Symbol::new(&env, "reward"), referrer.clone());
        let mut balance: i128 = env.storage().instance().get(&reward_key).unwrap_or(0);
        balance += amount;
        env.storage().instance().set(&reward_key, &balance);

        // Update total earnings
        let total_key = (Symbol::new(&env, "total"), referrer.clone());
        let mut total: i128 = env.storage().instance().get(&total_key).unwrap_or(0);
        total += amount;
        env.storage().instance().set(&total_key, &total);

        env.events().publish(
            (
                Symbol::new(&env, "referral"),
                Symbol::new(&env, "reward_added"),
            ),
            (referrer, amount),
        );

        Ok(())
    }

    /// Claim accumulated rewards.
    pub fn claim_rewards(env: Env, referrer: Address) -> Result<i128, ContractError> {
        referrer.require_auth();

        let reward_key = (Symbol::new(&env, "reward"), referrer.clone());
        let balance: i128 = env.storage().instance().get(&reward_key).unwrap_or(0);

        if balance <= 0 {
            return Ok(0);
        }

        // Reset balance
        env.storage().instance().set(&reward_key, &0i128);

        // Here we would normally call a token contract to transfer the rewards
        // For this task, we emit an event indicating the claim.
        env.events().publish(
            (Symbol::new(&env, "referral"), Symbol::new(&env, "claimed")),
            (referrer, balance),
        );

        Ok(balance)
    }

    /// Get referral count for a user.
    pub fn get_referral_count(env: Env, referrer: Address) -> u32 {
        let count_key = (Symbol::new(&env, "count"), referrer);
        env.storage().instance().get(&count_key).unwrap_or(0)
    }

    /// Get pending rewards for a user.
    pub fn get_pending_rewards(env: Env, referrer: Address) -> i128 {
        let reward_key = (Symbol::new(&env, "reward"), referrer);
        env.storage().instance().get(&reward_key).unwrap_or(0)
    }

    /// Get commission rate based on trading volume.
    fn get_commission_rate(volume: i128) -> i128 {
        if volume < TIER1_VOLUME {
            TIER1_RATE
        } else if volume < TIER2_VOLUME {
            TIER2_RATE
        } else {
            TIER3_RATE
        }
    }

    /// Distribute commission on fee collection (automatic).
    pub fn distribute_commission(
        env: Env,
        caller: Address,
        referee: Address,
        volume: i128,
        fee_amount: i128,
    ) -> Result<(), ContractError> {
        caller.require_auth();
        admin::verify_admin(&env, &caller)?;

        let rate = Self::get_commission_rate(volume);
        let commission = fee_amount * rate / 100;

        let ref_key = (Symbol::new(&env, "ref"), referee.clone());
        if let Some(info) = env.storage().instance().get::<_, ReferralInfo>(&ref_key) {
            let direct_referrer = info.referrer.clone();

            // Add to direct referrer
            let reward_key = (Symbol::new(&env, "reward"), direct_referrer.clone());
            let mut balance: i128 = env.storage().instance().get(&reward_key).unwrap_or(0);
            balance += commission;
            env.storage().instance().set(&reward_key, &balance);

            // Update total for direct
            let total_key = (Symbol::new(&env, "total"), direct_referrer.clone());
            let mut total: i128 = env.storage().instance().get(&total_key).unwrap_or(0);
            total += commission;
            env.storage().instance().set(&total_key, &total);

            // Indirect referrer
            let indirect_ref_key = (Symbol::new(&env, "ref"), direct_referrer.clone());
            if let Some(indirect_info) = env
                .storage()
                .instance()
                .get::<_, ReferralInfo>(&indirect_ref_key)
            {
                let indirect_referrer = indirect_info.referrer.clone();
                let indirect_commission = commission * INDIRECT_MULTIPLIER / 100;

                let indirect_reward_key = (Symbol::new(&env, "reward"), indirect_referrer.clone());
                let mut indirect_balance: i128 = env
                    .storage()
                    .instance()
                    .get(&indirect_reward_key)
                    .unwrap_or(0);
                indirect_balance += indirect_commission;
                env.storage()
                    .instance()
                    .set(&indirect_reward_key, &indirect_balance);

                // Update total for indirect
                let indirect_total_key = (Symbol::new(&env, "total"), indirect_referrer.clone());
                let mut indirect_total: i128 = env
                    .storage()
                    .instance()
                    .get(&indirect_total_key)
                    .unwrap_or(0);
                indirect_total += indirect_commission;
                env.storage()
                    .instance()
                    .set(&indirect_total_key, &indirect_total);
            }
        }

        Ok(())
    }

    /// Get referral stats for a user.
    pub fn get_referral_stats(env: Env, user: Address) -> ReferralStats {
        let direct = Self::get_referral_count(env.clone(), user.clone());

        let referrals_key = (Symbol::new(&env, "referrals"), user.clone());
        let referrals: Vec<Address> = env
            .storage()
            .instance()
            .get(&referrals_key)
            .unwrap_or(Vec::new(&env));

        let mut indirect = 0u32;
        for i in 0..referrals.len() {
            if let Some(r) = referrals.get(i) {
                indirect += Self::get_referral_count(env.clone(), r);
            }
        }

        let total_earnings_key = (Symbol::new(&env, "total"), user.clone());
        let total_earnings: i128 = env
            .storage()
            .instance()
            .get(&total_earnings_key)
            .unwrap_or(0);

        let pending = Self::get_pending_rewards(env, user);

        ReferralStats {
            direct_referrals: direct,
            indirect_referrals: indirect,
            total_earnings,
            pending_rewards: pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn test_referral_flow() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let referrer = Address::generate(&env);
        let referred = Address::generate(&env);

        let contract_id = env.register_contract(None, ReferralRewards);
        let client = ReferralRewardsClient::new(&env, &contract_id);

        client.init_contract(&admin);

        client.register_referral(&referred, &referrer);
        assert_eq!(client.get_referral_count(&referrer), 1);

        client.add_reward(&admin, &referrer, &1000);
        assert_eq!(client.get_pending_rewards(&referrer), 1000);

        let claimed = client.claim_rewards(&referrer);
        assert_eq!(claimed, 1000);
        assert_eq!(client.get_pending_rewards(&referrer), 0);
    }
}
