use super::errors::MusigError;
use super::key::{Multikey, VerificationKey};
use crate::transcript::TranscriptProtocol;
use curve25519_dalek::constants::RISTRETTO_BASEPOINT_POINT;
use curve25519_dalek::ristretto::{CompressedRistretto, RistrettoPoint};
use curve25519_dalek::scalar::Scalar;
use merlin::Transcript;
use subtle::ConstantTimeEq;

#[derive(Copy, Clone)]
pub struct NoncePrecommitment([u8; 32]);

#[derive(Copy, Clone, Debug)]
pub struct NonceCommitment(RistrettoPoint);

impl NonceCommitment {
    pub(super) fn new(commitment: RistrettoPoint) -> Self {
        NonceCommitment(commitment)
    }

    pub(super) fn precommit(&self) -> NoncePrecommitment {
        let mut h = Transcript::new(b"Musig.nonce-precommit");
        h.commit_point(b"R", &self.0.compress());
        let mut precommitment = [0u8; 32];
        h.challenge_bytes(b"precommitment", &mut precommitment);
        NoncePrecommitment(precommitment)
    }

    pub(super) fn compress(&self) -> CompressedRistretto {
        self.0.compress()
    }

    pub(super) fn sum(commitments: &Vec<Self>) -> RistrettoPoint {
        commitments.iter().map(|R_i| R_i.0).sum()
    }
}

pub struct Counterparty {
    pubkey: VerificationKey,
}

pub struct CounterpartyPrecommitted {
    precommitment: NoncePrecommitment,
    pubkey: VerificationKey,
}

pub struct CounterpartyCommitted {
    commitment: NonceCommitment,
    pubkey: VerificationKey,
}

impl Counterparty {
    pub(super) fn new(pubkey: VerificationKey) -> Self {
        Counterparty { pubkey }
    }

    pub(super) fn precommit_nonce(
        self,
        precommitment: NoncePrecommitment,
    ) -> CounterpartyPrecommitted {
        CounterpartyPrecommitted {
            precommitment,
            pubkey: self.pubkey,
        }
    }
}

impl CounterpartyPrecommitted {
    pub(super) fn commit_nonce(
        self,
        commitment: NonceCommitment,
    ) -> Result<CounterpartyCommitted, MusigError> {
        // Check H(commitment) =? precommitment
        let received_precommitment = commitment.precommit();
        let equal = self.precommitment.0.ct_eq(&received_precommitment.0);
        if equal.unwrap_u8() == 0 {
            return Err(MusigError::ShareError {
                pubkey: self.pubkey.0.to_bytes(),
            });
        }

        Ok(CounterpartyCommitted {
            commitment: commitment,
            pubkey: self.pubkey,
        })
    }
}

impl CounterpartyCommitted {
    pub(super) fn sign(
        self,
        share: Scalar,
        challenge: Scalar,
        multikey: &Multikey,
    ) -> Result<Scalar, MusigError> {
        // Check if s_i * G == R_i + c * a_i * X_i.
        //   s_i = share
        //   G = RISTRETTO_BASEPOINT_POINT
        //   R_i = self.commitment
        //   c = challenge
        //   a_i = multikey.factor_for_key(self.pubkey)
        //   X_i = self.pubkey
        let S_i = share * RISTRETTO_BASEPOINT_POINT;
        let a_i = multikey.factor_for_key(&self.pubkey);
        let X_i = self.pubkey.0.decompress().ok_or(MusigError::InvalidPoint)?;

        if S_i != self.commitment.0 + challenge * a_i * X_i {
            return Err(MusigError::ShareError {
                pubkey: self.pubkey.0.to_bytes(),
            });
        }

        Ok(share)
    }
}
