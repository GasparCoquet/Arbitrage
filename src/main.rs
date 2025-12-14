use anyhow::Result;
use chrono::Utc;
use ethers::prelude::*;
use ethers::types::{Address, U256};
use std::env;
use std::fs::{read_to_string, write, OpenOptions};
use std::io::Write;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

// Minimal ABI for UniswapV2-style router
abigen!(
    JoeRouter,
    r#"[function getAmountsOut(uint amountIn, address[] calldata path) external view returns (uint[] memory amounts)]"#,
);

abigen!(
    PangolinRouter,
    r#"[function getAmountsOut(uint amountIn, address[] calldata path) external view returns (uint[] memory amounts)]"#,
);

// UniswapV2 Pair ABI for getting reserves
abigen!(
    UniswapV2Pair,
    r#"[
        {
            "type": "function",
            "name": "getReserves",
            "inputs": [],
            "outputs": [
                {"name": "reserve0", "type": "uint112"},
                {"name": "reserve1", "type": "uint112"},
                {"name": "blockTimestampLast", "type": "uint32"}
            ],
            "stateMutability": "view"
        },
        {
            "type": "function",
            "name": "token0",
            "inputs": [],
            "outputs": [{"name": "", "type": "address"}],
            "stateMutability": "view"
        },
        {
            "type": "function",
            "name": "token1",
            "inputs": [],
            "outputs": [{"name": "", "type": "address"}],
            "stateMutability": "view"
        }
    ]"#,
);

/// Calculate gas cost in AVAX for a given gas limit
/// Formula: gas_cost_avax = gas_limit * gas_price
/// Where gas_price is fetched dynamically from the blockchain
async fn calculate_gas_cost(provider: &Arc<Provider<Http>>, gas_limit: u64) -> Result<f64> {
    // Fetch current gas price (in Wei)
    let gas_price = provider.get_gas_price().await?;

    // Calculate total cost: gas_limit * gas_price (in Wei)
    let total_cost_wei = gas_price * U256::from(gas_limit);

    // Convert from Wei to AVAX (1 AVAX = 10^18 Wei)
    let total_cost_avax = total_cost_wei.as_u128() as f64 / 1e18;

    Ok(total_cost_avax)
}

/// Get current AVAX price in USDC.e by querying the DEX
async fn get_avax_price_in_usdc(
    _provider: &Arc<Provider<Http>>,
    router: &JoeRouter<Provider<Http>>,
    usdc: Address,
    wavax: Address,
) -> Result<f64> {
    // Get price by swapping 1 WAVAX to USDC.e
    let one_wavax = U256::from(1_000_000_000_000_000_000u64); // 1 WAVAX (18 decimals)

    let amounts = router
        .get_amounts_out(one_wavax, vec![wavax, usdc])
        .call()
        .await?;

    if amounts.len() >= 2 {
        // amounts[1] is USDC.e received for 1 WAVAX
        let usdc_received = amounts[1].as_u128() as f64 / 1e6; // USDC.e has 6 decimals
        Ok(usdc_received)
    } else {
        anyhow::bail!("Invalid amounts returned from router")
    }
}

/// Calculate slippage percentage for a trade
/// Slippage = (spot_price - execution_price) / spot_price * 100
/// Where:
///   - spot_price = price for 1 unit (theoretical, linear extrapolation)
///   - execution_price = actual price for our trade size
///
/// We calculate by comparing:
///   - What we'd get if price was linear (spot_price * amount)
///   - What we actually get (execution_price * amount)
async fn calculate_slippage_joe(
    router: &JoeRouter<Provider<Http>>,
    amount_in: U256,
    exec_out: f64,
    token_in: Address,
    token_out: Address,
    decimals_in: u32,
    decimals_out: u32,
) -> Result<f64> {
    // Get spot price: swap 1 USDC.e (small amount for spot price)
    let one_usdc = U256::from(1_000_000u64); // 1 USDC.e (6 decimals)
    let spot_amounts = router
        .get_amounts_out(one_usdc, vec![token_in, token_out])
        .call()
        .await?;

    if spot_amounts.len() < 2 {
        anyhow::bail!("Invalid spot price amounts");
    }

    // Calculate spot price per USDC.e
    let spot_out_per_usdc = spot_amounts[1].as_u128() as f64 / 10f64.powi(decimals_out as i32);

    // Calculate what we'd get at spot price (linear)
    let amount_in_usdc = amount_in.as_u128() as f64 / 10f64.powi(decimals_in as i32);
    let expected_out = spot_out_per_usdc * amount_in_usdc;

    // Slippage = (expected - actual) / expected * 100
    // Positive = you get less than expected (normal for large trades)
    let slippage_pct = ((expected_out - exec_out) / expected_out) * 100.0;

    Ok(slippage_pct)
}

