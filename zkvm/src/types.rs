//! Core ZkVM stack types: data, variables, values, contracts etc.

use bulletproofs::r1cs;
use curve25519_dalek::scalar::Scalar;
use merlin::Transcript;
use spacesuit::SignedInteger;

use crate::constraints::{Commitment, Constraint, Expression, Variable};
use crate::contract::{Contract, Output, PortableItem};
use crate::encoding::SliceReader;
use crate::errors::VMError;
use crate::predicate::Predicate;
use crate::program::Program;
use crate::scalar_witness::ScalarWitness;
use crate::transcript::TranscriptProtocol;

/// An item on a VM stack.
#[derive(Debug)]
pub enum Item {
    /// A data item.
    Data(Data),

    /// A contract.
    Contract(Contract),

    /// A value type.
    Value(Value),

    /// A wide value type.
    WideValue(WideValue),

    /// A variable type.
    Variable(Variable),

    /// An expression type.
    Expression(Expression),

    /// A constraint type.
    Constraint(Constraint),
}

/// A data item.
#[derive(Clone, Debug)]
pub enum Data {
    /// Opaque data item.
    Opaque(Vec<u8>),

    /// A program.
    Program(Program),

    /// A predicate.
    Predicate(Box<Predicate>),

    /// A Pedersen commitment.
    Commitment(Box<Commitment>),

    /// A scalar witness (scalar or integer).
    Scalar(Box<ScalarWitness>),

    /// An input object (claimed UTXO).
    Output(Box<Output>),
}

/// Represents a value of an issued asset in the VM.
/// Note: values do not necessarily have open commitments. Some can be reblinded,
/// others can be passed-through to an output without going through `cloak` and the constraint system.
#[derive(Clone, Debug)]
pub struct Value {
    /// Commitment to value's quantity
    pub qty: Commitment,
    /// Commitment to value's flavor
    pub flv: Commitment,
}

/// A wide value type (for negative values created by `borrow`).
#[derive(Debug)]
pub struct WideValue {
    pub(crate) r1cs_qty: r1cs::Variable,
    pub(crate) r1cs_flv: r1cs::Variable,
    pub(crate) witness: Option<(SignedInteger, Scalar)>,
}

impl Item {
    /// Downcasts item to `Data` type.
    pub fn to_data(self) -> Result<Data, VMError> {
        match self {
            Item::Data(x) => Ok(x),
            _ => Err(VMError::TypeNotData),
        }
    }

    /// Downcasts item to `Contract` type.
    pub fn to_contract(self) -> Result<Contract, VMError> {
        match self {
            Item::Contract(c) => Ok(c),
            _ => Err(VMError::TypeNotContract),
        }
    }

    /// Downcasts item to `Value` type.
    pub fn to_value(self) -> Result<Value, VMError> {
        match self {
            Item::Value(v) => Ok(v),
            _ => Err(VMError::TypeNotValue),
        }
    }

    /// Downcasts item to `WideValue` type (Value is NOT casted to WideValue).
    pub fn to_wide_value(self) -> Result<WideValue, VMError> {
        match self {
            Item::WideValue(w) => Ok(w),
            _ => Err(VMError::TypeNotWideValue),
        }
    }

    /// Downcasts item to `Variable` type.
    pub fn to_variable(self) -> Result<Variable, VMError> {
        match self {
            Item::Variable(v) => Ok(v),
            _ => Err(VMError::TypeNotVariable),
        }
    }

    /// Downcasts item to `Expression` type (Variable is NOT casted to Expression).
    pub fn to_expression(self) -> Result<Expression, VMError> {
        match self {
            Item::Expression(expr) => Ok(expr),
            _ => Err(VMError::TypeNotExpression),
        }
    }

    /// Downcasts item to `Constraint` type.
    pub fn to_constraint(self) -> Result<Constraint, VMError> {
        match self {
            Item::Constraint(c) => Ok(c),
            _ => Err(VMError::TypeNotConstraint),
        }
    }

    /// Downcasts item to a portable type (`Data` or `Value`).
    pub fn to_portable(self) -> Result<PortableItem, VMError> {
        match self {
            Item::Data(x) => Ok(PortableItem::Data(x)),
            Item::Value(x) => Ok(PortableItem::Value(x)),
            _ => Err(VMError::TypeNotPortable),
        }
    }
}

impl Data {
    /// Returns the number of bytes needed to serialize the Data.
    pub fn serialized_length(&self) -> usize {
        match self {
            Data::Opaque(data) => data.len(),
            Data::Program(program) => program.serialized_length(),
            Data::Predicate(predicate) => predicate.serialized_length(),
            Data::Commitment(commitment) => commitment.serialized_length(),
            Data::Scalar(scalar) => scalar.serialized_length(),
            Data::Output(output) => output.serialized_length(),
        }
    }

