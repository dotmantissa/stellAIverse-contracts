#![no_std]

mod errors;

use errors::ContractError;
use soroban_sdk::{contract, contractimpl, contracttype, token, Address, Env, Symbol};
use stellai_lib::ADMIN_KEY;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StakeKey {
    pub user: Address,
    pub token: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StakeInfo {
    pub token: Address,
    pub amt: i128,
    pub start_ts: u64,
    pub last_ts: u64,
    pub earned: i128,
}

#[contract]
pub struct StakingBonuses;

const DAY_IN_SECONDS: u64 = 86_400;
const LOCK_PERIOD_SECONDS: u64 = 7 * DAY_IN_SECONDS;
const REWARD_PERIOD_SECONDS: u64 = 30 * DAY_IN_SECONDS;
const BONUS_PERCENT_PER_MONTH: i128 = 5;

/// Governance vote count thresholds for multiplier tiers (issue #136).
/// Stakers who have cast ≥ threshold votes in the last month receive the multiplier.
const MULTIPLIER_TIER1_VOTES: u32 = 1;  // ≥1 vote  → 1.25× (125 bps)
const MULTIPLIER_TIER2_VOTES: u32 = 5;  // ≥5 votes → 1.50× (150 bps)
const MULTIPLIER_TIER3_VOTES: u32 = 10; // ≥10 votes → 2.00× (200 bps)

/// Storage key for per-user governance vote count (set by governance contract or admin).
const GOV_VOTES_KEY: &str = "gov_votes";

#[contractimpl]
impl StakingBonuses {
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

    /// Record governance participation for a user (called by admin / governance contract).
    /// `vote_count` is the number of governance votes cast in the current monthly window.
    pub fn record_governance_votes(
        env: Env,
        caller: Address,
        user: Address,
        vote_count: u32,
    ) -> Result<(), ContractError> {
        // Only admin may update governance vote counts.
        let admin: Address = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, ADMIN_KEY))
            .ok_or(ContractError::Unauthorized)?;
        if caller != admin {
            return Err(ContractError::Unauthorized);
        }
        caller.require_auth();
        let key = (Symbol::new(&env, GOV_VOTES_KEY), user.clone());
        env.storage().instance().set(&key, &vote_count);
        env.events().publish(
            (
                Symbol::new(&env, "staking"),
                Symbol::new(&env, "gov_votes_recorded"),
            ),
            (user, vote_count),
        );
        Ok(())
    }

    /// Return the governance-activity multiplier for a user in basis points (100 = 1×).
    /// Tier 1 (≥1 vote): 125 bps, Tier 2 (≥5 votes): 150 bps, Tier 3 (≥10 votes): 200 bps.
    pub fn get_governance_multiplier(env: Env, user: Address) -> u32 {
        let key = (Symbol::new(&env, GOV_VOTES_KEY), user);
        let votes: u32 = env.storage().instance().get(&key).unwrap_or(0);
        if votes >= MULTIPLIER_TIER3_VOTES {
            200
        } else if votes >= MULTIPLIER_TIER2_VOTES {
            150
        } else if votes >= MULTIPLIER_TIER1_VOTES {
            125
        } else {
            100
        }
    }

    /// Stake a specific token on behalf of `user`.
    pub fn stake(
        env: Env,
        user: Address,
        token: Address,
        amount: i128,
    ) -> Result<(), ContractError> {
        user.require_auth();

        if amount <= 0 {
            return Err(ContractError::InvalidAmount);
        }

        let now = env.ledger().timestamp();
        let key = Self::stake_key(&user, &token);
        let mut info = Self::load_stake(&env, &key).unwrap_or(StakeInfo {
            token: token.clone(),
            amt: 0,
            start_ts: now,
            last_ts: now,
            earned: 0,
        });

        Self::accrue_rewards(&env, &mut info, now, &user)?;

        let token_client = token::Client::new(&env, &token);
        token_client.transfer(&user, &env.current_contract_address(), &amount);

        info.token = token.clone();
        info.amt = info
            .amt
            .checked_add(amount)
            .ok_or(ContractError::OverflowError)?;
        info.start_ts = now;
        info.last_ts = now;

        env.storage().instance().set(&key, &info);

        env.events().publish(
            (Symbol::new(&env, "staking"), Symbol::new(&env, "staked")),
            (user, token, amount, info.amt),
        );

        Ok(())
    }

    /// Calculate the current claimable bonus for a given token stake.
    pub fn calculate_bonus(env: Env, user: Address, token: Address) -> i128 {
        let key = Self::stake_key(&user, &token);
        let mut info = match Self::load_stake(&env, &key) {
            Some(info) => info,
            None => return 0,
        };

        Self::accrue_rewards(&env, &mut info, env.ledger().timestamp(), &user).unwrap_or(());
        info.earned
    }

    /// Claim staking bonus for one token position.
    pub fn claim_bonus(env: Env, user: Address, token: Address) -> Result<i128, ContractError> {
        user.require_auth();

        let now = env.ledger().timestamp();
        let key = Self::stake_key(&user, &token);
        let mut info = Self::load_stake(&env, &key).ok_or(ContractError::StakeNotFound)?;
        Self::accrue_rewards(&env, &mut info, now, &user)?;

        let bonus = info.earned;
        info.earned = 0;
        env.storage().instance().set(&key, &info);

        env.events().publish(
            (
                Symbol::new(&env, "staking"),
                Symbol::new(&env, "bonus_claimed"),
            ),
            (user, token, bonus),
        );

        Ok(bonus)
    }

    /// Unstake all tokens for a specific asset.
    pub fn unstake(env: Env, user: Address, token: Address) -> Result<i128, ContractError> {
        user.require_auth();

        let now = env.ledger().timestamp();
        let key = Self::stake_key(&user, &token);
        let mut info = Self::load_stake(&env, &key).ok_or(ContractError::StakeNotFound)?;

        if now < info.start_ts.saturating_add(LOCK_PERIOD_SECONDS) {
            return Err(ContractError::StakeLocked);
        }

        Self::accrue_rewards(&env, &mut info, now, &user)?;

        let token_client = token::Client::new(&env, &token);
        token_client.transfer(&env.current_contract_address(), &user, &info.amt);

        let principal = info.amt;
        let bonus = info.earned;
        env.storage().instance().remove(&key);

        env.events().publish(
            (Symbol::new(&env, "staking"), Symbol::new(&env, "unstaked")),
            (user, token, principal, bonus),
        );

        Ok(principal)
    }

    /// Get stake info for a specific token.
    pub fn get_stake_info(env: Env, user: Address, token: Address) -> Option<StakeInfo> {
        let key = Self::stake_key(&user, &token);
        Self::load_stake(&env, &key)
    }

    fn stake_key(user: &Address, token: &Address) -> StakeKey {
        StakeKey {
            user: user.clone(),
            token: token.clone(),
        }
    }

    fn load_stake(env: &Env, key: &StakeKey) -> Option<StakeInfo> {
        env.storage().instance().get(key)
    }

    fn accrue_rewards(
        env: &Env,
        info: &mut StakeInfo,
        now: u64,
        user: &Address,
    ) -> Result<(), ContractError> {
        let elapsed = now.saturating_sub(info.last_ts);
        let periods = elapsed / REWARD_PERIOD_SECONDS;
        if periods == 0 {
            return Ok(());
        }

        let periods_i = periods as i128;
        // Apply governance multiplier (basis points, 100 = 1×).
        let multiplier = Self::get_governance_multiplier(env.clone(), user.clone()) as i128;
        let reward = info
            .amt
            .checked_mul(BONUS_PERCENT_PER_MONTH)
            .ok_or(ContractError::OverflowError)?
            .checked_mul(periods_i)
            .ok_or(ContractError::OverflowError)?
            .checked_mul(multiplier)
            .ok_or(ContractError::OverflowError)?
            .checked_div(100 * 100) // percent × bps
            .ok_or(ContractError::OverflowError)?;

        info.earned = info
            .earned
            .checked_add(reward)
            .ok_or(ContractError::OverflowError)?;
        info.last_ts = info
            .last_ts
            .checked_add(periods.saturating_mul(REWARD_PERIOD_SECONDS))
            .ok_or(ContractError::OverflowError)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        contract, contractimpl, contracttype,
        testutils::{Address as _, Ledger},
        Address, Env,
    };

    #[contract]
    pub struct MockToken;

    #[contracttype]
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub enum DataKey {
        Admin,
        Bal(Address),
    }

    #[contractimpl]
    impl MockToken {
        pub fn initialize(env: Env, admin: Address) {
            if env.storage().instance().has(&DataKey::Admin) {
                panic!("already initialized");
            }
            env.storage().instance().set(&DataKey::Admin, &admin);
        }

        pub fn mint(env: Env, to: Address, amount: i128) {
            let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
            admin.require_auth();

            let key = DataKey::Bal(to.clone());
            let cur: i128 = env.storage().instance().get(&key).unwrap_or(0);
            env.storage().instance().set(&key, &(cur + amount));
        }

        pub fn balance(env: Env, id: Address) -> i128 {
            env.storage().instance().get(&DataKey::Bal(id)).unwrap_or(0)
        }

        pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
            from.require_auth();

            let from_key = DataKey::Bal(from.clone());
            let from_bal: i128 = env.storage().instance().get(&from_key).unwrap_or(0);
            if from_bal < amount {
                panic!("insufficient balance");
            }

            env.storage()
                .instance()
                .set(&from_key, &(from_bal - amount));

            let to_key = DataKey::Bal(to.clone());
            let to_bal: i128 = env.storage().instance().get(&to_key).unwrap_or(0);
            env.storage().instance().set(&to_key, &(to_bal + amount));
        }
    }

    fn setup_token(env: &Env, admin: &Address) -> Address {
        let token_id = env.register_contract(None, MockToken);
        let token = MockTokenClient::new(env, &token_id);
        token.initialize(admin);
        token_id
    }

    #[test]
    fn test_multi_asset_staking_flow() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let user = Address::generate(&env);

        let token_a = setup_token(&env, &admin);
        let token_b = setup_token(&env, &admin);

        let token_a_client = MockTokenClient::new(&env, &token_a);
        let token_b_client = MockTokenClient::new(&env, &token_b);

        token_a_client.mint(&user, &10_000);
        token_b_client.mint(&user, &20_000);

        let contract_id = env.register_contract(None, StakingBonuses);
        let client = StakingBonusesClient::new(&env, &contract_id);
        client.init_contract(&admin);

        client.stake(&user, &token_a, &1_000);
        client.stake(&user, &token_b, &2_000);

        assert_eq!(token_a_client.balance(&user), 9_000);
        assert_eq!(token_b_client.balance(&user), 18_000);
        assert_eq!(token_a_client.balance(&contract_id), 1_000);
        assert_eq!(token_b_client.balance(&contract_id), 2_000);

        env.ledger().set_timestamp(31 * DAY_IN_SECONDS);

        // No governance votes → 100 bps multiplier → 5% * 100/100 = 5%
        assert_eq!(client.calculate_bonus(&user, &token_a), 50);
        assert_eq!(client.calculate_bonus(&user, &token_b), 100);

        let claimed_a = client.claim_bonus(&user, &token_a);
        assert_eq!(claimed_a, 50);
        assert_eq!(client.calculate_bonus(&user, &token_a), 0);
        assert_eq!(client.calculate_bonus(&user, &token_b), 100);

        match client.try_unstake(&user, &token_a) {
            Err(Ok(ContractError::StakeLocked)) => {}
            other => panic!("expected locked stake error, got {other:?}"),
        }

        env.ledger()
            .set_timestamp(31 * DAY_IN_SECONDS + LOCK_PERIOD_SECONDS + 1);

        let unstaked_a = client.unstake(&user, &token_a);
        assert_eq!(unstaked_a, 1_000);
        assert_eq!(token_a_client.balance(&user), 10_000);
        assert_eq!(token_a_client.balance(&contract_id), 0);

        let unstaked_b = client.unstake(&user, &token_b);
        assert_eq!(unstaked_b, 2_000);
        assert_eq!(token_b_client.balance(&user), 20_000);
        assert_eq!(token_b_client.balance(&contract_id), 0);

        assert_eq!(client.get_stake_info(&user, &token_a), None);
        assert_eq!(client.get_stake_info(&user, &token_b), None);
    }

    #[test]
    fn test_governance_multiplier_tiers() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let user = Address::generate(&env);

        let contract_id = env.register_contract(None, StakingBonuses);
        let client = StakingBonusesClient::new(&env, &contract_id);
        client.init_contract(&admin);

        // No votes → base multiplier 100 bps
        assert_eq!(client.get_governance_multiplier(&user), 100);

        // 1 vote → tier 1: 125 bps
        client.record_governance_votes(&admin, &user, &1);
        assert_eq!(client.get_governance_multiplier(&user), 125);

        // 5 votes → tier 2: 150 bps
        client.record_governance_votes(&admin, &user, &5);
        assert_eq!(client.get_governance_multiplier(&user), 150);

        // 10 votes → tier 3: 200 bps
        client.record_governance_votes(&admin, &user, &10);
        assert_eq!(client.get_governance_multiplier(&user), 200);
    }

    #[test]
    fn test_governance_multiplier_applied_to_rewards() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let user = Address::generate(&env);
        let token = setup_token(&env, &admin);
        let token_client = MockTokenClient::new(&env, &token);
        token_client.mint(&user, &10_000);

        let contract_id = env.register_contract(None, StakingBonuses);
        let client = StakingBonusesClient::new(&env, &contract_id);
        client.init_contract(&admin);

        // Give user tier-3 multiplier (200 bps = 2×)
        client.record_governance_votes(&admin, &user, &10);

        client.stake(&user, &token, &1_000);
        env.ledger().set_timestamp(31 * DAY_IN_SECONDS);

        // 5% * 2× = 10% of 1000 = 100
        assert_eq!(client.calculate_bonus(&user, &token), 100);
    }
}