async fn calculate_slippage_pangolin(
    router: &PangolinRouter<Provider<Http>>,
    amount_in: U256,
    exec_out: f64,
    token_in: Address,
    token_out: Address,
    decimals_in: u32,
    decimals_out: u32,
) -> Result<f64> {
    // Get spot price: swap 1 USDC.e (small amount for spot price)
    let one_usdc = U256::from(1_000_000u64); // 1 USDC.e (6 decimals)
    let spot_amounts = router
        .get_amounts_out(one_usdc, vec![token_in, token_out])
        .call()
        .await?;

    if spot_amounts.len() < 2 {
        anyhow::bail!("Invalid spot price amounts");
    }

    // Calculate spot price per USDC.e
    let spot_out_per_usdc = spot_amounts[1].as_u128() as f64 / 10f64.powi(decimals_out as i32);

    // Calculate what we'd get at spot price (linear)
    let amount_in_usdc = amount_in.as_u128() as f64 / 10f64.powi(decimals_in as i32);
    let expected_out = spot_out_per_usdc * amount_in_usdc;

    // Slippage = (expected - actual) / expected * 100
    // Positive = you get less than expected (normal for large trades)
    let slippage_pct = ((expected_out - exec_out) / expected_out) * 100.0;

    Ok(slippage_pct)
}

/// Calculate amount_out for XYK (constant product) pools with fees
///
/// Formula: x * y = k (constant product)
/// With fees: amount_in_after_fee = amount_in * (10000 - fee_bps) / 10000
/// amount_out = (reserve_out * amount_in_after_fee) / (reserve_in + amount_in_after_fee)
///
/// Example for 0.3% fee (30 bps):
/// - amount_in = 1000 USDC.e (1_000_000_000 raw units with 6 decimals)
/// - reserve_in = 1,000,000 USDC.e
/// - reserve_out = 50,000 WAVAX
/// - fee_bps = 30 (0.3%)
/// - amount_in_after_fee = 1_000_000_000 * 9970 / 10000 = 997_000_000
/// - amount_out = (50_000 * 10^18 * 997_000_000) / (1_000_000 * 10^6 + 997_000_000)
///
/// Decimal handling:
/// - All calculations use raw units (with decimals)
/// - USDC.e: 6 decimals (1 USDC.e = 1_000_000)
/// - WAVAX: 18 decimals (1 WAVAX = 1_000_000_000_000_000_000)
///
/// Parameters:
/// - reserve_in: Reserve of input token in the pool (raw units with decimals)
/// - reserve_out: Reserve of output token in the pool (raw units with decimals)
/// - amount_in: Amount of input token (raw units with decimals)
/// - fee_bps: Fee in basis points (e.g., 30 = 0.3% = 30/10000)
///
/// Returns: amount_out in raw units (with decimals)
#[allow(dead_code)] // Will be used when pair addresses are available
fn calculate_xyk_amount_out(
    reserve_in: U256,
    reserve_out: U256,
    amount_in: U256,
    fee_bps: u16, // Fee in basis points (30 = 0.3%)
) -> Result<U256> {
    if reserve_in.is_zero() || reserve_out.is_zero() {
        anyhow::bail!("Reserves cannot be zero");
    }
    if amount_in.is_zero() {
        return Ok(U256::zero());
    }

    // Calculate amount_in after fee
    // fee_bps = 30 means 0.3% fee, so we keep 9970/10000 = 0.997
    let fee_multiplier = U256::from(10000u64 - fee_bps as u64);
    let amount_in_after_fee = (amount_in * fee_multiplier) / U256::from(10000u64);

    // XYK formula: amount_out = (reserve_out * amount_in_after_fee) / (reserve_in + amount_in_after_fee)
    // Using checked arithmetic to prevent overflow
    let numerator = reserve_out
        .checked_mul(amount_in_after_fee)
        .ok_or_else(|| anyhow::anyhow!("Overflow in numerator calculation"))?;
    let denominator = reserve_in
        .checked_add(amount_in_after_fee)
        .ok_or_else(|| anyhow::anyhow!("Overflow in denominator calculation"))?;

    if denominator.is_zero() {
        anyhow::bail!("Denominator is zero");
    }

    let amount_out = numerator / denominator;
    Ok(amount_out)
}

/// Get pool reserves from UniswapV2-style pair contract
///
/// Note: To use this, you need the pair contract address.
/// You can get it from the factory contract using getPair(token0, token1)
#[allow(dead_code)] // Will be used when pair addresses are available
async fn get_pair_reserves(
    pair_address: Address,
    provider: &Arc<Provider<Http>>,
    token_in: Address,
    token_out: Address,
) -> Result<(U256, U256)> {
    let pair = UniswapV2Pair::new(pair_address, provider.clone());
    let (reserve0, reserve1, _) = pair.get_reserves().call().await?;
    let t0 = pair.token_0().call().await?;
    let t1 = pair.token_1().call().await?;

    let r0 = U256::from(reserve0);
    let r1 = U256::from(reserve1);

    if token_in == t0 && token_out == t1 {
        Ok((r0, r1))
    } else if token_in == t1 && token_out == t0 {
        Ok((r1, r0))
    } else {
        anyhow::bail!(
            "Pair tokens mismatch: pair({:?},{:?}) vs requested({:?},{:?})",
            t0,
            t1,
            token_in,
            token_out
        );
    }
}

