//! Various datatypes common to all I2P protocols.
//!
//! [Common structures specification](https://geti2p.net/spec/common-structures)

use cookie_factory::GenError;
use nom::{Err, IResult};
use rand::{OsRng, Rng};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Write};
use std::iter::repeat;
use std::net::SocketAddr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use constants;
use crypto::{
    self, EncType, PrivateKey, PublicKey, SigType, Signature, SigningPrivateKey, SigningPublicKey,
};

pub(crate) mod frame;

//
// Simple data types
//

/// The SHA-256 hash of some data.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Hash(pub [u8; 32]);

impl Hash {
    pub fn from_bytes(buf: &[u8; 32]) -> Self {
        let mut x = [0u8; 32];
        x.copy_from_slice(buf);
        Hash(x)
    }

    pub fn digest(buf: &[u8]) -> Self {
        let mut hasher = Sha256::default();
        hasher.input(buf);
        let hash = hasher.result();
        Hash::from_bytes(array_ref![hash.as_slice(), 0, 32])
    }

    pub fn xor(&mut self, other: &Hash) {
        for i in 0..32 {
            self.0[i] ^= other.0[i];
        }
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", constants::I2P_BASE64.encode(&self.0))
    }
}

/// The number of milliseconds since midnight on January 1, 1970 in the GMT
/// timezone. If the number is 0, the date is undefined or null.
#[derive(Clone, Debug, PartialEq)]
pub struct I2PDate(pub(crate) u64);

impl I2PDate {
    pub fn from_system_time(t: SystemTime) -> Self {
        let d = t.duration_since(UNIX_EPOCH).unwrap_or(Duration::new(0, 0));
        I2PDate(d.as_secs() * 1_000 + (d.subsec_nanos() / 1_000_000) as u64)
    }
}

/// A UTF-8-encoded string.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct I2PString(pub String);

impl I2PString {
    pub fn new(string: &str) -> Self {
        I2PString(String::from(string))
    }

    pub fn to_csv(&self) -> Vec<Self> {
        self.0.split(',').map(|s| Self::new(s)).collect()
    }
}

/// A set of key/value mappings or properties.
#[derive(Clone, Debug, PartialEq)]
pub struct Mapping(pub HashMap<I2PString, I2PString>);

/// A random number.
pub struct SessionTag(pub [u8; 32]);

impl SessionTag {
    fn from_bytes(buf: &[u8; 32]) -> Self {
        let mut x = [0u8; 32];
        x.copy_from_slice(buf);
        SessionTag(x)
    }
}

/// Defines an identifier that is unique to each router in a tunnel. A TunnelId
/// is generally greater than zero; do not use a value of zero except in
/// special cases.
#[derive(Debug)]
pub struct TunnelId(pub u32);

/// A key certificate provides a mechanism to indicate the type of the PublicKey
/// and SigningPublicKey in the Destination or RouterIdentity, and to package
/// any key data in excess of the standard lengths.
///
/// By maintaining exactly 384 bytes before the certificate, and putting any
/// excess key data inside the certificate, we maintain compatibility for any
/// software that parses Destinations and RouterIdentities.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyCertificate {
    pub sig_type: SigType,
    enc_type: EncType,
    sig_data: Vec<u8>,
    enc_data: Vec<u8>,
}

/// A container for various receipts or proof of works used throughout the I2P
/// network.
#[derive(Clone, Debug, PartialEq)]
pub enum Certificate {
    Null,
    HashCash(Vec<u8>),
    Hidden,
    Signed(Vec<u8>),
    Multiple(Vec<u8>),
    Key(KeyCertificate),
}

impl Certificate {
    pub fn from(buf: &[u8]) -> Option<Self> {
        match frame::certificate(buf) {
            Ok((_, s)) => Some(s),
            Err(Err::Incomplete(_)) => None,
            Err(Err::Error(_)) | Err(Err::Failure(_)) => panic!("Unsupported Certificate"),
        }
    }

