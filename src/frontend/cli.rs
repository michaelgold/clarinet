use std::env;
use std::fs::{self, File};
use std::collections::{HashMap, BTreeMap};
use std::io::{prelude::*, BufReader, Read};

use crate::{generators::{self, changes::{Changes, TOMLEdition}}, utils::mnemonic};
use crate::types::{MainConfig, MainConfigFile, LinkConfig};
use crate::console::load_session;
use crate::test::run_tests;

use clarity_repl::{clarity::{codec::{StacksString, transaction::{RecoverableSignature, SinglesigHashMode, SinglesigSpendingCondition, TransactionVersion}}, util::{StacksAddress, address::AddressHashMode, secp256k1::{Secp256k1PrivateKey, Secp256k1PublicKey}}}, repl};
use clarity_repl::clarity::codec::transaction::{StacksTransaction, TransactionAnchorMode, TransactionSmartContract, TransactionSpendingCondition, TransactionAuth, TransactionPostConditionMode, TransactionPayload, TransactionPublicKeyEncoding, StacksTransactionSigner};
use clarity_repl::clarity::codec::StacksMessageCodec;

use clap::Clap;
use secp256k1::{PublicKey, SecretKey};
use tiny_hderive::bip32::ExtendedPrivKey;
use toml;

#[derive(Clap)]
#[clap(version = "1.0")]
struct Opts {
    #[clap(subcommand)]
    command: Command,
}

#[derive(Clap)]
enum Command {
    /// New subcommand
    #[clap(name = "new")]
    New(GenerateProject),
    /// Contract subcommand
    #[clap(name = "contract")]
    Contract(Contract),
    /// Console subcommand
    #[clap(name = "console")]
    Console(Console),
    /// Test subcommand
    #[clap(name = "test")]
    Test(Test),
    /// Check subcommand
    #[clap(name = "check")]
    Check(Check),
    /// Deploy subcommand
    #[clap(name = "deploy")]
    Deploy(Deploy),
}

#[derive(Clap)]
enum Contract {
    /// New contract subcommand
    #[clap(name = "new")]
    NewContract(NewContract),
    /// Import contract subcommand
    #[clap(name = "link")]
    LinkContract(LinkContract),
    /// Fork contract subcommand
    #[clap(name = "fork")]
    ForkContract(ForkContract),
}

#[derive(Clap)]
struct GenerateProject {
    /// Project's name
    pub name: String,
    /// Print debug info
    #[clap(short = 'd')]
    pub debug: bool,
}

#[derive(Clap)]
struct NewContract {
    /// Contract's name
    pub name: String,
    /// Print debug info
    #[clap(short = 'd')]
    pub debug: bool,
}

#[derive(Clap)]
struct LinkContract {
    /// Contract id
    pub contract_id: String,
    /// Print debug info
    #[clap(short = 'd')]
    pub debug: bool,
}

#[derive(Clap, Debug)]
struct ForkContract {
    /// Contract id
    pub contract_id: String,
    /// Print debug info
    #[clap(short = 'd')]
    pub debug: bool,
    // /// Fork contract and all its dependencies
    // #[clap(short = 'r')]
    // pub recursive: bool,
}

#[derive(Clap)]
struct Console {
    /// Print debug info
    #[clap(short = 'd')]
    pub debug: bool,
}

#[derive(Clap)]
struct Test {
    /// Print debug info
    #[clap(short = 'd')]
    pub debug: bool,
    pub files: Vec<String>,
}

#[derive(Clap)]
struct Deploy {
    /// Print debug info
    #[clap(short = 'd')]
    pub debug: bool,
    /// Deploy contracts on mocknet, using settings/Mocknet.toml
    #[clap(long = "mocknet", conflicts_with = "testnet")]
    pub mocknet: bool,
    /// Deploy contracts on mocknet, using settings/Testnet.toml
    #[clap(long = "testnet", conflicts_with = "mocknet")]
    pub testnet: bool,
}

#[derive(Clap)]
struct Check {
    /// Print debug info
    #[clap(short = 'd')]
    pub debug: bool,
}

