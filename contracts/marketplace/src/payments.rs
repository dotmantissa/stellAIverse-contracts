use soroban_sdk::{token, Address, Env, String, Symbol, Vec};

use crate::payment_types::{PaymentRecord, PaymentSplit, PaymentStatus, RoyaltyPaymentSplit};
use crate::storage;
use stellai_lib::{
    AssetClass, AssetClassRoyaltySettings, RoyaltyConfig, RoyaltyPaymentRecord, RoyaltyRecipient,
};

/// Maximum royalty rate that can be routed (25% = 2500 bps).
pub const MAX_ROYALTY_CAP_BPS: u32 = 2500;

/// Context required to build a payment split for a single sale.
#[derive(Clone)]
pub struct PaymentRoutingContext {
    pub agent_id: u64,
    pub transaction_id: u64,
    pub buyer: Address,
    pub seller: Address,
    pub platform_address: Address,
    /// Ordered royalty recipients (creator, previous owner, etc.).
    pub royalty_recipients: Vec<(Address, u32, String)>,
    /// Asset class for royalty configuration
    pub asset_class: AssetClass,
}

/// Calculate how a sale price should be distributed between royalty recipients, the platform, and the seller.
pub fn calculate_splits(
    env: &Env,
    sale_price: i128,
    royalty_rate: u32,
    platform_fee: u32,
    context: &PaymentRoutingContext,
) -> RoyaltyPaymentSplit {
    assert!(sale_price > 0, "Sale price must be positive");
    assert!(
        royalty_rate <= MAX_ROYALTY_CAP_BPS,
        "Royalty rate exceeds maximum cap"
    );

    let mut splits = Vec::new(env);
    let mut royalty_allocated: i128 = 0;
    let mut royalty_bps_sum: u32 = 0;

    for i in 0..context.royalty_recipients.len() {
        let entry = context.royalty_recipients.get(i).unwrap();
        royalty_bps_sum = royalty_bps_sum + entry.1;
        let amount = (sale_price * (entry.1 as i128)) / 10000;
        royalty_allocated = royalty_allocated + amount;

        splits.push_back(PaymentSplit {
            recipient: entry.0.clone(),
            amount,
            label: entry.2.clone(),
        });
    }

    assert!(
        royalty_bps_sum <= royalty_rate,
        "Combined royalty recipients exceed configured royalty rate"
    );

    let platform_amount = (sale_price * (platform_fee as i128)) / 10000;
    let total_allocated = royalty_allocated + platform_amount;
    assert!(total_allocated <= sale_price, "Fees exceed the sale price");

    if platform_amount > 0 {
        splits.push_back(PaymentSplit {
            recipient: context.platform_address.clone(),
            amount: platform_amount,
            label: String::from_str(env, "platform"),
        });
    }

    let seller_amount = sale_price - total_allocated;
    splits.push_back(PaymentSplit {
        recipient: context.seller.clone(),
        amount: seller_amount,
        label: String::from_str(env, "seller_net"),
    });

    RoyaltyPaymentSplit {
        agent_id: context.agent_id,
        transaction_id: context.transaction_id,
        sale_price,
        royalty_rate_bps: royalty_rate,
        platform_fee_bps: platform_fee,
        splits,
    }
}

/// Execute routing and persist the payment record.
pub fn execute_payment_routing(env: &Env, split: RoyaltyPaymentSplit) {
    let mut record_splits = Vec::new(env);

    for i in 0..split.splits.len() {
        let entry = split.splits.get(i).unwrap();
        record_splits.push_back((entry.recipient.clone(), entry.amount, entry.label.clone()));
    }

    // Execute actual token transfers
    let payment_token = storage::get_payment_token(env);
    let token_client = token::Client::new(env, &payment_token);

    for i in 0..split.splits.len() {
        let entry = split.splits.get(i).unwrap();
        if entry.amount > 0 {
            token_client.transfer(
                &env.current_contract_address(),
                &entry.recipient,
                &entry.amount,
            );
        }
    }

    let payment_id = storage::increment_payment_counter(env);
    let record = PaymentRecord {
        payment_id,
        transaction_id: split.transaction_id,
        agent_id: split.agent_id,
        total_amount: split.sale_price,
        splits: record_splits.clone(),
        timestamp: env.ledger().timestamp(),
        status: PaymentStatus::Completed,
    };

    storage::set_payment_record(env, &record);
    storage::add_payment_history(env, split.agent_id, payment_id);

    env.events().publish(
        (Symbol::new(env, "PaymentRouted"),),
        (
            record.payment_id,
            record.transaction_id,
            record.agent_id,
            record.splits,
        ),
    );
}

