# remit-cli

Command-line interface for [Remit](https://remit.md) — USDC payments for AI agents on Base.

## Install

### Binary (recommended)

Download from [GitHub Releases](https://github.com/remit-md/remit-cli/releases/latest):

```bash
# Linux (x86_64)
curl -L https://github.com/remit-md/remit-cli/releases/latest/download/remit-linux-x86_64 -o remit
chmod +x remit && sudo mv remit /usr/local/bin/

# Linux (aarch64)
curl -L https://github.com/remit-md/remit-cli/releases/latest/download/remit-linux-aarch64 -o remit
chmod +x remit && sudo mv remit /usr/local/bin/

# macOS (Apple Silicon)
curl -L https://github.com/remit-md/remit-cli/releases/latest/download/remit-macos-aarch64 -o remit
chmod +x remit && sudo mv remit /usr/local/bin/

# macOS (Intel)
curl -L https://github.com/remit-md/remit-cli/releases/latest/download/remit-macos-x86_64 -o remit
chmod +x remit && sudo mv remit /usr/local/bin/

# Windows (PowerShell)
irm https://github.com/remit-md/remit-cli/releases/latest/download/remit-windows-x86_64.exe -OutFile remit.exe
```

### crates.io

```bash
cargo install remit-cli
```

### Build from source

```bash
git clone https://github.com/remit-md/remit-cli.git
cd remit-cli
cargo install --path .
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
remit status                         # Wallet info + balance
remit pay 0xRecipient 10.00          # Send 10 USDC
remit tab open 0xCounterparty 100    # Open a tab with 100 USDC limit
remit tab charge <tab-id> 5.00       # Charge 5 USDC to the tab
remit fund                           # Get a link to fund your wallet
remit --testnet mint 100             # Mint 100 testnet USDC
```

## Commands

| Command | Description |
|---------|-------------|
| `remit init` | Generate keypair and configure auth |
| `remit status` | Wallet status and balance |
| `remit balance` | USDC balance |
| `remit pay <to> <amount>` | One-time payment |
| `remit tab open/charge/close` | Tab (running balance) |
| `remit stream open/close` | Streaming payments |
| `remit escrow create/release/cancel/claim-start` | Escrow |
| `remit bounty post/submit/award` | Bounties |
| `remit deposit create` | Deposit address |
| `remit fund` | Generate fund link |
| `remit withdraw` | Generate withdraw link |
| `remit mint <amount>` | Mint testnet USDC (max 2500/hr) |
| `remit webhook create/list/delete` | Webhook subscriptions |
| `remit a2a discover/pay/card` | A2A agent discovery and payments |
| `remit config set/get/show` | Configuration |
| `remit completions <shell>` | Shell completions (bash, zsh, fish, powershell) |

## Flags

| Flag | Description |
|------|-------------|
| `--json` | Machine-readable JSON output |
| `--testnet` | Use Base Sepolia testnet |
| `--no-permit` | Skip EIP-2612 permit auto-signing |

## Auth

Set your private key via environment variable or `.env` file:

```bash
export REMITMD_KEY=0x<your-private-key>
```

Or run `remit init` to generate a fresh keypair stored in `~/.remit/config.toml`.

**Never commit your private key to git.**

## Shell Completions

```bash
remit completions bash >> ~/.bash_completion           # bash
remit completions zsh > "${fpath[1]}/_remit"           # zsh
remit completions fish > ~/.config/fish/completions/remit.fish  # fish
remit completions powershell >> $PROFILE               # PowerShell
```

## JSON Output

All commands support `--json` for scripting:

```bash
remit --json balance | jq '.usdc'
remit --json status | jq '.monthly_volume'
remit --json webhook list | jq '.[].url'
```

## License

MIT
