//! Encoding utils for ZkVM
//! All methods err using VMError::FormatError for convenience.

use byteorder::{ByteOrder, LittleEndian};
use curve25519_dalek::ristretto::CompressedRistretto;
use curve25519_dalek::scalar::Scalar;

use crate::errors::VMError;

/// API for reading from byte slices and advancing internal cursor.
#[derive(Debug)]
pub struct SliceReader<'a> {
    whole: &'a [u8],
    start: usize,
    end: usize,
}

impl<'a> SliceReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        SliceReader {
            start: 0,
            end: data.len(),
            whole: data,
        }
    }

    /// Remaining number of the unread bytes.
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    /// Wraps the reading logic in a block that checks that all bytes have been read.
    /// If some are left unread, returns `Err(VMError::TrailingBytes)`.
    /// Use method `skip_trailing_bytes` to ignore trailing bytes.
    pub fn parse<F, T>(data: &'a [u8], parse_fn: F) -> Result<T, VMError>
    where
        F: FnOnce(&mut Self) -> Result<T, VMError>,
    {
        let mut reader = Self::new(data);
        let result = parse_fn(&mut reader)?;
        if reader.len() != 0 {
            return Err(VMError::TrailingBytes);
        }
        Ok(result)
    }

    /// Marks remaining unread bytes as read so that `parse` does not fail.
    /// After calling this method, no more bytes can be read.
    pub fn skip_trailing_bytes(&mut self) -> usize {
        let trailing = self.end - self.start;
        self.start = self.end;
        trailing
    }

    /// Returns a slice of the first `prefix_size` of bytes and advances
    /// the internal offset.
    pub fn read_bytes(&mut self, prefix_size: usize) -> Result<&[u8], VMError> {
        if prefix_size > self.len() {
            return Err(VMError::FormatError);
        }
        let prefix = &self.whole[self.start..(self.start + prefix_size)];
        self.start += prefix_size;
        Ok(prefix)
    }

    /// Reads a single byte.
    pub fn read_u8(&mut self) -> Result<u8, VMError> {
        let bytes = self.read_bytes(1)?;
        Ok(bytes[0])
    }

    /// Reads a 4-byte LE32 integer.
    pub fn read_u32(&mut self) -> Result<u32, VMError> {
        let bytes = self.read_bytes(4)?;
        let x = LittleEndian::read_u32(&bytes);
        Ok(x)
    }

    /// Reads an 8-byte LE64 integer.
    pub fn read_u64(&mut self) -> Result<u64, VMError> {
        let bytes = self.read_bytes(8)?;
        let x = LittleEndian::read_u64(&bytes);
        Ok(x)
    }

    /// Reads a 4-byte LE32 integer that's typically used as a length prefix.
    pub fn read_size(&mut self) -> Result<usize, VMError> {
        let n = self.read_u32()?;
        Ok(n as usize)
    }

    /// Reads a 32-byte string (typically a hash).
    pub fn read_u8x32(&mut self) -> Result<[u8; 32], VMError> {
        let mut buf = [0u8; 32];
        let bytes = self.read_bytes(32)?;
        buf[..].copy_from_slice(&bytes);
        Ok(buf)
    }

    /// Reads a 64-byte string (typically a signature).
    pub fn read_u8x64(&mut self) -> Result<[u8; 64], VMError> {
        let mut buf = [0u8; 64];
        let bytes = self.read_bytes(64)?;
        buf[..].copy_from_slice(&bytes);
        Ok(buf)
    }

    /// Reads a compressed Ristretto255 point (32 bytes).
    pub fn read_point(&mut self) -> Result<CompressedRistretto, VMError> {
        let buf = self.read_u8x32()?;
        Ok(CompressedRistretto(buf))
    }

    /// Reads a Ristretto255 scalar (32 bytes).
    pub fn read_scalar(&mut self) -> Result<Scalar, VMError> {
        let buf = self.read_u8x32()?;
        Scalar::from_canonical_bytes(buf).ok_or(VMError::FormatError)
    }
}

// Writing API
// This currently writes into the Vec, but later can be changed to support Arenas to minimize allocations

/// Writes a single byte.
pub fn write_u8<'a>(x: u8, target: &mut Vec<u8>) {
    target.push(x);
}

/// Writes a LE32-encoded integer.
pub fn write_u32<'a>(x: u32, target: &mut Vec<u8>) {
    let mut buf = [0u8; 4];
    LittleEndian::write_u32(&mut buf, x);
    target.extend_from_slice(&buf);
}

/// Writes a LE64-encoded integer.
pub fn write_u64<'a>(x: u64, target: &mut Vec<u8>) {
    let mut buf = [0u8; 8];
    LittleEndian::write_u64(&mut buf, x);
    target.extend_from_slice(&buf);
}

/// Writes a usize as a LE32-encoded integer.
pub fn write_size<'a>(x: usize, target: &mut Vec<u8>) {
    write_u32(x as u32, target);
}

/// Writes a 32-byte array and returns the subsequent slice.
pub fn write_bytes(x: &[u8], target: &mut Vec<u8>) {
    target.extend_from_slice(&x);
}

/// Writes a compressed point
pub fn write_point(x: &CompressedRistretto, target: &mut Vec<u8>) {
    write_bytes(x.as_bytes(), target);
}

/// A trait for consensus-critical encoding format for ZkVM data structures.
/// Note: serde is not used for consesus-critical operations.
pub trait Encodable {
    /// Encodes receiver into bytes appending them to a provided buffer.
    fn encode(&self, buf: &mut Vec<u8>);
    /// Returns precise length in bytes for the serialized representation of the receiver.
    fn encoded_length(&self) -> usize;
    /// Encodes the receiver into a newly allocated vector of bytes.
    fn encode_to_vec(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.encoded_length());
        self.encode(&mut buf);
        buf
    }
}
