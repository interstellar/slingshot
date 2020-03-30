//! ```ascii
//! ┌──────────────────────────────────────────────────────────────────────────────────────┐
//! │    _______ __   __ ______  _______  ______ _______ _     _ _______ _     _ _______   │
//! │    |         \_/   |_____] |______ |_____/ |______ |_____| |_____| |____/  |______   │
//! │    |_____     |    |_____] |______ |    \_ ______| |     | |     | |    \_ |______   │
//! │                                                                                      │
//! └──────────────────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # C Y B E R S H A K E
//!
//! Yet Another Handshake Protocol for p2p sessions.
//!
//! You start with a local private key, remote public key (optional),
//! and a pair of `AsyncRead` and `AsyncWrite` interfaces.
//!
//! The protocol performs mutual authentication and, if it succeeded,
//! returns a pair of wrappers around these interfaces,
//! that keep track of the encryption keys.
//!
//! ## Features
//!
//! * **Symmetric and low-latency.** Handshake is performed by both ends simultaneously.
//! * **Mutual-authentication.** Each party receives other's long-term public key by the end of handshake.
//! * **Key blinding.** Long-term identity keys are never transmitted in the clear.
//! * **Foward secrecy.** Keys are rotated on each sent message.
//! * **Robust encryption.** cipher AES-SIV-PMAC-128 provides high speed and resistance to nonce-misuse.
//!
//! ## TODO
//!
//! * Streaming API to send larger portions of data wrapped in async streams.
//! * Add custom header to be sent in the first encrypted frame:
//!   users can put the protocol version there, certificate info etc.

use byteorder::{ByteOrder, LittleEndian};
use core::marker::Unpin;
use miscreant::{generic_array::GenericArray, Aes128PmacSiv};
use rand_core::{CryptoRng, RngCore};

use curve25519_dalek::constants::RISTRETTO_BASEPOINT_POINT;
use curve25519_dalek::ristretto::{CompressedRistretto, RistrettoPoint};
use curve25519_dalek::scalar::Scalar;
use curve25519_dalek::traits::VartimeMultiscalarMul;
use merlin::Transcript; // TODO: change for raw Strobe.

use tokio::io;
use tokio::prelude::*;

use futures::task::{Context, Poll};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::pin::Pin;

/// The current version of the protocol is 0.
/// In the future we may add more versions, version bits or whatever.
const ONLY_SUPPORTED_VERSION: u64 = 0;
const BUF_SIZE: u16 = 4096;

/// Private key for encrypting and authenticating connection.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct PrivateKey {
    secret: Scalar,
    pubkey: PublicKey,
}

/// Public key for authenticating connection.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct PublicKey {
    point: CompressedRistretto,
}

/// An endpoint for sending messages to remote party.
/// All messages are ordered and encryption key is ratcheted after each sent message.
pub struct Outgoing<W: io::AsyncWrite + Unpin> {
    writer: Pin<Box<W>>,
    seq: u64,
    kdf: Transcript,
    plaintext_buf: Vec<u8>,
    ciphertext_buf: Vec<u8>,
    plaintext_needs_flushing: bool,
    ciphertext_sent: usize,
}

/// An endpoint for receiving messages from a remote party.
/// All messages are ordered and encryption key is ratcheted after each received message.
/// Recipient's incoming.seq corresponds to the sender's outgoing.seq.
pub struct Incoming<R: io::AsyncRead + Unpin> {
    reader: Pin<Box<R>>,
    seq: u64,
    kdf: Transcript,
    ciphertext_buf: Vec<u8>,
    plaintext_buf: Vec<u8>,
    plaintext_read: usize,
    need_to_get: u16,
    now_read: u16,
}

/// Kinds of failures that may happen during the handshake.
#[derive(Debug)]
pub enum Error {
    /// I/O error (connection closed, not enough data, etc).
    IoError(io::Error),

    /// Point failed to decode correctly.
    ProtocolError,

