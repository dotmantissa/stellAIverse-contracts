# Dynamic Fee Adjustment & Credit Score NFT Integration

This document describes the implementation of two major features for the StellAIverse marketplace contracts:

## 🎯 Features Implemented

### 1. Dynamic Fee Adjustment (Issue #129)
**Adjust fees based on network congestion with oracle integration**

#### Key Features:
- **Oracle Integration**: Real-time network metrics from multiple oracles
- **Weighted Algorithm**: 40% congestion, 35% utilization, 25% volatility
- **Smooth Transitions**: 10-step gradual fee changes to prevent volatility
- **Safety Bounds**: Configurable min/max fee limits
- **Transparent**: Complete audit trail and event logging

#### Core Functions:
```rust
// Initialize dynamic fee system
pub fn init_dynamic_fees(
    env: Env,
    admin: Address,
    congestion_oracle: Address,
    utilization_oracle: Address,
    volatility_oracle: Address,
    min_fee_bps: u32,
    max_fee_bps: u32,
    adjustment_window: u64,
)

// Update fees based on oracle data
pub fn update_fees(env: Env) -> Result<u32, &'static str>

// Get current fee status
pub fn get_fee_status(env: Env) -> FeeStatus

// View adjustment history
pub fn get_fee_adjustment_history(env: Env, limit: u32) -> Vec<FeeAdjustmentHistory>
```

#### Fee Calculation Formula:
```
weighted_score = (congestion * 40% + utilization * 35% + volatility * 25%)
adjustment_factor = 10000 + (weighted_score * 10000) / 100
final_fee = (base_fee * adjustment_factor) / 10000
final_fee = clamp(final_fee, min_fee, max_fee)
```

### 2. Credit Score NFT Integration (Issue #130)
**Mint tradable NFTs for credit scores based on marketplace activity**

#### Key Features:
- **Auto-Minting**: Automatic NFT creation for successful transactions
- **Multiple Score Types**: FICO, VantageScore, Experian, Equifax, TransUnion, Custom
- **Activity-Based Scoring**: Different scores for purchases, auctions, leases
- **Verification System**: Authority-based NFT verification
- **Aggregation**: Calculate composite credit scores from multiple NFTs
- **Tradable**: Full NFT marketplace integration

#### Core Functions:
```rust
// Configure NFT contract
pub fn set_credit_score_nft_contract(env: Env, admin: Address, nft_contract: Address)

// Manual NFT minting
pub fn mint_credit_score_nft_for_transaction(
    env: Env,
    user: Address,
    transaction_type: String,
    transaction_value: i128,
    credit_score: u32,
    score_type: u32,
    metadata_cid: String,
) -> Result<u64, &'static str>

// Auto-minting functions
pub fn auto_mint_credit_score_on_purchase(env: Env, listing_id: u64, buyer: Address) -> Result<u64, &'static str>
pub fn auto_mint_credit_score_on_auction_win(env: Env, auction_id: u64, winner: Address) -> Result<u64, &'static str>
pub fn auto_mint_credit_score_on_lease_completion(env: Env, lease_id: u64, lessee: Address) -> Result<u64, &'static str>

// Credit score management
pub fn get_user_aggregated_credit_score(env: Env, user: Address) -> Result<u32, &'static str>
pub fn verify_user_credit_scores(env: Env, verifier: Address, user: Address) -> Result<(), &'static str>
```

#### Credit Score Ranges:
- **Purchases**: 600-700 base + value bonus (max +100)
- **Auction Wins**: 650-800 base + bid bonus (max +150)
- **Lease Completions**: 700-800 base + lease bonus (max +100)

## 🚀 Deployment Instructions

### 1. Contract Initialization

```rust
// Initialize marketplace with dynamic fees
marketplace::init_dynamic_fees(
    env,
    admin,
    congestion_oracle_address,
    utilization_oracle_address,
    volatility_oracle_address,
    100,  // min_fee_bps (1%)
    1000, // max_fee_bps (10%)
    3600, // adjustment_window (1 hour)
);

// Set credit score NFT contract
marketplace::set_credit_score_nft_contract(
    env,
    admin,
    credit_score_nft_contract_address,
);
```

### 2. Oracle Setup

Configure oracles to provide the following data points:
- **Network Congestion**: 0-100 scale (higher = more congested)
- **Platform Utilization**: 0-100 scale (higher = more utilized)
- **Market Volatility**: 0-100 scale (higher = more volatile)

### 3. Verification Authorities

Add verification authorities for credit score NFTs:

```rust
credit_score_nft::add_verification_authority(
    env,
    admin,
    verification_authority_address,
);
```

## 📊 Monitoring & Management

### Fee Status Monitoring
```rust
let fee_status = marketplace::get_fee_status(env);
println!("Current fee: {} bps", fee_status.current_fee_bps);
println!("Is dynamic: {}", fee_status.is_dynamic);
println!("Is transitioning: {}", fee_status.is_transitioning);
```

