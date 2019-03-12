#![allow(non_snake_case)]

use crate::errors::VMError;
use crate::signature::multikey::Multikey;
use crate::signature::musig::*;
use crate::signature::VerificationKey;
use crate::transcript::TranscriptProtocol;
use curve25519_dalek::constants::RISTRETTO_BASEPOINT_POINT;
use curve25519_dalek::ristretto::RistrettoPoint;
use curve25519_dalek::scalar::Scalar;
use merlin::Transcript;
use rand;

#[derive(Copy, Clone, Debug)]
pub struct Nonce(Scalar);

#[derive(Copy, Clone)]
pub struct NoncePrecommitment([u8; 32]);

// TODO: compress & decompress RistrettoPoint into CompressedRistretto when sending as message
#[derive(Copy, Clone, Debug)]
pub struct NonceCommitment(RistrettoPoint);

#[derive(Copy, Clone, Debug)]
pub struct Siglet(Scalar);

pub struct PartyAwaitingPrecommitments {
    transcript: Transcript,
    multikey: Multikey,
    x_i: PrivKey,
    r_i: Nonce,
    R_i: NonceCommitment,
}

pub struct PartyAwaitingCommitments {
    transcript: Transcript,
    multikey: Multikey,
    x_i: PrivKey,
    r_i: Nonce,
    nonce_precommitments: Vec<NoncePrecommitment>,
}

pub struct PartyAwaitingSiglets {
    multikey: Multikey,
    nonce_commitments: Vec<NonceCommitment>,
    c: Scalar,
}

impl NonceCommitment {
    fn precommit(&self) -> NoncePrecommitment {
        let mut h = Transcript::new(b"MuSig.nonce-precommit");
        h.commit_point(b"R_i", &self.0.compress());
        let mut precommitment = [0u8; 32];
        h.challenge_bytes(b"precommitment", &mut precommitment);
        NoncePrecommitment(precommitment)
    }
}

impl PartyAwaitingPrecommitments {
    pub fn new(
        transcript: &Transcript,
        x_i: PrivKey,
        multikey: Multikey,
    ) -> (Self, NoncePrecommitment) {
        let transcript = transcript.clone();
        let mut rng = transcript.build_rng().finalize(&mut rand::thread_rng());

        // Generate ephemeral keypair (r_i, R_i). r_i is a random nonce.
        let r_i = Nonce(Scalar::random(&mut rng));
        // R_i = generator * r_i
        let R_i = NonceCommitment(RISTRETTO_BASEPOINT_POINT * r_i.0);

        // Make H(R_i)
        let precommitment = R_i.precommit();

        (
            PartyAwaitingPrecommitments {
                transcript,
                multikey,
                x_i,
                r_i,
                R_i,
            },
            precommitment,
        )
    }

    pub fn receive_precommitments(
        self,
        nonce_precommitments: Vec<NoncePrecommitment>,
    ) -> (PartyAwaitingCommitments, NonceCommitment) {
        // Store received nonce precommitments in next state
        (
            PartyAwaitingCommitments {
                transcript: self.transcript,
                multikey: self.multikey,
                x_i: self.x_i,
                r_i: self.r_i,
                nonce_precommitments,
            },
            self.R_i,
        )
    }
}

impl PartyAwaitingCommitments {
    pub fn receive_commitments(
        mut self,
        m: Message,
        nonce_commitments: Vec<NonceCommitment>,
    ) -> Result<(PartyAwaitingSiglets, Siglet), VMError> {
        // Check stored precommitments against received commitments
        for (pre_comm, comm) in self
            .nonce_precommitments
            .iter()
            .zip(nonce_commitments.iter())
        {
            // Make H(comm) = H(R_i)
            let actual_precomm = comm.precommit();

            // Compare H(comm) with pre_comm, they should be equal
            // TBD: make it return Result instead of panic
            // TBD: should we use ct_eq?
            if pre_comm.0 != actual_precomm.0 {
                return Err(VMError::InconsistentWitness);
            }
        }

        // Make R = sum_i(R_i). nonce_commitments = R_i from all the parties.
        let R: RistrettoPoint = nonce_commitments.iter().map(|R_i| R_i.0).sum();

        // Make c = H(X_agg, R, m)
        let c = {
            self.transcript
                .commit_point(b"X_agg", &self.multikey.aggregated_key().0);
            self.transcript.commit_point(b"R", &R.compress());
            // TBD: remove this - it should be within a transcript already, outside the musig protocol
            self.transcript.commit_bytes(b"m", &m.0);
            self.transcript.challenge_scalar(b"c")
        };

        // Make a_i = H(L, X_i)
        let X_i = VerificationKey((self.x_i.0 * RISTRETTO_BASEPOINT_POINT).compress());
        let a_i = self.multikey.a_i(&X_i);

        // Generate siglet: s_i = r_i + c * a_i * x_i
        let s_i = self.r_i.0 + c * a_i * self.x_i.0;

        // Store received nonce commitments in next state
        Ok((
            PartyAwaitingSiglets {
                multikey: self.multikey,
                nonce_commitments,
                c,
            },
            Siglet(s_i),
        ))
    }
}

impl PartyAwaitingSiglets {
    pub fn receive_siglets(self, siglets: Vec<Siglet>) -> Signature {
        // s = sum(siglets)
        let s: Scalar = siglets.iter().map(|siglet| siglet.0).sum();
        // R = sum(R_i). nonce_commitments = R_i
        let R: RistrettoPoint = self.nonce_commitments.iter().map(|R_i| R_i.0).sum();

        Signature { s, R }
    }

    pub fn receive_and_verify_siglets(
        self,
        siglets: Vec<Siglet>,
        pubkeys: Vec<PubKey>,
    ) -> Signature {
        // Check that all siglets are valid
        for (i, s_i) in siglets.iter().enumerate() {
            let S_i = s_i.0 * RISTRETTO_BASEPOINT_POINT;
            let X_i = pubkeys[i].0;
            let R_i = self.nonce_commitments[i].0;

            // Make a_i = H(L, X_i)
            let a_i = self.multikey.a_i(&VerificationKey(X_i.compress()));

            // Check that S_i = R_i + c * a_i * X_i
            assert_eq!(S_i, R_i + self.c * a_i * X_i);
        }

        self.receive_siglets(siglets)
    }
}
