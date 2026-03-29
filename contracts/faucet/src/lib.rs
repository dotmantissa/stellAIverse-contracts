#![no_std]

// use soroban_sdk::{contract, contractimpl, Address, Env, String, Symbol, Map};
use soroban_sdk::{contract, contractimpl, Address, Env, Symbol};
use stellai_lib::{
    admin,
    storage_keys::{
        CLAIM_COOLDOWN_KEY, DEFAULT_COOLDOWN_SECONDS, DEFAULT_MAX_CLAIMS,
        MAX_CLAIMS_PER_PERIOD_KEY, TESTNET_FLAG_KEY,
    },
    ADMIN_KEY,
};

#[contract]
pub struct Faucet;

#[contractimpl]
impl Faucet {
    /// Initialize faucet (admin only)
    pub fn init_faucet(env: Env, admin: Address, testnet_only: bool) {
        let admin_data = env
            .storage()
            .instance()
            .get::<_, Address>(&Symbol::new(&env, ADMIN_KEY));
        if admin_data.is_some() {
            panic!("Contract already initialized");
        }

        admin.require_auth();
        env.storage()
            .instance()
            .set(&Symbol::new(&env, ADMIN_KEY), &admin);
        env.storage().instance().set(
            &Symbol::new(&env, CLAIM_COOLDOWN_KEY),
            &DEFAULT_COOLDOWN_SECONDS,
        );
        env.storage().instance().set(
            &Symbol::new(&env, MAX_CLAIMS_PER_PERIOD_KEY),
            &DEFAULT_MAX_CLAIMS,
        );
        env.storage()
            .instance()
            .set(&Symbol::new(&env, TESTNET_FLAG_KEY), &testnet_only);
    }

    /// Verify caller is admin
    fn verify_admin(env: &Env, caller: &Address) {
        if admin::verify_admin(env, caller).is_err() {
            panic!("Unauthorized: caller is not admin");
        }
    }

    /// Check if testnet mode is enabled
    fn is_testnet_mode(env: &Env) -> bool {
        env.storage()
            .instance()
            .get::<_, bool>(&Symbol::new(env, TESTNET_FLAG_KEY))
            .unwrap_or(true)
    }

    /// Claim a test agent from the faucet
    pub fn claim_test_agent(env: Env, claimer: Address) -> u64 {
        claimer.require_auth();

        // Security: Verify testnet mode
        if !Self::is_testnet_mode(&env) {
            panic!("Faucet is not available on mainnet");
        }

        // Check eligibility
        if !Self::check_eligibility(env.clone(), claimer.clone()) {
            panic!("Address is not eligible for faucet claim at this time");
        }

        let agent_id = 1u64; // Placeholder ID
        let now = env.ledger().timestamp();

        // Store last claim time using tuple key
        let last_claim_key = (Symbol::new(&env, "last_claim"), claimer.clone());
        env.storage().instance().set(&last_claim_key, &now);

        // Store claim count using tuple key
        let claim_count_key = (Symbol::new(&env, "claim_count"), claimer.clone());
        env.storage().instance().set(&claim_count_key, &1u32);

        env.events()
            .publish((Symbol::new(&env, "agent_claimed"),), (agent_id, claimer));

        agent_id
    }

    /// Check if an address is eligible for a faucet claim
    pub fn check_eligibility(env: Env, address: Address) -> bool {
        let cooldown: u64 = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, CLAIM_COOLDOWN_KEY))
            .unwrap_or(DEFAULT_COOLDOWN_SECONDS);

        let max_claims: u32 = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, MAX_CLAIMS_PER_PERIOD_KEY))
            .unwrap_or(DEFAULT_MAX_CLAIMS);

        let last_claim_key = (Symbol::new(&env, "last_claim"), address.clone());
        let last_claim: Option<u64> = env.storage().instance().get(&last_claim_key);

        match last_claim {
            Some(timestamp) => {
                let now = env.ledger().timestamp();
                let elapsed = now.saturating_sub(timestamp);

                // If cooldown has passed, eligible again
                if elapsed >= cooldown {
                    return true;
                }

                // Check claim count within current period
                let claim_count_key = (Symbol::new(&env, "claim_count"), address.clone());
                let claims: u32 = env.storage().instance().get(&claim_count_key).unwrap_or(0);

                claims < max_claims
            }
            None => true, // First claim ever
        }
    }

    /// Admin function: Set faucet parameters
    pub fn set_parameters(
        env: Env,
        admin: Address,
        claim_cooldown_seconds: u64,
        max_claims_per_period: u32,
    ) {
        admin.require_auth();
        Self::verify_admin(&env, &admin);

        // Validation
        if claim_cooldown_seconds == 0 {
            panic!("Cooldown must be greater than 0");
        }
        if max_claims_per_period == 0 || max_claims_per_period > 100 {
            panic!("Max claims must be between 1 and 100");
        }

        env.storage().instance().set(
            &Symbol::new(&env, CLAIM_COOLDOWN_KEY),
            &claim_cooldown_seconds,
        );
        env.storage().instance().set(
            &Symbol::new(&env, MAX_CLAIMS_PER_PERIOD_KEY),
            &max_claims_per_period,
        );

        env.events().publish(
            (Symbol::new(&env, "parameters_updated"),),
            (claim_cooldown_seconds, max_claims_per_period),
        );
    }

    /// Get current faucet parameters
    pub fn get_parameters(env: Env) -> (u64, u32) {
        let cooldown: u64 = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, CLAIM_COOLDOWN_KEY))
            .unwrap_or(DEFAULT_COOLDOWN_SECONDS);

        let max_claims: u32 = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, MAX_CLAIMS_PER_PERIOD_KEY))
            .unwrap_or(DEFAULT_MAX_CLAIMS);

        (cooldown, max_claims)
    }

    /// Get remaining cooldown time for an address
    pub fn get_remaining_cooldown(env: Env, address: Address) -> u64 {
        let cooldown: u64 = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, CLAIM_COOLDOWN_KEY))
            .unwrap_or(DEFAULT_COOLDOWN_SECONDS);

        let last_claim_key = (Symbol::new(&env, "last_claim"), address.clone());
        let last_claim: Option<u64> = env.storage().instance().get(&last_claim_key);

        match last_claim {
            Some(timestamp) => {
                let now = env.ledger().timestamp();
                let elapsed = now.saturating_sub(timestamp);

                if elapsed >= cooldown {
                    0
                } else {
                    cooldown.saturating_sub(elapsed)
                }
            }
            None => 0,
        }
    }

    /// Admin emergency pause function
    pub fn pause_faucet(env: Env, admin: Address, paused: bool) {
        admin.require_auth();
        Self::verify_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, TESTNET_FLAG_KEY), &!paused);
        env.events()
            .publish((Symbol::new(&env, "faucet_paused"),), (paused,));
    }
}