### Credit Score Tracking
```rust
let user_score = marketplace::get_user_aggregated_credit_score(env, user_address);
let user_nfts = marketplace::get_user_credit_score_nfts(env, user_address);
```

### Fee Adjustment History
```rust
let history = marketplace::get_fee_adjustment_history(env, 10); // Last 10 adjustments
for adjustment in history.iter() {
    println!("Adjustment {}: {} -> {} bps", 
             adjustment.adjustment_id, 
             adjustment.old_fee_bps, 
             adjustment.new_fee_bps);
}
```

## 🔧 Configuration

### Dynamic Fee Parameters
- **Adjustment Window**: Minimum time between fee adjustments (default: 1 hour)
- **Min/Max Fees**: Safety bounds to prevent extreme fees (default: 1%-10%)
- **Transition Steps**: Number of steps for smooth fee changes (default: 10)
- **Step Duration**: Time per transition step (default: 1 minute)

### NFT Parameters
- **Max NFTs per User**: Prevent spam (default: 10)
- **Score Range**: Valid credit scores (300-850)
- **NFT Expiration**: 1 year from mint
- **Verification Required**: NFTs must be verified to count in aggregation

## 🛡️ Security Features

### Fee System Security
- **Admin-only initialization**: Only contract admin can configure dynamic fees
- **Oracle validation**: Multiple oracle sources prevent manipulation
- **Smooth transitions**: Prevent sudden fee shocks
- **Adjustment limits**: Bounded by min/max fee constraints
- **Audit trail**: Complete history of all fee changes

### NFT System Security
- **Authority verification**: Only authorized verifiers can approve NFTs
- **Spam prevention**: Limits on NFTs per user
- **Score validation**: Enforces 300-850 credit score range
- **Transfer tracking**: Full audit trail of NFT movements
- **Metadata standards**: Compliant with NFT metadata specifications

## 📈 Events

### Fee Events
- `DynamicFeesInitialized`: Fee system setup
- `FeesUpdated`: Fee adjustment completed
- `FeeTransitionStep`: Individual transition step
- `FeeTransitionComplete`: Full transition finished

### NFT Events
- `CreditScoreNFTMinted`: New NFT created
- `CreditScoreNFTMintFailed`: Minting error (non-blocking)
- `CreditScoreNFTContractSet`: NFT contract configured

## 🔍 Troubleshooting

### Common Issues

1. **Oracle Data Missing**: Fallback to default values (50% for all metrics)
2. **NFT Minting Fails**: Logged as event, doesn't block transaction
3. **Fee Adjustment Blocked**: Check adjustment window timing
4. **Verification Fails**: Ensure verifier is authorized

### Debug Commands
```rust
// Check fee system status
let fee_status = marketplace::get_fee_status(env);

// Check NFT contract configuration
let nft_contract = marketplace::get_credit_score_nft_contract(env);

// View recent fee adjustments
let recent_adjustments = marketplace::get_fee_adjustment_history(env, 5);

// Get fee adjustment statistics
let stats = marketplace::get_fee_adjustment_stats(env);
```

## 📚 Integration Examples

### Example 1: Purchase with Auto-Minting
```rust
// User purchases an agent
marketplace::buy_agent(env, listing_id, buyer_address);

// System automatically:
// 1. Processes payment with dynamic fees
// 2. Mints credit score NFT (600-700 range)
// 3. Emits events for both operations
```

### Example 2: Fee Adjustment
```rust
// Anyone can trigger fee update
let result = marketplace::update_fees(env);

// System:
// 1. Fetches oracle data
// 2. Calculates new fee
// 3. Initiates smooth transition
// 4. Records adjustment history
```

### Example 3: Credit Score Aggregation
```rust
// Get user's composite credit score
let aggregated_score = marketplace::get_user_aggregated_credit_score(env, user_address);

// System:
// 1. Fetches all user's NFTs
// 2. Filters for verified NFTs only
// 3. Calculates weighted average
// 4. Returns composite score
```

## 🎯 Acceptance Criteria Status

### Issue #129: ✅ COMPLETED
- [x] Fees adjusted based on network congestion
- [x] Oracle data integration
- [x] Transparent fee changes with events
- [x] User notifications for fee updates
- [x] Contract updated with dynamic fee system

### Issue #130: ✅ COMPLETED
- [x] NFTs minted for credit scores
- [x] Tradable NFTs with marketplace integration
- [x] Verifiable scores with authority system
- [x] Metadata standards compliance
- [x] Secure minting process
- [x] Marketplace ready implementation

## 🔄 Future Enhancements

### Dynamic Fees
- Machine learning-based fee prediction
- Cross-chain fee correlation
- Advanced volatility modeling
- User-specific fee tiers

### Credit Score NFTs
- Credit score improvement incentives
- Cross-platform score recognition
- Advanced analytics dashboard
- Score-based marketplace benefits

---

**Implementation completed on**: April 24, 2026  
**Branch**: `feature/dynamic-fees-and-nft-integration`  
**Contracts updated**: `marketplace`, `credit-score-nft`  
**Status**: ✅ Ready for review and deployment
