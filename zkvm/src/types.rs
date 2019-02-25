//! Core ZkVM stack types: data, variables, values, contracts etc.

use bulletproofs::{r1cs, PedersenGens};
use curve25519_dalek::ristretto::CompressedRistretto;
use curve25519_dalek::scalar::Scalar;
use merlin::Transcript;
use spacesuit::SignedInteger;

use crate::contract::{Contract, Input, PortableItem};
use crate::encoding::SliceReader;
use crate::errors::VMError;
use crate::ops::Instruction;
use crate::predicate::Predicate;
use crate::transcript::TranscriptProtocol;

use std::ops::{Add, Mul, Neg};

#[derive(Debug)]
pub enum Item {
    Data(Data),
    Contract(Contract),
    Value(Value),
    WideValue(WideValue),
    Variable(Variable),
    Expression(Expression),
    Constraint(Constraint),
}

#[derive(Clone, Debug)]
pub enum Data {
    Opaque(Vec<u8>),
    Witness(DataWitness),
}

#[derive(Debug)]
pub struct Value {
    pub(crate) qty: Variable,
    pub(crate) flv: Variable,
}

#[derive(Debug)]
pub struct WideValue {
    pub(crate) r1cs_qty: r1cs::Variable,
    pub(crate) r1cs_flv: r1cs::Variable,
    pub(crate) witness: Option<(SignedInteger, Scalar)>,
}

#[derive(Copy, Clone, Debug)]
pub struct Variable {
    pub(crate) index: usize,
    // the witness is located indirectly in vm::VariableCommitment
}

pub type ExpressionTerm = (r1cs::Variable, Scalar);
#[derive(Clone, Debug)]
pub enum Expression {
    Constant(ScalarWitness),
    LinearCombination(Vec<ExpressionTerm>, Option<ScalarWitness>),
}

#[derive(Clone, Debug)]
pub enum Constraint {
    Eq(Expression, Expression),
    And(Vec<Constraint>),
    Or(Vec<Constraint>),
    // no witness needed as it's normally true/false and we derive it on the fly during processing.
    // this also allows us not to wrap this enum in a struct.
}

#[derive(Clone, Debug)]
pub enum Commitment {
    Closed(CompressedRistretto),
    Open(Box<CommitmentWitness>),
}

/// Prover's representation of the witness.
#[derive(Clone, Debug)]
pub enum DataWitness {
    Program(Vec<Instruction>),
    Predicate(Box<Predicate>),
    Commitment(Box<Commitment>),
    Scalar(Box<ScalarWitness>),
    Input(Box<Input>),
}

/// Prover's representation of the commitment secret: witness and blinding factor
#[derive(Clone, Debug)]
pub struct CommitmentWitness {
    pub value: ScalarWitness,
    pub blinding: Scalar,
}

/// Represents a concrete kind of a number represented by a scalar:
/// `ScalarKind::Integer` represents a signed integer with 64-bit absolute value (aka i65)
/// `ScalarKind::Scalar` represents a scalar modulo group order.
#[derive(Copy, Clone, Debug)]
pub enum ScalarWitness {
    Integer(SignedInteger),
    Scalar(Scalar),
}

impl Commitment {
    /// Returns the number of bytes needed to serialize the Commitment.
    pub fn serialized_length(&self) -> usize {
        32
    }

    /// Converts a Commitment to a compressed point.
    pub fn to_point(&self) -> CompressedRistretto {
        match self {
            Commitment::Closed(x) => *x,
            Commitment::Open(w) => w.to_point(),
        }
    }

    pub(crate) fn encode(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.to_point().to_bytes());
    }
}

impl CommitmentWitness {
    pub fn to_point(&self) -> CompressedRistretto {
        let gens = PedersenGens::default();
        gens.commit(self.value.into(), self.blinding).compress()
    }

    pub fn unblinded<T>(x: T) -> Self
    where
        T: Into<ScalarWitness>,
    {
        CommitmentWitness {
            blinding: Scalar::zero(),
            value: x.into(),
        }
    }

    pub fn blinded<T>(x: T) -> Self
    where
        T: Into<ScalarWitness>,
    {
        CommitmentWitness {
            blinding: Scalar::random(&mut rand::thread_rng()),
            value: x.into(),
        }
    }
}