    /// Received message is declared too large - not reading.
    MessageTooLong(usize),

    /// Version used by remote peer is not supported.
    UnsupportedVersion,
}

/// Performs the key exchange with a remote end using byte-oriented read- and write- interfaces
/// (e.g. TcpSocket halves).
/// Returns the identity key of the remote peer, along with read- and write- interfaces
/// that perform encryption and authentication behind the scenes.
/// If you need to verify the identity per local policy or certificates, use the returned public key.
pub async fn cybershake<R, W, RNG>(
    local_identity: &PrivateKey,
    mut reader: R,
    mut writer: W,
    rng: &mut RNG,
) -> Result<(PublicKey, Outgoing<W>, Incoming<R>), Error>
where
    R: io::AsyncRead + Unpin,
    W: io::AsyncWrite + Unpin,
    RNG: RngCore + CryptoRng,
{
    // We are going to need an additional ephemeral D-H key,
    // and a salt for blinding the reusable identity key.

    let mut keygen_rng = Transcript::new(b"Cybershake.randomness")
        .build_rng()
        .rekey_with_witness_bytes(b"local_privkey", local_identity.as_secret_bytes())
        .finalize(rng);

    let local_ephemeral = PrivateKey::from(Scalar::random(&mut keygen_rng));

    const SALT_LEN: usize = 16;
    let mut local_salt = [0u8; SALT_LEN];
    keygen_rng.fill_bytes(&mut local_salt[..]);
    let local_blinded_identity = local_identity.blind(&local_salt);

    // Now we send our first, unencrypted, message:
    //
    // [version] [blinded local identity pubkey] [local ephemeral pubkey]
    // u64-le     32 bytes                        32 bytes
    writer
        .write(&encode_u64le(ONLY_SUPPORTED_VERSION)[..])
        .await?;
    writer
        .write(local_blinded_identity.pubkey.as_bytes())
        .await?;
    writer.write(local_ephemeral.pubkey.as_bytes()).await?;
    writer.flush().await?;

    // Receive the similar message from the other end (that was sent simultaneously).
    let mut remote_version_buf = [0u8; 8];
    reader.read_exact(&mut remote_version_buf[..]).await?;
    let remote_version = LittleEndian::read_u64(&remote_version_buf);
    if remote_version != ONLY_SUPPORTED_VERSION {
        return Err(Error::UnsupportedVersion);
    }
    let remote_blinded_identity = PublicKey::read_from(&mut reader).await?;
    let remote_ephemeral = PublicKey::read_from(&mut reader).await?;

    // Now, perform a triple Diffie-Hellman shared key generation.
    let t = cybershake_x3dh(
        &local_blinded_identity,
        &local_ephemeral,
        &remote_blinded_identity,
        &remote_ephemeral,
    )?;

    // We will have two independent derivations of the shared key:
    // one for the outgoing messages, and another one for incoming messages.
    let mut kdf_outgoing = t.clone();
    let mut kdf_incoming = t;
    kdf_outgoing.append_message(b"src", local_blinded_identity.pubkey.as_bytes());
    kdf_incoming.append_message(b"src", remote_blinded_identity.as_bytes());

    // Now we prepare endpoints for reading and writing messages,
    // but don't give them to the user until we authenticate the connection.
    let mut outgoing = Outgoing {
        writer: Box::pin(writer),
        seq: 0,
        kdf: kdf_outgoing,
        plaintext_buf: Vec::with_capacity(BUF_SIZE as usize),
        ciphertext_buf: Vec::with_capacity(BUF_SIZE as usize + 2), // 2 - length of buffer
        plaintext_needs_flushing: false,
        ciphertext_sent: 0,
    };
    let mut incoming = Incoming {
        reader: Box::pin(reader),
        seq: 0,
        kdf: kdf_incoming,
        ciphertext_buf: vec![0u8; BUF_SIZE as usize],
        plaintext_buf: Vec::with_capacity(BUF_SIZE as usize), // TODO: allow user redefine this parameter
        plaintext_read: 0,
        need_to_get: 0,
        now_read: 0,
    };

    // In order to authenticate the session, we send our first encrypted message
    // in which we show the salt and the root key.
    // If the transmission was successful (authenticated decryption succeeded),
    // we check the blinded key and then let user continue using the session.

    // Prepare and send the message: salt and local identity pubkey.
    let msg_len = SALT_LEN + 32;
    let mut local_salt_and_id = Vec::<u8>::with_capacity(msg_len);
    local_salt_and_id.extend_from_slice(&local_salt[..]);
    local_salt_and_id.extend_from_slice(local_identity.pubkey.as_bytes());
    outgoing.send_message(&local_salt_and_id).await?;

    // Receive the message from another end: their salt and their identity pubkey.
    let remote_salt_and_id = incoming.receive_message().await?;
    if remote_salt_and_id.len() != msg_len {
        return Err(Error::ProtocolError);
    }
    let mut remote_salt = [0u8; SALT_LEN];
    remote_salt[..].copy_from_slice(&remote_salt_and_id[0..SALT_LEN]);
    let received_remote_identity =
        PublicKey::read_from(&mut &remote_salt_and_id[SALT_LEN..]).await?;

    // Blinded key is also a secure commitment to the underlying key.
    // Here we check that the remote party has sent us the correct identity key
    // matching the blinded key they used for X3DH.
    let received_remote_id_blinded = received_remote_identity
        .blind(&remote_salt)
        .ok_or(Error::ProtocolError)?;
    if received_remote_id_blinded != remote_blinded_identity {
        return Err(Error::ProtocolError);
    }

    Ok((received_remote_identity, outgoing, incoming))
}