    /// Converts the Data item into a vector of bytes.
    /// Opaque item is converted without extra allocations,
    /// non-opaque item is encoded to a newly allocated buffer.
    pub fn to_bytes(self) -> Vec<u8> {
        match self {
            Data::Opaque(d) => d,
            _ => {
                let mut buf = Vec::with_capacity(self.serialized_length());
                self.encode(&mut buf);
                buf
            }
        }
    }

    /// Downcast the data item to a `Predicate` type.
    pub fn to_predicate(self) -> Result<Predicate, VMError> {
        match self {
            Data::Opaque(data) => {
                let point = SliceReader::parse(&data, |r| r.read_point())?;
                Ok(Predicate::Opaque(point))
            }
            Data::Predicate(p) => Ok(*p),
            _ => Err(VMError::TypeNotPredicate),
        }
    }

    /// Downcast the data item to a `Program` type.
    pub fn to_program(self) -> Result<Program, VMError> {
        match self {
            Data::Opaque(data) => Program::parse(&data),
            Data::Program(program) => Ok(program),
            _ => Err(VMError::TypeNotProgram),
        }
    }

    /// Downcast the data item to a `Commitment` type.
    pub fn to_commitment(self) -> Result<Commitment, VMError> {
        match self {
            Data::Opaque(data) => {
                let point = SliceReader::parse(&data, |r| r.read_point())?;
                Ok(Commitment::Closed(point))
            }
            Data::Commitment(c) => Ok(*c),
            _ => Err(VMError::TypeNotCommitment),
        }
    }

    /// Downcast the data item to an `Output` type.
    pub fn to_output(self) -> Result<Output, VMError> {
        match self {
            Data::Opaque(data) => SliceReader::parse(&data, |r| Output::decode(r)),
            Data::Output(i) => Ok(*i),
            _ => Err(VMError::TypeNotOutput),
        }
    }

    /// Downcast the data item to an `ScalarWitness` type.
    pub fn to_scalar(self) -> Result<ScalarWitness, VMError> {
        match self {
            Data::Opaque(data) => {
                let scalar = SliceReader::parse(&data, |r| r.read_scalar())?;
                Ok(ScalarWitness::Scalar(scalar))
            }
            Data::Scalar(scalar_witness) => Ok(*scalar_witness),
            _ => Err(VMError::TypeNotScalar),
        }
    }

    /// Encodes the data item to an opaque bytestring.
    pub fn encode(&self, buf: &mut Vec<u8>) {
        match self {
            Data::Opaque(x) => buf.extend_from_slice(x),
            Data::Program(program) => program.encode(buf),
            Data::Predicate(predicate) => predicate.encode(buf),
            Data::Commitment(commitment) => commitment.encode(buf),
            Data::Scalar(scalar) => scalar.encode(buf),
            Data::Output(output) => output.encode(buf),
        };
    }
}

impl Default for Data {
    fn default() -> Self {
        Data::Opaque(Vec::new())
    }
}

impl Value {
    /// Computes a flavor as defined by the `issue` instruction from a predicate.
    pub fn issue_flavor(predicate: &Predicate, metadata: Data) -> Scalar {
        let mut t = Transcript::new(b"ZkVM.issue");
        t.commit_bytes(b"predicate", predicate.to_point().as_bytes());
        t.commit_bytes(b"metadata", &metadata.to_bytes());
        t.challenge_scalar(b"flavor")
    }

    /// Returns a (qty,flavor) assignment to a value, or None if both fields are unassigned.
    /// Fails if the assigment is inconsistent.
    pub(crate) fn assignment(&self) -> Result<Option<(SignedInteger, Scalar)>, VMError> {
        match (self.qty.assignment(), self.flv.assignment()) {
            (None, None) => Ok(None),
            (Some(ScalarWitness::Integer(q)), Some(ScalarWitness::Scalar(f))) => Ok(Some((q, f))),
            (_, _) => return Err(VMError::InconsistentWitness),
        }
    }
}

// Upcasting all witness data types to Data

impl<T> From<T> for Data
where
    T: Into<ScalarWitness>,
{
    fn from(x: T) -> Self {
        Data::Scalar(Box::new(x.into()))
    }
}

impl From<Program> for Data {
    fn from(x: Program) -> Self {
        Data::Program(x)
    }
}

impl From<Predicate> for Data {
    fn from(x: Predicate) -> Self {
        Data::Predicate(Box::new(x))
    }
}

impl From<Commitment> for Data {
    fn from(x: Commitment) -> Self {
        Data::Commitment(Box::new(x))
    }
}

impl From<Output> for Data {
    fn from(x: Output) -> Self {
        Data::Output(Box::new(x))
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
