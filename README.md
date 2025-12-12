# Arbitrage Bot

A real-time cryptocurrency arbitrage detection bot for the Avalanche C-Chain. This bot monitors price differences between Trader Joe v1 and Pangolin DEXs to identify profitable arbitrage opportunities.

## 🚀 Features

- **Real-time Price Monitoring**: Continuously monitors USDC.e/WAVAX prices on multiple DEXs
- **Arbitrage Detection**: Automatically detects profitable arbitrage opportunities with round-trip calculations
- **Profit Calculation**: Calculates actual round-trip profit after accounting for gas costs
- **Multi-DEX Support**: Currently monitors Trader Joe v1 and Pangolin
- **Accurate Calculations**: Uses router's `getAmountsOut` for precise price quotes

## 📋 Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (latest stable version)
- Internet connection for RPC calls to Avalanche network

## 🛠️ Installation

1. Clone the repository:
```bash
git clone <your-repo-url>
cd CleanArbitrage
```

2. Build the project:
```bash
cargo build
```

## 🎯 Usage

### Running the Bot

Simply run:
```bash
cargo run
```

The bot will:
- Connect to Avalanche C-Chain via public RPC
- Start monitoring prices every 8 seconds
- Display price quotes from both exchanges
- Alert you when profitable arbitrage opportunities are detected

### Example Output

```
[11:52:52] starting…
Comparing Trader Joe V1 vs Pangolin for USDC.e/WAVAX arbitrage
[11:52:52] tick
Joe V1   1000 USDC.e -> 70.386247 WAVAX
Pangolin  1000 USDC.e -> 68.667390 WAVAX
   No profitable arbitrage (price diff: 2.47%, but round-trip unprofitable after gas)
   Strategy 1: Buy on Joe, Sell on Pangolin
     - Receive back: 998.50 USDC.e (loss: 1.50 USDC.e)
     - Gas cost: 0.60 USDC.e
     - Need 2.11 USDC.e more profit to break even
   💡 Estimated minimum price diff needed: ~3-5% (depends on liquidity/slippage)
```

When a profitable opportunity is found:
```
🚀 ARBITRAGE OPPORTUNITY DETECTED!
   Strategy: Buy WAVAX on Joe, Sell WAVAX on Pangolin
   Step 1: Buy 70.386247 WAVAX on Joe (cost: 1000 USDC.e)
   Step 2: Sell 70.386247 WAVAX on Pangolin (receive: 1002.50 USDC.e)
   Gross Profit:    2.50 USDC.e
   Gas Cost:        0.60 USDC.e (0.03 AVAX)
   Net Profit:      1.90 USDC.e
   Profit in USD:   $1.90
```

### Stopping the Bot

Press `Ctrl+C` to stop the bot gracefully.

**Note**: If you encounter "Access is denied" errors when recompiling, the previous instance may still be running. Kill it with:
```bash
taskkill /F /IM arbitrage-bot.exe
```

## 🔧 How It Works

1. **Price Fetching**:
   - Both DEXs: Uses router's `getAmountsOut` function to get accurate quotes
   - Trader Joe V1: Router at `0x60aE616a2155Ee3d9A68541Ba4544862310933d4`
   - Pangolin: Router at `0xE54Ca86531e17Ef3616d22Ca28b0D458b6C89106`

2. **Arbitrage Detection**:
   - Gets quotes for USDC.e → WAVAX on both exchanges
   - Calculates round-trip profit:
     - Strategy 1: Buy WAVAX on Joe, sell on Pangolin
     - Strategy 2: Buy WAVAX on Pangolin, sell on Joe
   - Uses actual reverse swap calculations (WAVAX → USDC.e) for accurate profit

3. **Profit Calculation**:
   - Gross Profit = USDC.e received back - 1000 USDC.e invested
   - Net Profit = Gross Profit - Gas Cost (0.03 AVAX ≈ 0.6 USDC.e)
   - Only alerts if net profit is positive after gas costs

## ⚙️ Configuration

Current configuration (in `src/main.rs`):

- **RPC URL**: `https://api.avax.network/ext/bc/C/rpc` (Avalanche public RPC)
- **Tokens**: USDC.e (bridged, 6 decimals) and WAVAX (18 decimals)
- **Amount**: 1000 USDC.e per check
- **Polling Interval**: 8 seconds
- **Gas Cost Estimate**: 0.015 AVAX per swap (0.03 AVAX total for round-trip)
- **Minimum Profit Threshold**: ~0.6 USDC.e (to cover gas costs)

To modify these values, edit the constants in `src/main.rs`.

## 📊 Current Status

### ✅ Implemented
- Real-time price monitoring
- Arbitrage opportunity detection
- Profit calculation with gas costs
- Multi-DEX price comparison

### ⚠️ Limitations
- **Read-only**: Bot only detects opportunities, doesn't execute trades
- **Manual execution**: Trades must be executed manually
- **Single pair**: Currently only monitors USDC.e/WAVAX
- **Fixed amount**: Uses fixed 1000 USDC.e for calculations
- **Price differences**: Typically need 3-5%+ price difference to be profitable after gas

## 🔮 Future Improvements

- [ ] Automatic trade execution
- [ ] Wallet integration and transaction signing
- [ ] Support for multiple token pairs
- [ ] Dynamic gas price estimation
- [ ] Slippage protection
- [ ] Configurable minimum profit thresholds
- [ ] Historical data logging
- [ ] Web interface/dashboard
- [ ] Support for additional DEXs

## 🏗️ Project Structure

```
CleanArbitrage/
├── src/
│   └── main.rs          # Main bot logic
├── Cargo.toml          # Dependencies
└── README.md           # This file
```

## 📦 Dependencies

- `ethers` - Ethereum/Avalanche blockchain interaction
- `tokio` - Async runtime
- `anyhow` - Error handling
- `chrono` - Timestamp formatting

## ⚠️ Disclaimer

This bot is for **educational and monitoring purposes only**. Arbitrage trading involves risks including:
- Smart contract risks
- Gas price volatility
- Slippage
- Front-running by MEV bots
- Exchange liquidity issues

**Always test with small amounts first and never invest more than you can afford to lose.**

## 📝 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## 🧹 Clean Arbitrage

Clean arbitrage refers to arbitrage opportunities that are truly profitable after accounting for all costs:

- **Gas costs**: Network transaction fees (calculated dynamically)
- **DEX fees**: Trading fees on both exchanges (typically 0.3% per swap)
- **Slippage**: Price impact from trade size
- **Minimum ROI threshold**: Currently set to 1% to ensure meaningful profit

The bot only alerts on "clean" opportunities where:
- Net profit > 0 after all costs
- ROI exceeds the minimum threshold (1%)
- Round-trip calculations confirm profitability

This ensures you only see real, executable arbitrage opportunities, not false positives from price differences that don't account for all costs.

## 📝 Notes

- The bot uses accurate round-trip calculations to determine real arbitrage profitability
- Price differences of 2-3% are usually not profitable due to gas costs and slippage
- Real arbitrage opportunities are rare and typically disappear quickly due to MEV bots
