extern crate alloc;
use alloc::string::ToString;
use soroban_sdk::{contracttype, Env, String, Symbol, Val, Vec, TryIntoVal, ConversionError, Address, format};

use stellai_lib::{
    atomic::AtomicTransactionSupport,
    audit::{create_audit_log, OperationType},
    types::{TransactionStatus, TransactionStep},
};

#[derive(Clone, Debug)]
#[contracttype]
pub struct AtomicStepState {
    pub transaction_id: u64,
    pub step_id: u32,
    pub contract: Address,
    pub rollback_contract: Option<Address>,
    pub prepared: bool,
    pub executed: bool,
    pub result: Option<String>,
    pub prepared_at: u64,
    pub executed_at: Option<u64>,
    pub rolled_back: bool,
    pub rolled_back_at: Option<u64>,
}

#[derive(Clone, Debug)]
#[contracttype]
pub struct AtomicTransactionState {
    pub transaction_id: u64,
    pub status: TransactionStatus,
    pub created_at: u64,
    pub updated_at: u64,
    pub prepared_steps: Vec<u32>,
    pub executed_steps: Vec<u32>,
    pub rolled_back_steps: Vec<u32>,
    pub failure_reason: Option<String>,
}

pub struct MarketplaceAtomicSupport;

impl MarketplaceAtomicSupport {
    /// Get next transaction ID
    pub fn get_next_transaction_id(env: &Env) -> u64 {
        let tx_counter_key = Symbol::new(env, "atomic_tx_counter");
        let current_id: u64 = env.storage().instance().get(&tx_counter_key).unwrap_or(0);
        let next_id = current_id + 1;
        env.storage().instance().set(&tx_counter_key, &next_id);
        next_id
    }

    /// Initialize atomic transaction support in the contract
    pub fn initialize(env: &Env) {
        // Set up any required storage for atomic transactions
        let tx_counter_key = Symbol::new(env, "atomic_tx_counter");
        if !env.storage().instance().has(&tx_counter_key) {
            env.storage().instance().set(&tx_counter_key, &0u64);
        }
        
        // Create audit log for initialization
        let before_state = String::from_str(env, "{}");
        let after_state = String::from_str(env, "{\"atomic_support_initialized\":true}");
        let tx_hash = String::from_str(env, "initialize_atomic");
        let description = Some(String::from_str(env, "Atomic transaction support initialized"));
        
        let _ = create_audit_log(
            env,
            env.current_contract_address(),
            OperationType::ConfigurationChange,
            before_state,
            after_state,
            tx_hash,
            description,
        );
    }

    /// Check if all dependencies for a step are satisfied
    fn check_dependencies(env: &Env, transaction_id: u64, step: &TransactionStep) -> bool {
        if let Some(depends_on) = step.depends_on {
            let dep_key = (Symbol::new(env, "atomic_step"), transaction_id, depends_on);
            if let Some(dep_state) = env.storage().instance().get::<_, AtomicStepState>(&dep_key) {
                return dep_state.prepared && dep_state.executed;
            }
            return false;
        }
        true
    }

    /// Update transaction state
    fn update_transaction_state(env: &Env, transaction_id: u64, status: TransactionStatus, failure_reason: Option<&str>) {
        let tx_key = (Symbol::new(env, "atomic_tx"), transaction_id);
        let mut tx_state = if let Some(state) = env.storage().instance().get::<_, AtomicTransactionState>(&tx_key) {
            state
        } else {
            AtomicTransactionState {
                transaction_id,
                status: TransactionStatus::Initiated,
                created_at: env.ledger().timestamp(),
                updated_at: env.ledger().timestamp(),
                prepared_steps: Vec::new(env),
                executed_steps: Vec::new(env),
                rolled_back_steps: Vec::new(env),
                failure_reason: None,
            }
        };

        tx_state.status = status;
        tx_state.updated_at = env.ledger().timestamp();
        tx_state.failure_reason = failure_reason.map(|s| String::from_str(env, s));
        
        env.storage().instance().set(&tx_key, &tx_state);
    }