impl ScalarWitness {
    /// Returns the number of bytes needed to serialize the ScalarWitness.
    pub fn serialized_length(&self) -> usize {
        32
    }

    pub(crate) fn encode(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.to_scalar().to_bytes())
    }

    /// Converts the witness to an integer if it is an integer
    pub fn to_integer(self) -> Result<SignedInteger, VMError> {
        match self {
            ScalarWitness::Integer(i) => Ok(i),
            ScalarWitness::Scalar(_) => Err(VMError::TypeNotSignedInteger),
        }
    }

    // Converts the witness to a scalar.
    pub fn to_scalar(self) -> Scalar {
        match self {
            ScalarWitness::Integer(i) => i.into(),
            ScalarWitness::Scalar(s) => s,
        }
    }

    /// Converts `Option<ScalarWitness>` into optional integer if it is one.
    pub fn option_to_integer(assignment: Option<Self>) -> Result<Option<SignedInteger>, VMError> {
        match assignment {
            None => Ok(None),
            Some(ScalarWitness::Integer(i)) => Ok(Some(i)),
            Some(ScalarWitness::Scalar(_)) => Err(VMError::TypeNotSignedInteger),
        }
    }
}

impl Neg for ScalarWitness {
    type Output = ScalarWitness;

    fn neg(self) -> ScalarWitness {
        match self {
            ScalarWitness::Integer(a) => ScalarWitness::Integer(-a),
            ScalarWitness::Scalar(a) => ScalarWitness::Scalar(-a),
        }
    }
}
impl Add for ScalarWitness {
    type Output = ScalarWitness;

    fn add(self, rhs: ScalarWitness) -> ScalarWitness {
        match (self, rhs) {
            (ScalarWitness::Integer(a), ScalarWitness::Integer(b)) => match a + b {
                Some(res) => ScalarWitness::Integer(res),
                None => ScalarWitness::Scalar(a.to_scalar() + b.to_scalar()),
            },
            (a, b) => ScalarWitness::Scalar(a.to_scalar() + b.to_scalar()),
        }
    }
}

impl Mul for ScalarWitness {
    type Output = ScalarWitness;

    fn mul(self, rhs: ScalarWitness) -> ScalarWitness {
        match (self, rhs) {
            (ScalarWitness::Integer(a), ScalarWitness::Integer(b)) => match a * b {
                Some(res) => ScalarWitness::Integer(res),
                None => ScalarWitness::Scalar(a.to_scalar() * b.to_scalar()),
            },
            (a, b) => ScalarWitness::Scalar(a.to_scalar() * b.to_scalar()),
        }
    }
}

impl Into<Scalar> for ScalarWitness {
    fn into(self) -> Scalar {
        self.to_scalar()
    }
}

impl Item {
    // Downcasts to Data type
    pub fn to_data(self) -> Result<Data, VMError> {
        match self {
            Item::Data(x) => Ok(x),
            _ => Err(VMError::TypeNotData),
        }
    }

    // Downcasts to a portable type
    pub fn to_portable(self) -> Result<PortableItem, VMError> {
        match self {
            Item::Data(x) => Ok(PortableItem::Data(x)),
            Item::Value(x) => Ok(PortableItem::Value(x)),
            _ => Err(VMError::TypeNotPortable),
        }
    }

    // Downcasts to Variable type
    pub fn to_variable(self) -> Result<Variable, VMError> {
        match self {
            Item::Variable(v) => Ok(v),
            _ => Err(VMError::TypeNotVariable),
        }
    }

    // Downcasts to Expression type (Variable is NOT casted to Expression)
    pub fn to_expression(self) -> Result<Expression, VMError> {
        match self {
            Item::Expression(expr) => Ok(expr),
            _ => Err(VMError::TypeNotExpression),
        }
    }

    // Downcasts to Value type
    pub fn to_value(self) -> Result<Value, VMError> {
        match self {
            Item::Value(v) => Ok(v),
            _ => Err(VMError::TypeNotValue),
        }
    }

