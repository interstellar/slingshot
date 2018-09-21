#![allow(non_snake_case)]

use bulletproofs::circuit_proof::assignment::Assignment;
use bulletproofs::circuit_proof::{ConstraintSystem, Variable};
use bulletproofs::R1CSError;
use curve25519_dalek::scalar::Scalar;
use util::Value;

pub struct KShuffleGadget {}

impl KShuffleGadget {
    fn fill_cs<CS: ConstraintSystem>(
        cs: &mut CS,
        x: Vec<(Variable, Assignment)>,
        y: Vec<(Variable, Assignment)>,
    ) -> Result<(), R1CSError> {
        let one = Scalar::one();
        let var_one = Variable::One();
        let z = cs.challenge_scalar(b"k-shuffle challenge");
        let neg_z = -z;

        if x.len() != y.len() {
            return Err(R1CSError::InvalidR1CSConstruction);
        }
        let k = x.len();
        if k == 1 {
            cs.add_constraint([(x[0].0, -one), (y[0].0, one)].iter().collect());
            return Ok(());
        }

        // Make last x multiplier for i = k-1 and k-2
        let mut mulx_left = x[k - 1].1 + neg_z;
        let mut mulx_right = x[k - 2].1 + neg_z;
        let mut mulx_out = mulx_left * mulx_right;
        let (mulx_left_var, mulx_right_var, mulx_out_var) =
            cs.assign_multiplier(mulx_left, mulx_right, mulx_out)?;
        cs.add_constraint(
            [(mulx_left_var, -one), (var_one, neg_z), (x[k - 1].0, one)]
                .iter()
                .collect(),
        );
        cs.add_constraint(
            [(mulx_right_var, -one), (var_one, neg_z), (x[k - 2].0, one)]
                .iter()
                .collect(),
        );
        let mut mulx_out_var_prev = mulx_out_var;

        // Make multipliers for x from i == [0, k-3]
        for i in (0..k - 2).rev() {
            mulx_left = mulx_out;
            mulx_right = x[i].1 + neg_z;
            mulx_out = mulx_left * mulx_right;

            let (mulx_left_var, mulx_right_var, mulx_out_var) =
                cs.assign_multiplier(mulx_left, mulx_right, mulx_out)?;
            cs.add_constraint(
                [(mulx_left_var, -one), (mulx_out_var_prev, one)]
                    .iter()
                    .collect(),
            );
            cs.add_constraint(
                [(mulx_right_var, -one), (var_one, neg_z), (x[i].0, one)]
                    .iter()
                    .collect(),
            );

            mulx_out_var_prev = mulx_out_var;
        }

        // Make last y multiplier for i = k-1 and k-2
        let mut muly_left = y[k - 1].1 - z;
        let mut muly_right = y[k - 2].1 - z;
        let mut muly_out = muly_left * muly_right;
        let (muly_left_var, muly_right_var, muly_out_var) =
            cs.assign_multiplier(muly_left, muly_right, muly_out)?;
        cs.add_constraint(
            [(muly_left_var, -one), (var_one, neg_z), (y[k - 1].0, one)]
                .iter()
                .collect(),
        );
        cs.add_constraint(
            [(muly_right_var, -one), (var_one, neg_z), (y[k - 2].0, one)]
                .iter()
                .collect(),
        );
        let mut muly_out_var_prev = muly_out_var;

        // Make multipliers for y from i == [0, k-3]
        for i in (0..k - 2).rev() {
            muly_left = muly_out;
            muly_right = y[i].1 + neg_z;
            muly_out = muly_left * muly_right;

            let (muly_left_var, muly_right_var, muly_out_var) =
                cs.assign_multiplier(muly_left, muly_right, muly_out)?;
            cs.add_constraint(
                [(muly_left_var, -one), (muly_out_var_prev, one)]
                    .iter()
                    .collect(),
            );
            cs.add_constraint(
                [(muly_right_var, -one), (var_one, neg_z), (y[i].0, one)]
                    .iter()
                    .collect(),
            );

            muly_out_var_prev = muly_out_var;
        }

        // Check equality between last x mul output and last y mul output
        cs.add_constraint(
            [(muly_out_var_prev, -one), (mulx_out_var_prev, one)]
                .iter()
                .collect(),
        );

        Ok(())
    }
}

