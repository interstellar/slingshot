use super::nodes::*;
use super::schema::*;
use super::util;
use curve25519_dalek::scalar::Scalar;
use zkvm::blockchain::{Block, BlockchainState};
use zkvm::{Tx, TxEntry};

use serde_json::Value as JsonValue;
use std::collections::HashMap;

// Stored data

#[derive(Debug, Queryable, Insertable)]
pub struct BlockRecord {
    pub height: i32, // FIXME: diesel doesn't allow u64 here...
    pub block_json: String,
    pub state_json: String, // latest state will be used for *the* network state
}

#[derive(Debug, Queryable, Insertable)]
pub struct AssetRecord {
    pub alias: String,
    pub key_json: String,
}

#[derive(Debug, Queryable, Insertable, AsChangeset)]
pub struct NodeRecord {
    pub alias: String,
    pub state_json: String,
}

impl BlockRecord {
    pub fn network_status_summary(&self) -> JsonValue {
        json!({
            "height": self.height,
            "block_id": hex::encode(self.block().header.id().0),
            "block": serde_json::from_str::<JsonValue>(&self.block_json).expect("Block should be valid JSON."),
            "state": serde_json::from_str::<JsonValue>(&self.state_json).expect("State should be valid JSON."),
            "utxos_count": self.state().utreexo.count(),
        })
    }

    pub fn to_table_item(&self) -> JsonValue {
        let blk = self.block();
        json!({
            "height": self.height,
            "id": hex::encode(self.block().header.id().0),
            "header": blk.header,
            "txs": blk.txs.len(),
        })
    }

    pub fn to_details(&self) -> JsonValue {
        let blk = self.block();
        json!({
            "height": self.height,
            "id": hex::encode(self.block().header.id().0),
            "header": &util::to_json_value(&blk.header),
            "txs": blk.txs.into_iter().map(|tx| {
                Self::tx_details(&tx)
            }).collect::<Vec<_>>(),
        })
    }

    pub fn tx_details(tx: &Tx) -> JsonValue {
        let (txid, txlog) = tx
            .precompute()
            .expect("Our blockchain does not have invalid transactions.");
        json!({
            "id": hex::encode(&txid),
            "header": &util::to_json_value(&tx.header),
            "inputs": &util::to_json_value(&txlog.iter().filter_map(|e| {
                match e {
                    TxEntry::Input(cid) => Some(cid),
                    _ => None
                }
            }).collect::<Vec<_>>()),
            "outputs": &util::to_json_value(&txlog.iter().filter_map(|e| {
                match e {
                    TxEntry::Output(c) => Some(c.id()),
                    _ => None
                }
            }).collect::<Vec<_>>()),
            "tx": &util::to_json_value(&tx),
            "program_hex": hex::encode(&tx.program),
            "program_asm": format!("{:?}", zkvm::Program::parse(&tx.program).expect("Our blockchain does not have invalid txs.")),
        })
    }

    pub fn block(&self) -> Block {
        util::from_valid_json(&self.block_json)
    }

    pub fn state(&self) -> BlockchainState {
        util::from_valid_json(&self.state_json)
    }
}

impl NodeRecord {
    pub fn new(node: Node) -> Self {
        Self {
            alias: node.wallet.alias.clone(),
            state_json: util::to_json(&node),
        }
    }

    /// Converts the record to the Node instance.
    pub fn node(&self) -> Node {
        util::from_valid_json(&self.state_json)
    }

    /// Converts the node to JSON object tree.
    pub fn to_json(&self) -> JsonValue {
        serde_json::from_str(&self.state_json)
            .expect("Stored json state must be correctly encoded.")
    }

    pub fn balances(&self, assets: &[AssetRecord]) -> JsonValue {
        // 1. Enumerate all confirmed utxos and stack up values by flavor.
        // 2. Then, annotate each flavor with the asset name.
        let map = self
            .node()
            .wallet
            .utxos
            .iter()
            .map(|utxo| utxo.receiver_witness.receiver.value)
            .fold(HashMap::new(), |mut hm, value| {
                let key = value.flv.as_bytes().to_vec();
                let total = *hm.get(&key).unwrap_or(&0u64);
                hm.insert(key, total + value.qty);
                hm
            });
        json!(map
            .iter()
            .map(|(flv, balance)| {
                let alias = assets
                    .iter()
                    .find(|&asset| asset.flavor().as_bytes() == &flv[..])
                    .map(|x| x.alias.clone())
                    .unwrap_or(hex::encode(flv));

                json!({
                    "alias": alias,
                    "flv": flv,
                    "qty": balance
                })
            })
            .collect::<Vec<_>>())
    }
}

impl AssetRecord {
    /// Creates a new asset record with key derived from the alias.
    pub fn new(alias: impl Into<String>) -> Self {
        let alias = alias.into();
        let key = util::scalar_from_string(&alias);
        AssetRecord {
            alias,
            key_json: util::to_json(&key),
        }
    }

    pub fn issuance_key(&self) -> Scalar {
        util::from_valid_json(&self.key_json)
    }

    pub fn issuance_predicate(&self) -> zkvm::Predicate {
        let vkey = zkvm::VerificationKey::from_secret(&self.issuance_key());
        zkvm::Predicate::Key(vkey)
    }

    pub fn metadata(&self) -> zkvm::String {
        zkvm::String::Opaque(self.alias.as_bytes().to_vec())
    }

    pub fn flavor(&self) -> Scalar {
        zkvm::Value::issue_flavor(&self.issuance_predicate(), self.metadata())
    }

    /// Converts the node to JSON object tree.
    pub fn to_json(&self) -> JsonValue {
        // stored json is guaranteed to be valid
        json!({
            "alias": self.alias,
            "prv": serde_json::from_str::<JsonValue>(&self.key_json).expect("DB should contain valid key_json"),
            "pub": hex::encode(self.issuance_predicate().to_point().as_bytes()),
            "flv": hex::encode(self.flavor().as_bytes())
        })
    }
}
