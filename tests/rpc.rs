use std::{fs::write, str::FromStr};

use near_client::{
    crypto::prelude::*,
    near_primitives_light::{types::Finality, views::AccessKeyView},
    prelude::*,
};

use near_primitives_core::types::AccountId;
use rand::{RngCore, SeedableRng};
use rand_chacha::ChaChaRng;
use reqwest::Url;
use serde_json::json;
use workspaces::{network::Sandbox, types::SecretKey, Worker};

// auxiliary structs and methods
fn near_client(worker: &Worker<Sandbox>) -> NearClient {
    let rpc_url = Url::parse(format!("http://localhost:{}", worker.rpc_port()).as_str()).unwrap();
    NearClient::new(rpc_url).unwrap()
}

async fn create_signer(
    worker: &Worker<Sandbox>,
    client: &NearClient,
    signer_acc_id: &AccountId,
) -> Signer {
    let sk = Ed25519SecretKey::try_from_bytes(&random_bits()).unwrap();
    let pk = Ed25519PublicKey::from(&sk);
    let keypair = Keypair::new(sk).to_string();
    let workspaces_sk = SecretKey::from_str(&keypair).unwrap();
    let _ = worker
        .create_tla(
            workspaces::AccountId::from_str(signer_acc_id).unwrap(),
            workspaces_sk,
        )
        .await
        .unwrap();

    let view_access_key = client
        .view_access_key(signer_acc_id, &pk, Finality::None)
        .await
        .unwrap();

    match view_access_key.result {
        ViewAccessKeyResult::Ok(AccessKeyView { nonce, .. }) => {
            Signer::from_secret_str(&keypair, signer_acc_id.clone(), nonce).unwrap()
        }
        ViewAccessKeyResult::Err { error, .. } => panic!("{error}"),
    }
}

async fn download_contract() -> Vec<u8> {
    let target = "https://github.com/near-examples/FT/raw/master/res/fungible_token.wasm";
    let target_path = temp_dir().into_path();
    let fname = "contract.wasm";
    let full_dest = format!("{}/{}", target_path.to_string_lossy(), fname);

    let contract_bytes = reqwest::get(target).await.unwrap().bytes().await.unwrap();
    write(full_dest, &contract_bytes).unwrap();
    contract_bytes.to_vec()
}

async fn clone_and_compile_wasm() -> Vec<u8> {
    let target_path = format!(
        "{}/tests/test-contract",
        std::env::current_dir().unwrap().display()
    );

    workspaces::compile_project(target_path.as_str())
        .await
        .unwrap()
}

fn random_bits() -> [u8; ED25519_SECRET_KEY_LENGTH] {
    let mut chacha = ChaChaRng::from_entropy();
    let mut secret_bytes = [0_u8; ED25519_SECRET_KEY_LENGTH];
    chacha.fill_bytes(&mut secret_bytes);
    secret_bytes
}

// tests themselves
#[tokio::test]
async fn contract_creation() {
    let worker = workspaces::sandbox().await.unwrap();
    let client = near_client(&worker);
    let signer_account_id = AccountId::from_str("alice.test.near").unwrap();
    let signer = create_signer(&worker, &client, &signer_account_id).await;
    let wasm = download_contract().await;

    client
        .deploy_contract(&signer, &signer_account_id, wasm)
        .commit(Finality::None)
        .await
        .unwrap();
}

#[tokio::test]
async fn contract_function_call() {
    let worker = workspaces::sandbox().await.unwrap();
    let client = near_client(&worker);
    let signer_account_id = AccountId::from_str("alice.test.near").unwrap();
    let signer = create_signer(&worker, &client, &signer_account_id).await;
    let wasm = download_contract().await;

    client
        .deploy_contract(&signer, &signer_account_id, wasm)
        .commit(Finality::None)
        .await
        .unwrap();

    client
        .function_call(&signer, &signer_account_id, "new_default_meta")
        .args(json!({
            "owner_id": &signer_account_id,
            "total_supply": "100",
        }))
        .gas(near_units::parse_gas!("300 T") as u64)
        .commit(Finality::None)
        .await
        .unwrap();
}