pub struct MergeGadget {}

impl MergeGadget {
    fn fill_cs<CS: ConstraintSystem>(
        cs: &mut CS,
        A: Value,
        B: Value,
        C: Value,
        D: Value,
    ) -> Result<(), R1CSError> {
        let one = Scalar::one();
        let w = cs.challenge_scalar(b"merge challenge");

        // create variables for multiplication
        let (mul_left, mul_right, mul_out) = cs.assign_multiplier(
            // left gate to multiplier
            (A.q.1 - C.q.1)
                + (A.a.1 - C.a.1) * w
                + (A.t.1 - C.t.1) * w * w
                + (B.q.1 - D.q.1) * w * w * w
                + (B.a.1 - D.a.1) * w * w * w * w
                + (B.t.1 - D.t.1) * w * w * w * w * w,
            // right gate to multiplier
            C.q.1
                + (A.a.1 - B.a.1) * w
                + (A.t.1 - B.t.1) * w * w
                + (D.q.1 - A.q.1 - B.q.1) * w * w * w
                + (D.a.1 - A.a.1) * w * w * w * w
                + (D.t.1 - A.t.1) * w * w * w * w * w,
            // out gate to multiplier
            Assignment::zero(),
        )?;
        // mul_left  = (A.q - C.q) +
        //             (A.a - C.a) * w +
        //             (A.t - C.t) * w^2 +
        //             (B.q - D.q) * w^3 +
        //             (B.a - D.a) * w^4 +
        //             (B.t - D.t) * w^5
        cs.add_constraint(
            [
                (mul_left, -one),
                (A.q.0, one),
                (C.q.0, -one),
                (A.a.0, w),
                (C.a.0, -w),
                (A.t.0, w * w),
                (C.t.0, -w * w),
                (B.q.0, w * w * w),
                (D.q.0, -w * w * w),
                (B.a.0, w * w * w * w),
                (D.a.0, -w * w * w * w),
                (B.t.0, w * w * w * w * w),
                (D.t.0, -w * w * w * w * w),
            ]
                .iter()
                .collect(),
        );
        // mul_right = (C.q - 0) +
        //             (A.a - B.a) * w +
        //             (A.t - B.t) * w^2 +
        //             (D.q - A.q - B.q) * w^3 +
        //             (D.a - A.a) * w^4
        //             (D.t - A.t) * w^5
        cs.add_constraint(
            [
                (mul_right, -one),
                (C.q.0, one),
                (A.a.0, w),
                (B.a.0, -w),
                (A.t.0, w * w),
                (B.t.0, -w * w),
                (D.q.0, w * w * w),
                (A.q.0, -w * w * w),
                (B.q.0, -w * w * w),
                (D.a.0, w * w * w * w),
                (A.a.0, -w * w * w * w),
                (D.t.0, w * w * w * w * w),
                (A.t.0, -w * w * w * w * w),
            ]
                .iter()
                .collect(),
        );
        // mul_out   = 0
        cs.add_constraint([(mul_out, -one)].iter().collect());

        Ok(())
    }
}

pub struct SplitGadget {}

impl SplitGadget {
    fn fill_cs<CS: ConstraintSystem>(
        cs: &mut CS,
        A: Value,
        B: Value,
        C: Value,
        D: Value,
    ) -> Result<(), R1CSError> {
        MergeGadget::fill_cs(cs, D, C, B, A)
    }
}
// TODO: write split tests

#[cfg(test)]
mod tests {
    use super::*;
    use bulletproofs::circuit_proof::{prover, verifier};
    use bulletproofs::R1CSError;
    use bulletproofs::{Generators, PedersenGenerators, Transcript};

