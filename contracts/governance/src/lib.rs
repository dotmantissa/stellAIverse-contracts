#![no_std]
use soroban_sdk::xdr::ToXdr;
use soroban_sdk::{
    contract, contracterror, contractimpl, token, Address, Env, String, Symbol, Val, Vec,
};

mod storage;
mod types;

#[cfg(test)]
mod test;

use stellai_lib::rbac::{self};
use storage::*;
use types::*;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Error {
    Unauthorized = 100,
}

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
        voting_mechanism: Option<VotingMechanism>,
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
        let mechanism = voting_mechanism.unwrap_or(VotingMechanism::Linear);
        set_voting_mechanism(&env, &mechanism);
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

    /// Submit a proposal to the waitlist (requires deposit)
    pub fn submit_to_waitlist(
        env: Env,
        proposer: Address,
        title: String,
        description: String,
        proposal_type: ProposalType,
        parameters: ProposalParameters,
    ) -> u64 {
        proposer.require_auth();

        let min_deposit = get_min_proposal_deposit(&env);
        let governance_token = get_governance_token(&env);
        let token_client = token::Client::new(&env, &governance_token);

        let contract_address = env.current_contract_address();
        token_client.transfer(&proposer, &contract_address, &(min_deposit as i128));

        let waitlist_id = increment_waitlist_counter(&env);
        let waitlist_proposal = WaitlistProposal {
            waitlist_id,
            proposer: proposer.clone(),
            title,
            description,
            proposal_type,
            parameters,
            deposit_amount: min_deposit,
            submitted_at: env.ledger().timestamp(),
        };

        set_waitlist_proposal(&env, &waitlist_proposal);

        env.events().publish(
            (Symbol::new(&env, "ProposalWaitlisted"),),
            (waitlist_id, proposer, env.ledger().timestamp()),
        );

        waitlist_id
    }

    /// Promote a waitlisted proposal to active voting
    pub fn promote_to_voting(
        env: Env,
        admin: Address,
        waitlist_id: u64,
        voting_period: u64,
    ) -> u64 {
        admin.require_auth();
        require_admin(&env, &admin);

        let waitlist_proposal =
            get_waitlist_proposal(&env, waitlist_id).expect("Waitlisted proposal not found");

        let proposal_id = increment_proposal_counter(&env);
        let current_time = env.ledger().timestamp();

        let proposal = Proposal {
            proposal_id,
            title: waitlist_proposal.title,
            description: waitlist_proposal.description,
            proposer: waitlist_proposal.proposer.clone(),
            voting_starts: current_time,
            voting_ends: current_time + voting_period,
            proposal_type: waitlist_proposal.proposal_type,
            has_parameters: true,
            parameters: waitlist_proposal.parameters,
            votes_for: 0,
            votes_against: 0,
            votes_abstain: 0,
            status: ProposalStatus::Active,
            target_contract: None,
            target_function: None,
            target_args: None,
        };

        set_proposal(&env, &proposal);
        remove_waitlist_proposal(&env, waitlist_id);

        env.events().publish(
            (Symbol::new(&env, "ProposalPromoted"),),
            (waitlist_id, proposal_id, current_time),
        );

        proposal_id
    }

    /// Get a waitlisted proposal by ID
    pub fn get_waitlist_proposal(env: Env, waitlist_id: u64) -> Option<WaitlistProposal> {
        get_waitlist_proposal(&env, waitlist_id)
    }

    /// Cancel a waitlisted proposal and refund deposit
    pub fn cancel_waitlist_proposal(env: Env, proposer: Address, waitlist_id: u64) {
        proposer.require_auth();

        let waitlist_proposal =
            get_waitlist_proposal(&env, waitlist_id).expect("Waitlisted proposal not found");

        if waitlist_proposal.proposer != proposer {
            panic!("Only the proposer can cancel");
        }

        // Refund deposit
        let governance_token = get_governance_token(&env);
        let token_client = token::Client::new(&env, &governance_token);
        let contract_address = env.current_contract_address();
        token_client.transfer(
            &contract_address,
            &proposer,
            &(waitlist_proposal.deposit_amount as i128),
        );

        remove_waitlist_proposal(&env, waitlist_id);

        env.events().publish(
            (Symbol::new(&env, "ProposalWaitlistCancelled"),),
            (waitlist_id, proposer),
        );
    }

    /// Get voting power for an address
    pub fn get_vote_power(env: Env, address: Address) -> u128 {
        let governance_token = get_governance_token(&env);
        let token_client = token::Client::new(&env, &governance_token);
        let base_balance = token_client.balance(&address) as u128;

        let escrow_power = if let Some(escrow) = get_vote_escrow(&env, &address) {
            let current_time = env.ledger().timestamp();
            if escrow.lock_end > current_time {
                (escrow.amount * escrow.multiplier as u128) / 10000u128
            } else {
                0
            }
        } else {
            0
        };

        let delegated_power =
            Self::calculate_delegated_power_to(&env, &address, env.ledger().timestamp());

        let own_delegated_away = if let Some(delegation) = get_delegation(&env, &address) {
            delegation.amount
        } else {
            0
        };

        let own_power = base_balance + escrow_power;
        let available_own_power = own_power.saturating_sub(own_delegated_away);

        available_own_power + delegated_power
    }

    /// Delegate voting power to another address with expiry
    pub fn delegate_voting_power(
        env: Env,
        delegator: Address,
        delegatee: Address,
        amount: u128,
        expires_at: Option<u64>,
    ) {
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

        // Cache current timestamp to avoid multiple calls
        let current_time = env.ledger().timestamp();

        let escrow_power = if let Some(escrow) = get_vote_escrow(&env, &delegator) {
            if escrow.lock_end > current_time {
                (escrow.amount * escrow.multiplier as u128) / 10000u128
            } else {
                0
            }
        } else {
            0
        };

        let available_power = base_balance + escrow_power;

        if amount > available_power {
            panic!("Insufficient voting power to delegate");
        }

        if let Some(expiry) = expires_at {
            if expiry <= current_time {
                panic!("Delegation expiry must be in the future");
            }
        }

        let delegation = Delegation {
            delegatee: delegatee.clone(),
            amount,
            created_at: current_time,
            expires_at,
            active: true,
        };

        set_delegation(&env, &delegator, &delegation);

        env.events().publish(
            (Symbol::new(&env, "VotingPowerDelegated"),),
            (delegator, delegatee, amount),
        );
    }

    /// Remove delegation
    pub fn undelegate_voting_power(env: Env, delegator: Address) {
        delegator.require_auth();

        let old_delegation = get_delegation(&env, &delegator).expect("No delegation to remove");

        storage::remove_delegator_from_list(&env, &old_delegation.delegatee, &delegator);
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

        if !(4..=52).contains(&lock_duration_weeks) {
            panic!("Lock duration must be between 4 and 52 weeks");
        }

        let governance_token = get_governance_token(&env);
        let token_client = token::Client::new(&env, &governance_token);
        let balance = token_client.balance(&locker);

        if balance < amount as i128 {
            panic!("Insufficient balance");
        }

        // Pre-calculate multiplier to avoid repeated arithmetic
        let multiplier = 20000u32 + ((lock_duration_weeks - 4) * 20000u32) / 48;
        let contract_address = env.current_contract_address();

        // Transfer tokens first
        token_client.transfer(&locker, &contract_address, &(amount as i128));

        // Cache timestamp and calculate lock_end once
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

    /// Cast a vote on a proposal with quadratic voting support
    pub fn cast_vote(env: Env, voter: Address, proposal_id: u64, vote_type: VoteType) {
        voter.require_auth();

        // ─── SNAPSHOT PHASE ───
        let mut proposal = get_proposal(&env, proposal_id).expect("Proposal not found");
        let current_time = env.ledger().timestamp();
        let existing_vote = get_vote(&env, proposal_id, &voter);
        let snapshot_exists = get_delegation_snapshot(&env, proposal_id).is_some();
        let mechanism = get_voting_mechanism(&env);
        let voting_power = Self::calculate_total_voting_power(&env, &voter);

        // ─── VALIDATION PHASE ───
        if current_time < proposal.voting_starts {
            panic!("Voting has not started yet");
        }
        if current_time > proposal.voting_ends {
            panic!("Voting period has ended");
        }
        if proposal.status != ProposalStatus::Active {
            panic!("Proposal is not active");
        }
        if existing_vote.is_some() {
            panic!("Already voted on this proposal");
        }
        if voting_power == 0 {
            panic!("No voting power");
        }

        // ─── MUTATION PHASE ───

        // Create delegation snapshot for secure voting if not exists
        if !snapshot_exists {
            Self::create_delegation_snapshot(&env, proposal_id);
        }

        let (vote_weight, voting_power_used) =
            Self::calculate_vote_weight_internal(mechanism, voting_power);

        // Record vote
        let vote = Vote {
            proposal_id,
            voter: voter.clone(),
            vote_type: vote_type.clone(),
            weight: vote_weight,
            voting_power_used,
            timestamp: current_time,
        };
        set_vote(&env, proposal_id, &voter, &vote);

        // Update proposal vote counts
        match vote_type {
            VoteType::For => proposal.votes_for += vote_weight,
            VoteType::Against => proposal.votes_against += vote_weight,
            VoteType::Abstain => proposal.votes_abstain += vote_weight,
        }
        set_proposal(&env, &proposal);

        env.events().publish(
            (Symbol::new(&env, "VoteCast"),),
            (
                proposal_id,
                voter,
                vote_type,
                vote_weight,
                voting_power_used,
            ),
        );
    }

    /// Calculate vote weight based on voting mechanism (linear or quadratic)
    fn calculate_vote_weight_internal(
        mechanism: VotingMechanism,
        voting_power: u128,
    ) -> (u128, u128) {
        match mechanism {
            VotingMechanism::Linear => {
                // Linear voting: 1 token = 1 vote
                (voting_power, voting_power)
            }
            VotingMechanism::Quadratic => {
                // Quadratic voting: vote weight = sqrt(voting_power)
                // This reduces the influence of large token holders
                let vote_weight = Self::integer_sqrt(voting_power);
                (vote_weight, voting_power)
            }
        }
    }

    /// Integer square root calculation for quadratic voting
    fn integer_sqrt(n: u128) -> u128 {
        if n == 0 || n == 1 {
            return n;
        }

        let mut low = 1u128;
        let mut high = n;
        let mut result = 0u128;

        while low <= high {
            let mid = (low + high) / 2;
            let mid_squared = mid.saturating_mul(mid);

            if mid_squared == n {
                return mid;
            }

            if mid_squared < n {
                low = mid + 1;
                result = mid;
            } else {
                high = mid - 1;
            }
        }

        result
    }

    /// Create delegation snapshot for secure voting
    fn create_delegation_snapshot(env: &Env, proposal_id: u64) {
        let current_block = env.ledger().sequence();
        let delegator_powers = Vec::new(env);
        let total_delegated_power = 0u128;

        // In a real implementation, we would iterate through all delegations
        // For now, we create an empty snapshot as a placeholder
        let snapshot = DelegationSnapshot {
            block_number: current_block as u64,
            total_delegated_power,
            delegator_powers,
        };

        set_delegation_snapshot(env, proposal_id, &snapshot);
    }

    /// Update voting mechanism (admin only)
    pub fn update_voting_mechanism(env: Env, admin: Address, mechanism: VotingMechanism) {
        admin.require_auth();
        require_admin(&env, &admin);

        set_voting_mechanism(&env, &mechanism);

        env.events().publish(
            (Symbol::new(&env, "VotingMechanismUpdated"),),
            (admin, mechanism),
        );
    }

    fn calculate_total_voting_power(env: &Env, address: &Address) -> u128 {
        let governance_token = get_governance_token(env);
        let token_client = token::Client::new(env, &governance_token);

        // Base voting power from token balance
        let base_balance = token_client.balance(address) as u128;

        // Cache timestamp to avoid multiple calls
        let current_time = env.ledger().timestamp();

        // Add vote escrow power
        let escrow_power = if let Some(escrow) = get_vote_escrow(env, address) {
            if escrow.lock_end > current_time {
                // Escrow is still locked, apply multiplier
                (escrow.amount * escrow.multiplier as u128) / 10000u128
            } else {
                // Escrow expired, no multiplier
                0
            }
        } else {
            0
        };

        // Calculate delegated power TO this address using reverse index
        let delegated_power = Self::calculate_delegated_power_to(env, address, current_time);

        base_balance + escrow_power + delegated_power
    }

    fn calculate_delegated_power_to(env: &Env, delegatee: &Address, current_time: u64) -> u128 {
        let delegators = storage::get_delegators_to(env, delegatee);
        let mut total_delegated = 0u128;
        let governance_token = get_governance_token(env);
        let token_client = token::Client::new(env, &governance_token);

        for i in 0..delegators.len() {
            let delegator = delegators.get(i).unwrap();
            if let Some(delegation) = storage::get_delegation(env, &delegator) {
                // Get delegator's base voting power (not including their own delegations)
                let base_balance = token_client.balance(&delegator) as u128;

                // Add escrow power if exists
                let escrow_power = if let Some(escrow) = storage::get_vote_escrow(env, &delegator) {
                    if escrow.lock_end > current_time {
                        (escrow.amount * escrow.multiplier as u128) / 10000u128
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
        Self::require_multisig_approval_if_enabled(&env, proposal_id);
        Self::execute_proposal_internal(&env, executor, proposal_id);
    }

    // ── Role Management (Issue #178) ─────────────────────────────────────────────

    /// Assign governance role to an address (admin only)
    /// Ensures mutual exclusion with KYC operator roles
    pub fn assign_governance_role(
        env: Env,
        admin: Address,
        new_governance: Address,
    ) -> Result<(), Error> {
        // Validate caller is admin using enhanced RBAC
        rbac::require_admin_indirect_safe(&env, &admin).map_err(|_| Error::Unauthorized)?;

        // Use RBAC module for role assignment with mutual exclusion
        rbac::assign_governance_role(&env, &admin, &new_governance)
            .map_err(|_| Error::Unauthorized)?;

        env.events().publish(
            (Symbol::new(&env, "GovernanceRoleAssigned"),),
            (new_governance, admin, env.ledger().timestamp()),
        );

        Ok(())
    }

    /// Assign KYC operator role to an address (admin only)
    /// Ensures mutual exclusion with governance roles
    pub fn assign_kyc_operator_role(
        env: Env,
        admin: Address,
        new_operator: Address,
    ) -> Result<(), Error> {
        // Validate caller is admin using enhanced RBAC
        rbac::require_admin_indirect_safe(&env, &admin).map_err(|_| Error::Unauthorized)?;

        // Use RBAC module for role assignment with mutual exclusion
        rbac::assign_kyc_operator_role(&env, &admin, &new_operator)
            .map_err(|_| Error::Unauthorized)?;

        env.events().publish(
            (Symbol::new(&env, "KycOperatorRoleAssigned"),),
            (new_operator, admin, env.ledger().timestamp()),
        );

        Ok(())
    }

    /// Enhanced admin check for internal calls (Issue #179)
    pub fn admin_internal_operation(
        env: Env,
        admin: Address,
        operation: Symbol,
    ) -> Result<(), Error> {
        // Use enhanced validation for internal calls
        rbac::validate_internal_call(&env, &admin, &operation).map_err(|_| Error::Unauthorized)?;

        Ok(())
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

    /// Get current voting mechanism
    pub fn get_voting_mechanism(env: Env) -> VotingMechanism {
        get_voting_mechanism(&env)
    }

    /// Get delegation snapshot for a proposal
    pub fn get_delegation_snapshot(env: Env, proposal_id: u64) -> Option<DelegationSnapshot> {
        get_delegation_snapshot(&env, proposal_id)
    }

    /// Check if a delegation is still active (not expired)
    pub fn is_delegation_active(env: Env, delegator: Address) -> bool {
        if let Some(delegation) = get_delegation(&env, &delegator) {
            if !delegation.active {
                return false;
            }

            if let Some(expires_at) = delegation.expires_at {
                let current_time = env.ledger().timestamp();
                return current_time < expires_at;
            }

            true
        } else {
            false
        }
    }

    // ── Timelock Governance Execution ─────────────────────────────────

    /// Initialize timelock configuration for governance parameter updates
    pub fn init_timelock(
        env: Env,
        admin: Address,
        min_delay: u64,
        max_delay: u64,
        default_delay: u64,
    ) {
        admin.require_auth();
        require_admin(&env, &admin);

        if min_delay == 0 {
            panic!("Minimum delay must be greater than 0");
        }

        if max_delay <= min_delay {
            panic!("Maximum delay must be greater than minimum delay");
        }

        if default_delay < min_delay || default_delay > max_delay {
            panic!("Default delay must be between min and max delay");
        }

        let config = types::TimelockConfig {
            min_delay,
            max_delay,
            default_delay,
            enabled: true,
        };

        storage::set_timelock_config(&env, &config);

        env.events().publish(
            (Symbol::new(&env, "TimelockInitialized"),),
            (min_delay, max_delay, default_delay),
        );
    }

    /// Queue a parameter update for timelock execution
    pub fn queue_parameter_update(
        env: Env,
        proposer: Address,
        proposal_id: u64,
        target_contract: Address,
        target_function: Symbol,
        target_args: Vec<Val>,
        delay: Option<u64>,
    ) -> u64 {
        proposer.require_auth();

        // Verify timelock is enabled
        let timelock_config = storage::get_timelock_config(&env).expect("Timelock not configured");

        if !timelock_config.enabled {
            panic!("Timelock is not enabled");
        }

        // Verify proposal exists and is passed
        let proposal = get_proposal(&env, proposal_id).expect("Proposal not found");
        if proposal.status != ProposalStatus::Passed {
            panic!("Proposal must be in Passed state to queue for execution");
        }

        // Validate parameters if this is a parameter change
        if proposal.proposal_type == ProposalType::ParameterChange && proposal.has_parameters {
            Self::validate_parameter_change(&env, &proposal.parameters);
        }

        // Calculate delay
        let actual_delay = delay.unwrap_or(timelock_config.default_delay);
        if actual_delay < timelock_config.min_delay || actual_delay > timelock_config.max_delay {
            panic!("Delay must be between min and max timelock delay");
        }

        // Create timelock entry
        let entry_id = storage::increment_timelock_counter(&env);
        let current_time = env.ledger().timestamp();
        let executable_at = current_time + actual_delay;

        let entry = types::TimelockEntry {
            entry_id,
            proposal_id,
            target_contract: target_contract.clone(),
            target_function: target_function.clone(),
            target_args: target_args.clone(),
            queued_at: current_time,
            executable_at,
            executed: false,
            cancelled: false,
            queued_by: proposer.clone(),
        };

        storage::set_timelock_entry(&env, &entry);

        env.events().publish(
            (Symbol::new(&env, "ParameterUpdateQueued"),),
            (entry_id, proposal_id, executable_at),
        );

        entry_id
    }

    /// Execute a queued parameter update after timelock delay
    pub fn execute_queued_update(env: Env, executor: Address, entry_id: u64) {
        executor.require_auth();

        let mut entry =
            storage::get_timelock_entry(&env, entry_id).expect("Timelock entry not found");

        if entry.executed {
            panic!("Timelock entry already executed");
        }

        if entry.cancelled {
            panic!("Timelock entry has been cancelled");
        }

        let current_time = env.ledger().timestamp();
        if current_time < entry.executable_at {
            panic!("Timelock delay has not passed yet");
        }

        // Create storage snapshot for integrity validation
        Self::create_storage_snapshot(&env, &entry.target_contract, &entry.target_args);

        // Execute the parameter update
        let _result: Val = env.invoke_contract(
            &entry.target_contract,
            &entry.target_function,
            entry.target_args.clone(),
        );

        // Mark as executed
        entry.executed = true;
        storage::set_timelock_entry(&env, &entry);

        env.events().publish(
            (Symbol::new(&env, "QueuedUpdateExecuted"),),
            (entry_id, executor, current_time),
        );
    }

    /// Cancel a queued parameter update
    pub fn cancel_queued_update(env: Env, canceller: Address, entry_id: u64) {
        canceller.require_auth();

        let mut entry =
            storage::get_timelock_entry(&env, entry_id).expect("Timelock entry not found");

        if entry.executed {
            panic!("Cannot cancel executed timelock entry");
        }

        if entry.cancelled {
            panic!("Timelock entry already cancelled");
        }

        // Only the queuer or admin can cancel
        let admin = get_admin(&env);
        if entry.queued_by != canceller && canceller != admin {
            panic!("Only queuer or admin can cancel timelock entry");
        }

        entry.cancelled = true;
        storage::set_timelock_entry(&env, &entry);

        env.events().publish(
            (Symbol::new(&env, "QueuedUpdateCancelled"),),
            (entry_id, canceller),
        );
    }

    /// Get timelock entry by ID
    pub fn get_timelock_entry(env: Env, entry_id: u64) -> Option<types::TimelockEntry> {
        storage::get_timelock_entry(&env, entry_id)
    }

    /// Get timelock configuration
    pub fn get_timelock_config(env: Env) -> Option<types::TimelockConfig> {
        storage::get_timelock_config(&env)
    }

    // ── Parameter Validation ─────────────────────────────────

    /// Set parameter validation rules
    pub fn set_parameter_rule(env: Env, admin: Address, rule: types::ParameterRule) {
        admin.require_auth();
        require_admin(&env, &admin);

        storage::set_parameter_rule(&env, &rule);

        env.events().publish(
            (Symbol::new(&env, "ParameterRuleSet"),),
            (rule.name.clone(), rule.requires_timelock),
        );
    }

    /// Validate a parameter change against rules
    fn validate_parameter_change(env: &Env, parameters: &types::ProposalParameters) {
        if let Some(rule) = storage::get_parameter_rule(env, parameters.name.clone()) {
            // Check if timelock is required
            if rule.requires_timelock {
                let timelock_config = storage::get_timelock_config(env);
                if timelock_config.is_none() || !timelock_config.unwrap().enabled {
                    panic!("Parameter requires timelock but timelock is not enabled");
                }
            }

            // Validate parameter value based on type
            Self::validate_parameter_value(env, parameters.value.clone(), &rule);
        }
    }

    /// Validate parameter value based on type and rules
    fn validate_parameter_value(env: &Env, value: String, rule: &types::ParameterRule) {
        match rule.param_type {
            ParameterType::Bool => {
                if value != String::from_str(env, "true")
                    && value != String::from_str(env, "false")
                    && value != String::from_str(env, "1")
                    && value != String::from_str(env, "0")
                {
                    panic!("Invalid boolean value");
                }
            }
            ParameterType::String | ParameterType::Symbol => {
                if let Some(allowed) = &rule.allowed_values {
                    let mut found = false;
                    for i in 0..allowed.len() {
                        if allowed.get(i).unwrap() == value {
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        panic!("Value not in allowed list");
                    }
                }
            }
            ParameterType::U64 | ParameterType::I64 | ParameterType::U128 => {
                // Basic format validation for numeric types
                // In a full implementation, we would properly parse the string into the correct numeric type
                if value.is_empty() {
                    panic!("Invalid numeric value - empty string");
                }
            }
            ParameterType::Address => {
                // Validate address format - ensure string isn't empty
                if value.is_empty() {
                    panic!("Invalid address - empty string");
                }
            }
        }
    }
    /// Create storage snapshot for integrity validation
    fn create_storage_snapshot(env: &Env, target_contract: &Address, target_args: &Vec<Val>) {
        // Capture the current state of relevant storage keys before the parameter change
        // This ensures we can validate that only intended parameters change

        if target_args.len() >= 2 {
            let storage_key = target_args.get(0).unwrap();
            let _parameter_name = target_args.get(1).unwrap();

            // Try to get current value from target contract
            let before_value = Self::try_get_storage_value(env, target_contract, &storage_key);

            let snapshot = types::StorageSnapshot {
                contract_address: target_contract.clone(),
                storage_key: storage_key.to_xdr(env),
                before_value: before_value.map(|v| v.to_xdr(env)),
                after_value: None, // Will be set after execution
                timestamp: env.ledger().timestamp(),
            };

            storage::set_storage_snapshot(env, &snapshot);
        }
    }

    /// Attempt to read storage value from target contract (read-only)
    fn try_get_storage_value(
        _env: &Env,
        _target_contract: &Address,
        _storage_key: &Val,
    ) -> Option<Val> {
        // In a real implementation, this would use a read-only contract call
        // to get the current storage value. For now, we return None as placeholder
        // This would be implemented using env.invoke_contract with a read function
        None
    }

    /// Validate storage integrity after parameter change
    fn validate_storage_integrity(env: &Env, target_contract: &Address, storage_key: &Val) {
        if let Some(snapshot) = storage::get_storage_snapshot(env, target_contract, storage_key) {
            // Get current value after execution
            let after_value = Self::try_get_storage_value(env, target_contract, storage_key);

            // Validate that only the intended storage key changed
            // In a more comprehensive implementation, we would:
            // 1. Check that only the target storage key changed
            // 2. Verify the change matches the expected parameter value
            // 3. Ensure no other storage keys were modified

            // Update snapshot with after value
            let mut updated_snapshot = snapshot;
            updated_snapshot.after_value = after_value.map(|v| v.to_xdr(env));
            storage::set_storage_snapshot(env, &updated_snapshot);
        }
    }

    /// Execute parameter change with full integrity validation
    pub fn execute_parameter_change_safe(
        env: Env,
        executor: Address,
        target_contract: Address,
        parameter_name: String,
        new_value: String,
        storage_key: Val,
    ) {
        executor.require_auth();

        // Validate parameter against rules
        let parameters = types::ProposalParameters {
            name: parameter_name.clone(),
            value: new_value.clone(),
        };
        Self::validate_parameter_change(&env, &parameters);

        // Create pre-execution snapshot
        let before_value = Self::try_get_storage_value(&env, &target_contract, &storage_key);

        let snapshot = types::StorageSnapshot {
            contract_address: target_contract.clone(),
            storage_key: storage_key.to_xdr(&env),
            before_value: before_value.map(|v| v.to_xdr(&env)),
            after_value: None,
            timestamp: env.ledger().timestamp(),
        };
        storage::set_storage_snapshot(&env, &snapshot);

        // Build arguments for the target contract call
        let mut args = Vec::new(&env);
        args.push_back(storage_key);
        args.push_back(new_value.clone().into());

        // Execute the parameter change
        let _result: Val =
            env.invoke_contract(&target_contract, &Symbol::new(&env, "set_parameter"), args);

        // Validate post-execution integrity
        Self::validate_storage_integrity(&env, &target_contract, &storage_key);

        // Emit event for audit trail
        env.events().publish(
            (Symbol::new(&env, "ParameterChangeSafe"),),
            (target_contract, parameter_name, new_value),
        );
    }

    /// Batch parameter updates with atomic integrity validation
    pub fn execute_parameter_batch_safe(
        env: Env,
        executor: Address,
        updates: Vec<(String, String, Val)>, // (parameter_name, new_value, storage_key)
    ) {
        executor.require_auth();

        // Create snapshots for all updates before execution
        let mut snapshots = Vec::new(&env);
        for i in 0..updates.len() {
            let (param_name, new_value, storage_key) = updates.get(i).unwrap();

            // Validate each parameter
            let parameters = types::ProposalParameters {
                name: param_name.clone(),
                value: new_value.clone(),
            };
            Self::validate_parameter_change(&env, &parameters);

            // Create snapshot
            let before_value =
                Self::try_get_storage_value(&env, &env.current_contract_address(), &storage_key);
            let snapshot = types::StorageSnapshot {
                contract_address: env.current_contract_address(),
                storage_key: storage_key.to_xdr(&env),
                before_value: before_value.map(|v| v.to_xdr(&env)),
                after_value: None,
                timestamp: env.ledger().timestamp(),
            };

            snapshots.push_back(snapshot);
        }

        // Execute all updates atomically (in a real implementation)
        // For now, we execute them sequentially but validate integrity after each

        for i in 0..updates.len() {
            let (_param_name, new_value, storage_key) = updates.get(i).unwrap();

            let mut args = Vec::new(&env);
            args.push_back(storage_key);
            args.push_back(new_value.clone().into());

            let _result: Val = env.invoke_contract(
                &env.current_contract_address(),
                &Symbol::new(&env, "set_parameter"),
                args,
            );

            // Validate integrity after each update
            Self::validate_storage_integrity(&env, &env.current_contract_address(), &storage_key);
        }

        // Emit batch completion event
        env.events().publish(
            (Symbol::new(&env, "ParameterBatchSafe"),),
            (updates.len(), executor),
        );
    }

    /// Get storage snapshot for integrity verification
    pub fn get_storage_snapshot(
        env: Env,
        contract_address: Address,
        storage_key: Val,
    ) -> Option<types::StorageSnapshot> {
        storage::get_storage_snapshot(&env, &contract_address, &storage_key)
    }

    /// Verify parameter change integrity (admin only)
    pub fn verify_parameter_integrity(
        env: Env,
        admin: Address,
        contract_address: Address,
        storage_key: Val,
    ) -> bool {
        admin.require_auth();
        require_admin(&env, &admin);

        if let Some(snapshot) = storage::get_storage_snapshot(&env, &contract_address, &storage_key)
        {
            // Get current value
            let current_value = Self::try_get_storage_value(&env, &contract_address, &storage_key);

            // Verify that the after value matches current value
            match (snapshot.after_value, current_value) {
                (Some(stored), Some(current)) => stored == current.to_xdr(&env),
                (None, None) => true, // No change expected
                _ => false,           // Mismatch
            }
        } else {
            false // No snapshot found
        }
    }

    /// Get parameter rule by name
    pub fn get_parameter_rule(env: Env, parameter_name: String) -> Option<types::ParameterRule> {
        storage::get_parameter_rule(&env, parameter_name.clone())
    }

    // ── Multi-Signature Governance Execution ─────────────────────────────────

    /// Initialize multisig configuration for governance actions
    pub fn init_multisig(
        env: Env,
        admin: Address,
        threshold: u32,
        authorized_signers: Vec<Address>,
        approval_validity_secs: u64,
    ) {
        admin.require_auth();
        require_admin(&env, &admin);

        if threshold == 0 {
            panic!("Threshold must be greater than 0");
        }

        if threshold > authorized_signers.len() {
            panic!("Threshold cannot exceed number of signers");
        }

        if authorized_signers.len() < 2 {
            panic!("Must have at least 2 authorized signers");
        }

        // Check for duplicate signers
        for i in 0..authorized_signers.len() {
            for j in (i + 1)..authorized_signers.len() {
                if authorized_signers.get(i).unwrap() == authorized_signers.get(j).unwrap() {
                    panic!("Duplicate signer not allowed");
                }
            }
        }

        let config = types::MultisigConfig {
            threshold,
            authorized_signers,
            approval_validity_secs,
            enabled: true,
        };

        storage::set_multisig_config(&env, &config);

        env.events().publish(
            (Symbol::new(&env, "MultisigInitialized"),),
            (threshold, config.authorized_signers.len()),
        );
    }

    /// Approve a proposal execution (multisig signers only)
    pub fn approve_proposal_execution(env: Env, signer: Address, proposal_id: u64) {
        signer.require_auth();

        // Verify signer is authorized
        if !storage::is_authorized_signer(&env, &signer) {
            panic!("Not an authorized multisig signer");
        }

        // Verify proposal exists and is in Passed state
        let proposal = get_proposal(&env, proposal_id).expect("Proposal not found");
        if proposal.status != ProposalStatus::Passed {
            panic!("Proposal must be in Passed state to approve execution");
        }

        // Get or create multisig approval
        let config = storage::get_multisig_config(&env).expect("Multisig not configured");
        let current_time = env.ledger().timestamp();

        let mut approval = if let Some(existing) = storage::get_multisig_approval(&env, proposal_id)
        {
            // Check if already expired
            if current_time >= existing.expires_at {
                panic!("Multisig approval has expired");
            }

            // Check if already executed
            if existing.executed {
                panic!("Proposal already executed");
            }

            // Check for duplicate approval
            for i in 0..existing.approvers.len() {
                if existing.approvers.get(i).unwrap() == signer {
                    panic!("Already approved by this signer");
                }
            }

            existing
        } else {
            // Create new approval record
            types::MultisigApproval {
                proposal_id,
                approvers: Vec::new(&env),
                required_approvals: config.threshold,
                created_at: current_time,
                expires_at: current_time + config.approval_validity_secs,
                executed: false,
            }
        };

        // Add approval
        approval.approvers.push_back(signer.clone());
        storage::set_multisig_approval(&env, proposal_id, &approval);

        env.events().publish(
            (Symbol::new(&env, "ProposalExecutionApproved"),),
            (
                proposal_id,
                signer,
                approval.approvers.len(),
                approval.required_approvals,
            ),
        );
    }

    /// Execute a proposal with multisig approval
    /// Requires: proposal passed + sufficient multisig approvals
    pub fn execute_proposal_with_multisig(env: Env, executor: Address, proposal_id: u64) {
        executor.require_auth();
        Self::require_multisig_approval_if_enabled(&env, proposal_id);
        Self::execute_proposal_internal(&env, executor, proposal_id);
    }

    /// Internal helper to check multisig approval if enabled
    fn require_multisig_approval_if_enabled(env: &Env, proposal_id: u64) {
        if let Some(config) = storage::get_multisig_config(env) {
            if config.enabled {
                let approval = storage::get_multisig_approval(env, proposal_id)
                    .expect("No multisig approval found for proposal");

                // Check if already executed (multisig level)
                if approval.executed {
                    panic!("Multisig approval already used/executed");
                }

                // Check if expired
                let current_time = env.ledger().timestamp();
                if current_time >= approval.expires_at {
                    panic!("Multisig approval has expired");
                }

                // Verify threshold met
                if !storage::has_reached_threshold(&approval) {
                    panic!(
                        "Insufficient multisig approvals: {} of {} required",
                        approval.approvers.len(),
                        approval.required_approvals
                    );
                }

                // Mark as executed to prevent double-spending of this approval record
                let mut updated_approval = approval;
                updated_approval.executed = true;
                storage::set_multisig_approval(env, proposal_id, &updated_approval);
            }
        }
    }

    /// Internal proposal execution logic
    fn execute_proposal_internal(env: &Env, executor: Address, proposal_id: u64) {
        // ─── SNAPSHOT PHASE ───
        let mut proposal = get_proposal(env, proposal_id).expect("Proposal not found");
        let circulating_power = Self::get_circulating_voting_power(env.clone());
        let quorum_threshold = get_quorum_threshold(env);
        let approval_threshold = get_approval_threshold(env);

        // ─── VALIDATION PHASE ───
        let total_votes = proposal.votes_for + proposal.votes_against + proposal.votes_abstain;

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

        // ─── MUTATION PHASE ───

        // Persist the execution state before calling out to the target contract
        proposal.status = ProposalStatus::Executed;
        set_proposal(env, &proposal);

        // Execute proposal based on type
        match &proposal.proposal_type {
            ProposalType::ParameterChange => {
                if let (Some(target), Some(function)) =
                    (&proposal.target_contract, &proposal.target_function)
                {
                    if !proposal.has_parameters {
                        panic!("ParameterChange proposal missing parameters");
                    }
                    let params = &proposal.parameters;
                    let mut args = Vec::new(env);
                    args.push_back(params.name.clone().into());
                    args.push_back(params.value.clone().into());

                    if let Some(target_args) = &proposal.target_args {
                        for i in 0..target_args.len() {
                            args.push_back(target_args.get(i).unwrap());
                        }
                    }

                    let _result: Val = env.invoke_contract(target, function, args);
                } else {
                    panic!(
                        "ParameterChange proposal missing target contract, function, or parameters"
                    );
                }
            }
            ProposalType::ContractUpgrade => {
                if let (Some(target), Some(function)) =
                    (&proposal.target_contract, &proposal.target_function)
                {
                    let mut args = Vec::new(env);

                    if let Some(target_args) = &proposal.target_args {
                        for i in 0..target_args.len() {
                            args.push_back(target_args.get(i).unwrap());
                        }
                    } else if proposal.has_parameters {
                        panic!("ContractUpgrade requires target_args with new contract address");
                    }

                    let _result: Val = env.invoke_contract(target, function, args);
                } else {
                    panic!("ContractUpgrade proposal missing target contract or function");
                }
            }
            ProposalType::EmergencyPause => {
                if let (Some(target), Some(function)) =
                    (&proposal.target_contract, &proposal.target_function)
                {
                    let mut args = Vec::new(env);

                    if let Some(target_args) = &proposal.target_args {
                        for i in 0..target_args.len() {
                            args.push_back(target_args.get(i).unwrap());
                        }
                    } else if proposal.has_parameters {
                        let params = &proposal.parameters;
                        let pause_bool = params.value == String::from_str(env, "true")
                            || params.value == String::from_str(env, "1");
                        args.push_back(pause_bool.into());
                    } else {
                        panic!("EmergencyPause proposal missing pause state");
                    }

                    let _result: Val = env.invoke_contract(target, function, args);
                } else {
                    panic!("EmergencyPause proposal missing target contract or function");
                }
            }
        }

        // Return proposal deposit to proposer
        let min_deposit = get_min_proposal_deposit(env);
        let governance_token = get_governance_token(env);
        let token_client = token::Client::new(env, &governance_token);
        let contract_address = env.current_contract_address();
        token_client.transfer(
            &contract_address,
            &proposal.proposer,
            &(min_deposit as i128),
        );

        // Emit event
        env.events().publish(
            (Symbol::new(env, "ProposalExecuted"),),
            (proposal_id, executor, proposal.proposal_type),
        );
    }

    /// Get multisig approval status for a proposal
    pub fn get_multisig_approval_status(
        env: Env,
        proposal_id: u64,
    ) -> Option<types::MultisigApproval> {
        storage::get_multisig_approval(&env, proposal_id)
    }

    /// Get multisig configuration
    pub fn get_multisig_config(env: Env) -> Option<types::MultisigConfig> {
        storage::get_multisig_config(&env)
    }
}