    pub fn code(&self) -> u8 {
        match *self {
            Certificate::Null => constants::NULL_CERT,
            Certificate::HashCash(_) => constants::HASH_CERT,
            Certificate::Hidden => constants::HIDDEN_CERT,
            Certificate::Signed(_) => constants::SIGNED_CERT,
            Certificate::Multiple(_) => constants::MULTI_CERT,
            Certificate::Key(_) => constants::KEY_CERT,
        }
    }
}

/// Defines the way to uniquely identify a particular router.
#[derive(Clone, Debug, PartialEq)]
pub struct RouterIdentity {
    public_key: PublicKey,
    padding: Option<Vec<u8>>,
    pub signing_key: SigningPublicKey,
    pub certificate: Certificate,
}

impl RouterIdentity {
    pub fn from_file(path: &str) -> io::Result<Self> {
        let mut rid = File::open(path)?;
        let mut data: Vec<u8> = Vec::new();
        rid.read_to_end(&mut data)?;
        match frame::router_identity(&data[..]) {
            Ok((_, res)) => Ok(res),
            Err(Err::Incomplete(n)) => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("needed: {:?}", n),
            )),
            Err(Err::Error(e)) | Err(Err::Failure(e)) => Err(io::Error::new(
                io::ErrorKind::Other,
                e.into_error_kind().description(),
            )),
        }
    }

    fn from_secrets(private_key: &PrivateKey, signing_private_key: &SigningPrivateKey) -> Self {
        let public_key = PublicKey::from_secret(private_key);
        let signing_key = SigningPublicKey::from_secret(signing_private_key).unwrap();
        let certificate = match signing_key.sig_type() {
            SigType::DsaSha1 => Certificate::Null,
            SigType::Ed25519 => Certificate::Key(KeyCertificate {
                sig_type: SigType::Ed25519,
                enc_type: EncType::ElGamal2048,
                sig_data: vec![],
                enc_data: vec![],
            }),
            _ => panic!("Not implemented!"),
        };
        let padding = match signing_key.sig_type().pad_len(&EncType::ElGamal2048) {
            0 => None,
            sz => {
                let mut rng = OsRng::new().expect("should be able to construct RNG");
                let mut padding = Vec::new();
                padding.resize(sz, 0);
                rng.fill(&mut padding[..]);
                Some(padding)
            }
        };
        RouterIdentity {
            public_key,
            padding,
            signing_key,
            certificate,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let base_len = 387;
        let mut buf = Vec::with_capacity(base_len);
        buf.extend(repeat(0).take(base_len));
        loop {
            match frame::gen_router_identity((&mut buf[..], 0), self).map(|tup| tup.1) {
                Ok(sz) => {
                    buf.truncate(sz);
                    return buf;
                }
                Err(e) => match e {
                    GenError::BufferTooSmall(sz) => {
                        buf.extend(repeat(0).take(sz - base_len));
                    }
                    _ => panic!("Couldn't serialize RouterIdentity"),
                },
            }
        }
    }

    pub fn to_file(&self, path: &str) -> io::Result<()> {
        let mut rid = File::create(path)?;
        rid.write(&self.to_bytes()).map(|_| ())
    }

    pub fn hash(&self) -> Hash {
        Hash::digest(&self.to_bytes()[..])
    }
}

/// Key material for a RouterIdentity.
#[derive(Clone)]
pub struct RouterSecretKeys {
    pub rid: RouterIdentity,
    private_key: PrivateKey,
    pub signing_private_key: SigningPrivateKey,
}

impl RouterSecretKeys {
    pub fn new() -> Self {
        let private_key = PrivateKey::new();
        let signing_private_key = SigningPrivateKey::new();
        RouterSecretKeys {
            rid: RouterIdentity::from_secrets(&private_key, &signing_private_key),
            private_key,
            signing_private_key,
        }
    }

