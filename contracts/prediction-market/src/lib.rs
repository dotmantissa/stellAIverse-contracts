#![no_std]

mod storage;
mod types;

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, String, Symbol};
use storage::*;
use types::*;

#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum ReputationReason {
    Execution = 0,
    Marketplace = 1,
    Prediction = 2,
}

#[contract]
pub struct PredictionMarket;

#[contractimpl]
impl PredictionMarket {
    pub fn create_market(env: Env, creator: Address, market_id: u64, description: String) {
        creator.require_auth();
        let now = env.ledger().timestamp();
        let m = Market {
            market_id,
            creator: creator.clone(),
            description: description.clone(),
            status: MarketStatus::Open,
            outcome_a_reserve: 0i128,
            outcome_b_reserve: 0i128,
            total_liquidity: 0i128,
            created_at: now,
            resolved_outcome: Outcome::Unresolved,
        };
        store_market(&env, &m);
        env.events()
            .publish((Symbol::new(&env, "market_created"),), (market_id,));
    }

    pub fn provide_liquidity(
        env: Env,
        provider: Address,
        market_id: u64,
        amount_a: i128,
        amount_b: i128,
    ) {
        provider.require_auth();
        let mut m = match get_market(&env, market_id) {
            Some(x) => x,
            None => panic!("market not found"),
        };
        m.outcome_a_reserve = m.outcome_a_reserve.saturating_add(amount_a);
        m.outcome_b_reserve = m.outcome_b_reserve.saturating_add(amount_b);
        m.total_liquidity = m
            .total_liquidity
            .saturating_add(amount_a.saturating_add(amount_b));
        store_market(&env, &m);
        env.events()
            .publish((Symbol::new(&env, "liquidity_added"),), (market_id,));
    }

    pub fn place_bet(env: Env, bettor: Address, market_id: u64, outcome: Outcome, amount: i128) {
        bettor.require_auth();
        let mut m = match get_market(&env, market_id) {
            Some(x) => x,
            None => panic!("market not found"),
        };
        match outcome {
            Outcome::A => m.outcome_a_reserve = m.outcome_a_reserve.saturating_add(amount),
            Outcome::B => m.outcome_b_reserve = m.outcome_b_reserve.saturating_add(amount),
            _ => panic!("invalid outcome"),
        }
        store_market(&env, &m);
        env.events()
            .publish((Symbol::new(&env, "bet_placed"),), (market_id,));
    }

    pub fn resolve_market(env: Env, caller: Address, market_id: u64, winning: Outcome) {
        caller.require_auth();
        if stellai_lib::admin::verify_admin(&env, &caller).is_err() {
            panic!("unauthorized");
        }
        let mut m = match get_market(&env, market_id) {
            Some(x) => x,
            None => panic!("market not found"),
        };
        m.status = MarketStatus::Resolved;
        m.resolved_outcome = winning;
        store_market(&env, &m);
        env.events().publish(
            (Symbol::new(&env, "market_resolved"),),
            (market_id, winning as u32),
        );
    }

    pub fn claim_winnings(env: Env, bettor: Address, market_id: u64) -> i128 {
        bettor.require_auth();
        let m = match get_market(&env, market_id) {
            Some(x) => x,
            None => panic!("market not found"),
        };

        if m.status != MarketStatus::Resolved {
            panic!("market not resolved");
        }

        let pos = match get_bet_position(&env, &bettor, market_id) {
            Some(x) => x,
            None => panic!("no bet position"),
        };

        let winning_reserve = match m.resolved_outcome {
            Outcome::A => m.outcome_a_reserve,
            Outcome::B => m.outcome_b_reserve,
            _ => return 0,
        };

        let total_winning_tokens = winning_reserve;
        let winnings = if total_winning_tokens > 0 {
            pos.tokens
                .saturating_mul(total_winning_tokens as u128)
                .checked_div(1000000)
                .unwrap_or(0) as i128
        } else {
            0
        };

        let key = (Symbol::new(&env, "pm_bet_pos"), bettor, market_id);
        env.storage().persistent().remove(&key);

        env.events().publish(
            (Symbol::new(&env, "winnings_claimed"),),
            (market_id, winnings),
        );
        winnings
    }

    pub fn get_agent_reputation(env: Env, agent: Address, _reason: ReputationReason) -> u128 {
        // Placeholder implementation - would integrate with agent contract
        1000u128
    }

    pub fn update_agent_reputation(
        env: Env,
        agent: Address,
        amount: i128,
        reason: ReputationReason,
    ) {
        // Placeholder implementation - would update agent reputation
        env.events().publish(
            (Symbol::new(&env, "reputation_updated"),),
            (agent, amount, reason as u32),
        );
    }
}
