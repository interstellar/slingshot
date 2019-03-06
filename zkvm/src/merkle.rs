use merlin::Transcript;
use subtle::ConstantTimeEq;

use crate::errors::VMError;

pub trait MerkleItem {
    fn commit(&self, t: &mut Transcript);
}

/// MerkleNeighbor represents a step in a Merkle proof of inclusion.
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum MerkleNeighbor {
    Left([u8; 32]),
    Right([u8; 32]),
}

/// MerkleTree represents a Merkle tree of hashes with a given size.
pub struct MerkleTree {
    size: usize,
    label: &'static [u8],
    root: MerkleNode,
}

enum MerkleNode {
    Leaf([u8; 32]),
    Node([u8; 32], Box<MerkleNode>, Box<MerkleNode>),
}

impl MerkleTree {
    /// Constructs a new MerkleTree based on the input list of entries.
    pub fn new(label: &'static [u8], list: &[&MerkleItem]) -> Option<MerkleTree> {
        if list.len() == 0 {
            return None;
        }
        let t = Transcript::new(label);
        Some(MerkleTree {
            size: list.len(),
            label,
            root: Self::build_tree(t, list),
        })
    }

    /// Builds a proof of inclusion for entry at the given index for the Merkle tree.
    pub fn proof(&self, index: usize) -> Result<Vec<MerkleNeighbor>, VMError> {
        if index >= self.size {
            return Err(VMError::InvalidMerkleProof);
        }
        let t = Transcript::new(self.label);
        let mut result = Vec::new();
        self.root.subproof(t, index, self.size, &mut result);
        Ok(result)
    }

    pub fn verify_proof(
        label: &'static [u8],
        entry: &MerkleItem,
        proof: Vec<MerkleNeighbor>,
        root: &[u8; 32],
    ) -> Result<(), VMError> {
        let transcript = Transcript::new(label);
        let mut result = [0u8; 32];
        Self::leaf(transcript.clone(), entry, &mut result);
        for node in proof.iter() {
            let mut t = transcript.clone();
            match node {
                MerkleNeighbor::Left(l) => {
                    t.commit_bytes(b"L", l);
                    t.commit_bytes(b"R", &result);
                    t.challenge_bytes(b"merkle.node", &mut result);
                }
                MerkleNeighbor::Right(r) => {
                    t.commit_bytes(b"L", &result);
                    t.commit_bytes(b"R", r);
                    t.challenge_bytes(b"merkle.node", &mut result);
                }
            }
        }
        if result.ct_eq(root).unwrap_u8() == 1 {
            Ok(())
        } else {
            Err(VMError::InvalidMerkleProof)
        }
    }

    /// Returns the root hash of the Merkle tree
    pub fn root(&self) -> &[u8; 32] {
        self.root.hash()
    }

    fn build_tree(mut t: Transcript, list: &[&MerkleItem]) -> MerkleNode {
        match list.len() {
            0 => {
                let mut leaf = [0u8; 32];
                Self::empty(t, &mut leaf);
                return MerkleNode::Leaf(leaf);
            }
            1 => {
                let mut leaf = [0u8; 32];
                Self::leaf(t, list[0], &mut leaf);
                return MerkleNode::Leaf(leaf);
            }
            n => {
                let k = n.next_power_of_two() / 2;
                let mut node = [0u8; 32];
                let left = Self::build_tree(t.clone(), &list[..k]);
                let right = Self::build_tree(t.clone(), &list[k..]);
                t.commit_bytes(b"L", left.hash());
                t.commit_bytes(b"R", right.hash());
                t.challenge_bytes(b"merkle.node", &mut node);
                return MerkleNode::Node(node, Box::new(left), Box::new(right));
            }
        }
    }

