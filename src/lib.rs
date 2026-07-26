#![no_std]
//! Adapts a push-input AMM to the UniswapV2/Soroswap-shaped interface used by
//! the ConfiBatch pool. The adapter exposes pair metadata and forwards swaps to
//! the configured AMM.
//!
//! Routing occurs atomically: the pool transfers input to the adapter, invokes
//! `swap`, and receives output from the AMM.

use soroban_sdk::{
    contract, contractimpl, contracttype, token, vec, Address, Env, IntoVal, Symbol, Val, Vec,
};

const TTL_DAY: u32 = 17_280;
const TTL_BUMP: u32 = 30 * TTL_DAY;
const TTL_THRESH: u32 = 20 * TTL_DAY;

#[derive(Clone)]
#[contracttype]
enum DataKey {
    Amm,
    Token0,
    Token1,
    Recipient,
}

#[contract]
pub struct RouteAdapter;

#[cfg(not(test))]
#[contractimpl]
impl RouteAdapter {
    /// Fresh adapters are atomically bound to their AMM, tokens, and only valid
    /// recipient. There is no uninitialized contract address to front-run.
    pub fn __constructor(
        env: Env,
        amm: Address,
        token_0: Address,
        token_1: Address,
        recipient: Address,
    ) {
        initialize_adapter(&env, &amm, &token_0, &token_1, Some(&recipient));
    }
}

#[contractimpl]
impl RouteAdapter {
    /// Legacy unguarded initializer for an AMM whose token order matches
    /// `(token_0, token_1)`. New deployments should use the constructor or
    /// `init_guarded`.
    pub fn init(env: Env, amm: Address, token_0: Address, token_1: Address) {
        initialize_adapter(&env, &amm, &token_0, &token_1, None);
    }

    /// New deployments should pin the only valid swap recipient to the pool. This
    /// keeps accidental adapter balances from being swept to an arbitrary caller.
    pub fn init_guarded(
        env: Env,
        amm: Address,
        token_0: Address,
        token_1: Address,
        recipient: Address,
    ) {
        initialize_adapter(&env, &amm, &token_0, &token_1, Some(&recipient));
    }

    pub fn bump_ttl(env: Env) {
        bump_instance(&env);
    }

    pub fn token_0(env: Env) -> Address {
        env.storage().instance().get(&DataKey::Token0).unwrap()
    }
    pub fn token_1(env: Env) -> Address {
        env.storage().instance().get(&DataKey::Token1).unwrap()
    }

    /// Proxy the wrapped AMM's reserves, in (token_0, token_1) order.
    pub fn get_reserves(env: Env) -> (i128, i128) {
        let amm: Address = env.storage().instance().get(&DataKey::Amm).unwrap();
        env.invoke_contract(&amm, &Symbol::new(&env, "reserves"), Vec::new(&env))
    }

    /// UniswapV2-style swap entrypoint the pool calls. The pool has already
    /// transferred the input token to this adapter; the nonzero `*_out` marks the
    /// output side, so the other token is the input.
    pub fn swap(env: Env, amount_0_out: i128, amount_1_out: i128, to: Address) {
        if amount_0_out <= 0 && amount_1_out <= 0 {
            panic!("bad output shape");
        }
        if amount_0_out > 0 && amount_1_out > 0 {
            panic!("bad output shape");
        }
        if let Some(recipient) = env.storage().instance().get::<_, Address>(&DataKey::Recipient) {
            if to != recipient {
                panic!("bad recipient");
            }
        }
        let amm: Address = env.storage().instance().get(&DataKey::Amm).unwrap();
        let t0: Address = env.storage().instance().get(&DataKey::Token0).unwrap();
        let t1: Address = env.storage().instance().get(&DataKey::Token1).unwrap();
        let here = env.current_contract_address();

        let (token_in, min_out) = if amount_0_out > 0 {
            (t1, amount_0_out)
        } else {
            (t0, amount_1_out)
        };

        // Forward the received input before invoking the push-input AMM.
        let amount_in = token::TokenClient::new(&env, &token_in).balance(&here);
        if amount_in <= 0 {
            panic!("no input");
        }
        token::TokenClient::new(&env, &token_in).transfer(&here, &amm, &amount_in);
        let args: Vec<Val> = vec![
            &env,
            token_in.into_val(&env),
            amount_in.into_val(&env),
            min_out.into_val(&env),
            to.into_val(&env),
        ];
        let _: i128 = env.invoke_contract(&amm, &Symbol::new(&env, "swap"), args);
        bump_instance(&env);
    }
}

fn initialize_adapter(
    env: &Env,
    amm: &Address,
    token_0: &Address,
    token_1: &Address,
    recipient: Option<&Address>,
) {
    if env.storage().instance().has(&DataKey::Amm) {
        panic!("already initialized");
    }
    if token_0 == token_1 || *amm == env.current_contract_address() {
        panic!("invalid adapter configuration");
    }
    env.storage().instance().set(&DataKey::Amm, amm);
    env.storage().instance().set(&DataKey::Token0, token_0);
    env.storage().instance().set(&DataKey::Token1, token_1);
    if let Some(value) = recipient {
        env.storage().instance().set(&DataKey::Recipient, value);
    }
    bump_instance(env);
}

fn bump_instance(env: &Env) {
    env.storage().instance().extend_ttl(TTL_THRESH, TTL_BUMP);
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn swap_rejects_bad_output_shape() {
        let env = Env::default();
        let id = env.register(RouteAdapter, ());
        let client = RouteAdapterClient::new(&env, &id);
        let amm = Address::generate(&env);
        let t0 = Address::generate(&env);
        let t1 = Address::generate(&env);
        let to = Address::generate(&env);
        client.init(&amm, &t0, &t1);

        assert!(client.try_swap(&0, &0, &to).is_err());
        assert!(client.try_swap(&1, &1, &to).is_err());
    }

    #[test]
    fn guarded_swap_rejects_wrong_recipient() {
        let env = Env::default();
        let id = env.register(RouteAdapter, ());
        let client = RouteAdapterClient::new(&env, &id);
        let amm = Address::generate(&env);
        let t0 = Address::generate(&env);
        let t1 = Address::generate(&env);
        let recipient = Address::generate(&env);
        let attacker = Address::generate(&env);
        client.init_guarded(&amm, &t0, &t1, &recipient);

        assert!(client.try_swap(&1, &0, &attacker).is_err());
    }
}