    #[test]
    fn shuffle_gadget() {
        // k=1
        assert!(shuffle_helper(vec![3], vec![3]).is_ok());
        assert!(shuffle_helper(vec![6], vec![6]).is_ok());
        assert!(shuffle_helper(vec![3], vec![6]).is_err());
        assert!(shuffle_helper(vec![3], vec![3, 6]).is_ok());
        // k=2
        assert!(shuffle_helper(vec![3, 6], vec![3, 6]).is_ok());
        assert!(shuffle_helper(vec![3, 6], vec![6, 3]).is_ok());
        assert!(shuffle_helper(vec![6, 6], vec![6, 6]).is_ok());
        assert!(shuffle_helper(vec![3, 3], vec![6, 3]).is_err());
        assert!(shuffle_helper(vec![3, 6], vec![3, 6, 10]).is_ok());
        // k=3
        assert!(shuffle_helper(vec![3, 6, 10], vec![3, 6, 10]).is_ok());
        assert!(shuffle_helper(vec![3, 6, 10], vec![3, 10, 6]).is_ok());
        assert!(shuffle_helper(vec![3, 6, 10], vec![6, 3, 10]).is_ok());
        assert!(shuffle_helper(vec![3, 6, 10], vec![6, 10, 3]).is_ok());
        assert!(shuffle_helper(vec![3, 6, 10], vec![10, 3, 6]).is_ok());
        assert!(shuffle_helper(vec![3, 6, 10], vec![10, 6, 3]).is_ok());
        assert!(shuffle_helper(vec![3, 6, 10], vec![30, 6, 10]).is_err());
        assert!(shuffle_helper(vec![3, 6, 10], vec![3, 60, 10]).is_err());
        assert!(shuffle_helper(vec![3, 6, 10], vec![3, 6, 100]).is_err());
        // k=4
        assert!(shuffle_helper(vec![3, 6, 10, 15], vec![3, 6, 10, 15]).is_ok());
        assert!(shuffle_helper(vec![3, 6, 10, 15], vec![15, 6, 10, 3]).is_ok());
        assert!(shuffle_helper(vec![3, 6, 10, 15], vec![3, 6, 10, 3]).is_err());
        // k=5
        assert!(shuffle_helper(vec![3, 6, 10, 15, 17], vec![3, 6, 10, 15, 17]).is_ok());
        assert!(shuffle_helper(vec![3, 6, 10, 15, 17], vec![10, 17, 3, 15, 6]).is_ok());
        assert!(shuffle_helper(vec![3, 6, 10, 15, 17], vec![3, 6, 10, 15, 3]).is_err());
    }

    fn shuffle_helper(input: Vec<u64>, output: Vec<u64>) -> Result<(), R1CSError> {
        // Common
        let gens = Generators::new(PedersenGenerators::default(), 128, 1);

        // Prover's scope
        let (proof, commitments) = {
            // Prover makes a `ConstraintSystem` instance representing a shuffle gadget
            // v and v_blinding empty because we are only testing low-level variable constraints
            let v = vec![];
            let v_blinding = vec![];
            let mut prover_transcript = Transcript::new(b"ShuffleTest");
            let (mut prover_cs, _variables, commitments) =
                prover::ProverCS::new(&mut prover_transcript, &gens, v, v_blinding.clone());

            // Prover allocates variables and adds constraints to the constraint system
            let in_assignments = input
                .iter()
                .map(|in_i| Assignment::from(in_i.clone()))
                .collect();
            let out_assignments = output
                .iter()
                .map(|out_i| Assignment::from(out_i.clone()))
                .collect();
            shuffle_cs(&mut prover_cs, in_assignments, out_assignments)?;
            let proof = prover_cs.prove()?;

            (proof, commitments)
        };

        // Verifier makes a `ConstraintSystem` instance representing a shuffle gadget
        let mut verifier_transcript = Transcript::new(b"ShuffleTest");
        let (mut verifier_cs, _variables) =
            verifier::VerifierCS::new(&mut verifier_transcript, &gens, commitments);

        // Verifier allocates variables and adds constraints to the constraint system
        let in_assignments = input.iter().map(|_| Assignment::Missing()).collect();
        let out_assignments = output.iter().map(|_| Assignment::Missing()).collect();
        assert!(shuffle_cs(&mut verifier_cs, in_assignments, out_assignments,).is_ok());
        // Verifier verifies proof
        Ok(verifier_cs.verify(&proof)?)
    }

