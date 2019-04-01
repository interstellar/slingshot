use super::errors::MusigError;
use super::transcript::TranscriptProtocol;
use curve25519_dalek::constants::RISTRETTO_BASEPOINT_POINT;
use curve25519_dalek::ristretto::{CompressedRistretto, RistrettoPoint};
use curve25519_dalek::scalar::Scalar;
use merlin::Transcript;

#[derive(Clone)]
pub struct Multikey {
    transcript: Option<Transcript>,
    aggregated_key: VerificationKey,
}

impl Multikey {
    pub fn new(pubkeys: Vec<VerificationKey>) -> Result<Self, MusigError> {
        match pubkeys.len() {
            0 => return Err(MusigError::BadArguments),
            1 => {
                return Ok(Multikey {
                    transcript: None,
                    aggregated_key: pubkeys[0],
                });
            }
            _ => {}
        }

        // Create transcript for Multikey
        let mut transcript = Transcript::new(b"Musig.aggregated-key");
        transcript.commit_u64(b"n", pubkeys.len() as u64);

        // Commit pubkeys into the transcript
        // <L> = H(X_1 || X_2 || ... || X_n)
        for X in &pubkeys {
            transcript.commit_point(b"X", &X.0);
        }

        // aggregated_key = sum_i ( a_i * X_i )
        let mut aggregated_key = RistrettoPoint::default();
        for X in &pubkeys {
            let a = Multikey::compute_factor(&transcript, X);
            let X = X.0.decompress().ok_or(MusigError::InvalidPoint)?;
            aggregated_key = aggregated_key + a * X;
        }

        Ok(Multikey {
            transcript: Some(transcript),
            aggregated_key: VerificationKey(aggregated_key.compress()),
        })
    }

    fn compute_factor(transcript: &Transcript, X_i: &VerificationKey) -> Scalar {
        // a_i = H(<L>, X_i). Components of <L> have already been fed to transcript.
        let mut a_i_transcript = transcript.clone();
        a_i_transcript.commit_point(b"X_i", &X_i.0);
        a_i_transcript.challenge_scalar(b"a_i")
    }

    pub fn factor_for_key(&self, X_i: &VerificationKey) -> Scalar {
        match &self.transcript {
            Some(t) => Multikey::compute_factor(&t, X_i),
            None => Scalar::one(),
        }
    }

    pub fn aggregated_key(&self) -> VerificationKey {
        self.aggregated_key
    }
}

/// Verification key (aka "pubkey") is a wrapper type around a Ristretto point
/// that lets the verifier to check the signature.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct VerificationKey(pub CompressedRistretto);

impl VerificationKey {
    /// Constructs a VerificationKey from a private key.
    pub fn from_secret(privkey: &Scalar) -> Self {
        VerificationKey(Self::from_secret_uncompressed(privkey).compress())
    }

    /// Constructs an uncompressed VerificationKey point from a private key.
    pub(crate) fn from_secret_uncompressed(privkey: &Scalar) -> RistrettoPoint {
        (privkey * RISTRETTO_BASEPOINT_POINT)
    }
}

impl From<CompressedRistretto> for VerificationKey {
    fn from(x: CompressedRistretto) -> Self {
        VerificationKey(x)
    }
}