    fn node(mut t: Transcript, list: &[&MerkleItem], result: &mut [u8; 32]) {
        match list.len() {
            0 => Self::empty(t, result),
            1 => Self::leaf(t, list[0], result),
            n => {
                let k = n.next_power_of_two() / 2;
                let mut righthash = [0u8; 32];
                Self::node(t.clone(), &list[..k], result);
                Self::node(t.clone(), &list[k..], &mut righthash);
                t.commit_bytes(b"L", result);
                t.commit_bytes(b"R", &righthash);
                t.challenge_bytes(b"merkle.node", result);
            }
        }
    }

    fn empty(mut t: Transcript, result: &mut [u8; 32]) {
        t.challenge_bytes(b"merkle.empty", result);
    }

    fn leaf(mut t: Transcript, entry: &MerkleItem, result: &mut [u8; 32]) {
        entry.commit(&mut t);
        t.challenge_bytes(b"merkle.leaf", result);
    }
}

impl MerkleNode {
    fn subproof(&self, t: Transcript, index: usize, size: usize, result: &mut Vec<MerkleNeighbor>) {
        match self {
            MerkleNode::Leaf(_) => return,
            MerkleNode::Node(_, l, r) => {
                let k = size.next_power_of_two() / 2;
                if index >= k {
                    result.insert(0, MerkleNeighbor::Left(*l.hash()));
                    return r.subproof(t, index - k, size - k, result);
                } else {
                    result.insert(0, MerkleNeighbor::Right(*r.hash()));
                    return l.subproof(t, index, k, result);
                }
            }
        }
    }

    /// Returns the hash of a Merkle tree.
    fn hash(&self) -> &[u8; 32] {
        match self {
            MerkleNode::Leaf(h) => h,
            MerkleNode::Node(h, _, _) => h,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct TestItem(u64);

    impl MerkleItem for TestItem {
        fn commit(&self, t: &mut Transcript) {
            t.commit_u64(b"item", self.0)
        }
    }

    fn test_items(num: usize) -> Vec<TestItem> {
        let mut items = Vec::with_capacity(num);
        for i in 0..num {
            items.push(TestItem(i as u64))
        }
        items
    }

    fn to_merkle(items: &[TestItem]) -> Vec<&MerkleItem> {
        items.iter().map(|i| i as &MerkleItem).collect::<Vec<_>>()
    }

    macro_rules! assert_proof {
        ($num:ident, $idx:ident) => {
            let (item, root, proof) = {
                let items = test_items(*$num as usize);
                let tree = MerkleTree::new(b"test", &to_merkle(&items)).unwrap();
                let proof = tree.proof(*$idx as usize).unwrap();
                (items[*$idx as usize].clone(), tree.root().clone(), proof)
            };
            MerkleTree::verify_proof(b"test", &item, proof, &root).unwrap();
        };
    }

    macro_rules! assert_proof_err {
        ($num:ident, $idx:ident, $wrong_idx:ident) => {
            let (item, root, proof) = {
                let items = test_items(*$num as usize);
                let tree = MerkleTree::new(b"test", &to_merkle(&items)).unwrap();
                let proof = tree.proof(*$idx as usize).unwrap();
                (
                    items[*$wrong_idx as usize].clone(),
                    tree.root().clone(),
                    proof,
                )
            };
            assert!(MerkleTree::verify_proof(b"test", &item, proof, &root).is_err());
        };
    }

    #[test]
    fn empty() {
        assert!(MerkleTree::new(b"test", &[]).is_none());
    }

    #[test]
    fn invalid_range() {
        let entries = test_items(5);
        let root = MerkleTree::new(b"test", &to_merkle(&entries)).unwrap();
        assert!(root.proof(7).is_err())
    }

    #[test]
    fn valid_proofs() {
        let tests = [(10, 7), (11, 3), (12, 0), (5, 3), (25, 9)];
        for (num, idx) in tests.iter() {
            assert_proof!(num, idx);
        }
    }

    #[test]
    fn invalid_proofs() {
        let tests = [(10, 7, 8), (11, 3, 5), (12, 0, 2), (5, 3, 1), (25, 9, 8)];
        for (num, idx, wrong_idx) in tests.iter() {
            assert_proof_err!(num, idx, wrong_idx);
        }
    }
}