    fn shuffle_cs<CS: ConstraintSystem>(
        cs: &mut CS,
        input: Vec<Assignment>,
        output: Vec<Assignment>,
    ) -> Result<(), R1CSError> {
        let mut in_pairs = vec![];
        let mut out_pairs = vec![];
        let k = input.len();

        // Allocate pairs of low-level variables and their assignments
        for i in 0..k / 2 {
            let idx_l = i * 2;
            let idx_r = idx_l + 1;
            let (in_var_left, in_var_right) = cs.assign_uncommitted(input[idx_l], input[idx_r])?;
            in_pairs.push((in_var_left, input[idx_l]));
            in_pairs.push((in_var_right, input[idx_r]));

            let (out_var_left, out_var_right) =
                cs.assign_uncommitted(output[idx_l], output[idx_r])?;
            out_pairs.push((out_var_left, output[idx_l]));
            out_pairs.push((out_var_right, output[idx_r]));
        }
        if k % 2 == 1 {
            let idx = k - 1;
            let (in_var_left, _) = cs.assign_uncommitted(input[idx], Assignment::zero())?;
            in_pairs.push((in_var_left, input[idx]));
            let (out_var_left, _) = cs.assign_uncommitted(output[idx], Assignment::zero())?;
            out_pairs.push((out_var_left, output[idx]));
        }

        KShuffleGadget::fill_cs(cs, in_pairs, out_pairs)
    }

    #[test]
    fn merge_gadget() {
        let peso = 66;
        let peso_tag = 77;
        let yuan = 88;
        let yuan_tag = 99;

        // no merge, same asset types
        assert!(
            merge_helper(
                (6, peso, peso_tag),
                (6, peso, peso_tag),
                (6, peso, peso_tag),
                (6, peso, peso_tag),
            ).is_ok()
        );
        // no merge, different asset types
        assert!(
            merge_helper(
                (3, peso, peso_tag),
                (6, yuan, yuan_tag),
                (3, peso, peso_tag),
                (6, yuan, yuan_tag),
            ).is_ok()
        );
        // merge, same asset types
        assert!(
            merge_helper(
                (3, peso, peso_tag),
                (6, peso, peso_tag),
                (0, peso, peso_tag),
                (9, peso, peso_tag),
            ).is_ok()
        );
        // merge, zero value is different asset type
        assert!(
            merge_helper(
                (3, peso, peso_tag),
                (6, peso, peso_tag),
                (0, yuan, yuan_tag),
                (9, peso, peso_tag),
            ).is_ok()
        );
        // error when merging different asset types
        assert!(
            merge_helper(
                (3, peso, peso_tag),
                (3, yuan, yuan_tag),
                (0, peso, peso_tag),
                (6, yuan, yuan_tag),
            ).is_err()
        );
        // error when not merging, but asset type changes
        assert!(
            merge_helper(
                (3, peso, peso_tag),
                (3, yuan, yuan_tag),
                (3, peso, peso_tag),
                (3, peso, peso_tag),
            ).is_err()
        );
        // error when creating more value (same asset types)
        assert!(
            merge_helper(
                (3, peso, peso_tag),
                (3, peso, peso_tag),
                (3, peso, peso_tag),
                (6, peso, peso_tag),
            ).is_err()
        );
        // error when creating more value (different asset types)
        assert!(
            merge_helper(
                (3, peso, peso_tag),
                (3, yuan, yuan_tag),
                (3, peso, peso_tag),
                (6, yuan, yuan_tag),
            ).is_err()
        );
    }