#[tokio::test]
async fn contract_function_call_failed() {
    let worker = workspaces::sandbox().await.unwrap();
    let client = near_client(&worker);
    let signer_account_id = AccountId::from_str("alice.test.near").unwrap();
    let signer = create_signer(&worker, &client, &signer_account_id).await;
    let wasm = download_contract().await;

    client
        .deploy_contract(&signer, &signer_account_id, wasm)
        .commit(Finality::None)
        .await
        .unwrap();

    assert!(client
        .function_call(&signer, &signer_account_id, "new_default_meta")
        .args(json!({
            "owner_id": &signer_account_id,
            "total_suppl": "100",
        }))
        .gas(near_units::parse_gas!("300 T") as u64)
        .commit(Finality::None)
        .await
        .is_err());

    client
        .function_call(&signer, &signer_account_id, "new_default_meta")
        .args(json!({
            "owner_id": &signer_account_id,
            "total_supply": "100",
        }))
        .gas(near_units::parse_gas!("300 T") as u64)
        .commit(Finality::None)
        .await
        .unwrap();
}

#[tokio::test]
async fn multiple_tests() {
    let worker = workspaces::sandbox().await.unwrap();
    let client = near_client(&worker);
    let signer_account_id = AccountId::from_str("alice.test.near").unwrap();
    let signer = create_signer(&worker, &client, &signer_account_id).await;

    let wasm = clone_and_compile_wasm().await;

    init_contract(&client, &signer_account_id, &signer, wasm).await;
    fc_no_params(&client, &signer_account_id, &signer).await;
    fc_with_one_param_and_result(&client, &signer_account_id, &signer).await;
    fc_with_param_and_result(&client, &signer_account_id, &signer).await;
    view_no_params(&client, &signer_account_id).await;
    view_with_params(&client, &signer_account_id).await;
}

async fn init_contract(
    client: &NearClient,
    contract_id: &AccountId,
    signer: &Signer,
    wasm: Vec<u8>,
) {
    client
        .deploy_contract(signer, contract_id, wasm)
        .commit(Finality::None)
        .await
        .unwrap();
}

async fn view_no_params(client: &NearClient, contract_id: &AccountId) {
    client
        .view::<u64>(contract_id, Finality::None, "show_id", None)
        .await
        .unwrap();
}

async fn view_with_params(client: &NearClient, contract_id: &AccountId) {
    client
        .view::<String>(
            contract_id,
            Finality::None,
            "show_type",
            Some(json!({"is_message": true})),
        )
        .await
        .unwrap();
}

// fc = function call
async fn fc_no_params(client: &NearClient, contract_id: &AccountId, signer: &Signer) {
    client
        .function_call(signer, contract_id, "increment")
        .gas(near_units::parse_gas!("300 T") as u64)
        .commit(Finality::None)
        .await
        .unwrap();
}

async fn fc_with_one_param_and_result(
    client: &NearClient,
    contract_id: &AccountId,
    signer: &Signer,
) {
    let expected_result = "change message";
    let message = client
        .function_call(signer, contract_id, "change_message")
        .args(json!({ "message": expected_result }))
        .gas(near_units::parse_gas!("300 T") as u64)
        .commit(Finality::Final)
        .await
        .unwrap()
        .output::<String>()
        .unwrap();

    assert_eq!(message, expected_result);
}

async fn fc_with_param_and_result(client: &NearClient, contract_id: &AccountId, signer: &Signer) {
    let expected_id = 666u64;
    let id = client
        .function_call(signer, contract_id, "change_id")
        .args(json!({ "id": expected_id }))
        .gas(near_units::parse_gas!("300 T") as u64)
        .commit(Finality::Final)
        .await
        .unwrap()
        .output::<u64>()
        .unwrap();

    assert_eq!(id, expected_id);
}

#[tokio::test]
async fn async_transaction() {
    let worker = workspaces::sandbox().await.unwrap();
    let client = near_client(&worker);
    let signer_account_id = AccountId::from_str("alice.test.near").unwrap();
    let signer = create_signer(&worker, &client, &signer_account_id).await;

    let wasm = clone_and_compile_wasm().await;

    client
        .deploy_contract(&signer, &signer_account_id, wasm)
        .commit(Finality::None)
        .await
        .unwrap();

    let expected_result = "change message";
    let transaction_id = client
        .function_call(&signer, &signer_account_id, "change_message")
        .args(json!({ "message": expected_result }))
        .gas(near_units::parse_gas!("300 T") as u64)
        .commit_async(Finality::Final)
        .await
        .unwrap();

    let (tx, rx) = tokio::sync::oneshot::channel::<()>();

    tokio::spawn(async move {
        tokio::time::timeout(std::time::Duration::from_secs(3), rx)
            .await
            .expect("Wait async transaction timeout")
    });

    loop {
        let res = client.view_transaction(&transaction_id, &signer).await;

        if let Err(near_client::Error::ViewTransaction(_)) = &res {
            // try one more time
            continue;
        }

        // cancel timeout
        tx.send(()).unwrap();
        let msg = res.unwrap().output::<String>().unwrap();

        assert_eq!(msg, expected_result);
        break;
    }
}

