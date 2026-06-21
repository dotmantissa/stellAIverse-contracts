#[cfg(test)]
mod tests {
    use crate::{storage, Marketplace};
    use soroban_sdk::{testutils::{Address as _, Events as _}, Address, Env, String};

    #[test]
    fn test_dynamic_fee_adjustment_enhancement() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let oracle_id = Address::generate(&env);
        let token_address = env.register_stellar_asset_contract(admin.clone());

        // Initialize marketplace with dynamic fee adjustment
        Marketplace::initialize(env.clone(), admin.clone(), token_address, 250);

        // Initialize fee adjustment with enhanced parameters
        Marketplace::init_fee_adjustment(
            env.clone(),
            admin.clone(),
            250, // 2.5% base fee
            oracle_id.clone(),
            oracle_id.clone(), // Use same oracle for simplicity in test
            oracle_id.clone(),
            50,   // min 0.5%
            1000, // max 10%
            300,  // 5 minute adjustment window
        );

        // Test fee status monitoring
        let fee_status = Marketplace::get_fee_status(env.clone());
        assert_eq!(fee_status.current_fee_bps, 250);
        assert!(!fee_status.is_transitioning);
        assert_eq!(fee_status.transition_progress, Some(100));

        // Test network metrics transparency
        let metrics = Marketplace::get_network_metrics(env.clone());
        assert!(metrics.network_congestion >= 0 && metrics.network_congestion <= 100);
        assert!(metrics.platform_utilization >= 0 && metrics.platform_utilization <= 100);
        assert!(metrics.market_volatility >= 0 && metrics.market_volatility <= 100);

        // Test fee adjustment statistics
        let stats = Marketplace::get_fee_adjustment_stats(env.clone());
        assert_eq!(stats.total_adjustments, 0);
        assert_eq!(stats.current_fee_bps, 250);
        assert!(!stats.is_transitioning);

        // Test user notification system
        let user = Address::generate(&env);
        Marketplace::notify_fee_change(env.clone(), user.clone());

        // Test monitoring and automatic adjustment
        Marketplace::monitor_and_adjust_fees(env.clone());

        // Verify events were published
        let events = env.events().all();
        assert!(events.len() > 0);

        // Test oracle integration with fallback
        let congestion_value = Marketplace::get_oracle_value_by_key(
            &env,
            &oracle_id,
            "network_congestion",
            75, // fallback value
        );
        assert_eq!(congestion_value, 75); // Should use fallback in test
    }

    #[test]
    fn test_fee_calculation_factors() {
        let env = Env::default();

        // Test congestion factor calculation
        let congestion_low = Marketplace::calculate_congestion_factor(10);
        assert_eq!(congestion_low, 650); // 0.5x + (10 * 15) = 650

        let congestion_high = Marketplace::calculate_congestion_factor(90);
        assert_eq!(congestion_high, 1850); // 0.5x + (90 * 15) = 1850

        // Test utilization factor calculation
        let utilization_low = Marketplace::calculate_utilization_factor(20);
        assert_eq!(utilization_low, 860); // 0.7x + (20 * 8) = 860

        let utilization_high = Marketplace::calculate_utilization_factor(80);
        assert_eq!(utilization_high, 1340); // 0.7x + (80 * 8) = 1340

        // Test volatility factor calculation
        let volatility_low = Marketplace::calculate_volatility_factor(15);
        assert_eq!(volatility_low, 960); // 0.9x + (15 * 4) = 960

        let volatility_high = Marketplace::calculate_volatility_factor(70);
        assert_eq!(volatility_high, 1180); // 0.9x + (70 * 4) = 1180
    }

    #[test]
    fn test_fee_transition_system() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let token_address = env.register_stellar_asset_contract(admin.clone());

        Marketplace::initialize(env.clone(), admin.clone(), token_address, 250);

        // Test transition state
        let transition_state = storage::FeeTransitionState {
            is_transitioning: true,
            start_fee_bps: 250,
            target_fee_bps: 350,
            transition_start: env.ledger().timestamp(),
            transition_steps: 10,
            current_step: 5,
        };

        storage::set_fee_transition_state(&env, &transition_state);

        // Test transition fee calculation
        let transition_fee = Marketplace::calculate_transition_fee(&env, &transition_state);
        assert!(transition_fee > 250 && transition_fee < 350);

        // Test processing transition step
        Marketplace::process_fee_transition(env.clone());

        let updated_state = storage::get_fee_transition_state(&env).unwrap();
        assert_eq!(updated_state.current_step, 6);
    }

    #[test]
    fn test_oracle_data_validation() {
        let env = Env::default();
        let oracle_id = Address::generate(&env);

        // Test with valid range
        let valid_value =
            Marketplace::get_oracle_value_by_key(&env, &oracle_id, "network_congestion", 50);
        assert_eq!(valid_value, 50);

        // Test oracle data age validation
        // In test mode, this will use fallback values
        let stale_value =
            Marketplace::get_oracle_value_by_key(&env, &oracle_id, "platform_utilization", 25);
        assert_eq!(stale_value, 25);
    }

    #[test]
    fn test_fee_adjustment_history() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let token_address = env.register_stellar_asset_contract(admin.clone());

        Marketplace::initialize(env.clone(), admin.clone(), token_address, 250);

        // Create a fee adjustment history entry
        let history = storage::FeeAdjustmentHistory {
            adjustment_id: 1,
            timestamp: env.ledger().timestamp(),
            old_fee_bps: 250,
            new_fee_bps: 300,
            congestion_value: 1200,
            utilization_value: 1100,
            volatility_value: 1050,
            adjustment_reason: String::from_str(&env, "network_congestion_high"),
        };

        storage::add_fee_adjustment_history(&env, &history);

        // Test retrieval
        let retrieved = storage::get_fee_adjustment_history(&env, 1).unwrap();
        assert_eq!(retrieved.adjustment_id, 1);
        assert_eq!(retrieved.old_fee_bps, 250);
        assert_eq!(retrieved.new_fee_bps, 300);
    }
}
