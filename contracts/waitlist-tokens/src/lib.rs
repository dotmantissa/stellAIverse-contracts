#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol, Vec};
use stellai_lib::{admin, errors::ContractError, ADMIN_KEY};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WaitlistInfo {
    pub user: Address,
    pub joined_at: u64,
    pub priority_score: u32,
    pub access_granted: bool,
}

#[contract]
pub struct WaitlistTokens;

#[contractimpl]
impl WaitlistTokens {
    /// Initialize the contract with an admin address.
    pub fn init_contract(env: Env, admin_addr: Address) -> Result<(), ContractError> {
        if env.storage().instance().has(&Symbol::new(&env, ADMIN_KEY)) {
            return Err(ContractError::AlreadyInitialized);
        }
        admin_addr.require_auth();
        env.storage().instance().set(&Symbol::new(&env, ADMIN_KEY), &admin_addr);
        
        let empty_queue: Vec<Address> = Vec::new(&env);
        env.storage().instance().set(&Symbol::new(&env, "queue"), &empty_queue);
        
        Ok(())
    }

    /// User joins the waitlist.
    pub fn join_waitlist(env: Env, user: Address) -> Result<(), ContractError> {
        user.require_auth();
        
        let key = (Symbol::new(&env, "waitlist"), user.clone());
        if env.storage().instance().has(&key) {
            return Err(ContractError::AlreadyInitialized); // Or a specific AlreadyRegistered error
        }

        let info = WaitlistInfo {
            user: user.clone(),
            joined_at: env.ledger().timestamp(),
            priority_score: 0,
            access_granted: false,
        };

        env.storage().instance().set(&key, &info);

        let mut queue: Vec<Address> = env.storage().instance().get(&Symbol::new(&env, "queue")).unwrap();
        queue.push_back(user.clone());
        env.storage().instance().set(&Symbol::new(&env, "queue"), &queue);

        env.events().publish(
            (Symbol::new(&env, "waitlist"), Symbol::new(&env, "joined")),
            (user, env.ledger().timestamp()),
        );

        Ok(())
    }

    /// Admin grants access to a list of users.
    pub fn grant_access(env: Env, admin_addr: Address, users: Vec<Address>) -> Result<(), ContractError> {
        admin_addr.require_auth();
        admin::verify_admin(&env, &admin_addr)?;

        for user in users.iter() {
            let key = (Symbol::new(&env, "waitlist"), user.clone());
            if let Some(mut info) = env.storage().instance().get::<_, WaitlistInfo>(&key) {
                info.access_granted = true;
                env.storage().instance().set(&key, &info);
                
                env.events().publish(
                    (Symbol::new(&env, "waitlist"), Symbol::new(&env, "access_granted")),
                    user,
                );
            }
        }
        Ok(())
    }

    /// Check if a user has been granted access.
    pub fn has_access(env: Env, user: Address) -> bool {
        let key = (Symbol::new(&env, "waitlist"), user);
        if let Some(info) = env.storage().instance().get::<_, WaitlistInfo>(&key) {
            return info.access_granted;
        }
        false
    }

    /// Get user's position in the queue (1-indexed).
    pub fn get_position(env: Env, user: Address) -> u32 {
        let queue: Vec<Address> = env.storage().instance().get(&Symbol::new(&env, "queue")).unwrap_or_else(|| Vec::new(&env));
        for (i, addr) in queue.iter().enumerate() {
            if addr == user {
                return (i as u32) + 1;
            }
        }
        0
    }

    /// Get total number of users in waitlist.
    pub fn total_waitlisted(env: Env) -> u32 {
        let queue: Vec<Address> = env.storage().instance().get(&Symbol::new(&env, "queue")).unwrap_or_else(|| Vec::new(&env));
        queue.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    
    #[test]
    fn test_waitlist_flow() {
        let env = Env::default();
        env.mock_all_auths();
        
        let admin = Address::generate(&env);
        let user1 = Address::generate(&env);
        let user2 = Address::generate(&env);
        
        let contract_id = env.register_contract(None, WaitlistTokens);
        let client = WaitlistTokensClient::new(&env, &contract_id);
        
        client.init_contract(&admin);
        
        client.join_waitlist(&user1);
        client.join_waitlist(&user2);
        
        assert_eq!(client.total_waitlisted(), 2);
        assert_eq!(client.get_position(&user1), 1);
        assert_eq!(client.get_position(&user2), 2);
        
        assert_eq!(client.has_access(&user1), false);
        
        let mut access_list = Vec::new(&env);
        access_list.push_back(user1.clone());
        client.grant_access(&admin, &access_list);
        
        assert_eq!(client.has_access(&user1), true);
        assert_eq!(client.has_access(&user2), false);
    }
}