// TODO: implement AsyncWrite for this, buffering the data and encrypting on flush or on each N-byte chunk.
impl<W: AsyncWrite + Unpin> Outgoing<W> {
    pub async fn send_message(&mut self, msg: &[u8]) -> Result<(), Error> {
        self.kdf.append_u64(b"seq", self.seq);
        let mut key = [0u8; 32];
        self.kdf.challenge_bytes(b"key", &mut key);

        let ad = encode_u64le(self.seq);

        let ciphertext = Aes128PmacSiv::new(GenericArray::clone_from_slice(&key))
            .encrypt(&[&ad], msg)
            .map_err(|_| Error::ProtocolError)?;

        self.seq += 1;

        // Write the length prefix and the ciphertext.
        self.writer
            .write(&encode_u16le(ciphertext.len() as u16)[..])
            .await?;
        self.writer.write(&ciphertext[..]).await?;
        self.writer.flush().await?;
        Ok(())
    }
}

impl<W: AsyncWrite + Unpin> Outgoing<W> {
    fn cipher_buf(&mut self) {
        self.kdf.append_u64(b"seq", self.seq);
        let mut key = [0u8; 32];
        self.kdf.challenge_bytes(b"key", &mut key);

        let ad = encode_u64le(self.seq);

        self.ciphertext_buf.clear();

        let ciphertext = Aes128PmacSiv::new(GenericArray::clone_from_slice(&key))
            .encrypt(&[&ad], &self.plaintext_buf)
            .map_err(|_| unimplemented!())
            .unwrap();
        Write::write(
            &mut self.ciphertext_buf,
            &encode_u16le(ciphertext.len() as u16)[..],
        )
        .unwrap(); // TODO: remove unwrap?
        Write::write(&mut self.ciphertext_buf, &ciphertext).unwrap(); // TODO: remove unwrap?

        self.plaintext_buf.clear();
        self.seq += 1;
    }