/// Validate router quote against our XYK calculation
/// This helps detect phantom profits from incorrect formulas
///
/// Usage: Compare router.get_amounts_out() with our calculate_xyk_amount_out()
/// If difference > 1%, there may be an issue with fee assumptions or formula.
///
/// Returns: (router_amount_out, calculated_amount_out, difference_percentage)
#[allow(dead_code)] // Will be used when pair addresses are available
#[allow(clippy::too_many_arguments)]
async fn validate_router_quote(
    router_amount_out: U256,
    pair_address: Address,
    provider: &Arc<Provider<Http>>,
    token_in: Address,
    token_out: Address,
    amount_in: U256,
    fee_bps: u16,
    _decimals_in: u32,
    decimals_out: u32,
) -> Result<(f64, f64, f64)> {
    // Get reserves
    let (reserve_in, reserve_out) =
        get_pair_reserves(pair_address, provider, token_in, token_out).await?;

    // Calculate expected amount_out using XYK formula
    let calculated_amount_out =
        calculate_xyk_amount_out(reserve_in, reserve_out, amount_in, fee_bps)?;

    // Convert to human-readable units for comparison
    let router_out_f64 = router_amount_out.as_u128() as f64 / 10f64.powi(decimals_out as i32);
    let calculated_out_f64 =
        calculated_amount_out.as_u128() as f64 / 10f64.powi(decimals_out as i32);

    // Calculate difference percentage
    let diff_pct = if calculated_out_f64 > 0.0 {
        ((router_out_f64 - calculated_out_f64) / calculated_out_f64) * 100.0
    } else {
        0.0
    };

    Ok((router_out_f64, calculated_out_f64, diff_pct))
}

// ---------- RPC ----------
const RPC_URL: &str = "https://api.avax.network/ext/bc/C/rpc";

// ---------- Tokens (Avalanche C-Chain) ----------
const USDC_E: &str = "0xA7D7079b0FEaD91F3e65f86E8915Cb59c1a4C664"; // USDC.e bridged (6 decimals)
const WAVAX: &str = "0xB31f66AA3C1e785363F0875A1b74E27b85FD66c7"; // WAVAX  (18 decimals)

// ---------- Trader Joe v1 Router ----------
const JOE_ROUTER_V1: &str = "0x60aE616a2155Ee3d9A68541Ba4544862310933d4";

// ---------- Pangolin Router ----------
const PANGOLIN_ROUTER: &str = "0xE54Ca86531e17Ef3616d22Ca28b0D458b6C89106";

