# remit-cli

> [Skill MD](https://remit.md) · [Docs](https://remit.md/docs) · [Agent Spec](https://remit.md/agent.md)

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

### With OWS (recommended)

[OWS](https://openwallet.sh) encrypts keys locally and evaluates spending policies before every signature. `remit init` handles the full setup:

```bash
remit init
```

This creates an OWS wallet (`remit-{hostname}`), a Base chain-lock policy, and an API key. It prints your wallet address and MCP config — set `OWS_API_KEY` in your environment.

```bash
# With options
remit init --name my-agent --chain base-sepolia

# Add spending limits
remit wallet set-policy --max-tx 500 --daily-limit 5000
```

### With a raw private key (legacy)

```bash
remit init --legacy --write-env   # generates key, saves to .env
# or
export REMITMD_KEY=0x<your-private-key>
```

## Quickstart

```bash
remit status                         # Wallet info + balance
remit pay 0xRecipient 10.00          # Send 10 USDC
remit tab open 0xCounterparty 100    # Open a tab with 100 USDC limit
remit tab charge <tab-id> 5.00       # Charge 5 USDC to the tab
remit wallet fund                    # Open fund link in browser
remit --testnet mint 100             # Mint 100 testnet USDC
```

## Commands

| Command | Description |
|---------|-------------|
| `remit init` | Create OWS wallet + policy + API key (or `--legacy` for raw key) |
| `remit wallet list` | List all OWS wallets |
| `remit wallet fund` | Open fund link in browser |
| `remit wallet set-policy` | Configure spending limits |
| `remit status` | Wallet status and balance |
| `remit balance` | USDC balance |
| `remit pay <to> <amount>` | One-time payment |
| `remit tab open/charge/close` | Tab (running balance) |
| `remit tab get <id>` | Show tab details |
| `remit tab list` | List open tabs |
| `remit stream open/close` | Streaming payments |
| `remit stream list` | List active streams |
| `remit escrow create/release/cancel/claim-start` | Escrow |
| `remit escrow list` | List escrows |
| `remit bounty post/submit/award` | Bounties |
| `remit bounty list` | List bounties |
| `remit deposit create` | Deposit address |
| `remit fund` | Generate fund link |
| `remit withdraw` | Generate withdraw link |
| `remit mint <amount>` | Mint testnet USDC (max 2500/hr) |
| `remit webhook create/list/delete` | Webhook subscriptions |
| `remit a2a discover/pay/card` | A2A agent discovery and payments |
| `remit config set/get/show` | Configuration |
| `remit completions <shell>` | Shell completions (bash, zsh, fish, powershell) |

## Global Flags

| Flag | Description |
|------|-------------|
| `--json` | Machine-readable JSON output |
| `--testnet` | Use Base Sepolia testnet |

## `init` Flags

| Flag | Description |
|------|-------------|
| `--name <NAME>` | Wallet name (default: `remit-{hostname}`) |
| `--chain <CHAIN>` | `base` or `base-sepolia` (default: `base`) |
| `--legacy` | Skip OWS, generate a raw private key instead |
| `--write-env` | (Legacy only) Write key to `.env` in current directory |

## `wallet set-policy` Flags

| Flag | Description |
|------|-------------|
| `--chain <CHAIN>` | `base` or `base-sepolia` (default: `base`) |
| `--max-tx <USDC>` | Per-transaction spending cap in dollars |
| `--daily-limit <USDC>` | Daily spending cap in dollars |

## `wallet fund` Flags

| Flag | Description |
|------|-------------|
| `--wallet <NAME>` | Wallet name or ID (default: `OWS_WALLET_ID` env var) |
| `--amount <USDC>` | Pre-fill fund amount |

## `pay` Flags

| Flag | Description |
|------|-------------|
| `--no-permit` | Skip EIP-2612 permit auto-signing (use existing on-chain approval instead) |
| `--memo <text>` | Attach a memo to the payment |

## Auth

### OWS (recommended)

```bash
remit init                            # one-time setup
export OWS_WALLET_ID=remit-my-agent   # wallet name from init
export OWS_API_KEY=<token>            # shown once by init
```

Keys live in `~/.ows/wallets/` (AES-256-GCM encrypted). Never appear in env vars.

### Raw key (legacy)

```bash
export REMITMD_KEY=0x<your-private-key>
```

Or `remit init --legacy --write-env` to generate and save to `.env`.

**Never commit private keys to git.**

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