    pub fn flush_write(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        match self
            .writer
            .as_mut()
            .poll_write(cx, &self.ciphertext_buf[self.ciphertext_sent..])
        {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Ok(n)) => {
                self.ciphertext_sent += n;
                if self.ciphertext_sent == self.ciphertext_buf.len() {
                    self.ciphertext_sent = 0;
                    self.ciphertext_buf.clear();
                    if self.plaintext_needs_flushing {
                        self.cipher_buf();
                        self.plaintext_needs_flushing = false;
                    }
                    Poll::Ready(Ok(()))
                } else {
                    Poll::Pending
                }
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
        }
    }
}

impl<W: AsyncWrite + Unpin> AsyncWrite for Outgoing<W> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        let me = self.get_mut();

        if me.plaintext_needs_flushing {
            match me.flush_write(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Ready(Ok(())) => {}
            }
        }

        if me.plaintext_buf.len() + buf.len() > BUF_SIZE as usize {
            let size_to_write = me.plaintext_buf.len() + buf.len() - 4096;
            if let Err(err) = Write::write(&mut me.plaintext_buf, &buf[..size_to_write]) {
                return Poll::Ready(Err(err));
            }
            me.cipher_buf();
            Poll::Ready(Ok(size_to_write))
        } else {
            Poll::Ready(Write::write(&mut me.plaintext_buf, buf))
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        let me = self.get_mut();
        if me.ciphertext_buf.len() == 0 {
            if me.plaintext_buf.len() == 0 {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "attempt to write empty message",
                )));
            } else {
                me.cipher_buf();
            }
        }
        me.flush_write(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        unimplemented!()
    }
}

impl<W: AsyncRead + Unpin> Incoming<W> {
    pub async fn receive_message(&mut self) -> Result<Vec<u8>, Error> {
        let mut lenbuf = [0u8; 2];
        let seq = self.seq;
        self.seq += 1;
        self.reader.read_exact(&mut lenbuf[..]).await?;
        let len = LittleEndian::read_u16(&lenbuf) as usize;

        // length must include IV prefix (16 bytes)
        if len < 16 {
            return Err(Error::ProtocolError);
        }
        // Check the message length and fail before changing any of the remaining state.
        let mut ciphertext = Vec::with_capacity(len);
        ciphertext.resize(len, 0u8);
        self.reader.read_exact(&mut ciphertext[..]).await?;

        self.kdf.append_u64(b"seq", seq);
        let mut key = [0u8; 32];
        self.kdf.challenge_bytes(b"key", &mut key);

        let ad = encode_u64le(seq);

        let plaintext = Aes128PmacSiv::new(GenericArray::clone_from_slice(&key))
            .decrypt(&[&ad], &ciphertext)
            .map_err(|_| Error::ProtocolError)?;

        Ok(plaintext)
    }

    /// Converts to the Stream
    pub fn into_stream(self) -> impl futures::stream::Stream<Item = Result<Vec<u8>, Error>> {
        futures::stream::unfold(self, |mut src| async move {
            let res = src.receive_message().await;
            Some((res, src))
        })
    }
}

impl<W: AsyncRead + Unpin> Incoming<W> {
    fn decipher_buf(&mut self) {
        let seq = self.seq;
        self.seq += 1;

        self.kdf.append_u64(b"seq", seq);
        let mut key = [0u8; 32];
        self.kdf.challenge_bytes(b"key", &mut key);

        let ad = encode_u64le(seq);

        let plaintext = match Aes128PmacSiv::new(GenericArray::clone_from_slice(&key))
            .decrypt(&[&ad], &self.ciphertext_buf[..self.need_to_get as usize])
        {
            Ok(text) => text,
            Err(_) => unimplemented!(),
        };
        self.plaintext_buf.extend_from_slice(&plaintext);
    }
}

impl<W: AsyncRead + Unpin> AsyncRead for Incoming<W> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<Result<usize, io::Error>> {
        let me = self.get_mut();

        if me.plaintext_buf.len() != 0 {
            return match Read::read(&mut &me.plaintext_buf[me.plaintext_read..], buf) {
                Ok(n) => {
                    me.plaintext_read += n;
                    if me.plaintext_read == me.plaintext_buf.len() {
                        me.plaintext_buf.clear();
                    }
                    Poll::Ready(Ok(n))
                }
                Err(e) => Poll::Ready(Err(e)),
            };
        }