#[tokio::main]
async fn main() -> Result<()> {
    // 1) provider & routers
    let provider = Arc::new(Provider::<Http>::try_from(RPC_URL)?);
    let joe_router = JoeRouter::new(JOE_ROUTER_V1.parse::<Address>()?, provider.clone());
    let pangolin_router =
        PangolinRouter::new(PANGOLIN_ROUTER.parse::<Address>()?, provider.clone());

    // 2) tokens
    let usdc = USDC_E.parse::<Address>()?;
    let wavax = WAVAX.parse::<Address>()?;

    // 3) Position size - configurable via POSITION_SIZE_USDC environment variable
    // Default: 1000 USDC.e
    // Test different sizes: POSITION_SIZE_USDC=500 cargo run (for 500 USDC.e)
    let initial_amount_usdc: f64 = env::var("POSITION_SIZE_USDC")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(1000.0); // Default to 1000 USDC.e if not set or invalid

    // Convert to raw units (USDC.e has 6 decimals)
    // e.g., 1000 USDC.e = 1_000_000_000 raw units
    let amount_in = U256::from((initial_amount_usdc * 1_000_000.0) as u64);

    // Estimated gas limit per swap (typical DEX swap uses 200k-300k gas)
    const GAS_LIMIT_PER_SWAP: u64 = 250_000;

    // Minimum ROI threshold for real arbitrage opportunities
    // Formula: price_diff_min% ≈ fees_total% + gas% + safety_buffer%
    // For Avalanche: aim for at least ~1% (0.01) to consider it a real opportunity
    // Can increase to 1.2-1.5% (0.012-0.015) for ultra-safe mode
    const MIN_ROI_THRESHOLD: f64 = 0.01; // 1% minimum ROI

    println!("[{}] starting…", Utc::now().format("%H:%M:%S"));
    println!("Comparing Trader Joe V1 vs Pangolin for USDC.e/WAVAX arbitrage");
    println!(
        "Position size: {:.2} USDC.e (configure via POSITION_SIZE_USDC env var)",
        initial_amount_usdc
    );

    // Earnings tracking
    let mut total_opportunities = 0u64;
    let mut total_gross_profit = 0.0;
    let mut total_gas_cost = 0.0;
    let mut total_net_profit = 0.0;
    let mut last_summary_time = Utc::now();

    // Market monitoring stats
    let mut total_checks = 0u64;
    let mut total_price_diffs: Vec<f64> = Vec::new();
    let mut total_gas_costs: Vec<f64> = Vec::new();

    // Maximum number of entries to keep in earnings.txt
    // Can be configured via MAX_EARNINGS_ENTRIES environment variable
    // Test different sizes:
    // - 100:   Small (recent history only, ~50KB file)
    // - 500:   Medium (~250KB file)
    // - 1000:  Default (~500KB file)
    // - 2000:  Large (~1MB file)
    // - 5000:  Very large (~2.5MB file)
    // - 0:     Disable limit (unlimited growth)
    // Usage: MAX_EARNINGS_ENTRIES=500 cargo run
    let max_earnings_entries: usize = env::var("MAX_EARNINGS_ENTRIES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000); // Default to 1000 if not set or invalid

    // Helper function to limit entries in earnings.txt to last N entries
    let limit_earnings_entries = {
        let max_entries = max_earnings_entries;
        move |filename: &str| {
            // Skip if limit is disabled (0)
            if max_entries == 0 {
                return;
            }

            if let Ok(content) = read_to_string(filename) {
                let separator = "═══════════════════════════════════════════════════════════";

                // Count entries by counting "EARNINGS SUMMARY" occurrences
                let entry_count = content.matches("EARNINGS SUMMARY").count();

                // Only process if we exceed the limit
                if entry_count > max_entries {
                    // Split by "EARNINGS SUMMARY" to get individual entries
                    let parts: Vec<&str> = content.split("EARNINGS SUMMARY").collect();

                    // First part is empty or header, rest are entries
                    let entries: Vec<&str> = parts
                        .into_iter()
                        .skip(1) // Skip first empty part
                        .collect();

                    // Keep only the last max_entries entries
                    let start_idx = entries.len() - max_entries;
                    let kept_entries: Vec<&str> = entries[start_idx..].to_vec();

                    // Reconstruct file content
                    let mut new_content = String::new();
                    for entry in kept_entries {
                        new_content.push_str(separator);
                        new_content.push_str("\nEARNINGS SUMMARY");
                        // Entry already contains the rest (timestamp, separator, content)
                        new_content.push_str(entry);
                        if !entry.ends_with('\n') {
                            new_content.push('\n');
                        }
                    }

                    // Write back to file
                    if let Err(e) = write(filename, new_content) {
                        eprintln!("Warning: Failed to limit earnings entries: {}", e);
                    }
                }
            }
        }
    };

    // Helper function to save earnings to file
    let save_earnings_to_file = |opps: u64,
                                 gross: f64,
                                 gas: f64,
                                 net: f64,
                                 checks: u64,
                                 price_diffs: &[f64],
                                 gas_costs: &[f64]| {
        let timestamp = Utc::now().format("%Y-%m-%d %H:%M:%S");
        let filename = "earnings.txt";

        // Limit entries before adding new one
        limit_earnings_entries(filename);

        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(filename) {
            writeln!(
                file,
                "═══════════════════════════════════════════════════════════"
            )
            .ok();
            writeln!(file, "EARNINGS SUMMARY - {}", timestamp).ok();
            writeln!(
                file,
                "═══════════════════════════════════════════════════════════"
            )
            .ok();
            writeln!(file, "Opportunities found:  {}", opps).ok();
            writeln!(file, "Total Gross Profit:   ${:.2} USDC.e", gross).ok();
            writeln!(file, "Total Gas Cost:       ${:.2} USDC.e", gas).ok();
            writeln!(file, "Total Net Profit:     ${:.2} USDC.e", net).ok();
            if opps > 0 {
                writeln!(
                    file,
                    "Average per trade:    ${:.2} USDC.e",
                    net / opps as f64
                )
                .ok();
            }
            writeln!(file).ok();
            writeln!(file, "MARKET CONDITIONS:").ok();
            writeln!(file, "  Total checks performed: {}", checks).ok();
            if !price_diffs.is_empty() {
                let avg_price_diff = price_diffs.iter().sum::<f64>() / price_diffs.len() as f64;
                let max_price_diff = price_diffs.iter().fold(0.0f64, |a, &b| a.max(b));
                let min_price_diff = price_diffs.iter().fold(f64::MAX, |a, &b| a.min(b));
                writeln!(file, "  Price difference stats:").ok();
                writeln!(file, "    Average: {:.3}%", avg_price_diff).ok();
                writeln!(file, "    Maximum: {:.3}%", max_price_diff).ok();
                writeln!(file, "    Minimum: {:.3}%", min_price_diff).ok();
            }
            if !gas_costs.is_empty() {
                let avg_gas = gas_costs.iter().sum::<f64>() / gas_costs.len() as f64;
                writeln!(file, "  Average gas cost: ${:.2} USDC.e", avg_gas).ok();
            }
            writeln!(file).ok();
            if opps == 0 && checks > 0 {
                writeln!(
                    file,
                    "NOTE: No profitable opportunities found. This is NORMAL because:"
                )
                .ok();
                writeln!(
                    file,
                    "  - Markets are efficient (arbitrage gets exploited quickly)"
                )
                .ok();
                writeln!(
                    file,
                    "  - Gas costs require significant price differences to be profitable"
                )
                .ok();
                writeln!(file, "  - Slippage reduces returns on larger trades").ok();
                writeln!(file, "  - Competition from other bots").ok();
                writeln!(file).ok();
            }
        }

        // Save cumulative summary file (overwrites each time with current totals)
        let summary_filename = "earnings_summary.txt";
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(summary_filename)
        {
            writeln!(
                file,
                "╔══════════════════════════════════════════════════════════╗"
            )
            .ok();
            writeln!(
                file,
                "║         📊 TOTAL EARNINGS SUMMARY (SINCE START)         ║"
            )
            .ok();
            writeln!(
                file,
                "╠══════════════════════════════════════════════════════════╣"
            )
            .ok();
            writeln!(file, "║  Last Updated:        {:>35} ║", timestamp).ok();
            writeln!(
                file,
                "╠══════════════════════════════════════════════════════════╣"
            )
            .ok();
            writeln!(file, "║  Opportunities found:  {:>35} ║", opps).ok();
            writeln!(file, "║  Total Gross Profit:   ${:>33.2} ║", gross).ok();
            writeln!(file, "║  Total Gas Cost:       ${:>33.2} ║", gas).ok();
            writeln!(file, "║  Total Net Profit:     ${:>33.2} ║", net).ok();
            if opps > 0 {
                writeln!(
                    file,
                    "║  Average per trade:    ${:>33.2} ║",
                    net / opps as f64
                )
                .ok();
            }
            writeln!(
                file,
                "╠══════════════════════════════════════════════════════════╣"
            )
            .ok();
            writeln!(
                file,
                "║  MARKET STATISTICS                                      ║"
            )
            .ok();
            writeln!(
                file,
                "╠══════════════════════════════════════════════════════════╣"
            )
            .ok();
            writeln!(file, "║  Total checks performed: {:>30} ║", checks).ok();
            if !price_diffs.is_empty() {
                let avg_price_diff = price_diffs.iter().sum::<f64>() / price_diffs.len() as f64;
                let max_price_diff = price_diffs.iter().fold(0.0f64, |a, &b| a.max(b));
                let min_price_diff = price_diffs.iter().fold(f64::MAX, |a, &b| a.min(b));
                writeln!(
                    file,
                    "║  Avg price difference:  {:>30.3}% ║",
                    avg_price_diff
                )
                .ok();
                writeln!(
                    file,
                    "║  Max price difference:  {:>30.3}% ║",
                    max_price_diff
                )
                .ok();
                writeln!(
                    file,
                    "║  Min price difference:  {:>30.3}% ║",
                    min_price_diff
                )
                .ok();
            }
            if !gas_costs.is_empty() {
                let avg_gas = gas_costs.iter().sum::<f64>() / gas_costs.len() as f64;
                writeln!(file, "║  Average gas cost:      ${:>32.2} ║", avg_gas).ok();
            }
            writeln!(
                file,
                "╚══════════════════════════════════════════════════════════╝"
            )
            .ok();
        }

        // Also save JSON summary
        let json_filename = "earnings.json";
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(json_filename)
        {
            let avg_price_diff = if !price_diffs.is_empty() {
                price_diffs.iter().sum::<f64>() / price_diffs.len() as f64
            } else {
                0.0
            };
            let max_price_diff = price_diffs.iter().fold(0.0f64, |a, &b| a.max(b));
            let avg_gas = if !gas_costs.is_empty() {
                gas_costs.iter().sum::<f64>() / gas_costs.len() as f64
            } else {
                0.0
            };

            let json = format!(
                r#"{{
  "last_updated": "{}",
  "opportunities_found": {},
                "total_gross_profit_usdc": {:.2},
                "total_gas_cost_usdc": {:.2},
                "total_net_profit_usdc": {:.2},
  "average_per_trade_usdc": {:.2},
  "market_stats": {{
    "total_checks": {},
    "average_price_diff_pct": {:.3},
    "max_price_diff_pct": {:.3},
    "average_gas_cost_usdc": {:.2}
  }}
}}"#,
                timestamp,
                opps,
                gross,
                gas,
                net,
                if opps > 0 { net / opps as f64 } else { 0.0 },
                checks,
                avg_price_diff,
                max_price_diff,
                avg_gas
            );
            writeln!(file, "{}", json).ok();
        }
    };

    // Save initial summary to file (no console output)
    save_earnings_to_file(
        total_opportunities,
        total_gross_profit,
        total_gas_cost,
        total_net_profit,
        total_checks,
        &total_price_diffs,
        &total_gas_costs,
    );
    println!("💾 Earnings are being saved to:");
    println!("   - earnings_summary.txt: Total earnings since start (always up-to-date)");
    if max_earnings_entries > 0 {
        println!(
            "   - earnings.txt: Detailed log (keeping last {} entries)",
            max_earnings_entries
        );
    } else {
        println!("   - earnings.txt: Detailed log (unlimited entries)");
    }
    println!("   - earnings.json: Latest summary in JSON format\n");

    // Watch for new blocks instead of fixed interval
    // Note: watch_blocks() may require WebSocket provider, so we poll block numbers
    let mut last_block_number = provider.get_block_number().await.unwrap_or_default();
    println!(
        "🔍 Watching for new blocks (starting from block #{})...",
        last_block_number
    );

    loop {
        // Check for new block
        match provider.get_block_number().await {
            Ok(current_block) if current_block > last_block_number => {
                last_block_number = current_block;
                println!(
                    "[{}] Block #{} detected",
                    Utc::now().format("%H:%M:%S"),
                    current_block
                );

                // Fetch current gas price and calculate gas cost
                // Calculate gas cost: gas_limit * gas_price (in AVAX)
                let gas_cost_avax = match calculate_gas_cost(&provider, GAS_LIMIT_PER_SWAP).await {
                    Ok(cost) => {
                        println!("   Current gas cost per swap: {cost:.6} AVAX (gas_limit: {}, gas_price: dynamic)", GAS_LIMIT_PER_SWAP);
                        cost
                    }
                    Err(e) => {
                        println!(
                            "   Warning: Could not fetch gas price: {e}, using fallback 0.015 AVAX"
                        );
                        0.015 // Fallback to original estimate
                    }
                };

                // ---------- Trader Joe ----------
                // Quote USDC.e -> WAVAX on Joe
                let (joe_wavax_out, _joe_slippage) = match joe_router
                    .get_amounts_out(amount_in, vec![usdc, wavax])
                    .call()
                    .await
                {
                    Ok(amounts) if amounts.len() >= 2 => {
                        let amount_out = amounts[1].as_u128() as f64 / 1e18; // WAVAX has 18 decimals
                                                                             // Calculate slippage
                        let slippage = calculate_slippage_joe(
                            &joe_router,
                            amount_in,
                            amount_out,
                            usdc,
                            wavax,
                            6,
                            18,
                        )
                        .await
                        .unwrap_or(0.0);
                        println!("Joe V1   {:.2} USDC.e -> {amount_out:.6} WAVAX (slippage: {slippage:.3}%)", initial_amount_usdc);
                        (Some(amount_out), slippage)
                    }
                    Ok(_) => {
                        println!("Joe V1   no path (amounts too short)");
                        (None, 0.0)
                    }
                    Err(e) => {
                        println!("Joe V1   error: {e:?}");
                        (None, 0.0)
                    }
                };

                // ---------- PANGOLIN ----------
                // Quote USDC.e -> WAVAX on Pangolin
                let (pangolin_wavax_out, _pangolin_slippage) = match pangolin_router
                    .get_amounts_out(amount_in, vec![usdc, wavax])
                    .call()
                    .await
                {
                    Ok(amounts) if amounts.len() >= 2 => {
                        let amount_out = amounts[1].as_u128() as f64 / 1e18; // WAVAX has 18 decimals
                                                                             // Calculate slippage
                        let slippage = calculate_slippage_pangolin(
                            &pangolin_router,
                            amount_in,
                            amount_out,
                            usdc,
                            wavax,
                            6,
                            18,
                        )
                        .await
                        .unwrap_or(0.0);
                        println!("Pangolin  {:.2} USDC.e -> {amount_out:.6} WAVAX (slippage: {slippage:.3}%)", initial_amount_usdc);
                        (Some(amount_out), slippage)
                    }
                    Ok(_) => {
                        println!("Pangolin  no path (amounts too short)");
                        (None, 0.0)
                    }
                    Err(e) => {
                        println!("Pangolin  error: {e:?}");
                        (None, 0.0)
                    }
                };

                // ---------- ARBITRAGE DETECTION ----------
                // Calculate actual round-trip arbitrage profit
                if let (Some(joe_out), Some(pangolin_out)) = (joe_wavax_out, pangolin_wavax_out) {
                    // Strategy 1: Buy on Joe, Sell on Pangolin
                    // Step 1: Buy WAVAX on Joe with initial_amount_usdc USDC.e -> get joe_out WAVAX
                    // Step 2: Sell joe_out WAVAX on Pangolin -> get USDC.e back
                    let joe_wavax_amount = U256::from((joe_out * 1e18) as u128);
                    let pangolin_usdc_back = match pangolin_router
                        .get_amounts_out(joe_wavax_amount, vec![wavax, usdc])
                        .call()
                        .await
                    {
                        Ok(amounts) if amounts.len() >= 2 => {
                            Some(amounts[1].as_u128() as f64 / 1e6) // USDC.e has 6 decimals
                        }
                        _ => None,
                    };

                    // Strategy 2: Buy on Pangolin, Sell on Joe
                    // Step 1: Buy WAVAX on Pangolin with initial_amount_usdc USDC.e -> get pangolin_out WAVAX
                    // Step 2: Sell pangolin_out WAVAX on Joe -> get USDC.e back
                    let pangolin_wavax_amount = U256::from((pangolin_out * 1e18) as u128);
                    let joe_usdc_back = match joe_router
                        .get_amounts_out(pangolin_wavax_amount, vec![wavax, usdc])
                        .call()
                        .await
                    {
                        Ok(amounts) if amounts.len() >= 2 => {
                            Some(amounts[1].as_u128() as f64 / 1e6) // USDC.e has 6 decimals
                        }
                        _ => None,
                    };

                    // Calculate profits for both strategies
                    if let Some(usdc_received) = pangolin_usdc_back {
                        // Strategy 1: Buy on Joe, Sell on Pangolin
                        let total_gas_cost_avax = gas_cost_avax * 2.0; // Two swaps
                                                                       // Get AVAX price to convert gas cost to USDC.e
                        let gas_cost_usdc =
                            match get_avax_price_in_usdc(&provider, &joe_router, usdc, wavax).await
                            {
                                Ok(avax_price) => total_gas_cost_avax * avax_price,
                                Err(_) => total_gas_cost_avax * 20.0, // Fallback: 1 AVAX ≈ 20 USDC
                            };

                        // Calculate net profit (gross profit already accounts for all fees via router quotes)
                        let profit_net = usdc_received - initial_amount_usdc - gas_cost_usdc;
                        let roi_net = profit_net / initial_amount_usdc;
                        let roi_net_pct = roi_net * 100.0;

                        let gross_profit_usdc = usdc_received - initial_amount_usdc;

                        if roi_net >= MIN_ROI_THRESHOLD {
                            // Update earnings tracking
                            total_opportunities += 1;
                            total_gross_profit += gross_profit_usdc;
                            total_gas_cost += gas_cost_usdc;
                            total_net_profit += profit_net;

                            println!("🚀 ARBITRAGE OPPORTUNITY DETECTED!");
                            println!("   Strategy: Buy WAVAX on Joe, Sell WAVAX on Pangolin");
                            println!("   Step 1: Buy {joe_out:.6} WAVAX on Joe (cost: {initial_amount_usdc:.2} USDC.e)");
                            println!(
                        "   Step 2: Sell {joe_out:.6} WAVAX on Pangolin (receive: {usdc_received:.2} USDC.e)"
                    );
                            println!("   Gross Profit:    {gross_profit_usdc:.2} USDC.e");
                            println!(
                        "   Gas Cost:        {gas_cost_usdc:.2} USDC.e ({total_gas_cost_avax:.6} AVAX)"
                    );
                            println!("   Net Profit:      {profit_net:.2} USDC.e");
                            println!(
                                "   Net ROI:         {roi_net_pct:.3}% (threshold: {:.1}%)",
                                MIN_ROI_THRESHOLD * 100.0
                            );
                            println!(
                                "   ✅ Net ROI exceeds minimum threshold ({:.1}%)",
                                MIN_ROI_THRESHOLD * 100.0
                            );
                            println!("   Profit in USD:   ${profit_net:.2}");
                            println!("   📊 Total Earnings: ${total_net_profit:.2} ({total_opportunities} opportunities)");

                            // Save to file immediately when opportunity is found
                            save_earnings_to_file(
                                total_opportunities,
                                total_gross_profit,
                                total_gas_cost,
                                total_net_profit,
                                total_checks,
                                &total_price_diffs,
                                &total_gas_costs,
                            );
                        }
                    }

                    if let Some(usdc_received) = joe_usdc_back {
                        // Strategy 2: Buy on Pangolin, Sell on Joe
                        let total_gas_cost_avax = gas_cost_avax * 2.0; // Two swaps
                                                                       // Get AVAX price to convert gas cost to USDC.e
                        let gas_cost_usdc =
                            match get_avax_price_in_usdc(&provider, &joe_router, usdc, wavax).await
                            {
                                Ok(avax_price) => total_gas_cost_avax * avax_price,
                                Err(_) => total_gas_cost_avax * 20.0, // Fallback: 1 AVAX ≈ 20 USDC
                            };

                        // Calculate net profit (gross profit already accounts for all fees via router quotes)
                        let profit_net = usdc_received - initial_amount_usdc - gas_cost_usdc;
                        let roi_net = profit_net / initial_amount_usdc;
                        let roi_net_pct = roi_net * 100.0;

                        let gross_profit_usdc = usdc_received - initial_amount_usdc;

                        if roi_net >= MIN_ROI_THRESHOLD {
                            // Update earnings tracking
                            total_opportunities += 1;
                            total_gross_profit += gross_profit_usdc;
                            total_gas_cost += gas_cost_usdc;
                            total_net_profit += profit_net;

                            println!("🚀 ARBITRAGE OPPORTUNITY DETECTED!");
                            println!("   Strategy: Buy WAVAX on Pangolin, Sell WAVAX on Joe");
                            println!(
                        "   Step 1: Buy {pangolin_out:.6} WAVAX on Pangolin (cost: {:.2} USDC.e)",
                        initial_amount_usdc
                    );
                            println!(
                        "   Step 2: Sell {pangolin_out:.6} WAVAX on Joe (receive: {usdc_received:.2} USDC.e)"
                    );
                            println!("   Gross Profit:    {gross_profit_usdc:.2} USDC.e");
                            println!(
                        "   Gas Cost:        {gas_cost_usdc:.2} USDC.e ({total_gas_cost_avax:.6} AVAX)"
                    );
                            println!("   Net Profit:      {profit_net:.2} USDC.e");
                            println!(
                                "   Net ROI:         {roi_net_pct:.3}% (threshold: {:.1}%)",
                                MIN_ROI_THRESHOLD * 100.0
                            );
                            println!(
                                "   ✅ Net ROI exceeds minimum threshold ({:.1}%)",
                                MIN_ROI_THRESHOLD * 100.0
                            );
                            println!("   Profit in USD:   ${profit_net:.2}");
                            println!("   📊 Total Earnings: ${total_net_profit:.2} ({total_opportunities} opportunities)");

                            // Save to file immediately when opportunity is found
                            save_earnings_to_file(
                                total_opportunities,
                                total_gross_profit,
                                total_gas_cost,
                                total_net_profit,
                                total_checks,
                                &total_price_diffs,
                                &total_gas_costs,
                            );
                        }
                    }

                    // Show status if no profitable arbitrage found
                    // Get AVAX price for accurate calculations
                    let avax_price = get_avax_price_in_usdc(&provider, &joe_router, usdc, wavax)
                        .await
                        .unwrap_or(20.0); // Fallback

                    // Calculate gas cost in USDC.e: gas_used * gas_price * avax_price_in_usdc
                    // Where:
                    //   - gas_used = GAS_LIMIT_PER_SWAP * 2 (for 2 swaps)
                    //   - gas_price = fetched dynamically from blockchain (in Wei, converted to AVAX)
                    //   - avax_price_in_usdc = current AVAX price in USDC.e
                    // Formula: gas_cost_usdc = gas_used * gas_price * avax_price_in_usdc
                    //          = (gas_limit * gas_price) * 2 * avax_price
                    let total_gas_cost_usdc = gas_cost_avax * 2.0 * avax_price;

                    // Always log gas cost, even if very small (use more decimals for small values)
                    if total_gas_cost_usdc < 0.01 {
                        println!("   Gas cost (2 swaps): {total_gas_cost_usdc:.8} USDC.e (formula: gas_limit * gas_price * 2 * avax_price)");
                    } else {
                        println!("   Gas cost (2 swaps): {total_gas_cost_usdc:.6} USDC.e (formula: gas_limit * gas_price * 2 * avax_price)");
                    }

                    // Track market stats
                    total_checks += 1;
                    let price_diff = (joe_out - pangolin_out).abs();
                    let price_diff_pct = if (joe_out + pangolin_out) > 0.0 {
                        (price_diff / ((joe_out + pangolin_out) / 2.0)) * 100.0
                    } else {
                        0.0
                    };
                    total_price_diffs.push(price_diff_pct);
                    total_gas_costs.push(total_gas_cost_usdc);

                    let mut found_profit = false;
                    if let Some(usdc_received) = pangolin_usdc_back {
                        if (usdc_received - initial_amount_usdc - total_gas_cost_usdc) > 0.0 {
                            found_profit = true;
                        }
                    }
                    if let Some(usdc_received) = joe_usdc_back {
                        if (usdc_received - initial_amount_usdc - total_gas_cost_usdc) > 0.0 {
                            found_profit = true;
                        }
                    }

                    if !found_profit {
                        // Calculate actual returns and show what's needed
                        let min_profit_needed = total_gas_cost_usdc + 0.01; // Add small buffer

                        // Calculate exact minimum price difference needed for profitability
                        // Based on current gas costs and observed slippage from round-trip
                        let min_price_diff_pct = if (joe_out + pangolin_out) > 0.0 {
                            // Calculate based on actual round-trip performance
                            let best_return = pangolin_usdc_back
                                .unwrap_or(0.0)
                                .max(joe_usdc_back.unwrap_or(0.0));

                            if best_return > 0.0 {
                                // Calculate slippage factor from actual round-trip
                                let slippage_factor =
                                    (initial_amount_usdc - best_return) / initial_amount_usdc;
                                // Need: min_profit_needed + current_loss
                                let total_needed =
                                    min_profit_needed + (initial_amount_usdc - best_return);
                                // Price diff needed = total_needed / (1 - slippage_factor)
                                // Then convert to percentage
                                let min_price_diff =
                                    total_needed / (1.0 - slippage_factor.max(0.01));
                                (min_price_diff / initial_amount_usdc) * 100.0
                            } else {
                                // Fallback: estimate based on gas cost
                                (min_profit_needed / initial_amount_usdc) * 100.0 * 2.0
                            }
                        } else {
                            0.0
                        };

                        if let Some(usdc_received) = pangolin_usdc_back {
                            let loss = initial_amount_usdc - usdc_received;
                            let additional_needed = min_profit_needed + loss;
                            println!("   No profitable arbitrage (current price diff: {price_diff_pct:.2}%)");
                            println!("   Strategy 1: Buy on Joe, Sell on Pangolin");
                            println!(
                        "     - Receive back: {usdc_received:.2} USDC.e (loss: {loss:.2} USDC.e)"
                    );
                            println!("     - Gas cost: {total_gas_cost_usdc:.2} USDC.e");
                            println!(
                                "     - Need {:.2} USDC.e more profit to break even",
                                additional_needed
                            );
                        }

                        if let Some(usdc_received) = joe_usdc_back {
                            let loss = initial_amount_usdc - usdc_received;
                            let additional_needed = min_profit_needed + loss;
                            println!("   Strategy 2: Buy on Pangolin, Sell on Joe");
                            println!(
                        "     - Receive back: {usdc_received:.2} USDC.e (loss: {loss:.2} USDC.e)"
                    );
                            println!("     - Gas cost: {total_gas_cost_usdc:.2} USDC.e");
                            println!(
                                "     - Need {:.2} USDC.e more profit to break even",
                                additional_needed
                            );
                        }

                        // Show calculated minimum price difference needed
                        println!("   💡 Minimum price diff needed: {min_price_diff_pct:.2}% (based on current gas: {total_gas_cost_usdc:.2} USDC.e)");
                    }
                }

                // Save earnings summary to file every 1 minute (no console output)
                let now = Utc::now();
                if (now - last_summary_time).num_seconds() >= 60 {
                    save_earnings_to_file(
                        total_opportunities,
                        total_gross_profit,
                        total_gas_cost,
                        total_net_profit,
                        total_checks,
                        &total_price_diffs,
                        &total_gas_costs,
                    );
                    last_summary_time = now;
                }
            }
            Ok(_) => {
                // Same block, wait a bit before checking again
                sleep(Duration::from_millis(500)).await;
                continue;
            }
            Err(e) => {
                eprintln!("Error getting block number: {e}, retrying in 1s...");
                sleep(Duration::from_secs(1)).await;
                continue;
            }
        }
    }
}