    // Downcasts to WideValue type (Value is NOT casted to WideValue)
    pub fn to_wide_value(self) -> Result<WideValue, VMError> {
        match self {
            Item::WideValue(w) => Ok(w),
            _ => Err(VMError::TypeNotWideValue),
        }
    }

    // Downcasts to Contract type
    pub fn to_contract(self) -> Result<Contract, VMError> {
        match self {
            Item::Contract(c) => Ok(c),
            _ => Err(VMError::TypeNotContract),
        }
    }
}

impl Data {
    /// Returns the number of bytes needed to serialize the Data.
    pub fn serialized_length(&self) -> usize {
        match self {
            Data::Opaque(data) => data.len(),
            Data::Witness(x) => x.serialized_length(),
        }
    }

    /// Converts the Data into a vector of bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            Data::Opaque(data) => data.clone(),
            Data::Witness(w) => {
                let mut bytes: Vec<u8> = Vec::with_capacity(self.serialized_length());
                w.encode(&mut bytes);
                bytes.clone()
            }
        }
    }

    /// Downcast to a Predicate type.
    pub fn to_predicate(self) -> Result<Predicate, VMError> {
        match self {
            Data::Opaque(data) => {
                let point = SliceReader::parse(&data, |r| r.read_point())?;
                Ok(Predicate::Opaque(point))
            }
            Data::Witness(witness) => match witness {
                DataWitness::Predicate(boxed_pred) => Ok(*boxed_pred),
                _ => Err(VMError::TypeNotPredicate),
            },
        }
    }

    pub fn to_commitment(self) -> Result<Commitment, VMError> {
        match self {
            Data::Opaque(data) => {
                let point = SliceReader::parse(&data, |r| r.read_point())?;
                Ok(Commitment::Closed(point))
            }
            Data::Witness(witness) => match witness {
                DataWitness::Commitment(w) => Ok(*w),
                _ => Err(VMError::TypeNotCommitment),
            },
        }
    }

    pub fn to_input(self) -> Result<Input, VMError> {
        match self {
            Data::Opaque(data) => Input::from_bytes(data),
            Data::Witness(witness) => match witness {
                DataWitness::Input(i) => Ok(*i),
                _ => Err(VMError::TypeNotInput),
            },
        }
    }

    /// Encodes blinded Data values.
    pub fn encode(&self, buf: &mut Vec<u8>) {
        match self {
            Data::Opaque(x) => {
                buf.extend_from_slice(x);
                return;
            }
            Data::Witness(w) => {
                w.encode(buf);
            }
        };
    }
}

impl DataWitness {
    fn encode(&self, buf: &mut Vec<u8>) {
        match self {
            DataWitness::Program(instr) => Instruction::encode_program(instr.iter(), buf),
            DataWitness::Predicate(p) => p.encode(buf),
            DataWitness::Commitment(c) => c.encode(buf),
            DataWitness::Scalar(s) => s.encode(buf),
            DataWitness::Input(b) => b.encode(buf),
        }
    }

    fn serialized_length(&self) -> usize {
        match self {
            DataWitness::Program(instr) => instr.iter().map(|p| p.serialized_length()).sum(),
            DataWitness::Input(b) => 32 + b.contract.serialized_length(),
            DataWitness::Predicate(p) => p.serialized_length(),
            DataWitness::Commitment(c) => c.serialized_length(),
            DataWitness::Scalar(s) => s.serialized_length(),
        }
    }
}

impl Value {
    /// Computes a flavor as defined by the `issue` instruction from a predicate.
    pub fn issue_flavor(predicate: &Predicate) -> Scalar {
        let mut t = Transcript::new(b"ZkVM.issue");
        t.commit_bytes(b"predicate", predicate.to_point().as_bytes());
        t.challenge_scalar(b"flavor")
    }
}

impl Expression {
    pub fn constant<S: Into<ScalarWitness>>(a: S) -> Self {
        Expression::Constant(a.into())
    }
}

impl Neg for Expression {
    type Output = Expression;

    fn neg(self) -> Expression {
        match self {
            Expression::Constant(a) => Expression::Constant(-a),
            Expression::LinearCombination(mut terms, assignment) => {
                for (_, n) in terms.iter_mut() {
                    *n = -*n;
                }
                Expression::LinearCombination(terms, assignment.map(|a| -a))
            }
        }
    }
}