/// Calculate royalties with complex splits and automatic distribution
pub fn calculate_and_distribute_royalties(
    env: &Env,
    agent_id: u64,
    sale_price: i128,
    asset_class: AssetClass,
    transaction_id: u64,
) -> Result<RoyaltyPaymentRecord, &'static str> {
    // Get asset class settings or use defaults
    let settings = storage::get_asset_class_royalty_settings(env, asset_class)
        .unwrap_or_else(|| storage::get_default_asset_class_settings(env, asset_class));

    // Check minimum threshold
    if sale_price < settings.min_threshold {
        return Err("Sale price below minimum royalty threshold");
    }

    // Use agent-specific royalty configuration if exists
    let royalty_config = storage::get_royalty_config(env, agent_id);

    // Determine royalty recipients and rates
    let recipients = if let Some(config) = &royalty_config {
        // Use agent-specific configuration
        config.recipients.clone()
    } else {
        // Use default single recipient (creator from legacy RoyaltyInfo)
        let legacy_royalty = storage::get_royalty(env, agent_id);
        if let Some(info) = legacy_royalty {
            let mut default_recipients = Vec::new(env);
            default_recipients.push_back(RoyaltyRecipient {
                recipient: info.recipient,
                share_bps: info.fee,
                role: String::from_str(env, "creator"),
            });
            default_recipients
        } else {
            // Use asset class default
            return Err("No royalty configuration found");
        }
    };

    // Calculate total royalty amount
    let mut total_royalty_bps: u32 = 0;
    for i in 0..recipients.len() {
        total_royalty_bps += recipients.get(i).unwrap().share_bps;
    }
    let total_royalty = (sale_price * (total_royalty_bps as i128)) / 10_000;

    // Apply maximum cap if configured
    let final_royalty = if let Some(config) = &royalty_config {
        if let Some(cap) = config.max_cap {
            total_royalty.min(cap)
        } else {
            total_royalty
        }
    } else {
        total_royalty
    };

    // Calculate individual shares
    let mut payment_recipients = Vec::new(env);

    for i in 0..recipients.len() {
        let recipient = recipients.get(i).unwrap();
        let share_amount = (final_royalty * (recipient.share_bps as i128)) / (total_royalty_bps as i128);
        payment_recipients.push_back((
            recipient.recipient.clone(),
            share_amount,
            recipient.role.clone(),
        ));
    }

    // Execute token transfers
    let payment_token = storage::get_payment_token(env);
    let token_client = token::Client::new(env, &payment_token);

    for i in 0..payment_recipients.len() {
        let (recipient, amount, _role) = payment_recipients.get(i).unwrap();
        if amount > 0 {
            token_client.transfer(
                &env.current_contract_address(),
                &recipient,
                &amount,
            );
        }
    }

    // Create royalty payment record for audit trail
    let payment_id = storage::increment_royalty_payment_counter(env);
    let royalty_record = RoyaltyPaymentRecord {
        payment_id,
        agent_id,
        transaction_id,
        sale_price,
        total_royalty_paid: final_royalty,
        recipients: payment_recipients.clone(),
        timestamp: env.ledger().timestamp(),
        asset_class,
    };

    // Store record and add to history
    storage::set_royalty_payment_record(env, &royalty_record);
    storage::add_royalty_payment_history(env, agent_id, payment_id);

    // Emit event
    env.events().publish(
        (Symbol::new(env, "RoyaltyPaymentDistributed"),),
        (
            payment_id,
            agent_id,
            final_royalty,
            payment_recipients.len() as u64,
        ),
    );

    Ok(royalty_record)
}