    /// Add step to prepared steps list
    fn add_prepared_step(env: &Env, transaction_id: u64, step_id: u32) {
        let tx_key = (Symbol::new(env, "atomic_tx"), transaction_id);
        if let Some(mut tx_state) = env.storage().instance().get::<_, AtomicTransactionState>(&tx_key) {
            if !tx_state.prepared_steps.contains(&step_id) {
                tx_state.prepared_steps.push_back(step_id);
                tx_state.updated_at = env.ledger().timestamp();
                env.storage().instance().set(&tx_key, &tx_state);
            }
        }
    }

    /// Add step to executed steps list
    fn add_executed_step(env: &Env, transaction_id: u64, step_id: u32) {
        let tx_key = (Symbol::new(env, "atomic_tx"), transaction_id);
        if let Some(mut tx_state) = env.storage().instance().get::<_, AtomicTransactionState>(&tx_key) {
            if !tx_state.executed_steps.contains(&step_id) {
                tx_state.executed_steps.push_back(step_id);
                tx_state.updated_at = env.ledger().timestamp();
                env.storage().instance().set(&tx_key, &tx_state);
            }
        }
    }

    /// Add step to rolled back steps list
    fn add_rolled_back_step(env: &Env, transaction_id: u64, step_id: u32) {
        let tx_key = (Symbol::new(env, "atomic_tx"), transaction_id);
        if let Some(mut tx_state) = env.storage().instance().get::<_, AtomicTransactionState>(&tx_key) {
            if !tx_state.rolled_back_steps.contains(&step_id) {
                tx_state.rolled_back_steps.push_back(step_id);
                tx_state.updated_at = env.ledger().timestamp();
                env.storage().instance().set(&tx_key, &tx_state);
            }
        }
    }

    /// Create audit log for atomic transaction events
    fn create_atomic_audit_log(
        env: &Env,
        _transaction_id: u64,
        _step_id: Option<u32>,
        _action: &str,
        _success: bool,
        details: Option<&str>,
    ) {
        let before_state = String::from_str(env, "{}");
        // Create simplified state string for audit log
        let after_state = String::from_str(env, "{\"atomic_transaction\":true}");
        let tx_hash = String::from_str(env, "atomic_transaction");
        let description = details.map(|s| String::from_str(env, s));

        let _ = create_audit_log(
            env,
            env.current_contract_address(),
            OperationType::SaleCompleted,
            before_state,
            after_state,
            tx_hash,
            description,
        );
    }
}

impl MarketplaceAtomicSupport {
    /// Execute full atomic transaction workflow
    pub fn execute_atomic_transaction(env: &Env, transaction_id: u64, steps: &Vec<TransactionStep>) -> bool {
        // First initialize the transaction state
        let tx_key = (Symbol::new(env, "atomic_tx"), transaction_id);
        if !env.storage().instance().has(&tx_key) {
            let initial_state = AtomicTransactionState {
                transaction_id,
                status: TransactionStatus::Initiated,
                created_at: env.ledger().timestamp(),
                updated_at: env.ledger().timestamp(),
                prepared_steps: Vec::new(env),
                executed_steps: Vec::new(env),
                rolled_back_steps: Vec::new(env),
                failure_reason: None,
            };
            env.storage().instance().set(&tx_key, &initial_state);
        }
        
        // Update transaction status to Preparing
        Self::update_transaction_state(env, transaction_id, TransactionStatus::Preparing, None);
        Self::create_atomic_audit_log(env, transaction_id, None, "transaction_started", true, Some("Starting atomic transaction execution"));
        
        let mut executed_steps: Vec<u32> = Vec::new(env);
        
        // First prepare all steps in order
        for step in steps.iter() {
            if !Self::prepare_step(env, transaction_id, step.step_id, &step.function, &step.args, &step) {
                // Preparation failed, trigger rollback for all executed steps
                Self::rollback_transaction(env, transaction_id, steps, executed_steps, "Step preparation failed");
                return false;
            }
        }
        
        // Now commit all steps
        for step in steps.iter() {
            let result = Self::commit_step(env, transaction_id, step.step_id, &step.function, &step.args);
            let success: bool = result.try_into_val(env).unwrap_or(false);
            
            if !success {
                // Commit failed, trigger rollback
                Self::rollback_transaction(env, transaction_id, steps, executed_steps, &format!("Step {} commit failed", step.step_id));
                return false;
            }
            
            executed_steps.push_back(step.step_id);
        }
        
        // All steps completed successfully
        Self::update_transaction_state(env, transaction_id, TransactionStatus::Committed, None);
        Self::create_atomic_audit_log(env, transaction_id, None, "transaction_committed", true, Some("All steps completed successfully"));
        
        env.events().publish(
            (Symbol::new(env, "atomic_tx_completed"),),
            (transaction_id, env.ledger().timestamp())
        );
        
        true
    }
    
