#![no_std]

mod types;

use crate::types::{ProposalStatus, ThresholdKeyShare, ThresholdProposal};
use soroban_sdk::{contract, contractimpl, Address, Bytes, Env, Symbol, Vec};

#[contract]
pub struct ThresholdAgentContract;

#[contractimpl]
impl ThresholdAgentContract {
    pub fn create_threshold_agent(env: Env, agent_id: u64, owners: Vec<Address>, threshold_m: u32) {
        assert!(
            threshold_m <= owners.len(),
            "Threshold cannot exceed number of owners"
        );
        for i in 0..owners.len() {
            let owner = owners.get(i).unwrap();
            let share = ThresholdKeyShare {
                agent_id,
                share_holder: owner,
                share_index: i,
                x_coordinate: i,
                y_coordinate_encrypted: Bytes::new(&env),
                commitment: Bytes::new(&env),
                created_at: env.ledger().timestamp(),
            };
            env.storage().persistent().set(&(agent_id, i), &share);
        }
        env.events().publish(
            (Symbol::new(&env, "ThresholdAgentCreated"), agent_id),
            owners,
        );
    }

    pub fn propose_action(env: Env, agent_id: u64, action_data: Bytes, proposer: Address) -> u64 {
        let proposal_id = env.ledger().sequence() as u64;
        let proposal = ThresholdProposal {
            proposal_id,
            agent_id,
            action_data,
            proposer,
            threshold_m: 2, // Example threshold
            signatures: Vec::new(&env),
            status: ProposalStatus::Pending,
        };
        env.storage()
            .persistent()
            .set(&(agent_id, proposal_id), &proposal);
        proposal_id
    }

    pub fn sign_proposal(env: Env, agent_id: u64, proposal_id: u64, signer: Address) {
        let mut proposal: ThresholdProposal = env
            .storage()
            .persistent()
            .get(&(agent_id, proposal_id))
            .unwrap();
        signer.require_auth();

        let mut already_signed = false;
        for i in 0..proposal.signatures.len() {
            if proposal.signatures.get(i).unwrap() == signer {
                already_signed = true;
                break;
            }
        }

        if !already_signed {
            proposal.signatures.push_back(signer.clone());
            env.events()
                .publish((Symbol::new(&env, "ProposalSigned"), proposal_id), signer);
        }

        if proposal.signatures.len() >= proposal.threshold_m {
            proposal.status = ProposalStatus::Executed;
            env.events().publish(
                (Symbol::new(&env, "ThresholdActionExecuted"), proposal_id),
                proposal.action_data.clone(),
            );
        }
        env.storage()
            .persistent()
            .set(&(agent_id, proposal_id), &proposal);
    }

    pub fn get_threshold_status(env: Env, agent_id: u64, proposal_id: u64) -> ThresholdProposal {
        env.storage()
            .persistent()
            .get(&(agent_id, proposal_id))
            .unwrap()
    }
}
