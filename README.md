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
git clone https://github.com/GasparCoquet/CleanArbitrage.git
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

Or with custom position size (default: 1000 USDC.e):
```bash
POSITION_SIZE_USDC=500 cargo run    # Test with 500 USDC.e
POSITION_SIZE_USDC=2000 cargo run   # Larger position size
```

Or with custom earnings log limit (default: 1000 entries, 0 = unlimited):
```bash
MAX_EARNINGS_ENTRIES=500 cargo run   # Keep last 500 entries (~250KB file)
MAX_EARNINGS_ENTRIES=0 cargo run     # Unlimited entries
```

Or combine both settings:
```bash
POSITION_SIZE_USDC=1000 MAX_EARNINGS_ENTRIES=500 cargo run
```

The bot will:
- Connect to Avalanche C-Chain via public RPC
- Watch for new blocks and check prices when new blocks are detected
- Fetch dynamic gas prices from the blockchain
- Display price quotes and slippage from both exchanges
- Alert you when profitable arbitrage opportunities are detected
- Save earnings to three files: `earnings.txt`, `earnings_summary.txt`, and `earnings.json`

### Example Output

```
[11:52:52] starting…
Comparing Trader Joe V1 vs Pangolin for USDC.e/WAVAX arbitrage
Position size: 1000.00 USDC.e (configure via POSITION_SIZE_USDC env var)
💾 Earnings are being saved to:
   - earnings_summary.txt: Total earnings since start (always up-to-date)
   - earnings.txt: Detailed log (keeping last 1000 entries)
   - earnings.json: Latest summary in JSON format

🔍 Watching for new blocks (starting from block #12345678)...
[11:53:00] Block #12345679 detected
   Current gas cost per swap: 0.005670 AVAX (gas_limit: 250000, gas_price: dynamic)
Joe V1   1000.00 USDC.e -> 70.386247 WAVAX (slippage: 2.340%)
Pangolin  1000.00 USDC.e -> 68.667390 WAVAX (slippage: 2.410%)
   No profitable arbitrage (current price diff: 2.47%)
   Strategy 1: Buy on Joe, Sell on Pangolin
     - Receive back: 998.50 USDC.e (loss: 1.50 USDC.e)
     - Gas cost: 0.60 USDC.e
     - Need 2.11 USDC.e more profit to break even
   💡 Minimum price diff needed: 3.21% (based on current gas: 0.60 USDC.e)
```

When an opportunity is found:
```
🚀 ARBITRAGE OPPORTUNITY DETECTED!
   Strategy: Buy WAVAX on Joe, Sell WAVAX on Pangolin
   Step 1: Buy 70.386247 WAVAX on Joe (cost: 1000.00 USDC.e)
   Step 2: Sell 70.386247 WAVAX on Pangolin (receive: 1002.50 USDC.e)
   Gross Profit:    2.50 USDC.e
   DEX Fees:        0.66 USDC.e (calculated dynamically from actual amounts)
   Gas Cost:        0.60 USDC.e (0.030000 AVAX)
   Net Profit:      1.24 USDC.e
   Net ROI:         0.124% (threshold: 1.0%)
   ✅ Net ROI exceeds minimum threshold (1.0%)
   Profit in USD:   $1.24
   📊 Total Earnings: $12.45 (15 opportunities)
```

### Stopping the Bot

Press `Ctrl+C` to stop the bot gracefully. The summary files will be saved before exit.

**Note**: If you encounter "Access is denied" errors when recompiling, the previous instance may still be running. Kill it with:
```bash
taskkill /F /IM arbitrage-bot.exe
```

## 🔧 How It Works

1. **Block Monitoring**:
   - Watches for new blocks on Avalanche C-Chain
   - Triggers price checks when new blocks are detected
   - More efficient than fixed interval polling

2. **Dynamic Gas Price Fetching**:
   - Fetches current gas prices from the blockchain
   - Calculates actual gas cost: `gas_limit * gas_price * 2 * avax_price`
   - Converts AVAX gas cost to USDC.e for profit calculations

3. **Price Fetching & Slippage Calculation**:
   - Both DEXs: Uses router's `getAmountsOut` function to get accurate quotes
   - Trader Joe V1: Router at `0x60aE616a2155Ee3d9A68541Ba4544862310933d4`
   - Pangolin: Router at `0xE54Ca86531e17Ef3616d22Ca28b0D458b6C89106`
   - Calculates slippage for each DEX separately