    /// Rollback entire transaction if any step fails
    fn rollback_transaction(env: &Env, transaction_id: u64, steps: &Vec<TransactionStep>, executed_steps: Vec<u32>, reason: &str) {
        Self::create_atomic_audit_log(env, transaction_id, None, "transaction_rolling_back", false, Some(reason));
        
        // Rollback steps in reverse order
        for step_id in executed_steps.iter().rev() {
            // Get the step state to access rollback info
            let step_key = (Symbol::new(env, "atomic_step"), transaction_id, step_id);
            if let Some(step_state) = env.storage().instance().get::<_, AtomicStepState>(&step_key) {
                // Find the original step to get rollback function and args
                if let Some(step) = steps.iter().find(|s| s.step_id == step_id) {
                    if let (Some(rollback_function), Some(rollback_args)) = 
                       (&step.rollback_function, &step.rollback_args) {
                        
                        // Execute rollback
                        let _ = Self::rollback_step(env, transaction_id, step_id, rollback_function, rollback_args);
                    }
                }
            }
        }
        
        // Mark transaction as rolled back
        Self::update_transaction_state(env, transaction_id, TransactionStatus::RolledBack, Some(reason));
        Self::create_atomic_audit_log(env, transaction_id, None, "transaction_rolled_back", true, Some(&format!("Rollback complete: {}", reason)));
        
        env.events().publish(
            (Symbol::new(env, "atomic_tx_rolled_back"),),
            (transaction_id, env.ledger().timestamp(), String::from_str(env, reason))
        );
    }
}

impl AtomicTransactionSupport for MarketplaceAtomicSupport {
    fn prepare_step(
        env: &Env,
        transaction_id: u64,
        step_id: u32,
        function: &Symbol,
        args: &Vec<Val>,
        step: &TransactionStep, // Added step parameter to check dependencies
    ) -> bool {
        // First check if all dependencies are satisfied
        if !Self::check_dependencies(env, transaction_id, step) {
            Self::create_atomic_audit_log(env, transaction_id, Some(step_id), "prepare_failed", false, Some("Dependencies not satisfied"));
            return false;
        }
        
        let step_key = (Symbol::new(env, "atomic_step"), transaction_id, step_id);
        
        // Check if step already exists
        if env.storage().instance().has(&step_key) {
            Self::create_atomic_audit_log(env, transaction_id, Some(step_id), "prepare_duplicate", false, Some("Step already prepared"));
            return false;
        }

        // Validate that we can actually call this function (dry run preparation check)
        // Validate that we can actually call this function (dry run preparation check)
        if let Err(_) = env.try_invoke_contract::<Val, ConversionError>(&step.contract, function, args.clone()) {
            let error_msg = String::from_str(env, "Preparation validation failed");
            Self::create_atomic_audit_log(env, transaction_id, Some(step_id), "prepare_failed", false, Some(error_msg.to_string().as_str()));
            return false;
        }

        // Create step state
        let state = AtomicStepState {
            transaction_id,
            step_id,
            contract: step.contract.clone(),
            rollback_contract: step.rollback_contract.clone(),
            prepared: true,
            executed: false,
            result: Some(String::from_str(env, "Step prepared successfully")),
            prepared_at: env.ledger().timestamp(),
            executed_at: None,
            rolled_back: false,
            rolled_back_at: None,
        };

        env.storage().instance().set(&step_key, &state);
        Self::add_prepared_step(env, transaction_id, step_id);
        Self::update_transaction_state(env, transaction_id, TransactionStatus::Preparing, None);
        
        // Log successful preparation
        let func_str = function.to_string();
        let log_msg = String::from_str(env, &format!("Function: {} prepared successfully", func_str));
        Self::create_atomic_audit_log(
            env, 
            transaction_id, 
            Some(step_id), 
            "step_prepared", 
            true, 
            Some(log_msg.to_string().as_str())
        );
        
        true
    }