    pub fn from_file(path: &str) -> io::Result<Self> {
        let mut rsk = File::open(path)?;
        let mut data: Vec<u8> = Vec::new();
        rsk.read_to_end(&mut data)?;
        match frame::router_secret_keys(&data[..]) {
            Ok((_, res)) => Ok(res),
            Err(Err::Incomplete(n)) => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("needed: {:?}", n),
            )),
            Err(Err::Error(e)) | Err(Err::Failure(e)) => Err(io::Error::new(
                io::ErrorKind::Other,
                e.into_error_kind().description(),
            )),
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let base_len = 387;
        let mut buf = Vec::with_capacity(base_len);
        buf.extend(repeat(0).take(base_len));
        loop {
            match frame::gen_router_secret_keys((&mut buf[..], 0), self).map(|tup| tup.1) {
                Ok(sz) => {
                    buf.truncate(sz);
                    return buf;
                }
                Err(e) => match e {
                    GenError::BufferTooSmall(sz) => {
                        buf.extend(repeat(0).take(sz - base_len));
                    }
                    _ => panic!("Couldn't serialize RouterSecretKeys"),
                },
            }
        }
    }

    pub fn to_file(&self, path: &str) -> io::Result<()> {
        let mut rid = File::create(path)?;
        rid.write(&self.to_bytes()).map(|_| ())
    }
}

/// A Destination defines a particular endpoint to which messages can be
/// directed for secure delivery.
pub struct Destination {
    public_key: PublicKey,
    padding: Option<Vec<u8>>,
    signing_key: SigningPublicKey,
    certificate: Certificate,
}

/// Defines the authorization for a particular tunnel to receive messages
/// targeting a Destination.
pub struct Lease {
    tunnel_gw: Hash,
    tid: TunnelId,
    end_date: I2PDate,
}

/// Contains all of the currently authorized Leases for a particular Destination,
/// the PublicKey to which garlic messages can be encrypted, and then the
/// SigningPublicKey that can be used to revoke this particular version of the
/// structure.
///
/// The LeaseSet is one of the two structures stored in the network database
/// (the other being RouterInfo), and is keyed under the SHA-256 of the contained
/// Destination.
pub struct LeaseSet {
    dest: Destination,
    enc_key: PublicKey,
    sig_key: SigningPublicKey,
    leases: Vec<Lease>,
    sig: Signature,
}

/// Defines the means to contact a router through a transport protocol.
#[derive(Clone, Debug, PartialEq)]
pub struct RouterAddress {
    cost: u8,
    expiration: I2PDate,
    transport_style: I2PString,
    options: Mapping,
}

impl RouterAddress {
    pub fn new(transport_style: &I2PString, addr: SocketAddr) -> Self {
        let mut options = HashMap::new();
        options.insert(
            I2PString(String::from("host")),
            I2PString(addr.ip().to_string()),
        );
        options.insert(
            I2PString(String::from("port")),
            I2PString(addr.port().to_string()),
        );
        RouterAddress {
            cost: 0,
            expiration: I2PDate(0),
            transport_style: transport_style.clone(),
            options: Mapping(options),
        }
    }

    pub fn option(&self, key: &I2PString) -> Option<&I2PString> {
        self.options.0.get(key)
    }

    pub fn set_option(&mut self, key: I2PString, value: I2PString) {
        self.options.0.insert(key, value);
    }

    pub fn addr(&self) -> Option<SocketAddr> {
        let host = self.options.0.get(&I2PString(String::from("host")));
        let port = self.options.0.get(&I2PString(String::from("port")));
        match (host, port) {
            (Some(host), Some(port)) => match (host.0.parse(), port.0.parse()) {
                (Ok(ip), Ok(port)) => Some(SocketAddr::new(ip, port)),
                _ => None,
            },
            _ => None,
        }
    }
}

/// Defines all of the data that a router wants to publish for the network to
/// see.
///
/// The RouterInfo is one of two structures stored in the network database (the
/// other being LeaseSet), and is keyed under the SHA-256 of the contained
/// RouterIdentity.
#[derive(Clone, Debug, PartialEq)]
pub struct RouterInfo {
    pub router_id: RouterIdentity,
    published: I2PDate,
    addresses: Vec<RouterAddress>,
    peers: Vec<Hash>,
    options: Mapping,
    signature: Option<Signature>,
}