/// Validate royalty configuration before setting it
pub fn validate_royalty_config(
    config: &RoyaltyConfig,
    asset_class: AssetClass,
    env: &Env,
) -> Result<(), &'static str> {
    // Check total basis points doesn't exceed 10000 (100%)
    if config.total_bps > 10000 {
        return Err("Total royalty basis points cannot exceed 10000");
    }

    // Check individual shares sum to total
    let calculated_total: u32 = config.recipients.iter().map(|r| r.share_bps).sum();
    if calculated_total != config.total_bps {
        return Err("Individual royalty shares must sum to total basis points");
    }

    // Validate against asset class limits
    let settings = storage::get_asset_class_royalty_settings(env, asset_class)
        .unwrap_or_else(|| storage::get_default_asset_class_settings(env, asset_class));

    if config.total_bps < settings.min_royalty_bps {
        return Err("Total royalty below minimum for asset class");
    }

    if config.total_bps > settings.max_royalty_bps {
        return Err("Total royalty exceeds maximum for asset class");
    }

    // Check minimum threshold is positive
    if config.min_threshold < 0 {
        return Err("Minimum threshold cannot be negative");
    }

    // Check max cap is positive if set
    if let Some(cap) = config.max_cap {
        if cap < 0 {
            return Err("Maximum cap cannot be negative");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Marketplace;
    use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

    fn build_context(
        env: &Env,
        seller: Address,
        platform_address: Address,
        royalty_bps: Option<u32>,
    ) -> PaymentRoutingContext {
        let mut royalty_recipients = Vec::new(env);
        if let Some(bps) = royalty_bps {
            royalty_recipients.push_back((
                Address::generate(env),
                bps,
                String::from_str(env, "creator"),
            ));
        }

        PaymentRoutingContext {
            agent_id: 1,
            transaction_id: 1,
            buyer: Address::generate(env),
            seller,
            platform_address,
            royalty_recipients,
            asset_class: AssetClass::Agent,
        }
    }

    #[test]
    fn test_calculate_splits_basic() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, Marketplace);
        let seller = Address::generate(&env);

        env.as_contract(&contract_id, || {
            let context = build_context(
                &env,
                seller.clone(),
                env.current_contract_address(),
                Some(500),
            );

            let split = calculate_splits(&env, 10_000, 500, 250, &context);

            assert_eq!(split.agent_id, 1);
            assert_eq!(split.royalty_rate_bps, 500);
            assert_eq!(split.platform_fee_bps, 250);
            assert!(split.splits.len() >= 3);
            let creator_split = split.splits.get(0).unwrap();
            assert_eq!(creator_split.amount, 500);
            let platform_split = split.splits.get(1).unwrap();
            assert_eq!(platform_split.amount, 250);
            let seller_split = split.splits.get(2).unwrap();
            assert_eq!(seller_split.recipient, seller);
            assert_eq!(seller_split.amount, 9250);
        });
    }

    #[test]
    fn test_calculate_splits_zero_royalty() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, Marketplace);
        let seller = Address::generate(&env);

        env.as_contract(&contract_id, || {
            let context = build_context(&env, seller.clone(), env.current_contract_address(), None);

            let split = calculate_splits(&env, 5_000, 0, 100, &context);
            assert_eq!(split.splits.len(), 2);
            let seller_split = split.splits.get(1).unwrap();
            assert_eq!(seller_split.amount, 4_950);
        });
    }

    #[test]
    #[should_panic(expected = "Royalty rate exceeds maximum cap")]
    fn test_calculate_splits_royalty_cap() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, Marketplace);
        let seller = Address::generate(&env);

        env.as_contract(&contract_id, || {
            let context = build_context(&env, seller, env.current_contract_address(), Some(500));

            calculate_splits(&env, 1_000, MAX_ROYALTY_CAP_BPS + 100, 250, &context);
        });
    }

    #[test]
    fn test_payment_record_history() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, Marketplace);
        let seller = Address::generate(&env);

        env.as_contract(&contract_id, || {
            let token_admin = Address::generate(&env);
            let payment_token = env.register_stellar_asset_contract(token_admin.clone());
            let token_admin_client = token::StellarAssetClient::new(&env, &payment_token);
            storage::set_payment_token(&env, payment_token.clone());
            token_admin_client.mint(&env.current_contract_address(), &2_000);

            let context = build_context(
                &env,
                seller.clone(),
                env.current_contract_address(),
                Some(100),
            );
            let split = calculate_splits(&env, 2_000, 100, 50, &context);

            execute_payment_routing(&env, split.clone());

            let history_count = storage::get_payment_history_count(&env, context.agent_id);
            assert_eq!(history_count, 1);

            let payment_id = storage::get_payment_history_entry(&env, context.agent_id, 0).unwrap();
            let record = storage::get_payment_record(&env, payment_id).unwrap();
            assert_eq!(record.total_amount, split.sale_price);
            assert_eq!(record.transaction_id, split.transaction_id);
        });
    }
}