        if me.need_to_get == 0 {
            loop {
                match me
                    .reader
                    .as_mut()
                    .poll_read(cx, &mut me.ciphertext_buf[me.now_read as usize..2])
                {
                    Poll::Ready(Ok(n)) => {
                        me.now_read += n as u16;
                        match me.now_read {
                            0 => {
                                return Poll::Ready(Err(io::Error::new(
                                    io::ErrorKind::WriteZero,
                                    "unexpected end of stream",
                                )))
                            }
                            1 => {}
                            2 => {
                                me.now_read = 0;
                                me.need_to_get = LittleEndian::read_u16(&me.ciphertext_buf[..2]);
                                if me.need_to_get < 16 {
                                    me.need_to_get = 0;
                                    return Poll::Ready(Err(io::Error::new(
                                        io::ErrorKind::InvalidData,
                                        "protocol error",
                                    )));
                                }
                                break;
                            }
                            _ => unreachable!(),
                        }
                    }
                    Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                    Poll::Pending => return Poll::Pending,
                }
            }
        }

        match me.reader.as_mut().poll_read(
            cx,
            &mut me.ciphertext_buf[me.now_read as usize..me.need_to_get as usize],
        ) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Ready(Ok(n)) => {
                me.now_read += n as u16;
                if me.now_read == me.need_to_get {
                    me.decipher_buf();
                    me.now_read = 0;
                    me.need_to_get = 0;
                    return match Read::read(&mut &me.plaintext_buf[me.plaintext_read..], buf) {
                        Ok(n) => {
                            me.plaintext_read += n;
                            if me.plaintext_read == me.plaintext_buf.len() {
                                me.plaintext_buf.clear();
                            }
                            Poll::Ready(Ok(n))
                        }
                        Err(e) => Poll::Ready(Err(e)),
                    };
                } else {
                    Poll::Pending
                }
            }
        }
    }
}

/// This is a YOLO variant of Signal's X3DH that's aimed at improved performance:
/// instead of doing independent computation of three DH instances,
/// compressing them, and feeding independently into a hash,
/// we add them all together, separated by a Fiat-Shamir challenges (x, y):
///
/// X3DH = Hash(DH(eph1, eph2) + x * DH(id1, eph2) + y * DH(id2, eph1))
///
/// This allows reusing doublings across all three instances,
/// and do a single point compression in the end instead of three.
///
/// To get consistent results on both ends, we reorder keys so the "first" party
/// is the one with the lower compressed identity public key.
fn cybershake_x3dh(
    id1: &PrivateKey,
    eph1: &PrivateKey,
    id2: &PublicKey,
    eph2: &PublicKey,
) -> Result<Transcript, Error> {
    let mut t = Transcript::new(b"Cybershake.X3DH");
    let keep_order = id1.pubkey.as_bytes() < id2.as_bytes();
    {
        let (id1, eph1, id2, eph2) = if keep_order {
            (&id1.pubkey, &eph1.pubkey, id2, eph2)
        } else {
            (id2, eph2, &id1.pubkey, &eph1.pubkey)
        };
        t.append_message(b"id1", id1.as_bytes());
        t.append_message(b"id2", id2.as_bytes());
        t.append_message(b"eph1", eph1.as_bytes());
        t.append_message(b"eph2", eph2.as_bytes());
    }

    let x = challenge_scalar(b"x", &mut t);
    let y = challenge_scalar(b"y", &mut t);

    let (x, y) = if keep_order { (x, y) } else { (y, x) };

    use core::iter;
    let shared_secret = RistrettoPoint::optional_multiscalar_mul(
        iter::once(&(eph1.as_scalar() + (x * id1.as_scalar())))
            .chain(iter::once(&(eph1.as_scalar() * y))),
        iter::once(eph2.as_point().decompress()).chain(iter::once(id2.as_point().decompress())),
    )
    .ok_or(Error::ProtocolError)?;

    t.append_message(b"x3dh", shared_secret.compress().as_bytes());

    Ok(t)
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Error::IoError(error)
    }
}

