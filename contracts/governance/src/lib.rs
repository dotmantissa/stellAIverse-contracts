#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, token, Address, Env, String, Symbol, Val, Vec,
};

mod storage;
mod types;

#[cfg(test)]
mod test;

use storage::*;
use types::*;

#[contract]
pub struct Governance;

#[contractimpl]
impl Governance {
    /// Initialize governance contract
    pub fn init_contract(
        env: Env,
        admin: Address,
        governance_token: Address,
        quorum_threshold: Option<u32>,
        approval_threshold: Option<u32>,
        min_voting_period: Option<u64>,
        max_voting_period: Option<u64>,
        min_proposal_deposit: Option<u128>,
    ) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("Contract already initialized");
        }

        admin.require_auth();

        set_admin(&env, &admin);
        set_governance_token(&env, &governance_token);

        set_quorum_threshold(&env, quorum_threshold.unwrap_or(3000));
        set_approval_threshold(&env, approval_threshold.unwrap_or(6600));
        set_min_voting_period(&env, min_voting_period.unwrap_or(7 * 24 * 60 * 60));
        set_max_voting_period(&env, max_voting_period.unwrap_or(14 * 24 * 60 * 60));
        set_min_proposal_deposit(&env, min_proposal_deposit.unwrap_or(1000u128));
        env.storage()
            .instance()
            .set(&DataKey::ProposalCounter, &0u64);
    }

    /// Create a new proposal (requires deposit)
    pub fn create_proposal(
        env: Env,
        proposer: Address,
        title: String,
        description: String,
        voting_period: u64,
        proposal_type: ProposalType,
        parameters: Option<ProposalParameters>,
        target_contract: Option<Address>,
        target_function: Option<Symbol>,
        target_args: Option<Vec<Val>>,
    ) -> u64 {
        proposer.require_auth();

        let min_period = get_min_voting_period(&env);
        let max_period = get_max_voting_period(&env);
        if voting_period < min_period || voting_period > max_period {
            panic!("Voting period must be between min and max");
        }

        let min_deposit = get_min_proposal_deposit(&env);
        let governance_token = get_governance_token(&env);
        let token_client = token::Client::new(&env, &governance_token);
        let balance = token_client.balance(&proposer);

        if balance < min_deposit as i128 {
            panic!("Insufficient balance for proposal deposit");
        }

        let contract_address = env.current_contract_address();
        token_client.transfer(&proposer, &contract_address, &(min_deposit as i128));

        let proposal_id = increment_proposal_counter(&env);
        let current_time = env.ledger().timestamp();
        let (has_parameters, params) = if let Some(p) = parameters {
            (true, p)
        } else {
            (
                false,
                ProposalParameters {
                    name: String::from_str(&env, ""),
                    value: String::from_str(&env, ""),
                },
            )
        };

        let proposal = Proposal {
            proposal_id,
            title,
            description,
            proposer: proposer.clone(),
            voting_starts: current_time,
            voting_ends: current_time + voting_period,
            proposal_type,
            has_parameters,
            parameters: params,
            votes_for: 0,
            votes_against: 0,
            votes_abstain: 0,
            status: ProposalStatus::Active,
            target_contract,
            target_function,
            target_args,
        };

        set_proposal(&env, &proposal);

        env.events().publish(
            (Symbol::new(&env, "ProposalCreated"),),
            (
                proposal_id,
                proposer.clone(),
                current_time,
                current_time + voting_period,
            ),
        );

        proposal_id
    }

    /// Get voting power for an address
    pub fn get_vote_power(env: Env, address: Address) -> u128 {
        let governance_token = get_governance_token(&env);
        let token_client = token::Client::new(&env, &governance_token);
        let base_balance = token_client.balance(&address) as u128;

        let escrow_power = if let Some(escrow) = get_vote_escrow(&env, &address) {
            let current_time = env.ledger().timestamp();
            if escrow.lock_end > current_time {
                (escrow.amount as u128 * escrow.multiplier as u128) / 10000u128
            } else {
                0
            }
        } else {
            0
        };

        let delegated_power = Self::calculate_delegated_power_to(&env, &address);

        let own_delegated_away = if let Some(delegation) = get_delegation(&env, &address) {
            delegation.amount
        } else {
            0
        };

        let own_power = base_balance + escrow_power;
        let available_own_power = if own_delegated_away > own_power {
            0
        } else {
            own_power - own_delegated_away
        };

        available_own_power + delegated_power
    }

    /// Delegate voting power to another address
    pub fn delegate_voting_power(env: Env, delegator: Address, delegatee: Address, amount: u128) {
        delegator.require_auth();

        if delegator == delegatee {
            panic!("Cannot delegate to self");
        }

        if amount == 0 {
            panic!("Amount must be greater than 0");
        }

        let governance_token = get_governance_token(&env);
        let token_client = token::Client::new(&env, &governance_token);
        let base_balance = token_client.balance(&delegator) as u128;

        let escrow_power = if let Some(escrow) = get_vote_escrow(&env, &delegator) {
            let current_time = env.ledger().timestamp();
            if escrow.lock_end > current_time {
                (escrow.amount as u128 * escrow.multiplier as u128) / 10000u128
            } else {
                0
            }
        } else {
            0
        };

        let available_power = base_balance + escrow_power;
        let existing_delegation = get_delegation(&env, &delegator);
        let new_total = amount;

        if new_total > available_power {
            panic!("Insufficient voting power to delegate");
        }

        let delegation = Delegation {
            delegatee: delegatee.clone(),
            amount: new_total,
        };
        set_delegation(&env, &delegator, &delegation);

        env.events().publish(
            (Symbol::new(&env, "VotingPowerDelegated"),),
            (delegator, delegatee, new_total),
        );
    }

    /// Remove delegation
    pub fn undelegate_voting_power(env: Env, delegator: Address) {
        delegator.require_auth();

        if get_delegation(&env, &delegator).is_none() {
            panic!("No delegation to remove");
        }

        if let Some(old_delegation) = get_delegation(&env, &delegator) {
            storage::remove_delegator_from_list(&env, &old_delegation.delegatee, &delegator);
        }

        env.storage()
            .instance()
            .remove(&DataKey::Delegation(delegator.clone()));

        env.events()
            .publish((Symbol::new(&env, "VotingPowerUndelegated"),), (delegator,));
    }

    /// Lock tokens for vote escrow (4-52 weeks, 2x-4x multiplier)
    pub fn lock_for_escrow(env: Env, locker: Address, amount: u128, lock_duration_weeks: u32) {
        locker.require_auth();

        if amount == 0 {
            panic!("Amount must be greater than 0");
        }

        if lock_duration_weeks < 4 || lock_duration_weeks > 52 {
            panic!("Lock duration must be between 4 and 52 weeks");
        }

        let governance_token = get_governance_token(&env);
        let token_client = token::Client::new(&env, &governance_token);
        let balance = token_client.balance(&locker);

        if balance < amount as i128 {
            panic!("Insufficient balance");
        }

        let multiplier = 20000u32 + ((lock_duration_weeks - 4) * 20000u32) / 48;

        let contract_address = env.current_contract_address();
        token_client.transfer(&locker, &contract_address, &(amount as i128));

        let current_time = env.ledger().timestamp();
        let lock_end = current_time + (lock_duration_weeks as u64 * 7 * 24 * 60 * 60);

        let existing_escrow = get_vote_escrow(&env, &locker);
        let (new_amount, new_lock_end) = if let Some(escrow) = &existing_escrow {
            let amount_to_add = if escrow.lock_end <= current_time {
                amount
            } else {
                escrow.amount + amount
            };
            let lock_end_to_use = if escrow.lock_end > lock_end {
                escrow.lock_end
            } else {
                lock_end
            };
            (amount_to_add, lock_end_to_use)
        } else {
            (amount, lock_end)
        };

        let escrow = VoteEscrow {
            amount: new_amount,
            lock_end: new_lock_end,
            multiplier,
        };
        set_vote_escrow(&env, &locker, &escrow);

        env.events().publish(
            (Symbol::new(&env, "VoteEscrowLocked"),),
            (locker, new_amount, new_lock_end, multiplier),
        );
    }

    /// Unlock escrowed tokens
    pub fn unlock_escrow(env: Env, locker: Address) {
        locker.require_auth();

        let escrow = get_vote_escrow(&env, &locker).expect("No escrow found");

        let current_time = env.ledger().timestamp();
        if escrow.lock_end > current_time {
            panic!("Escrow is still locked");
        }

        let governance_token = get_governance_token(&env);
        let token_client = token::Client::new(&env, &governance_token);
        let contract_address = env.current_contract_address();
        token_client.transfer(&contract_address, &locker, &(escrow.amount as i128));

        env.storage()
            .instance()
            .remove(&DataKey::VoteEscrow(locker.clone()));

        env.events().publish(
            (Symbol::new(&env, "VoteEscrowUnlocked"),),
            (locker, escrow.amount),
        );
    }

    /// Cast a vote on a proposal
    pub fn cast_vote(env: Env, voter: Address, proposal_id: u64, vote_type: VoteType) {
        voter.require_auth();

        let mut proposal = get_proposal(&env, proposal_id).expect("Proposal not found");

        // Check if voting period is active
        let current_time = env.ledger().timestamp();
        if current_time < proposal.voting_starts {
            panic!("Voting has not started yet");
        }
        if current_time > proposal.voting_ends {
            panic!("Voting period has ended");
        }

        if proposal.status != ProposalStatus::Active {
            panic!("Proposal is not active");
        }

        // Check if already voted
        if get_vote(&env, proposal_id, &voter).is_some() {
            panic!("Already voted on this proposal");
        }

        // Calculate voting power
        // This includes: base tokens + escrow multiplier + delegated power TO this voter
        let voting_power = Self::calculate_total_voting_power(&env, &voter);

        if voting_power == 0 {
            panic!("No voting power");
        }

        // Record vote
        let vote = Vote {
            proposal_id,
            voter: voter.clone(),
            vote_type: vote_type.clone(),
            weight: voting_power,
            timestamp: current_time,
        };
        set_vote(&env, proposal_id, &voter, &vote);

        // Update proposal vote counts
        match vote_type {
            VoteType::For => proposal.votes_for += voting_power,
            VoteType::Against => proposal.votes_against += voting_power,
            VoteType::Abstain => proposal.votes_abstain += voting_power,
        }
        set_proposal(&env, &proposal);

        env.events().publish(
            (Symbol::new(&env, "VoteCast"),),
            (proposal_id, voter, vote_type, voting_power),
        );
    }

    fn calculate_total_voting_power(env: &Env, address: &Address) -> u128 {
        let governance_token = get_governance_token(env);
        let token_client = token::Client::new(env, &governance_token);

        // Base voting power from token balance
        let base_balance = token_client.balance(address) as u128;

        // Add vote escrow power
        let escrow_power = if let Some(escrow) = get_vote_escrow(env, address) {
            let current_time = env.ledger().timestamp();
            if escrow.lock_end > current_time {
                // Escrow is still locked, apply multiplier
                (escrow.amount as u128 * escrow.multiplier as u128) / 10000u128
            } else {
                // Escrow expired, no multiplier
                0
            }
        } else {
            0
        };

        // Calculate delegated power TO this address using reverse index
        let delegated_power = Self::calculate_delegated_power_to(env, address);

        base_balance + escrow_power + delegated_power
    }

    fn calculate_delegated_power_to(env: &Env, delegatee: &Address) -> u128 {
        let delegators = storage::get_delegators_to(env, delegatee);
        let mut total_delegated = 0u128;

        for i in 0..delegators.len() {
            let delegator = delegators.get(i).unwrap();
            if let Some(delegation) = storage::get_delegation(env, &delegator) {
                // Get delegator's base voting power (not including their own delegations)
                let governance_token = get_governance_token(env);
                let token_client = token::Client::new(env, &governance_token);
                let base_balance = token_client.balance(&delegator) as u128;

                // Add escrow power if exists
                let escrow_power = if let Some(escrow) = storage::get_vote_escrow(env, &delegator) {
                    let current_time = env.ledger().timestamp();
                    if escrow.lock_end > current_time {
                        (escrow.amount as u128 * escrow.multiplier as u128) / 10000u128
                    } else {
                        0
                    }
                } else {
                    0
                };

                // The delegated amount is what was actually delegated
                // We cap it at the delegator's own power (base + escrow) at the time of delegation
                // Note: We don't subtract what the delegator has delegated away because
                // the delegation amount represents what was committed at delegation time
                let delegator_power = base_balance + escrow_power;
                let delegated_amount = if delegation.amount > delegator_power {
                    delegator_power
                } else {
                    delegation.amount
                };

                total_delegated += delegated_amount;
            }
        }

        total_delegated
    }

    /// Execute a passed proposal
    pub fn execute_proposal(env: Env, executor: Address, proposal_id: u64) {
        executor.require_auth();

        let mut proposal = get_proposal(&env, proposal_id).expect("Proposal not found");

        // Check if proposal has passed
        if proposal.status != ProposalStatus::Passed {
            panic!("Proposal has not passed");
        }

        // Check thresholds
        let total_votes = proposal.votes_for + proposal.votes_against + proposal.votes_abstain;
        let circulating_power = Self::get_circulating_voting_power(env.clone());

        let quorum_threshold = get_quorum_threshold(&env);
        let approval_threshold = get_approval_threshold(&env);

        // Check quorum (30% of circulating voting power)
        let quorum_required = (circulating_power * quorum_threshold as u128) / 10000u128;
        if total_votes < quorum_required {
            panic!("Quorum not met");
        }

        // Check approval (66% of votes cast must be For)
        if total_votes > 0 {
            let approval_required = (total_votes * approval_threshold as u128) / 10000u128;
            if proposal.votes_for < approval_required {
                panic!("Approval threshold not met");
            }
        }

        // Execute proposal based on type
        match &proposal.proposal_type {
            ProposalType::ParameterChange => {
                // For parameter changes, call the target function with parameters
                if let (Some(target), Some(function)) =
                    (&proposal.target_contract, &proposal.target_function)
                {
                    if !proposal.has_parameters {
                        panic!("ParameterChange proposal missing parameters");
                    }
                    let params = &proposal.parameters;
                    // Build arguments: parameter name and value as strings
                    let mut args = Vec::new(&env);
                    args.push_back(params.name.clone().into());
                    args.push_back(params.value.clone().into());

                    // Add any additional target args if provided
                    if let Some(target_args) = &proposal.target_args {
                        for i in 0..target_args.len() {
                            args.push_back(target_args.get(i).unwrap());
                        }
                    }

                    // Invoke target contract function
                    // Invoke target contract function
                    let _result: Val = env.invoke_contract(&target, function, args);
                } else {
                    panic!(
                        "ParameterChange proposal missing target contract, function, or parameters"
                    );
                }
            }
            ProposalType::ContractUpgrade => {
                // For contract upgrades, call upgrade function on target contract
                if let (Some(target), Some(function)) =
                    (&proposal.target_contract, &proposal.target_function)
                {
                    // Build arguments: new contract address from parameters or target args
                    let mut args = Vec::new(&env);

                    if let Some(target_args) = &proposal.target_args {
                        // Use provided target args (should contain new contract address)
                        for i in 0..target_args.len() {
                            args.push_back(target_args.get(i).unwrap());
                        }
                    } else if proposal.has_parameters {
                        // Try to extract address from parameters value
                        // Parameters value should be the new contract address as string
                        // For now, we'll require target_args to be provided
                        panic!("ContractUpgrade requires target_args with new contract address");
                    }

                    // Invoke upgrade function
                    let _result: Val = env.invoke_contract(&target, function, args);
                } else {
                    panic!("ContractUpgrade proposal missing target contract or function");
                }
            }
            ProposalType::EmergencyPause => {
                // For emergency pause, call pause/unpause on target contract
                if let (Some(target), Some(function)) =
                    (&proposal.target_contract, &proposal.target_function)
                {
                    // Build arguments: pause state (true/false)
                    let mut args = Vec::new(&env);

                    if let Some(target_args) = &proposal.target_args {
                        // Use provided target args (should contain pause boolean)
                        for i in 0..target_args.len() {
                            args.push_back(target_args.get(i).unwrap());
                        }
                    } else if proposal.has_parameters {
                        // Extract pause state from parameters value
                        // Value should be "true" or "false" as string
                        // Compare String directly (Soroban String doesn't have to_string())
                        let params = &proposal.parameters;
                        let pause_bool = params.value == String::from_str(&env, "true")
                            || params.value == String::from_str(&env, "1");
                        args.push_back(pause_bool.into());
                    } else {
                        panic!("EmergencyPause proposal missing pause state");
                    }

                    // Invoke pause/unpause function
                    let _result: Val = env.invoke_contract(&target, function, args);
                } else {
                    panic!("EmergencyPause proposal missing target contract or function");
                }
            }
        }

        // Mark as executed
        proposal.status = ProposalStatus::Executed;
        set_proposal(&env, &proposal);

        // Return proposal deposit to proposer (if proposal passed and executed)
        let min_deposit = get_min_proposal_deposit(&env);
        let governance_token = get_governance_token(&env);
        let token_client = token::Client::new(&env, &governance_token);
        let contract_address = env.current_contract_address();
        token_client.transfer(
            &contract_address,
            &proposal.proposer,
            &(min_deposit as i128),
        );

        // Emit event
        env.events().publish(
            (Symbol::new(&env, "ProposalExecuted"),),
            (proposal_id, executor, proposal.proposal_type),
        );
    }

    /// Update proposal status after voting period ends
    pub fn update_proposal_status(env: Env, proposal_id: u64) {
        let mut proposal = get_proposal(&env, proposal_id).expect("Proposal not found");

        if proposal.status != ProposalStatus::Active {
            return; // Already processed
        }

        let current_time = env.ledger().timestamp();
        if current_time <= proposal.voting_ends {
            return; // Voting period not ended yet
        }

        // Voting period ended, check if proposal passed
        let total_votes = proposal.votes_for + proposal.votes_against + proposal.votes_abstain;
        let circulating_power = Self::get_circulating_voting_power(env.clone());

        let quorum_threshold = get_quorum_threshold(&env);
        let approval_threshold = get_approval_threshold(&env);

        // Check quorum
        let quorum_required = (circulating_power * quorum_threshold as u128) / 10000u128;
        let quorum_met = total_votes >= quorum_required;

        // Check approval
        let approval_met = if total_votes > 0 {
            let approval_required = (total_votes * approval_threshold as u128) / 10000u128;
            proposal.votes_for >= approval_required
        } else {
            false
        };

        if quorum_met && approval_met {
            proposal.status = ProposalStatus::Passed;
            env.events().publish(
                (Symbol::new(&env, "ProposalPassed"),),
                (
                    proposal_id,
                    proposal.votes_for,
                    proposal.votes_against,
                    proposal.votes_abstain,
                ),
            );
        } else {
            proposal.status = ProposalStatus::Failed;
        }

        set_proposal(&env, &proposal);
    }

    fn get_circulating_voting_power(env: Env) -> u128 {
        // Try to get cached value
        if let Some(cached) = storage::get_circulating_voting_power(&env) {
            return cached;
        }

        // Calculate from governance token
        // For Stellar asset contracts, we need to track this manually
        // The admin should call update_circulating_voting_power when supply changes
        // For now, we calculate a reasonable estimate

        let governance_token = get_governance_token(&env);
        let token_client = token::Client::new(&env, &governance_token);

        // Get contract's token balance (tokens held by governance contract)
        // This includes escrowed tokens and proposal deposits
        let contract_balance = token_client.balance(&env.current_contract_address()) as u128;

        // Estimate circulating supply
        // In a real implementation, this would be tracked as a running total
        // For now, we use a conservative estimate: assume most tokens are in circulation
        // The admin should update this value when token supply changes significantly
        let circulating = contract_balance * 10; // Conservative multiplier estimate

        // Cache the value
        storage::set_circulating_voting_power(&env, circulating);
        circulating
    }

    /// Update circulating voting power (admin only)
    pub fn update_circulating_voting_power(env: Env, admin: Address, new_value: u128) {
        admin.require_auth();
        storage::require_admin(&env, &admin);
        storage::set_circulating_voting_power(&env, new_value);
    }

    /* ---------------- QUERY FUNCTIONS ---------------- */

    /// Get a proposal by ID
    pub fn get_proposal(env: Env, proposal_id: u64) -> Option<Proposal> {
        get_proposal(&env, proposal_id)
    }

    /// Get all active proposals
    pub fn get_active_proposals(env: Env) -> Vec<u64> {
        let counter = get_proposal_counter(&env);
        let mut active = Vec::new(&env);

        for i in 1..=counter {
            if let Some(proposal) = get_proposal(&env, i) {
                if proposal.status == ProposalStatus::Active {
                    active.push_back(i);
                }
            }
        }

        active
    }

    /// Get delegation for an address
    pub fn get_delegation(env: Env, delegator: Address) -> Option<Delegation> {
        get_delegation(&env, &delegator)
    }

    /// Get vote escrow for an address
    pub fn get_vote_escrow(env: Env, address: Address) -> Option<VoteEscrow> {
        get_vote_escrow(&env, &address)
    }

    /// Get vote record for a voter on a proposal
    pub fn get_vote(env: Env, proposal_id: u64, voter: Address) -> Option<Vote> {
        get_vote(&env, proposal_id, &voter)
    }
}
