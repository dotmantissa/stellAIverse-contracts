#![cfg(test)]
extern crate alloc;

use crate::{Oracle, OracleClient, RelayRequest};
use ed25519_dalek::{Signer, SigningKey};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::xdr::{self, Limited, Limits, WriteXdr};
use soroban_sdk::{
    contract, contractimpl, symbol_short, Address, BytesN, Env, Symbol, TryIntoVal, Val, Vec,
};

#[contract]
pub struct Receiver;

#[contractimpl]
impl Receiver {
    pub fn ping(env: Env, input: u32) -> u32 {
        env.storage().instance().set(&symbol_short!("last"), &input);
        input + 1
    }

    pub fn last(env: Env) -> Option<u32> {
        env.storage().instance().get(&symbol_short!("last"))
    }
}

fn build_signed_payload(
    env: &Env,
    oracle_contract: &Address,
    oracle_pubkey: &BytesN<32>,
    target_contract: &Address,
    function: &Symbol,
    args: &Vec<Val>,
    nonce: u64,
    deadline: u64,
    signing_key: &SigningKey,
) -> BytesN<64> {
    let req = RelayRequest {
        relay_contract: oracle_contract.clone(),
        oracle_pubkey: oracle_pubkey.clone(),
        target_contract: target_contract.clone(),
        function: function.clone(),
        args: args.clone(),
        nonce,
        deadline,
    };

    let scval: xdr::ScVal = req.try_into().unwrap();
    let mut buf: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
    scval
        .write_xdr(&mut Limited::new(&mut buf, Limits::none()))
        .unwrap();

    let sig = signing_key.sign(&buf);
    BytesN::from_array(env, &sig.to_bytes())
}

fn setup() -> (
    Env,
    OracleClient<'static>,
    Address,
    BytesN<32>,
    SigningKey,
    Address,
) {
    let env = Env::default();
    env.mock_all_auths();

    let oracle_contract_id = env.register_contract(None, Oracle);
    let oracle_client = OracleClient::new(&env, &oracle_contract_id);
    let admin = Address::generate(&env);
    oracle_client.init_contract(&admin);

    let receiver_id = env.register_contract(None, Receiver);

    let sk = SigningKey::from_bytes(&[7u8; 32]);
    let pk_bytes: [u8; 32] = sk.verifying_key().to_bytes();
    let pk = BytesN::from_array(&env, &pk_bytes);

    (env, oracle_client, admin, pk, sk, receiver_id)
}

#[test]
fn test_relay_signed_success_forwards_payload() {
    let (env, oracle, admin, pk, sk, receiver_id) = setup();
    oracle.register_oracle_key(&admin, &pk);

    let function = Symbol::new(&env, "ping");
    let args: Vec<Val> = (123u32,).try_into_val(&env).unwrap();
    let nonce = 1u64;
    let deadline = env.ledger().timestamp() + 100;
    let signature = build_signed_payload(
        &env,
        &oracle.address,
        &pk,
        &receiver_id,
        &function,
        &args,
        nonce,
        deadline,
        &sk,
    );

    let res = oracle.relay_signed(
        &pk,
        &receiver_id,
        &function,
        &args,
        &nonce,
        &deadline,
        &signature,
    );
    let res_u32: u32 = res.try_into_val(&env).unwrap();
    assert_eq!(res_u32, 124);

    // Verify target contract state updated
    let last: Option<u32> =
        env.invoke_contract(&receiver_id, &Symbol::new(&env, "last"), Vec::new(&env));
    assert_eq!(last, Some(123));
}

#[test]
#[should_panic(expected = "Oracle not approved")]
fn test_relay_signed_rejects_unapproved_oracle() {
    let (env, oracle, _admin, pk, sk, receiver_id) = setup();

    let function = Symbol::new(&env, "ping");
    let args: Vec<Val> = (1u32,).try_into_val(&env).unwrap();
    let nonce = 1u64;
    let deadline = env.ledger().timestamp() + 100;
    let signature = build_signed_payload(
        &env,
        &oracle.address,
        &pk,
        &receiver_id,
        &function,
        &args,
        nonce,
        deadline,
        &sk,
    );

    oracle.relay_signed(
        &pk,
        &receiver_id,
        &function,
        &args,
        &nonce,
        &deadline,
        &signature,
    );
}

#[test]
#[should_panic]
fn test_relay_signed_rejects_bad_signature() {
    let (env, oracle, admin, pk, _sk, receiver_id) = setup();
    oracle.register_oracle_key(&admin, &pk);

    let function = Symbol::new(&env, "ping");
    let args: Vec<Val> = (1u32,).try_into_val(&env).unwrap();
    let nonce = 1u64;
    let deadline = env.ledger().timestamp() + 100;

    // Wrong signature
    let signature = BytesN::from_array(&env, &[0u8; 64]);
    oracle.relay_signed(
        &pk,
        &receiver_id,
        &function,
        &args,
        &nonce,
        &deadline,
        &signature,
    );
}

#[test]
#[should_panic(expected = "Invalid nonce: replay protection triggered")]
fn test_relay_signed_prevents_replay() {
    let (env, oracle, admin, pk, sk, receiver_id) = setup();
    oracle.register_oracle_key(&admin, &pk);

    let function = Symbol::new(&env, "ping");
    let args: Vec<Val> = (5u32,).try_into_val(&env).unwrap();
    let nonce = 1u64;
    let deadline = env.ledger().timestamp() + 100;
    let signature = build_signed_payload(
        &env,
        &oracle.address,
        &pk,
        &receiver_id,
        &function,
        &args,
        nonce,
        deadline,
        &sk,
    );

    oracle.relay_signed(
        &pk,
        &receiver_id,
        &function,
        &args,
        &nonce,
        &deadline,
        &signature,
    );
    oracle.relay_signed(
        &pk,
        &receiver_id,
        &function,
        &args,
        &nonce,
        &deadline,
        &signature,
    );
}

#[test]
#[should_panic(expected = "Signature expired")]
fn test_relay_signed_rejects_expired_deadline() {
    let (env, oracle, admin, pk, sk, receiver_id) = setup();
    oracle.register_oracle_key(&admin, &pk);

    let function = Symbol::new(&env, "ping");
    let args: Vec<Val> = (1u32,).try_into_val(&env).unwrap();
    let nonce = 1u64;
    let deadline = env.ledger().timestamp();
    let signature = build_signed_payload(
        &env,
        &oracle.address,
        &pk,
        &receiver_id,
        &function,
        &args,
        nonce,
        deadline,
        &sk,
    );

    // Move ledger time forward
    env.ledger().set_timestamp(deadline + 1);
    oracle.relay_signed(
        &pk,
        &receiver_id,
        &function,
        &args,
        &nonce,
        &deadline,
        &signature,
    );
}
