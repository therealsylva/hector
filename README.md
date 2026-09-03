# Hector

Hector is a Linux-first Rust terminal for low-latency SportyBet market data and guarded bet execution. It combines an interactive REPL, rich terminal output, and a full-screen realtime monitor with an automation-safe CLI.

It speaks the same public REST, Engine.IO 3 / Socket.IO 2, and protected transaction protocols as the deployed Nigerian web client. Authentication is session-import only: Hector never asks for or stores an account password.

> Hector is an unofficial client. Endpoints can change without notice. Use it only where permitted and within the rules that apply to your account and location.

## What works

- Public sports, fixtures, events, market groups, and outcomes
- Public realtime GROUP, MULTI, and SPECIAL subscriptions
- Engine.IO heartbeat handling and Base64 push-body decoding
- Cookie-backed session and balance validation
- RSA/AES transaction-cipher negotiation
- Dry-run-first single-bet construction and encrypted execution
- Fixed-point stake arithmetic with no float rounding
- Durable pending/confirmed/rejected/ambiguous attempt journal
- Strict Rust CI and tagged Linux release packaging
- Persistent command history, completion, guided topic and order flows
- Adaptive tables and a full-screen realtime TUI

## Interactive terminal

Launch Hector without a subcommand:

```bash
hector
```

The terminal opens in public mode when no browser session is configured. Public sports and market data remain available; account and execution commands explain when an imported session is required.

```text
hector › sports
hector › events sportId=sr:sport:1 timeline=0
hector › event sr:match:123456
hector › markets sr:match:123456
hector › outcomes sr:match:123456 productId=3
hector › topic
hector › watch '1^1^sr:tournament:17^sr:match:123456^3^1^~'
hector › bet
hector › orders
```

`topic` and `bet` start guided flows. Live execution still passes through all three CLI guards, shows the maximum loss, defaults to cancellation, and requires typing the exact final confirmation phrase. `watch` opens the full-screen realtime monitor; press Space to pause and `q`, Escape, or Ctrl-C to return to the REPL.

Use `--plain`, `--color never`, or the standard `NO_COLOR` environment variable when rich output is undesirable. `--json` always emits undecorated machine-readable output, and `hector stream` retains its JSON Lines contract outside the interactive shell.

## Build

Rust is pinned through `rust-toolchain.toml`.

```bash
git clone https://github.com/therealsylva/hector.git
cd hector
cargo build --release --locked
install -Dm755 target/release/hector ~/.local/bin/hector
```

## Configure a browser session

Log in through the website, open the browser Network panel, and filter for `userBal`. Select the authenticated request ending in `/api/ng/pocket/v1/finAccs/finAcc/userBal/NGN`, then copy its complete `Cookie` request-header value. Cookies copied from CMS or static-asset requests are incomplete because the account credentials are scoped to `/api/ng`.

Copy `.env.example` to a private file, replace the placeholder with that `Cookie` value, then export it into the shell:

```bash
cp .env.example .env
chmod 600 .env
set -a
. ./.env
set +a
hector session check
```

Do not put a username, password, or OTP in this file. Never commit `.env`; it is ignored by Git.

| Variable | Purpose | Default |
| --- | --- | --- |
| `SPORTYBET_COOKIE` | Full imported browser `Cookie` header | Required for account/order calls |
| `SPORTYBET_DEVICE_ID` | Browser device ID used by HTTP and realtime registration | Generated for realtime if absent |
| `SPORTYBET_FINGERPRINT` | Optional browser fingerprint header | Unset |
| `SPORTYBET_BASE_URL` | Region API root | `https://www.sportybet.com/api/ng/` |
| `SPORTYBET_SOCKET_URL` | Engine.IO WebSocket endpoint | `wss://alive-ng.sportybet.com/socket.io/?EIO=3&transport=websocket` |
| `SPORTYBET_CURRENCY` | Balance currency | `NGN` |
| `SPORTYBET_OPER_ID` | Operator header | `2` |
| `SPORTYBET_CLIENT_ID` | Deployed web-client identifier | `web` |
| `SPORTYBET_PLATFORM` | Deployed web platform header | `web` |
| `SPORTYBET_LOCALE` | Accept-Language value | `en` |
| `HECTOR_TIMEOUT_MS` | HTTP timeout in milliseconds | `10000` |
| `HECTOR_JOURNAL_PATH` | Append-only order journal | XDG/user state directory |

## Market data

Every public endpoint accepts repeatable passthrough query parameters, so upstream frontend changes do not require a new Hector release just to add a filter.

```bash
hector market sports

hector --json market events \
  --param sportId=sr:sport:1 \
  --param timeline=0

hector market event --param eventId=sr:match:123456
hector market market-groups --param eventId=sr:match:123456
hector market outcomes --param eventId=sr:match:123456 --param marketId=1
```

## Realtime feed

Build a validated topic from its fields when you do not already have the raw value:

```bash
hector topic market \
  --sport-id 1 \
  --category-id 1 \
  --tournament-id sr:tournament:17 \
  --event-id sr:match:123456 \
  --product-id 3 \
  --market-id 18 \
  --specifier total=2.5
```

Pass the resulting caret-separated topic to the feed. Repeat `--topic` to multiplex subscriptions on one connection.

```bash
hector stream \
  --topic '1^1^sr:tournament:17^sr:match:123456^3^1^~'

hector stream \
  --topic '1^1^sr:tournament:17^sr:match:123456^3^1^~' \
  --topic '1^1^sr:tournament:17^sr:match:123456^3^18^total=2.5'
```

Decoded pushes are emitted as one compact JSON object per line. Use `--raw` to inspect the original Engine.IO frames. A market topic contains:

```text
sportId^categoryId^tournamentId^eventId^productId^marketId^marketSpecifiers
```

`~` is the upstream wildcard/empty marker. MULTI subscriptions require `--account-id`.

The stream reconnects and resubscribes after transport failures, using exponential backoff from 250 ms to 10 seconds. Tune that with `--initial-backoff-ms` and `--max-backoff-ms`, or use `--no-reconnect` for one-shot diagnostics.

## Bet dry-run and execution

Without `--execute`, the command performs no authenticated request and prints the exact order payload:

```bash
hector --json bet single \
  --event-id sr:match:123456 \
  --sport-id sr:sport:1 \
  --product-id 3 \
  --market-id 1 \
  --outcome-id 1 \
  --odds 2.10 \
  --probability 0.48 \
  --stake 25.50 \
  --payment-type 0
```

Execution needs all three guards. `--max-stake` is independently parsed and checked before the cipher bootstrap:

```bash
hector --json bet single \
  --event-id sr:match:123456 \
  --sport-id sr:sport:1 \
  --product-id 3 \
  --market-id 1 \
  --outcome-id 1 \
  --odds 2.10 \
  --probability 0.48 \
  --stake 25.50 \
  --payment-type 0 \
  --execute \
  --confirm-order \
  --max-stake 25.50
```

Hector submits an order at most once. If the connection fails after submission begins, the result is marked `ambiguous`; do not retry automatically. Reconcile the attempt against Bet History first.

```bash
hector orders journal --limit 20
```

## Development

```bash
cargo fmt --all -- --check
cargo test --locked --all-targets
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo build --release --locked
```

Every pull request builds an optimized Linux x86_64 binary and uploads a checksummed archive as a GitHub Actions artifact. Version tags publish the same form of artifact as a GitHub release.

The recovered wire contract is documented in [`docs/protocol.md`](docs/protocol.md).