    fn merge_helper(
        A: (u64, u64, u64),
        B: (u64, u64, u64),
        C: (u64, u64, u64),
        D: (u64, u64, u64),
    ) -> Result<(), R1CSError> {
        // Common
        let gens = Generators::new(PedersenGenerators::default(), 128, 1);

        // Prover's scope
        let (proof, commitments) = {
            // Prover makes a `ConstraintSystem` instance representing a merge gadget
            // v and v_blinding emptpy because we are only testing low-level variable constraints
            let v = vec![];
            let v_blinding = vec![];
            let mut prover_transcript = Transcript::new(b"MergeTest");
            let (mut prover_cs, _variables, commitments) =
                prover::ProverCS::new(&mut prover_transcript, &gens, v, v_blinding.clone());

            // Prover allocates variables and adds constraints to the constraint system
            let (A_q, B_q) =
                prover_cs.assign_uncommitted(Assignment::from(A.0), Assignment::from(B.0))?;
            let (C_q, D_q) =
                prover_cs.assign_uncommitted(Assignment::from(C.0), Assignment::from(D.0))?;
            let (A_a, B_a) =
                prover_cs.assign_uncommitted(Assignment::from(A.1), Assignment::from(B.1))?;
            let (C_a, D_a) =
                prover_cs.assign_uncommitted(Assignment::from(C.1), Assignment::from(D.1))?;
            let (A_t, B_t) =
                prover_cs.assign_uncommitted(Assignment::from(A.2), Assignment::from(B.2))?;
            let (C_t, D_t) =
                prover_cs.assign_uncommitted(Assignment::from(C.2), Assignment::from(D.2))?;
            let A = Value {
                q: (A_q, Assignment::from(A.0)),
                a: (A_a, Assignment::from(A.1)),
                t: (A_t, Assignment::from(A.2)),
            };
            let B = Value {
                q: (B_q, Assignment::from(B.0)),
                a: (B_a, Assignment::from(B.1)),
                t: (B_t, Assignment::from(B.2)),
            };
            let C = Value {
                q: (C_q, Assignment::from(C.0)),
                a: (C_a, Assignment::from(C.1)),
                t: (C_t, Assignment::from(C.2)),
            };
            let D = Value {
                q: (D_q, Assignment::from(D.0)),
                a: (D_a, Assignment::from(D.1)),
                t: (D_t, Assignment::from(D.2)),
            };
            assert!(MergeGadget::fill_cs(&mut prover_cs, A, B, C, D).is_ok());

            let proof = prover_cs.prove()?;

            (proof, commitments)
        };

        // Verifier makes a `ConstraintSystem` instance representing a merge gadget
        let mut verifier_transcript = Transcript::new(b"MergeTest");
        let (mut verifier_cs, _variables) =
            verifier::VerifierCS::new(&mut verifier_transcript, &gens, commitments);
        // Verifier allocates variables and adds constraints to the constraint system
        let (A_q, B_q) =
            verifier_cs.assign_uncommitted(Assignment::Missing(), Assignment::Missing())?;
        let (C_q, D_q) =
            verifier_cs.assign_uncommitted(Assignment::Missing(), Assignment::Missing())?;
        let (A_a, B_a) =
            verifier_cs.assign_uncommitted(Assignment::Missing(), Assignment::Missing())?;
        let (C_a, D_a) =
            verifier_cs.assign_uncommitted(Assignment::Missing(), Assignment::Missing())?;
        let (A_t, B_t) =
            verifier_cs.assign_uncommitted(Assignment::Missing(), Assignment::Missing())?;
        let (C_t, D_t) =
            verifier_cs.assign_uncommitted(Assignment::Missing(), Assignment::Missing())?;
        let A = Value {
            q: (A_q, Assignment::Missing()),
            a: (A_a, Assignment::Missing()),
            t: (A_t, Assignment::Missing()),
        };
        let B = Value {
            q: (B_q, Assignment::Missing()),
            a: (B_a, Assignment::Missing()),
            t: (B_t, Assignment::Missing()),
        };
        let C = Value {
            q: (C_q, Assignment::Missing()),
            a: (C_a, Assignment::Missing()),
            t: (C_t, Assignment::Missing()),
        };
        let D = Value {
            q: (D_q, Assignment::Missing()),
            a: (D_a, Assignment::Missing()),
            t: (D_t, Assignment::Missing()),
        };
        assert!(MergeGadget::fill_cs(&mut verifier_cs, A, B, C, D).is_ok());

        verifier_cs.verify(&proof)
    }
}
