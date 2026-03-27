# remit-cli

> [Skill MD](https://remit.md) · [Docs](https://remit.md/docs) · [Agent Spec](https://remit.md/agent.md)

Command-line interface for [Remit](https://remit.md) — USDC payments for AI agents on Base.

## Install

### Homebrew (macOS / Linux)

```bash
brew install remit-md/tap/remit
```

### Install script (Linux / macOS)

```bash
curl -fsSL https://remit.md/install.sh | sh
```

### Scoop (Windows)

```powershell
scoop bucket add remit https://github.com/remit-md/scoop-bucket
scoop install remit
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

### With CLI signer (default, recommended)

The CLI signer uses an encrypted keystore at `~/.remit/keys/`. Private keys are AES-256-GCM encrypted at rest with scrypt KDF. SDKs call `remit sign` as a subprocess — no HTTP server, no ports, no network.

```bash
remit signer init                 # generates wallet + encrypted keystore
export REMIT_KEY_PASSWORD=your-password
```

### With OWS

[OWS](https://openwallet.sh) encrypts keys locally and evaluates spending policies before every signature.

```bash
remit init --ows                  # creates OWS wallet + policy + API key
export OWS_WALLET_ID=remit-my-agent
export OWS_API_KEY=<shown by init>
```

### With a raw private key (legacy)

```bash
remit init --legacy --write-env   # generates key, saves to .env
# or
export REMITMD_KEY=0x<your-private-key>
```

## Wallet Modes

The CLI supports three wallet modes. It checks them in priority order:

| Priority | Mode | Env Var | Description |
|----------|------|---------|-------------|
| 1 | CLI signer | `REMIT_KEY_PASSWORD` | Encrypted keystore. SDKs invoke `remit sign` via subprocess. |
| 2 | OWS | `OWS_WALLET_ID` | Encrypted local vault with spending policies. Requires OWS binary. |
| 3 | Raw key | `REMITMD_KEY` | Private key in environment. Simple but less secure. |

If a keystore exists at `~/.remit/keys/default.enc` and `REMIT_KEY_PASSWORD` is set, the CLI signs in-process using the encrypted key. Otherwise it falls back to OWS, then raw key.

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
| `remit init` | Create CLI signer wallet (default), OWS (`--ows`), or raw key (`--legacy`) |
| `remit signer init` | Generate new wallet + encrypted keystore |
| `remit signer import --key 0x...` | Import an existing private key into keystore |
| `remit signer migrate` | Migrate V24 keystore (token-based) to V25 (password-based) |
| `remit sign --eip712` | Sign EIP-712 typed data from stdin (used by SDKs) |
| `remit sign --digest` | Sign raw 32-byte digest from stdin (used by SDKs) |
| `remit address` | Print wallet address from keystore (no password needed) |
| `remit update` | Self-update to latest version |
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
| `--ows` | Use OWS wallet instead of CLI signer |
| `--legacy` | Skip signer/OWS, generate a raw private key instead |
| `--write-env` | (Legacy only) Write key to `.env` in current directory |

## `sign` Flags

| Flag | Description |
|------|-------------|
| `--eip712` | Sign EIP-712 typed data (JSON on stdin) |
| `--digest` | Sign raw 32-byte digest (hex on stdin) |
| `--keystore <PATH>` | Keystore path (default: `~/.remit/keys/default.enc`) |
| `--password-file <PATH>` | Read password from file instead of `REMIT_KEY_PASSWORD` |

## Auth

### CLI signer (default, recommended)

```bash
remit signer init                     # one-time setup — generates encrypted keystore
export REMIT_KEY_PASSWORD=your-password
```

Keys are AES-256-GCM encrypted at `~/.remit/keys/default.enc`. The CLI decrypts in-process when signing. SDKs invoke `remit sign` as a subprocess — no HTTP server, no ports, no network exposure.

### OWS

```bash
remit init --ows                      # one-time setup
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