impl From<Scalar> for PrivateKey {
    fn from(secret: Scalar) -> Self {
        PrivateKey {
            secret,
            pubkey: PublicKey::from(secret * RISTRETTO_BASEPOINT_POINT),
        }
    }
}

impl From<CompressedRistretto> for PublicKey {
    fn from(point: CompressedRistretto) -> Self {
        PublicKey { point }
    }
}

impl From<RistrettoPoint> for PublicKey {
    fn from(point: RistrettoPoint) -> Self {
        PublicKey::from(point.compress())
    }
}

impl PrivateKey {
    /// Converts the private key to an underlying Ristretto scalar.
    pub fn as_scalar(&self) -> &Scalar {
        &self.secret
    }

    /// Converts the private key to its binary encoding.
    pub fn as_secret_bytes(&self) -> &[u8] {
        &self.secret.as_bytes()[..]
    }

    /// Converts the private key to its public counterpart.
    pub fn to_public_key(&self) -> PublicKey {
        self.pubkey
    }

    /// Blinds the private key.
    fn blind(&self, salt: &[u8; 16]) -> Self {
        PrivateKey::from(self.secret + keyblinding_factor(&self.pubkey.point, salt))
    }
}

impl PublicKey {
    /// Converts the public key to an underlying compressed Ristretto point.
    pub fn as_point(&self) -> &CompressedRistretto {
        &self.point
    }

    /// Converts the public key to its binary encoding.
    pub fn as_bytes(&self) -> &[u8] {
        &self.point.as_bytes()[..]
    }

    /// Blinds the public key.
    fn blind(&self, salt: &[u8; 16]) -> Option<Self> {
        self.point.decompress().map(|p| {
            PublicKey::from(p + keyblinding_factor(&self.point, salt) * RISTRETTO_BASEPOINT_POINT)
        })
    }

    /// Reads pubkey from a reader.
    async fn read_from<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Self, Error> {
        let mut buf = [0u8; 32];
        reader.read_exact(&mut buf[..]).await?;
        Ok(Self::from(CompressedRistretto(buf)))
    }
}

fn keyblinding_factor(pubkey: &CompressedRistretto, salt: &[u8; 16]) -> Scalar {
    let mut t = Transcript::new(b"Cybershake.keyblinding");
    t.append_message(b"key", pubkey.as_bytes());
    t.append_message(b"salt", &salt[..]);
    challenge_scalar(b"factor", &mut t)
}

fn challenge_scalar(label: &'static [u8], transcript: &mut Transcript) -> Scalar {
    let mut buf = [0u8; 64];
    transcript.challenge_bytes(label, &mut buf);
    Scalar::from_bytes_mod_order_wide(&buf)
}

fn encode_u64le(i: u64) -> [u8; 8] {
    let mut buf = [0u8; 8];
    LittleEndian::write_u64(&mut buf, i);
    buf
}