impl RouterInfo {
    pub fn new(rid: RouterIdentity) -> Self {
        RouterInfo {
            router_id: rid,
            published: I2PDate::from_system_time(SystemTime::now()),
            addresses: Vec::new(),
            peers: Vec::new(),
            options: Mapping(HashMap::new()),
            signature: None,
        }
    }

    /// Set the addresses in this RouterInfo.
    ///
    /// Caller must re-sign the RouterInfo afterwards.
    pub fn set_addresses(&mut self, addrs: Vec<RouterAddress>) {
        self.addresses = addrs;
        self.signature = None;
    }

    pub fn address<F>(&self, style: &I2PString, filter: F) -> Option<RouterAddress>
    where
        F: Fn(&RouterAddress) -> bool,
    {
        let addrs: Vec<&RouterAddress> = self
            .addresses
            .iter()
            .filter(|a| a.transport_style == *style)
            .filter(|a| a.addr().unwrap().is_ipv4())
            .filter(|a| filter(a))
            .collect();
        if addrs.len() > 0 {
            Some(addrs[0].clone())
        } else {
            None
        }
    }

    pub fn from_file(path: &str) -> io::Result<Self> {
        let mut ri = File::open(path)?;
        let mut data: Vec<u8> = Vec::new();
        ri.read_to_end(&mut data)?;
        match frame::router_info(&data[..]) {
            Ok((_, res)) => Ok(res),
            Err(Err::Incomplete(n)) => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("needed: {:?}", n),
            )),
            Err(Err::Error(e)) | Err(Err::Failure(e)) => Err(io::Error::new(
                io::ErrorKind::Other,
                e.into_error_kind().description(),
            )),
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let base_len = 435; // 387 + 4 + 1 + 1 + 2 + 40
        let mut buf = Vec::with_capacity(base_len);
        buf.extend(repeat(0).take(base_len));
        loop {
            match frame::gen_router_info((&mut buf[..], 0), self).map(|tup| tup.1) {
                Ok(sz) => {
                    buf.truncate(sz);
                    return buf;
                }
                Err(e) => match e {
                    GenError::BufferTooSmall(sz) => {
                        buf.extend(repeat(0).take(sz - base_len));
                    }
                    e => panic!("Couldn't serialize RouterInfo: {:?}", e),
                },
            }
        }
    }

    pub fn to_file(&self, path: &str) -> io::Result<()> {
        let mut ri = File::create(path)?;
        ri.write(&self.to_bytes()).map(|_| ())
    }

    fn signature_bytes(&self) -> Vec<u8> {
        let base_len = 395; // 387 + 4 + 1 + 1 + 2
        let mut buf = Vec::with_capacity(base_len);
        buf.extend(repeat(0).take(base_len));
        loop {
            match frame::gen_router_info_minus_sig((&mut buf[..], 0), self).map(|tup| tup.1) {
                Ok(sz) => {
                    buf.truncate(sz);
                    break;
                }
                Err(e) => match e {
                    GenError::BufferTooSmall(sz) => {
                        buf.extend(repeat(0).take(sz - base_len));
                    }
                    _ => panic!("Couldn't serialize RouterInfo signature message"),
                },
            }
        }
        buf
    }

    pub fn sign(&mut self, spk: &SigningPrivateKey) {
        let sig_msg = self.signature_bytes();
        self.signature = Some(spk.sign(&sig_msg).unwrap());
    }

    pub fn verify(&self) -> Result<(), crypto::Error> {
        match &self.signature.as_ref() {
            &Some(s) => {
                let sig_msg = self.signature_bytes();
                self.router_id.signing_key.verify(&sig_msg, s)
            }
            &None => Err(crypto::Error::NoSignature),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tests::{RI_SIGTYPE_1, RI_SIGTYPE_2, ROUTER_INFO};

    #[test]
    fn hash_xor() {
        let mut h = Hash::from_bytes(&[0u8; 32]);
        let h0 = Hash::from_bytes(&[0u8; 32]);
        let h1 = Hash::from_bytes(&[1u8; 32]);
        let h2 = Hash::from_bytes(&[2u8; 32]);
        let h3 = Hash::from_bytes(&[3u8; 32]);
        assert_eq!(h, h0);
        h.xor(&h1);
        assert_eq!(h, h1);
        h.xor(&h2);
        assert_eq!(h, h3);
        h.xor(&h1);
        assert_eq!(h, h2);
        h.xor(&h2);
        assert_eq!(h, h0);
    }

    #[test]
    fn i2pstring_to_csv() {
        let s1 = I2PString(String::from("a-b,c/d,1,2"));
        assert_eq!(
            s1.to_csv(),
            vec![
                I2PString(String::from("a-b")),
                I2PString(String::from("c/d")),
                I2PString(String::from("1")),
                I2PString(String::from("2")),
            ]
        );

        let s2 = I2PString(String::from("asdf"));
        assert_eq!(s2.to_csv(), vec![s2]);
    }

    #[test]
    fn router_identity_hash() {
        let ri_hash = Hash([
            0x26, 0x7a, 0x87, 0x78, 0x0d, 0x0c, 0xa0, 0x9a, 0x21, 0xa0, 0x29, 0xb7, 0x4d, 0x7b,
            0xc3, 0x4d, 0x07, 0xc3, 0x53, 0x02, 0x72, 0xc6, 0x30, 0xaa, 0x4c, 0xc1, 0x1d, 0x61,
            0x90, 0xc7, 0xb6, 0xb4,
        ]);
        match frame::router_info(ROUTER_INFO) {
            Ok((_, ri)) => {
                assert_eq!(ri.router_id.hash(), ri_hash);
            }
            _ => panic!("RouterIdentity parsing failed"),
        }
    }

    #[test]
    fn router_address_options() {
        let style = I2PString::new("test");
        let mut ra = RouterAddress::new(&style, "127.0.0.1:0".parse().unwrap());

        let key = I2PString::new("key");
        let value = I2PString::new("value");
        assert!(ra.option(&key).is_none());

        ra.set_option(key.clone(), value.clone());
        assert_eq!(ra.option(&key).unwrap(), &value);
    }

    #[test]
    fn router_info_address() {
        let rsk = RouterSecretKeys::new();
        let mut ri = RouterInfo::new(rsk.rid);
        let style = I2PString::new("test");
        assert!(ri.address(&style, |_| true).is_none());

        ri.set_addresses(vec![
            RouterAddress::new(&I2PString::new("other"), "127.0.0.1:12345".parse().unwrap()),
            RouterAddress::new(&style, "127.0.0.1:23456".parse().unwrap()),
            RouterAddress::new(&style, "127.0.0.1:34567".parse().unwrap()),
        ]);

        let ra = ri.address(&style, |_| true).unwrap();
        assert_eq!(ra.transport_style, style);
        assert_eq!(ra.addr().unwrap(), "127.0.0.1:23456".parse().unwrap());

        let ra = ri
            .address(&style, |ra| ra.addr().unwrap().port() == 34567)
            .unwrap();
        assert_eq!(ra.transport_style, style);
        assert_eq!(ra.addr().unwrap(), "127.0.0.1:34567".parse().unwrap());
    }

    #[test]
    fn router_info_sign() {
        let rsk = RouterSecretKeys::new();
        let mut ri = RouterInfo::new(rsk.rid);
        assert!(ri.signature.is_none());
        ri.sign(&rsk.signing_private_key);
        assert!(ri.signature.is_some());
        assert!(ri.verify().is_ok());
    }

    fn router_info_verify(data: &[u8]) {
        match frame::router_info(data) {
            Ok((_, ri)) => {
                assert!(ri.verify().is_ok());
            }
            Err(e) => panic!("RouterInfo parsing failed: {}", e),
        }
    }

    #[test]
    fn router_info_verify_sigtype_1() {
        router_info_verify(RI_SIGTYPE_1)
    }

    #[test]
    fn router_info_verify_sigtype_2() {
        router_info_verify(RI_SIGTYPE_2)
    }

    #[test]
    fn router_info_verify_sigtype_7() {
        router_info_verify(ROUTER_INFO)
    }
}