pub fn main() {
    let opts: Opts = Opts::parse();

    let current_path = {
        let current_dir = env::current_dir().expect("Unable to read current directory");
        current_dir.to_str().unwrap().to_owned()
    };

    match opts.command {
        Command::New(project_opts) => {
            let changes = generators::get_changes_for_new_project(current_path, project_opts.name);
            execute_changes(changes);
        }
        Command::Contract(subcommand) => match subcommand {
            Contract::NewContract(new_contract) => {
                let changes =
                    generators::get_changes_for_new_contract(current_path, new_contract.name, None, true, vec![]);
                execute_changes(changes);
            }
            Contract::LinkContract(link_contract) => {
                let path = format!("{}/Clarinet.toml", current_path);

                let change = TOMLEdition {
                    comment: format!("Indexing link {} in Clarinet.toml", link_contract.contract_id),
                    path,
                    contracts_to_add: HashMap::new(),
                    links_to_add: vec![LinkConfig {
                        contract_id: link_contract.contract_id.clone(),
                    }],
                };
                execute_changes(vec![Changes::EditTOML(change)]);
            }
            Contract::ForkContract(fork_contract) => {
                let path = format!("{}/Clarinet.toml", current_path);

                println!("Resolving {} and its dependencies...", fork_contract.contract_id);

                let settings = repl::SessionSettings::default();
                let mut session = repl::Session::new(settings);

                let res = session.resolve_link(&repl::settings::InitialLink {
                    contract_id: fork_contract.contract_id.clone(),
                    stacks_node_addr: None,
                    cache: None,
                });
                let contracts = res.unwrap();
                let mut changes = vec![];
                for (contract_id, code, deps) in contracts.into_iter() {
                    let components: Vec<&str> = contract_id.split('.').collect();
                    let contract_name = components.last().unwrap();

                    if &contract_id == &fork_contract.contract_id {
                        let mut change_set =
                            generators::get_changes_for_new_contract(current_path.clone(), contract_name.to_string(), Some(code), false, vec![]);
                        changes.append(&mut change_set);

                        for dep in deps.iter() {
                            let mut change_set =
                                generators::get_changes_for_new_link(path.clone(), dep.clone(), None);
                            changes.append(&mut change_set);
                        }
                    }
                }
                execute_changes(changes);
            }
        },
        Command::Console(_) => {
            let start_repl = true;
            load_session(start_repl, "development".into()).expect("Unable to start REPL");
        },
        Command::Check(_) => {
            let start_repl = false;
            let res = load_session(start_repl, "development".into());
            if let Err(e) = res {
                println!("{}", e);
                return;
            }
        },
        Command::Test(test) => {
            let start_repl = false;
            let res = load_session(start_repl, "development".into());
            if let Err(e) = res {
                println!("{}", e);
                return;
            }
            run_tests(test.files);
        },
        Command::Deploy(deploy) => {
            let start_repl = false;
            let mode = if deploy.mocknet == true {
                "mocknet"
            } else if deploy.testnet == true {
                "testnet"
            } else {
                panic!("Target deployment must be specified with --mocknet or --testnet")
            };
            let res = load_session(start_repl, mode.into());
            if let Err(e) = res {
                println!("{}", e);
                return;
            }
            let settings = res.unwrap();

            let mut deployers_nonces = BTreeMap::new();
            let mut deployers_lookup = BTreeMap::new();
            for account in settings.initial_accounts.iter() {
                if account.name == "deployer" {
                    deployers_lookup.insert("*", account.clone());
                }
            }

            #[derive(Deserialize, Debug)]
            struct Balance {
                balance: String,
                nonce: u64,
                balance_proof: String,
                nonce_proof: String,               
            }

            for initial_contract in settings.initial_contracts.iter() {
                let contract_name = initial_contract.name.clone().unwrap();
                let host = "http://localhost:20443";

                let payload = TransactionSmartContract {
                    name: contract_name.as_str().into(),
                    code_body: StacksString::from_string(&initial_contract.code).unwrap()
                };

                let deployer = match deployers_lookup.get(contract_name.as_str()) {
                    Some(deployer) => deployer,
                    None => deployers_lookup.get("*").unwrap()
                };

                let bip39_seed = match mnemonic::get_bip39_seed_from_mnemonic(&deployer.mnemonic, "") {
                    Ok(bip39_seed) => bip39_seed,
                    Err(_) => panic!(),
                };
                let ext = ExtendedPrivKey::derive(&bip39_seed[..], deployer.derivation.as_str()).unwrap();
                let secret_key = SecretKey::parse_slice(&ext.secret()).unwrap();
                let public_key = PublicKey::from_secret_key(&secret_key);
                
                let wrapped_public_key = Secp256k1PublicKey::from_slice(&public_key.serialize_compressed()).unwrap();
                let wrapped_secret_key = Secp256k1PrivateKey::from_slice(&ext.secret()).unwrap();

                let anchor_mode = TransactionAnchorMode::Any;
                let tx_fee = 200 + initial_contract.code.len() as u64;

                let nonce = match deployers_nonces.get(&deployer.name) {
                    Some(nonce) => {
                        *nonce
                    },
                    None => {
                        let request_url = format!(
                            "{host}/v2/accounts/{addr}",
                            host = host,
                            addr = deployer.address,
                        );
                
                        let response: Balance = reqwest::blocking::get(&request_url)
                            .expect("Unable to retrieve account")
                            .json()
                            .expect("Unable to parse contract");
                        let nonce = response.nonce;
                        deployers_nonces.insert(deployer.name.clone(), nonce);
                        nonce
                    }
                };

                let signer_addr = StacksAddress::from_public_keys(0, &AddressHashMode::SerializeP2PKH, 1, &vec![wrapped_public_key]).unwrap();
        
                let spending_condition = TransactionSpendingCondition::Singlesig(
                    SinglesigSpendingCondition {
                        signer: signer_addr.bytes.clone(),
                        nonce: nonce,
                        tx_fee: tx_fee,
                        hash_mode: SinglesigHashMode::P2PKH,
                        key_encoding: TransactionPublicKeyEncoding::Compressed,
                        signature: RecoverableSignature::empty(),
                    },
                );

                let auth = TransactionAuth::Standard(spending_condition);
                let unsigned_tx = StacksTransaction {
                    version: TransactionVersion::Testnet,
                    chain_id: 0x80000000, // MAINNET=0x00000001
                    auth: auth,
                    anchor_mode: anchor_mode,
                    post_condition_mode: TransactionPostConditionMode::Deny,
                    post_conditions: vec![],
                    payload: TransactionPayload::SmartContract(payload),
                };
            
                let mut unsigned_tx_bytes = vec![];
                unsigned_tx
                    .consensus_serialize(&mut unsigned_tx_bytes)
                    .expect("FATAL: invalid transaction");

                let mut tx_signer = StacksTransactionSigner::new(&unsigned_tx);
                tx_signer.sign_origin(&wrapped_secret_key).unwrap();
                let signed_tx = tx_signer.get_tx().unwrap();

                let tx_bytes = signed_tx.serialize_to_vec();
                let client = reqwest::blocking::Client::new();
                let path = format!("{}/v2/transactions", "http://localhost:20443");
                let res = client
                    .post(&path)
                    .header("Content-Type", "application/octet-stream")
                    .body(tx_bytes)
                    .send()
                    .unwrap();
        
                if !res.status().is_success() {
                    println!("{}", res.text().unwrap());
                    panic!()
                }        
                let txid: String = res.json().unwrap();

                println!("Deploying {} (txid: {}, nonce: {})", contract_name, txid, nonce);
                deployers_nonces.insert(deployer.name.clone(), nonce + 1);
            }

            // If mocknet, we should be pulling all the links.
            // Get ordered list of contracts
            // For each contract, get the nonce of the account deploying (if unknown)
            // Create a StacksTransaction with the contract, the name.
            // Sign the transaction
            // Send the transaction
        }
    };
}
  