    fn commit_step(
        env: &Env,
        transaction_id: u64,
        step_id: u32,
        function: &Symbol,
        args: &Vec<Val>,
    ) -> Val {
        let step_key = (Symbol::new(env, "atomic_step"), transaction_id, step_id);
        
        // Verify step is prepared
        if let Some(mut state) = env.storage().instance().get::<_, AtomicStepState>(&step_key) {
            if !state.prepared {
                Self::create_atomic_audit_log(env, transaction_id, Some(step_id), "commit_failed", false, Some("Step not prepared"));
                return false.into();
            }

            if state.executed {
                Self::create_atomic_audit_log(env, transaction_id, Some(step_id), "commit_duplicate", false, Some("Step already executed"));
                return true.into();
            }

            // Actually execute the step function using the stored contract address
            let execution_result = env.try_invoke_contract::<Val, ConversionError>(&state.contract, function, args.clone());
            let val = match execution_result {
                Ok(val) => val,
                Err(e) => {
                    let error_msg = String::from_str(env, "Execution failed");
                    state.result = Some(error_msg.clone());
                    env.storage().instance().set(&step_key, &state);
                    Self::create_atomic_audit_log(env, transaction_id, Some(step_id), "commit_failed", false, Some(error_msg.to_string().as_str()));
                    return false.into();
                }
            };

            // Mark as executed
            state.executed = true;
            state.executed_at = Some(env.ledger().timestamp());
            state.result = Some(String::from_str(env, "Step executed successfully"));
            env.storage().instance().set(&step_key, &state);
            
            Self::add_executed_step(env, transaction_id, step_id);
            Self::update_transaction_state(env, transaction_id, TransactionStatus::Committing, None);
            
            // Log successful commit
            let func_str = function.to_string();
            let log_msg = String::from_str(env, &format!("Function: {} executed successfully", func_str));
            Self::create_atomic_audit_log(
                env,
                transaction_id,
                Some(step_id),
                "step_committed",
                true,
                Some(log_msg.to_string().as_str())
            );
            
            val
        } else {
            Self::create_atomic_audit_log(env, transaction_id, Some(step_id), "commit_failed", false, Some("Step not found"));
            false.into()
        }
    }

    fn is_step_prepared(env: &Env, transaction_id: u64, step_id: u32) -> bool {
        let key = (Symbol::new(env, "atomic_step"), transaction_id, step_id);
        env.storage()
            .instance()
            .get::<_, AtomicStepState>(&key)
            .map(|state| state.prepared)
            .unwrap_or(false)
    }

    fn get_step_result(env: &Env, transaction_id: u64, step_id: u32) -> Option<Val> {
        let key = (Symbol::new(env, "atomic_step"), transaction_id, step_id);
        env.storage()
            .instance()
            .get::<_, AtomicStepState>(&key)
            .and_then(|state| if state.executed { Some(true.into()) } else { None })
    }