4. **Arbitrage Detection**:
   - Gets quotes for USDC.e → WAVAX on both exchanges
   - Calculates round-trip profit using reverse swaps:
     - Strategy 1: Buy WAVAX on Joe → Sell WAVAX on Pangolin
     - Strategy 2: Buy WAVAX on Pangolin → Sell WAVAX on Joe
   - Uses actual reverse swap calculations (WAVAX → USDC.e) for accurate profit
   - Tracks both gross and net profit

5. **Profit Calculation**:
   - Gross Profit = USDC.e received back - initial amount invested
   - DEX Fees = calculated dynamically from actual swap amounts (0.3% per swap)
   - Gas Cost = (gas_limit * gas_price) * 2 swaps * AVAX price in USDC.e
   - Net Profit = Gross Profit - Gas Cost - DEX Fees
   - Net ROI = Net Profit / Initial Amount
   - Only alerts if net ROI ≥ 1.0% (MIN_ROI_THRESHOLD)

6. **Earnings Tracking**:
   - Saves earnings to three formats:
     - `earnings.txt`: Detailed historical log (with configurable entry limit)
     - `earnings_summary.txt`: Always up-to-date summary with formatted tables
     - `earnings.json`: Machine-readable JSON format
   - Tracks market statistics: price differences, gas costs, check counts
   - Updates summary files every minute

## ⚙️ Configuration

Current configuration (in `src/main.rs`):

- **RPC URL**: `https://api.avax.network/ext/bc/C/rpc` (Avalanche public RPC)
- **Tokens**: USDC.e (bridged, 6 decimals) and WAVAX (18 decimals)
- **Position size**: Configurable via `POSITION_SIZE_USDC` environment variable (default: 1000 USDC.e)
- **Block monitoring**: Watches for new blocks instead of fixed intervals
- **Gas price fetching**: Dynamically fetches current gas prices from the blockchain
- **Gas limit per swap**: 250,000 gas (typical DEX swap: 200k-300k gas)
- **Minimum ROI threshold**: 1% (MIN_ROI_THRESHOLD in code) - only alerts on real opportunities
- **DEX fee rate**: 0.3% per swap (typical for Avalanche DEXs)
- **Earnings log limit**: Configurable via `MAX_EARNINGS_ENTRIES` environment variable
  - Default: 1000 entries
  - 0: Unlimited (file grows indefinitely)
  - Example: `MAX_EARNINGS_ENTRIES=500` for ~250KB files

To modify these values, edit the constants in `src/main.rs`.

### Environment Variables

- `POSITION_SIZE_USDC`: Trade amount in USDC.e (default: 1000)
  - Example: `POSITION_SIZE_USDC=500 cargo run`
  
- `MAX_EARNINGS_ENTRIES`: Maximum entries to keep in earnings.txt (default: 1000)
  - Example: `MAX_EARNINGS_ENTRIES=500 cargo run`
  - Set to 0 for unlimited entries

## 📊 Current Status

### ✅ Implemented
- **Real-time block monitoring**: Watches new blocks instead of fixed intervals
- **Dynamic gas price fetching**: Automatically fetches current gas prices from blockchain
- **Accurate slippage calculation**: Calculates slippage for both Joe V1 and Pangolin DEXs
- **Comprehensive profit calculation**: 
  - Gross profit tracking
  - Dynamic DEX fee calculation
  - Gas cost in USDC.e (converted from AVAX price)
  - Net profit after all costs
- **Multi-DEX price comparison**: Trader Joe V1 vs Pangolin
- **Configurable position size**: Set via `POSITION_SIZE_USDC` environment variable (default: 1000 USDC.e)
- **Configurable earnings log limit**: Set via `MAX_EARNINGS_ENTRIES` environment variable
- **Three output formats**:
  - `earnings.txt`: Detailed historical log
  - `earnings_summary.txt`: Always up-to-date summary with formatted tables
  - `earnings.json`: Machine-readable JSON format
- **Market statistics tracking**: Price differences, gas costs, check counts

### ⚠️ Limitations
- **Read-only**: Bot only detects opportunities, doesn't execute trades
- **Manual execution**: Trades must be executed manually
- **Single pair**: Currently only monitors USDC.e/WAVAX
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
