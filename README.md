# remit-cli

Command-line interface for [Remit](https://remit.md) — USDC payment protocol for AI agents on Base.

## Install

### From GitHub Releases (recommended)

Download the binary for your platform from [Releases](https://github.com/remit-md/remit-cli/releases):

```bash
# Linux (x86_64)
curl -L https://github.com/remit-md/remit-cli/releases/latest/download/remit-linux-x86_64 -o remit
chmod +x remit && mv remit /usr/local/bin/

# macOS (Apple Silicon)
curl -L https://github.com/remit-md/remit-cli/releases/latest/download/remit-macos-aarch64 -o remit
chmod +x remit && mv remit /usr/local/bin/

# Windows (PowerShell)
irm https://github.com/remit-md/remit-cli/releases/latest/download/remit-windows-x86_64.exe -OutFile remit.exe
```

### From crates.io

```bash
cargo install remit-cli
```

## Setup

```bash
# Generate a new keypair
remit init

# Or use an existing private key
export REMITMD_KEY=0x<your-private-key>
```

## Quickstart

```bash
# Check your wallet
remit status

# Send 10 USDC
remit pay 0xRecipient 10.00

# Open a tab
remit tab open 0xCounterparty 100.00

# Charge the tab
remit tab charge <tab-id> 5.00

# Close the tab
remit tab close <tab-id>

# Get a fund link
remit fund

# Use testnet
remit --testnet balance

# Mint testnet USDC
remit --testnet mint 100
```

## Commands

| Command | Description |
|---------|-------------|
| `remit init` | Generate keypair + configure auth |
| `remit status` | Wallet status and balance |
| `remit balance` | USDC balance |
| `remit pay <to> <amount>` | Send one-time payment |
| `remit tab open/charge/close` | Tab payment model |
| `remit stream open/close` | Streaming payments |
| `remit escrow create/release/cancel/claim-start` | Escrow |
| `remit bounty post/submit/award` | Bounty payments |
| `remit deposit create` | Deposit address |
| `remit fund` | Generate fund link |
| `remit withdraw` | Generate withdraw link |
| `remit mint <amount>` | Mint testnet USDC (max 2500, testnet only) |
| `remit config set/get/show` | Manage config |
| `remit webhook create/list/delete` | Manage webhook subscriptions |
| `remit a2a discover/pay/card` | A2A agent discovery and payments |

## Flags

| Flag | Description |
|------|-------------|
| `--json` | Output raw JSON (machine-readable) |
| `--testnet` | Use Base Sepolia testnet |

## Shell Completions

```bash
# bash
remit completions bash >> ~/.bash_completion

# zsh
remit completions zsh > "${fpath[1]}/_remit"

# fish
remit completions fish > ~/.config/fish/completions/remit.fish

# PowerShell
remit completions powershell >> $PROFILE
```

## Auth

Set `REMITMD_KEY` to your private key (hex, with or without `0x` prefix):

```bash
export REMITMD_KEY=0x<your-private-key>
```

Or add it to `.env` in your working directory.

**Never commit your private key to git.**

## Config

```bash
remit config set network testnet
remit config set output_format json
remit config show
```

Config is stored in `~/.remit/config.toml`.

## JSON Output

All commands support `--json` for use in scripts and pipelines:

```bash
remit --json balance | jq '.usdc'
remit --json status | jq '.monthly_volume'
remit --json webhook list | jq '.[].url'
```

## License

MIT — see [LICENSE](LICENSE)