fn execute_changes(changes: Vec<Changes>) {
    for mut change in changes.into_iter() {
        match change {
            Changes::AddFile(options) => {
                println!("{}", options.comment);
                let mut file = File::create(options.path.clone()).expect("Unable to create file");
                file.write_all(options.content.as_bytes())
                    .expect("Unable to write file");
            }
            Changes::AddDirectory(options) => {
                println!("{}", options.comment);
                fs::create_dir_all(options.path.clone()).expect("Unable to create directory");
            }
            Changes::EditTOML(ref mut options) => {
                let file = File::open(options.path.clone()).unwrap();
                let mut config_file_reader = BufReader::new(file);
                let mut config_file = vec![];
                config_file_reader.read_to_end(&mut config_file).unwrap();
                let config_file: MainConfigFile = toml::from_slice(&config_file[..]).unwrap();
                let mut config: MainConfig = MainConfig::from_config_file(config_file);
                let mut dirty = false;
                println!("BEFORE: {:?}", config);

                let mut links = match config.links.take() {
                    Some(links) => links,
                    None => vec![],
                };
                for link in options.links_to_add.drain(..) {
                    if links.contains(&link) {
                        links.push(link);
                        dirty = true;
                    }
                }
                config.links = Some(links);

                let mut contracts = match config.contracts.take() {
                    Some(contracts) => contracts,
                    None => BTreeMap::new(),
                };
                for (contract_name, contract_config) in options.contracts_to_add.iter() {
                    let res = contracts.insert(contract_name.clone(), contract_config.clone());
                    if res.is_none() {
                        dirty = true;
                    }
                }
                config.contracts = Some(contracts);

                println!("AFTER: {:?}", config);

                if dirty {
                    let toml = toml::to_string(&config).unwrap();
                    let mut file = File::create(options.path.clone()).unwrap();
                    file.write_all(&toml.as_bytes()).unwrap();    
                } 
                println!("{}", options.comment);
            }
        }
    }
}
