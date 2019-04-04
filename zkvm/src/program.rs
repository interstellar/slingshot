use crate::encoding::SliceReader;
use crate::errors::VMError;
use crate::ops::Instruction;
use crate::predicate::Predicate;
use crate::scalar_witness::ScalarWitness;
use crate::types::Data;
use core::borrow::Borrow;
use spacesuit::BitRange;

/// A builder type for assembling a sequence of `Instruction`s with chained method calls.
/// E.g. `let prog = Program::new().push(...).input().push(...).output(1).to_vec()`.
#[derive(Clone, Debug)]
pub struct Program(Vec<Instruction>);

macro_rules! def_op {
    ($func_name:ident, $op:ident) => (
           /// Adds a `$func_name` instruction.
           pub fn $func_name(&mut self) -> &mut Program{
             self.0.push(Instruction::$op);
             self
        }
    );
    ($func_name:ident, $op:ident, $type:ty) => (
           /// Adds a `$func_name` instruction.
           pub fn $func_name(&mut self, arg :$type) -> &mut Program{
             self.0.push(Instruction::$op(arg));
             self
        }
    );
    ($func_name:ident, $op:ident, $type1:ty, $type2:ty) => (
           /// Adds a `$func_name` instruction.
           pub fn $func_name(&mut self, arg1: $type1, arg2: $type2) -> &mut Program{
             self.0.push(Instruction::$op(arg1, arg2));
             self
        }
    );
}

impl Program {
    def_op!(add, Add);
    def_op!(alloc, Alloc, Option<ScalarWitness>);
    def_op!(and, And);
    def_op!(borrow, Borrow);
    def_op!(call, Call);
    def_op!(cloak, Cloak, usize, usize);
    def_op!(r#const, Const);
    def_op!(contract, Contract, usize);
    def_op!(delegate, Delegate);
    def_op!(drop, Drop);
    def_op!(dup, Dup, usize);
    def_op!(eq, Eq);
    def_op!(export, Export);
    def_op!(expr, Expr);
    def_op!(import, Import);
    def_op!(input, Input);
    def_op!(issue, Issue);
    def_op!(log, Log);
    def_op!(maxtime, Maxtime);
    def_op!(mintime, Mintime);
    def_op!(mul, Mul);
    def_op!(neg, Neg);
    def_op!(nonce, Nonce);
    def_op!(or, Or);
    def_op!(output, Output, usize);
    def_op!(range, Range, BitRange);
    def_op!(retire, Retire);
    def_op!(roll, Roll, usize);
    def_op!(select, Select, u8, u8);
    def_op!(sign_tx, Signtx);
    def_op!(unblind, Unblind);
    def_op!(var, Var);
    def_op!(verify, Verify);

    /// Creates an empty `Program`.
    pub fn new() -> Self {
        Program(vec![])
    }

    /// Creates an empty `Program` and passes its &mut to the closure to let it add the instructions.
    /// Returns the resulting program.
    pub fn build<F>(builder: F) -> Self
    where
        F: FnOnce(&mut Self) -> &mut Self,
    {
        let mut program = Self::new();
        builder(&mut program);
        program
    }

    /// Creates a program from parsing the opaque data slice of encoded instructions.
    pub(crate) fn parse(data: &[u8]) -> Result<Self, VMError> {
        SliceReader::parse(data, |r| {
            let mut program = Self::new();
            while r.len() > 0 {
                program.0.push(Instruction::parse(r)?);
            }
            Ok(program)
        })
    }

    /// Converts the program to a plain vector of instructions.
    pub fn to_vec(self) -> Vec<Instruction> {
        self.0
    }

    /// Returns the serialized length of the program.
    pub(crate) fn serialized_length(&self) -> usize {
        self.0.iter().map(|p| p.serialized_length()).sum()
    }

    /// Encodes a program into a buffer.
    pub(crate) fn encode(&self, buf: &mut Vec<u8>) {
        for i in self.0.iter() {
            i.borrow().encode(buf);
        }
    }

    /// Adds a `push` instruction with an immediate data type that can be converted into `Data`.
    pub fn push<T: Into<Data>>(&mut self, data: T) -> &mut Program {
        self.0.push(Instruction::Push(data.into()));
        self
    }

    /// Takes predicate and closure to add choose operations for
    /// predicate tree traversal.
    pub fn choose_predicate<F, T>(
        &mut self,
        pred: Predicate,
        choose_fn: F,
    ) -> Result<&mut Program, VMError>
    where
        F: FnOnce(PredicateTree) -> Result<T, VMError>,
    {
        choose_fn(PredicateTree {
            prog: self,
            pred: pred,
        })?;
        Ok(self)
    }
}

/// Adds data and instructions to traverse a predicate tree.
pub struct PredicateTree<'a> {
    prog: &'a mut Program,
    pred: Predicate,
}

impl<'a> PredicateTree<'a> {
    /// Kth Predicate branch
    pub fn select(self, k: usize) -> Result<Self, VMError> {
        let preds = self.pred.to_disjunction()?;
        let n = preds.len();
        let selected = preds[k].clone();
        if k >= n {
            return Err(VMError::PredicateIndexInvalid);
        }
        let prog = self.prog;
        for pred in preds.iter() {
            prog.push(pred.as_opaque());
        }
        prog.select(n as u8, k as u8);
        Ok(Self {
            pred: selected,
            prog,
        })
    }

    /// Pushes program to the stack and calls the contract protected
    /// by the program predicate.
    pub fn call(self) -> Result<(), VMError> {
        let (subprog, blinding) = self.pred.to_program()?;
        self.prog.push(Data::Opaque(blinding)).call();
        self.prog.push(subprog).call();
        Ok(())
    }
}
