#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol};
use stellai_lib::{admin, errors::ContractError, ADMIN_KEY};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StakeInfo {
    pub amount: i128,
    pub start_time: u64,
    pub last_claim_time: u64,
}

#[contract]
pub struct StakingBonuses;

const DAY_IN_SECONDS: u64 = 86400;
const BONUS_PERCENT_PER_MONTH: i128 = 5; // 5% bonus per 30 days

#[contractimpl]
impl StakingBonuses {
    /// Initialize the contract with an admin address.
    pub fn init_contract(env: Env, admin_addr: Address) -> Result<(), ContractError> {
        if env.storage().instance().has(&Symbol::new(&env, ADMIN_KEY)) {
            return Err(ContractError::AlreadyInitialized);
        }
        admin_addr.require_auth();
        env.storage().instance().set(&Symbol::new(&env, ADMIN_KEY), &admin_addr);
        Ok(())
    }

    /// Stake tokens.
    pub fn stake(env: Env, user: Address, amount: i128) -> Result<(), ContractError> {
        user.require_auth();
        
        if amount <= 0 {
            return Err(ContractError::InvalidAgentId); // Use appropriate error
        }

        let key = (Symbol::new(&env, "stake"), user.clone());
        let mut info = env.storage().instance().get::<_, StakeInfo>(&key).unwrap_or(StakeInfo {
            amount: 0,
            start_time: env.ledger().timestamp(),
            last_claim_time: env.ledger().timestamp(),
        });

        info.amount += amount;
        info.last_claim_time = env.ledger().timestamp(); // Reset claim timer on new stake
        
        env.storage().instance().set(&key, &info);

        env.events().publish(
            (Symbol::new(&env, "staking"), Symbol::new(&env, "staked")),
            (user, amount),
        );

        Ok(())
    }

    /// Calculate bonus based on duration.
    pub fn calculate_bonus(env: Env, user: Address) -> i128 {
        let key = (Symbol::new(&env, "stake"), user);
        let info: StakeInfo = match env.storage().instance().get(&key) {
            Some(i) => i,
            None => return 0,
        };

        let now = env.ledger().timestamp();
        let duration = now.saturating_sub(info.last_claim_time);
        
        if duration < 30 * DAY_IN_SECONDS {
            return 0;
        }

        let months = (duration / (30 * DAY_IN_SECONDS)) as i128;
        let bonus = (info.amount * BONUS_PERCENT_PER_MONTH * months) / 100;
        
        bonus
    }

    /// Claim staking bonus.
    pub fn claim_bonus(env: Env, user: Address) -> Result<i128, ContractError> {
        user.require_auth();

        let bonus = Self::calculate_bonus(env.clone(), user.clone());
        if bonus <= 0 {
            return Ok(0);
        }

        let key = (Symbol::new(&env, "stake"), user.clone());
        let mut info: StakeInfo = env.storage().instance().get(&key).unwrap();
        
        info.last_claim_time = env.ledger().timestamp();
        env.storage().instance().set(&key, &info);

        env.events().publish(
            (Symbol::new(&env, "staking"), Symbol::new(&env, "bonus_claimed")),
            (user, bonus),
        );

        Ok(bonus)
    }

    /// Unstake all tokens.
    pub fn unstake(env: Env, user: Address) -> Result<i128, ContractError> {
        user.require_auth();

        let key = (Symbol::new(&env, "stake"), user.clone());
        let info: StakeInfo = env.storage().instance().get(&key).ok_or(ContractError::AgentNotFound)?;

        // Simple 7-day lock check
        if env.ledger().timestamp() < info.start_time + (7 * DAY_IN_SECONDS) {
            return Err(ContractError::Unauthorized); // Use appropriate "Locked" error
        }

        env.storage().instance().remove(&key);

        env.events().publish(
            (Symbol::new(&env, "staking"), Symbol::new(&env, "unstaked")),
            (user, info.amount),
        );

        Ok(info.amount)
    }

    /// Get stake info.
    pub fn get_stake_info(env: Env, user: Address) -> Option<StakeInfo> {
        let key = (Symbol::new(&env, "stake"), user);
        env.storage().instance().get(&key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};

    #[test]
    fn test_staking_flow() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let user = Address::generate(&env);

        let contract_id = env.register_contract(None, StakingBonuses);
        let client = StakingBonusesClient::new(&env, &contract_id);

        client.init_contract(&admin);

        client.stake(&user, &1000);
        assert_eq!(client.calculate_bonus(&user), 0);

        // Advance ledger time by 31 days
        env.ledger().set_timestamp(31 * 86400);
        
        let bonus = client.calculate_bonus(&user);
        assert!(bonus > 0);
        assert_eq!(bonus, 50); // 5% of 1000

        client.claim_bonus(&user);
        assert_eq!(client.calculate_bonus(&user), 0);
    }
}
