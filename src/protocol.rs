//! The persistent terminal protocol of `com.deskconn.terminal.stream`, as deskconn's
//! `internal/common` defines it.
//!
//! A stream starts with a key exchange: each side sends a length-prefixed JSON frame with
//! its X25519 public key, and both derive a key per direction. Every frame after it is an
//! envelope: a kind byte, then the nonce-prefixed ChaCha20-Poly1305 encryption of either a
//! JSON [`ControlMsg`] or raw terminal input or output.

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use zeroize::Zeroizing;

/// The largest frame either side accepts.
const MAX_FRAME: usize = 1 << 20;

/// Envelope kinds.
pub const KIND_CONTROL: u8 = 0;
pub const KIND_DATA: u8 = 1;

/// Every control message of the protocol. Fields that don't apply to an operation stay unset.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ControlMsg {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub op: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub session: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub cols: u16,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rows: u16,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub code: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
    #[serde(default)]
    pub offset: u64,
    #[serde(default)]
    pub snapshot: usize,
    #[serde(default)]
    pub history_gap: bool,
}

fn is_zero(n: &u16) -> bool {
    *n == 0
}

pub const OP_CREATE: &str = "create";
pub const OP_ATTACH: &str = "attach";
pub const OP_SIZE: &str = "size";
pub const OP_PING: &str = "ping";
pub const OP_TERMINATE: &str = "terminate";
pub const OP_EXIT: &str = "exit";
pub const OP_DISPLACED: &str = "displaced";

/// A refusal's code for a session that has ended, or isn't the caller's.
pub const ERR_NOT_FOUND: &str = "not_found";
/// A refusal's code for a session the machine can't reach right now.
pub const ERR_UNAVAILABLE: &str = "unavailable";

/// One message read from the stream.
pub enum Message {
    Control(ControlMsg),
    Data(Vec<u8>),
}

#[derive(Serialize, Deserialize)]
struct KeyExchange {
    /// Base64, as Go encodes a []byte.
    public_key: String,
}

/// The keys of one stream, from the client's side.
#[derive(Clone)]
pub struct Keys {
    send: Zeroizing<[u8; xconn_crypto::KEY_SIZE]>,
    receive: Zeroizing<[u8; xconn_crypto::KEY_SIZE]>,
}

/// Runs the client's side of the key exchange on stream.
pub async fn exchange_keys<S: AsyncRead + AsyncWrite + Unpin>(stream: &mut S) -> Result<Keys> {
    let pair = xconn_crypto::generate_key_pair()?;
    let hello = serde_json::to_vec(&KeyExchange {
        public_key: BASE64.encode(pair.public),
    })?;
    write_frame(stream, &hello).await?;

    let reply: KeyExchange = serde_json::from_slice(&read_frame(stream).await?)?;
    let peer = BASE64.decode(reply.public_key).context("invalid peer public key")?;
    if peer.len() != xconn_crypto::KEY_SIZE {
        bail!("invalid peer public key length: {}", peer.len());
    }
    let secret = xconn_crypto::shared_secret(pair.private.as_ref(), &peer)?;
    Ok(Keys {
        send: xconn_crypto::derive_key(secret.as_ref(), b"frontendToBackend")?,
        receive: xconn_crypto::derive_key(secret.as_ref(), b"backendToFrontend")?,
    })
}

impl Keys {
    /// Encrypts a message into a frame, ready for [`write_frame`].
    pub fn seal(&self, kind: u8, plaintext: &[u8]) -> Result<Vec<u8>> {
        let packet = xconn_crypto::encrypt(plaintext, self.send.as_ref(), &[])?;
        let mut envelope = Vec::with_capacity(1 + packet.len());
        envelope.push(kind);
        envelope.extend_from_slice(&packet);
        Ok(envelope)
    }

    pub fn seal_control(&self, msg: &ControlMsg) -> Result<Vec<u8>> {
        self.seal(KIND_CONTROL, &serde_json::to_vec(msg)?)
    }

    /// Decrypts a frame read with [`read_frame`].
    pub fn open(&self, envelope: &[u8]) -> Result<Message> {
        let Some((&kind, packet)) = envelope.split_first() else {
            bail!("empty message");
        };
        let plaintext = xconn_crypto::decrypt(packet, self.receive.as_ref(), &[])?;
        match kind {
            KIND_DATA => Ok(Message::Data(plaintext)),
            KIND_CONTROL => Ok(Message::Control(
                serde_json::from_slice(&plaintext).context("invalid control message")?,
            )),
            _ => bail!("unknown message kind {kind}"),
        }
    }
}

/// Writes data with its 4-byte big-endian length, in one write.
pub async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, data: &[u8]) -> Result<()> {
    let mut frame = Vec::with_capacity(4 + data.len());
    frame.extend_from_slice(&u32::try_from(data.len())?.to_be_bytes());
    frame.extend_from_slice(data);
    w.write_all(&frame).await?;
    w.flush().await?;
    Ok(())
}

pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<Vec<u8>> {
    let mut length = [0; 4];
    r.read_exact(&mut length).await?;
    let length = u32::from_be_bytes(length) as usize;
    if length > MAX_FRAME {
        bail!("message too large: {length} bytes");
    }
    let mut data = vec![0; length];
    r.read_exact(&mut data).await?;
    Ok(data)
}
