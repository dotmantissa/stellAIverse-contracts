#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol};
use stellai_lib::{admin, errors::ContractError, ADMIN_KEY};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReferralInfo {
    pub referrer: Address,
    pub referred_at: u64,
}

#[contract]
pub struct ReferralRewards;

#[contractimpl]
impl ReferralRewards {
    /// Initialize the contract with an admin address.
    pub fn init_contract(env: Env, admin_addr: Address) -> Result<(), ContractError> {
        if env.storage().instance().has(&Symbol::new(&env, ADMIN_KEY)) {
            return Err(ContractError::AlreadyInitialized);
        }
        admin_addr.require_auth();
        env.storage().instance().set(&Symbol::new(&env, ADMIN_KEY), &admin_addr);
        Ok(())
    }

    /// Register a new referral.
    pub fn register_referral(env: Env, referred: Address, referrer: Address) -> Result<(), ContractError> {
        referred.require_auth();
        
        if referred == referrer {
            return Err(ContractError::SameAddressTransfer); // Use appropriate error
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

        env.events().publish(
            (Symbol::new(&env, "referral"), Symbol::new(&env, "registered")),
            (referred, referrer),
        );

        Ok(())
    }

    /// Add rewards to a referrer (called by an authorized contract/admin).
    pub fn add_reward(env: Env, caller: Address, referrer: Address, amount: i128) -> Result<(), ContractError> {
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

        env.events().publish(
            (Symbol::new(&env, "referral"), Symbol::new(&env, "reward_added")),
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