impl Add for Expression {
    type Output = Expression;

    fn add(self, rhs: Expression) -> Expression {
        match (self, rhs) {
            (Expression::Constant(left), Expression::Constant(right)) => {
                Expression::Constant(left + right)
            }
            (
                Expression::Constant(l),
                Expression::LinearCombination(mut right_terms, right_assignment),
            ) => {
                // prepend constant term to `term vector` in non-constant expression
                right_terms.insert(0, (r1cs::Variable::One(), l.into()));
                Expression::LinearCombination(right_terms, right_assignment.map(|r| l + r))
            }
            (
                Expression::LinearCombination(mut left_terms, left_assignment),
                Expression::Constant(r),
            ) => {
                // append constant term to term vector in non-constant expression
                left_terms.push((r1cs::Variable::One(), r.into()));
                Expression::LinearCombination(left_terms, left_assignment.map(|l| l + r))
            }
            (
                Expression::LinearCombination(mut left_terms, left_assignment),
                Expression::LinearCombination(right_terms, right_assignment),
            ) => {
                // append right terms to left terms in non-constant expression
                left_terms.extend(right_terms);
                Expression::LinearCombination(
                    left_terms,
                    left_assignment.and_then(|l| right_assignment.map(|r| l + r)),
                )
            }
        }
    }
}

// Upcasting witness/points into Commitment

impl From<CommitmentWitness> for Commitment {
    fn from(x: CommitmentWitness) -> Self {
        Commitment::Open(Box::new(x))
    }
}

impl From<CompressedRistretto> for Commitment {
    fn from(x: CompressedRistretto) -> Self {
        Commitment::Closed(x)
    }
}

// Upcasting integers/scalars into ScalarWitness

impl From<u64> for ScalarWitness {
    fn from(x: u64) -> Self {
        ScalarWitness::Integer(x.into())
    }
}

impl From<Scalar> for ScalarWitness {
    fn from(x: Scalar) -> Self {
        ScalarWitness::Scalar(x)
    }
}

// Upcasting all witness data types to Data and DataWitness

// Anything convertible to DataWitness is also convertible to Data
impl<T> From<T> for Data
where
    T: Into<DataWitness>,
{
    fn from(w: T) -> Self {
        Data::Witness(w.into())
    }
}

impl<T> From<T> for DataWitness
where
    T: Into<ScalarWitness>,
{
    fn from(x: T) -> Self {
        DataWitness::Scalar(Box::new(x.into()))
    }
}

impl From<Predicate> for DataWitness {
    fn from(x: Predicate) -> Self {
        DataWitness::Predicate(Box::new(x))
    }
}

impl From<Commitment> for DataWitness {
    fn from(x: Commitment) -> Self {
        DataWitness::Commitment(Box::new(x))
    }
}

impl From<Input> for DataWitness {
    fn from(x: Input) -> Self {
        DataWitness::Input(Box::new(x))
    }
}

// Upcasting all types to Item

impl From<Data> for Item {
    fn from(x: Data) -> Self {
        Item::Data(x)
    }
}

impl From<Value> for Item {
    fn from(x: Value) -> Self {
        Item::Value(x)
    }
}

impl From<WideValue> for Item {
    fn from(x: WideValue) -> Self {
        Item::WideValue(x)
    }
}

impl From<Contract> for Item {
    fn from(x: Contract) -> Self {
        Item::Contract(x)
    }
}

impl From<Variable> for Item {
    fn from(x: Variable) -> Self {
        Item::Variable(x)
    }
}

impl From<Expression> for Item {
    fn from(x: Expression) -> Self {
        Item::Expression(x)
    }
}

impl From<Constraint> for Item {
    fn from(x: Constraint) -> Self {
        Item::Constraint(x)
    }
}

// Upcast a portable item to any item
impl From<PortableItem> for Item {
    fn from(portable: PortableItem) -> Self {
        match portable {
            PortableItem::Data(x) => Item::Data(x),
            PortableItem::Value(x) => Item::Value(x),
        }
    }
}