fn encode_u16le(i: u16) -> [u8; 2] {
    let mut buf = [0u8; 2];
    LittleEndian::write_u16(&mut buf, i);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;
    use tokio::net::{TcpListener, TcpStream};

    #[tokio::test]
    async fn test() {
        let alice_private_key = PrivateKey::from(Scalar::from(1u8));
        let bob_private_key = PrivateKey::from(Scalar::from(2u8));

        let mut alice_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut bob_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let alice_addr = alice_listener.local_addr().unwrap();
        let bob_addr = bob_listener.local_addr().unwrap();

        let alice = tokio::spawn(async move {
            let (alice_reader, _) = alice_listener.accept().await.unwrap();
            let alice_writer = TcpStream::connect(bob_addr).await.unwrap();
            let mut rng = StdRng::from_entropy();
            let (received_key, mut alice_out, mut alice_inc) =
                cybershake(&alice_private_key, alice_reader, alice_writer, &mut rng)
                    .await
                    .unwrap();

            assert_eq!(received_key, bob_private_key.to_public_key());

            // Alice send message to bob
            let alice_message: Vec<u8> = "Hello, Bob".bytes().collect();
            alice_out.send_message(&alice_message).await.unwrap();

            // Then Alice receive message from bob
            let alice_rec = alice_inc.receive_message().await.unwrap();
            assert_eq!("Hello, Alice", String::from_utf8(alice_rec).unwrap());
        });

        let bob = tokio::spawn(async move {
            let bob_writer = TcpStream::connect(alice_addr).await.unwrap();
            let (bob_reader, _) = bob_listener.accept().await.unwrap();
            let mut rng = StdRng::from_entropy();
            let (received_key, mut bob_out, mut bob_inc) =
                cybershake(&bob_private_key, bob_reader, bob_writer, &mut rng)
                    .await
                    .unwrap();

            assert_eq!(received_key, alice_private_key.to_public_key());

            // Bob receive message from Alice
            let bob_rec = bob_inc.receive_message().await.unwrap();
            assert_eq!("Hello, Bob", String::from_utf8(bob_rec).unwrap());

            // Then bob send message to Alice
            let bob_message: Vec<u8> = "Hello, Alice".bytes().collect();
            bob_out.send_message(&bob_message).await.unwrap();
        });

        assert!(alice.await.is_ok());
        assert!(bob.await.is_ok());
    }

    #[tokio::test]
    async fn test2() {
        let alice_private_key = PrivateKey::from(Scalar::from(1u8));
        let bob_private_key = PrivateKey::from(Scalar::from(2u8));

        let mut alice_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut bob_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let alice_addr = alice_listener.local_addr().unwrap();
        let bob_addr = bob_listener.local_addr().unwrap();

        let alice = tokio::spawn(async move {
            let (alice_reader, _) = alice_listener.accept().await.unwrap();
            let alice_writer = TcpStream::connect(bob_addr).await.unwrap();
            let mut rng = StdRng::from_entropy();
            let (received_key, mut alice_out, mut alice_inc) =
                cybershake(&alice_private_key, alice_reader, alice_writer, &mut rng)
                    .await
                    .unwrap();

            assert_eq!(received_key, bob_private_key.to_public_key());

            // Alice send message to bob
            let alice_message: Vec<u8> = "Hello, Bob".bytes().collect();
            alice_out.write(&alice_message).await.unwrap();
            alice_out.flush().await.unwrap();

            // Then Alice receive message from bob
            let mut buf = vec![0u8; 4096];
            let message_len = alice_inc.read(&mut buf).await.unwrap();
            buf.truncate(message_len);
            assert_eq!("Hello, Alice", String::from_utf8(buf).unwrap());
        });

        let bob = tokio::spawn(async move {
            let bob_writer = TcpStream::connect(alice_addr).await.unwrap();
            let (bob_reader, _) = bob_listener.accept().await.unwrap();
            let mut rng = StdRng::from_entropy();
            let (received_key, mut bob_out, mut bob_inc) =
                cybershake(&bob_private_key, bob_reader, bob_writer, &mut rng)
                    .await
                    .unwrap();

            assert_eq!(received_key, alice_private_key.to_public_key());

            // Bob receive message from Alice
            let mut buf = vec![0u8; 4096];
            let message_len = bob_inc.read(&mut buf).await.unwrap();
            buf.truncate(message_len);
            assert_eq!("Hello, Bob", String::from_utf8(buf).unwrap());

            // Then bob send message to Alice
            let bob_message: Vec<u8> = "Hello, Alice".bytes().collect();
            bob_out.write(&bob_message).await.unwrap();
            bob_out.flush().await.unwrap();
        });

        assert!(alice.await.is_ok());
        assert!(bob.await.is_ok());
    }
}