    fn rollback_step(
        env: &Env,
        transaction_id: u64,
        step_id: u32,
        rollback_function: &Symbol,
        rollback_args: &Vec<Val>,
    ) -> bool {
        let step_key = (Symbol::new(env, "atomic_step"), transaction_id, step_id);
        
        if let Some(mut state) = env.storage().instance().get::<_, AtomicStepState>(&step_key) {
            // Use the stored rollback contract if available, otherwise fall back to the main contract
            let rollback_contract = state.rollback_contract.unwrap_or(state.contract.clone());
            
            // Actually execute the rollback function
            let rollback_result = env.try_invoke_contract::<Val, ConversionError>(&rollback_contract, rollback_function, rollback_args.clone())
                .map_or_else(|e| {
                    let error_msg = format!("Rollback execution warning: {:?}", e);
                    Self::create_atomic_audit_log(env, transaction_id, Some(step_id), "rollback_warning", false, Some(&error_msg));
                    false
                }, |_| true);

            // Mark as rolled back regardless (we still want to track it)
            state.rolled_back = true;
            state.rolled_back_at = Some(env.ledger().timestamp());
            env.storage().instance().set(&step_key, &state);
            
            Self::add_rolled_back_step(env, transaction_id, step_id);
            Self::update_transaction_state(env, transaction_id, TransactionStatus::RollingBack, None);
            
            // Log rollback
            Self::create_atomic_audit_log(
                env,
                transaction_id,
                Some(step_id),
                "step_rolled_back",
                rollback_result,
                Some(&format!("Rollback function: {} executed", rollback_function.to_string()))
            );
            
            true
        } else {
            Self::create_atomic_audit_log(env, transaction_id, Some(step_id), "rollback_failed", false, Some("Step not found"));
            false
        }
    }
}

/// Add extension to MarketplaceContract to support atomic transactions
use crate::MarketplaceContract;
use soroban_sdk::contractimpl;

#[contractimpl]
impl MarketplaceContract {
    /// Initialize atomic transaction support
    pub fn initialize_atomic_support(env: Env, admin: Address) {
        admin.require_auth();
        // Verify admin permissions first
        use crate::storage::is_admin;
        assert!(is_admin(&env, &admin), "Unauthorized: must be admin");
        
        MarketplaceAtomicSupport::initialize(&env);
        
        env.events().publish(
            (Symbol::new(&env, "atomic_support_initialized"),),
            (admin, env.ledger().timestamp())
        );
    }

    /// Get atomic transaction state
    pub fn get_atomic_transaction(env: Env, transaction_id: u64) -> Option<AtomicTransactionState> {
        let tx_key = (Symbol::new(&env, "atomic_tx"), transaction_id);
        env.storage().instance().get(&tx_key)
    }

    /// Get atomic step state
    pub fn get_atomic_step(env: Env, transaction_id: u64, step_id: u32) -> Option<AtomicStepState> {
        let step_key = (Symbol::new(&env, "atomic_step"), transaction_id, step_id);
        env.storage().instance().get(&step_key)
    }

    /// Execute an atomic transaction with full prepare/commit/rollback workflow
    pub fn execute_atomic_transaction(env: Env, caller: Address, transaction_id: u64, steps: Vec<TransactionStep>) -> bool {
        caller.require_auth();
        
        // Use the internal implementation
        MarketplaceAtomicSupport::execute_atomic_transaction(&env, transaction_id, &steps)
    }

    /// Manually trigger rollback for a transaction (emergency use only)
    pub fn rollback_atomic_transaction(env: Env, admin: Address, transaction_id: u64, steps: Vec<TransactionStep>, reason: String) -> bool {
        admin.require_auth();
        // Verify admin permissions
        use crate::storage::is_admin;
        assert!(is_admin(&env, &admin), "Unauthorized: must be admin");
        
        // Get all executed steps to rollback
        let tx_key = (Symbol::new(&env, "atomic_tx"), transaction_id);
        if let Some(tx_state) = env.storage().instance().get::<_, AtomicTransactionState>(&tx_key) {
            MarketplaceAtomicSupport::rollback_transaction(&env, transaction_id, &steps, tx_state.executed_steps, reason.to_string().as_str());
            true
        } else {
            false
        }
    }

    /// Get next available atomic transaction ID
    pub fn get_next_atomic_transaction_id(env: Env) -> u64 {
        MarketplaceAtomicSupport::get_next_transaction_id(&env)
    }
}