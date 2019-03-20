use super::VerificationKey;
use crate::transcript::TranscriptProtocol;
use curve25519_dalek::ristretto::RistrettoPoint;
use curve25519_dalek::scalar::Scalar;
use merlin::Transcript;

#[derive(Clone)]
pub struct Multikey {
    transcript: Transcript,
    aggregated_key: VerificationKey,
}

impl Multikey {
    pub fn new(pubkeys: Vec<VerificationKey>) -> Option<Self> {
        // Create transcript for Multikey
        let mut transcript = Transcript::new(b"MuSig.aggregated-key");
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
            let X = match X.0.decompress() {
                Some(X) => X,
                None => return None,
            };
            aggregated_key = aggregated_key + a * X;
        }

        Some(Multikey {
            transcript,
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
        Multikey::compute_factor(&self.transcript, X_i)
    }

    pub fn aggregated_key(&self) -> VerificationKey {
        self.aggregated_key
    }
}
