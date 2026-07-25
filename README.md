# route_adapter

`route_adapter` presents a Soroswap/Uniswap-V2-shaped pair interface to
`confibatch_pool` while forwarding execution to a push-input AMM such as
`mini_amm`.

This repository is the standalone build of the latest contract implementation.
It targets Soroban SDK `25.3.2`, Rust `1.91`, and Apache-2.0.

> Status: testnet/reference adapter. The contract intentionally has a narrow
> interface and should be configured for one AMM, one ordered token pair, and
> one permitted pool recipient.

## Why the adapter exists

The pool's low-level route expects a pair with:

```text
token_0() -> Address
get_reserves() -> (i128, i128)
swap(amount_0_out, amount_1_out, to)
```

`mini_amm` instead exposes:

```text
reserves() -> (i128, i128)
swap(token_in, amount_in, amount_out_min, to) -> i128
```

The adapter translates between those interfaces without changing the shielded
pool.

## Routing flow

```text
1. confibatch_pool transfers token_in to route_adapter
2. confibatch_pool calls route_adapter.swap(...)
3. route_adapter reads its complete token_in balance
4. route_adapter transfers that balance to mini_amm
5. route_adapter calls mini_amm.swap(..., to = confibatch_pool)
6. mini_amm transfers the output directly to confibatch_pool
```

All steps are intended to occur inside one Soroban transaction/invocation tree.
Contracts authorize movement of their own balances.

## Contract interface

| Function | Authorization | Description |
| --- | --- | --- |
| `__constructor(amm, token_0, token_1, recipient)` | deployment | Atomically binds a fresh adapter to its AMM, ordered token pair, and only permitted output recipient. |
| `init(amm, token_0, token_1)` | first call only | Legacy unguarded initializer. It leaves `to` unrestricted and should not be used for new deployments. |
| `init_guarded(amm, token_0, token_1, recipient)` | first call only | Legacy guarded initializer. |
| `bump_ttl()` | none | Permissionless instance-storage TTL keeper. |
| `token_0()` | none | Returns the configured token-0 SAC address. |
| `token_1()` | none | Returns the configured token-1 SAC address. |
| `get_reserves()` | none | Calls the wrapped AMM's `reserves()` and returns values in `(token_0, token_1)` order. |
| `swap(amount_0_out, amount_1_out, to)` | pushed funds | Forwards the adapter's input balance to the AMM and requests the selected output. |

The adapter uses panics for invalid pair-shaped calls rather than a
contract-specific error enum.

## Configuration invariants

- `token_0` and `token_1` must be different.
- `amm` cannot be the adapter's own address.
- Initialization is one-time.
- The wrapped AMM must use the same token ordering:
  `token_a = token_0`, `token_b = token_1`.
- New deployments should set `recipient` to the exact `confibatch_pool`
  contract address.

The constructor is the preferred deployment path because it closes the
deploy-to-init takeover window.

## Swap semantics

Exactly one output must be positive:

- `amount_0_out > 0`, `amount_1_out <= 0`: token 1 is input and token 0 is output.
- `amount_1_out > 0`, `amount_0_out <= 0`: token 0 is input and token 1 is output.

Both zero/non-positive or both positive are rejected as `bad output shape`.

For guarded adapters, `to` must equal the configured recipient. A mismatch is
rejected as `bad recipient`.

The adapter forwards its entire balance of the inferred input token. If that
balance is not positive, the call is rejected as `no input`. The nonzero output
argument becomes `amount_out_min` in the wrapped AMM call; the actual output is
calculated by the AMM.

### Important balance rule

Do not transfer tokens to the adapter in a separate, unattended transaction.
Because the adapter forwards its complete input-token balance, a later valid
caller could route tokens that were left behind. Compose the input transfer and
`swap` call atomically.

## Storage and TTL

Instance storage contains:

- wrapped AMM address;
- token-0 address;
- token-1 address; and
- optional guarded recipient.

Initialization and successful swaps renew the instance TTL. `bump_ttl` is a
permissionless keeper for idle routes. The implementation bumps to roughly 30
days when TTL falls below roughly 20 days, assuming about 17,280 ledgers per
day.

## Security properties and limitations

- A guarded deployment prevents arbitrary output redirection.
- The adapter never chooses the price; it forwards the minimum output to the
  wrapped AMM.
- The pool should separately allowlist the adapter as its configured route.
- The adapter assumes the wrapped AMM's `reserves` and `swap` methods are honest
  and ABI-compatible.
- There is no admin, upgrade function, or reconfiguration function. A new route
  requires a new adapter deployment.
- `init` without a recipient is retained for compatibility only; it allows any
  `to` address and is weaker than the constructor/`init_guarded` configuration.
- The contract emits no custom events. Routing evidence comes from the parent
  transaction, token transfers, and pool/AMM events or state.

## Build

Prerequisites:

- Rust `1.91` with the `wasm32v1-none` target;
- Stellar CLI compatible with Soroban SDK 25; and
- network access for the first Cargo dependency fetch.

```bash
rustup target add wasm32v1-none
stellar contract build
```

The optimized Wasm is produced under:

```text
target/wasm32v1-none/release/route_adapter.wasm
```

## Deployment outline

Deploy the AMM and pool first so their addresses are known, then deploy the
adapter:

```bash
stellar contract build

stellar contract deploy \
  --wasm target/wasm32v1-none/release/route_adapter.wasm \
  --source-account <DEPLOYER> \
  --network <NETWORK> \
  -- \
  --amm <MINI_AMM_CONTRACT> \
  --token_0 <TOKEN_0_SAC> \
  --token_1 <TOKEN_1_SAC> \
  --recipient <CONFIBATCH_POOL_CONTRACT>
```

Exact CLI argument spelling can vary with the installed Stellar CLI. Inspect
the contract specification first:

```bash
stellar contract info interface \
  --wasm target/wasm32v1-none/release/route_adapter.wasm
```

Finally call the pool's `set_route(adapter, token_0, token_1)` and verify:

- `token_0()` and `token_1()` match the registered SACs;
- `get_reserves()` matches the underlying AMM;
- the pool's `route_config()` points to this adapter; and
- a small atomic push-and-swap succeeds with the output returning to the pool.

## Repository contents

```text
src/lib.rs       Contract and generated client
Cargo.toml       Standalone Soroban build configuration
Cargo.lock       Reproducible dependency lock
```

## Security and license

Report vulnerabilities privately as described in [SECURITY.md](SECURITY.md).
The source is licensed under the [Apache License 2.0](LICENSE).
