use bulletproofs::r1cs::{Prover, Variable, Verifier};
use curve25519_dalek::ristretto::CompressedRistretto;
use curve25519_dalek::scalar::Scalar;
use rand::{CryptoRng, Rng};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Value {
    pub q: u64,    // quantity
    pub a: Scalar, // issuer
    pub t: Scalar, // tag
}

pub struct CommittedValue {
    pub q: CompressedRistretto,
    pub a: CompressedRistretto,
    pub t: CompressedRistretto,
}

/// Helper struct for ease of working with
/// 3-tuples of variables and assignments
#[derive(Copy, Clone, Debug)]
pub struct AllocatedValue {
    pub q: Variable, // quantity
    pub a: Variable, // issuer
    pub t: Variable, // tag
    pub assignment: Option<Value>,
}

/// Represents a variable for quantity, along with its assignment.
#[derive(Copy, Clone, Debug)]
pub struct AllocatedQuantity {
    pub variable: Variable,
    pub assignment: Option<u64>,
}

impl Value {
    /// Returns a zero quantity with a zero flavor.
    pub fn zero() -> Value {
        Value {
            q: 0,
            a: Scalar::zero(),
            t: Scalar::zero(),
        }
    }
}

impl AllocatedValue {
    /// Returns a quantity variable with its assignment.
    pub fn quantity(&self) -> AllocatedQuantity {
        AllocatedQuantity {
            variable: self.q,
            assignment: self.assignment.map(|v| v.q),
        }
    }
}

pub trait ProverCommittable {
    type Output;

    fn commit<R: Rng + CryptoRng>(&self, prover: &mut Prover, rng: &mut R) -> Self::Output;
}

impl ProverCommittable for Value {
    type Output = (CommittedValue, AllocatedValue);

    fn commit<R: Rng + CryptoRng>(&self, prover: &mut Prover, rng: &mut R) -> Self::Output {
        let (q_commit, q_var) = prover.commit(self.q.into(), Scalar::random(rng));
        let (a_commit, a_var) = prover.commit(self.a, Scalar::random(rng));
        let (t_commit, t_var) = prover.commit(self.t, Scalar::random(rng));
        let commitments = CommittedValue {
            q: q_commit,
            a: a_commit,
            t: t_commit,
        };
        let vars = AllocatedValue {
            q: q_var,
            a: a_var,
            t: t_var,
            assignment: Some(*self),
        };
        (commitments, vars)
    }
}

impl ProverCommittable for Vec<Value> {
    type Output = (Vec<CommittedValue>, Vec<AllocatedValue>);

    fn commit<R: Rng + CryptoRng>(&self, prover: &mut Prover, rng: &mut R) -> Self::Output {
        self.iter().map(|value| value.commit(prover, rng)).unzip()
    }
}

pub trait VerifierCommittable {
    type Output;
    fn commit(&self, verifier: &mut Verifier) -> Self::Output;
}

impl VerifierCommittable for CommittedValue {
    type Output = AllocatedValue;

    fn commit(&self, verifier: &mut Verifier) -> Self::Output {
        AllocatedValue {
            q: verifier.commit(self.q),
            a: verifier.commit(self.a),
            t: verifier.commit(self.t),
            assignment: None,
        }
    }
}

impl VerifierCommittable for Vec<CommittedValue> {
    type Output = Vec<AllocatedValue>;

    fn commit(&self, verifier: &mut Verifier) -> Self::Output {
        self.iter().map(|value| value.commit(verifier)).collect()
    }
}
