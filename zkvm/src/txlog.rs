use curve25519_dalek::ristretto::CompressedRistretto;
use merlin::Transcript;

use crate::contract::{ContractID, Output};
use crate::merkle::{MerkleItem, MerkleTree};
use crate::transcript::TranscriptProtocol;
use crate::vm::TxHeader;

/// Transaction log. `TxLog` is a type alias for `Vec<Entry>`.
pub type TxLog = Vec<Entry>;

/// Entry in a transaction log
#[derive(Clone, Debug)]
#[allow(missing_docs)]
pub enum Entry {
    Header(TxHeader),
    Issue(CompressedRistretto, CompressedRistretto),
    Retire(CompressedRistretto, CompressedRistretto),
    Input(ContractID),
    Output(Output),
    Data(Vec<u8>),
    Import, // TBD: parameters
    Export, // TBD: parameters
}

/// Transaction ID is a unique 32-byte identifier of a transaction
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct TxID(pub [u8; 32]);

impl MerkleItem for TxID {
    fn commit(&self, t: &mut Transcript) {
        t.commit_bytes(b"txid", &self.0)
    }
}

/// UTXO is a unique 32-byte identifier of a transaction output
#[derive(Copy, Clone, Eq, Hash, PartialEq, Debug)]
pub struct UTXO(pub [u8; 32]);

impl UTXO {
    /// Computes UTXO identifier from an output.
    pub fn from_output(output: &Output) -> Self {
        output.id().as_utxo()
    }
}

impl TxID {
    /// Computes TxID from a tx log
    pub fn from_log(list: &[Entry]) -> Self {
        TxID(MerkleTree::root(b"ZkVM.txid", list))
    }
}

impl MerkleItem for Entry {
    fn commit(&self, t: &mut Transcript) {
        match self {
            Entry::Header(h) => {
                t.commit_u64(b"tx.version", h.version);
                t.commit_u64(b"tx.mintime", h.mintime_ms);
                t.commit_u64(b"tx.maxtime", h.maxtime_ms);
            }
            Entry::Issue(q, f) => {
                t.commit_point(b"issue.q", q);
                t.commit_point(b"issue.f", f);
            }
            Entry::Retire(q, f) => {
                t.commit_point(b"retire.q", q);
                t.commit_point(b"retire.f", f);
            }
            Entry::Input(contract) => {
                t.commit_bytes(b"input", contract.as_bytes());
            }
            Entry::Output(output) => {
                t.commit_bytes(b"output", output.id().as_bytes());
            }
            Entry::Data(data) => {
                t.commit_bytes(b"data", data);
            }
            Entry::Import => {
                // TBD: commit parameters
                unimplemented!()
            }
            Entry::Export => {
                // TBD: commit parameters
                unimplemented!()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn txlog_helper() -> Vec<Entry> {
        vec![
            Entry::Header(TxHeader {
                mintime_ms: 0,
                maxtime_ms: 0,
                version: 0,
            }),
            Entry::Issue(
                CompressedRistretto::from_slice(&[0u8; 32]),
                CompressedRistretto::from_slice(&[1u8; 32]),
            ),
            Entry::Data(vec![0u8]),
            Entry::Data(vec![1u8]),
            Entry::Data(vec![2u8]),
        ]
    }

    #[test]
    fn valid_txid_proof() {
        let (entry, txid, proof) = {
            let entries = txlog_helper();
            let root = MerkleTree::build(b"ZkVM.txid", &entries);
            let index = 3;
            let proof = root.create_path(index).unwrap();
            (entries[index].clone(), TxID::from_log(&entries), proof)
        };
        MerkleTree::verify_path(b"ZkVM.txid", &entry, proof, &txid.0).unwrap();
    }

    #[test]
    fn invalid_txid_proof() {
        let (entry, txid, proof) = {
            let entries = txlog_helper();
            let root = MerkleTree::build(b"ZkVM.txid", &entries);
            let index = 3;
            let proof = root.create_path(index).unwrap();
            (entries[index + 1].clone(), TxID::from_log(&entries), proof)
        };
        assert!(MerkleTree::verify_path(b"ZkVM.txid", &entry, proof, &txid.0).is_err());
    }
}