#[tokio::test]
async fn view_access_key_success() {
    let worker = workspaces::sandbox().await.unwrap();
    let client = near_client(&worker);
    let signer_account_id = AccountId::from_str("alice.test.near").unwrap();
    let signer = create_signer(&worker, &client, &signer_account_id).await;

    let new_acc = AccountId::from_str("one.alice.test.near").unwrap();
    let secret_key = Ed25519SecretKey::try_from_bytes(&random_bits()).unwrap();
    let pk = Ed25519PublicKey::from(&secret_key);

    let _ = client
        .create_account(&signer, &new_acc, pk, near_units::parse_near!("3 N"))
        .commit(Finality::None)
        .await
        .unwrap()
        .output::<serde_json::Value>();

    let access_key = client
        .view_access_key(&new_acc, &pk, Finality::None)
        .await
        .unwrap();
    assert!(matches!(
        access_key,
        ViewAccessKey {
            result: ViewAccessKeyResult::Ok { .. },
            ..
        }
    ));
}

#[tokio::test]
async fn view_access_key_failure() {
    let worker = workspaces::sandbox().await.unwrap();
    let client = near_client(&worker);

    let new_acc = AccountId::from_str("one.alice.test.near").unwrap();
    let secret_key = Ed25519SecretKey::try_from_bytes(&random_bits()).unwrap();
    let pk = Ed25519PublicKey::from(&secret_key);

    let access_key = client
        .view_access_key(&new_acc, &pk, Finality::None)
        .await
        .unwrap();
    assert!(matches!(
        access_key,
        ViewAccessKey {
            result: ViewAccessKeyResult::Err { .. },
            ..
        }
    ));
}

#[tokio::test]
async fn create_account() {
    let worker = workspaces::sandbox().await.unwrap();
    let client = near_client(&worker);
    let signer_account_id = AccountId::from_str("alice.test.near").unwrap();
    let signer = create_signer(&worker, &client, &signer_account_id).await;

    let new_acc = AccountId::from_str("one.alice.test.near").unwrap();
    let secret_key = Ed25519SecretKey::try_from_bytes(&random_bits()).unwrap();
    let pk = Ed25519PublicKey::from(&secret_key);

    let _ = client
        .create_account(&signer, &new_acc, pk, near_units::parse_near!("3 N"))
        .commit(Finality::Final)
        .await
        .unwrap()
        .output::<serde_json::Value>();

    let access_key = client
        .view_access_key(&new_acc, &pk, Finality::None)
        .await
        .unwrap();
    assert!(matches!(
        access_key,
        ViewAccessKey {
            result: ViewAccessKeyResult::Ok { .. },
            ..
        }
    ));
}

#[tokio::test]
async fn delete_account() {
    let worker = workspaces::sandbox().await.unwrap();
    let client = near_client(&worker);
    let signer_account_id = AccountId::from_str("alice.test.near").unwrap();
    let signer = create_signer(&worker, &client, &signer_account_id).await;

    let new_acc = AccountId::from_str("one.alice.test.near").unwrap();
    let secret_key = Ed25519SecretKey::try_from_bytes(&random_bits()).unwrap();
    let pk = Ed25519PublicKey::from(&secret_key);

    client
        .create_account(&signer, &new_acc, pk, near_units::parse_near!("3 N"))
        .commit(Finality::Final)
        .await
        .unwrap();

    let access_key = client
        .view_access_key(&new_acc, &pk, Finality::None)
        .await
        .unwrap();

    let nonce = if let ViewAccessKey {
        result: ViewAccessKeyResult::Ok(AccessKeyView { nonce, .. }),
        ..
    } = access_key
    {
        nonce
    } else {
        panic!("Can't view access key for just created account")
    };

    let acc_signer = Signer::from_secret(secret_key, new_acc.clone(), nonce);

    client
        .delete_account(&acc_signer, &new_acc, &signer_account_id)
        .commit(Finality::Final)
        .await
        .unwrap();

    let access_key = client
        .view_access_key(&new_acc, &pk, Finality::None)
        .await
        .unwrap();
    assert!(matches!(
        access_key,
        ViewAccessKey {
            result: ViewAccessKeyResult::Err { .. },
            ..
        }
    ));
}

fn temp_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("near-client-test-")
        .tempdir()
        .unwrap()
}
