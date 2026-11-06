//! Hyperliquid L1 action signing, ported from the official Python SDK
//! (`hyperliquid/utils/signing.py`) and checked against its test vectors.
//!
//! An action (an order, a cancel...) is msgpack-encoded with its keys in the SDK's
//! order, followed by the nonce and a vault flag, then keccak-hashed. That hash
//! becomes the `connectionId` of a "phantom agent" (`source` "a" on mainnet, "b"
//! on testnet), which is signed as EIP-712 typed data under the "Exchange" domain
//! (chain id 1337). The signer is an API agent wallet: it can trade for the
//! account but cannot withdraw.

use anyhow::{Context, Result, bail};
use k256::ecdsa::SigningKey;
use serde::Serialize;
use sha3::{Digest, Keccak256};

/// A msgpack value with map keys kept in insertion order (the hash depends on it).
#[derive(Debug, Clone, PartialEq)]
pub enum Mp {
    Bool(bool),
    UInt(u64),
    Str(String),
    Array(Vec<Mp>),
    Map(Vec<(String, Mp)>),
}

impl Mp {
    pub fn map(kv: Vec<(&str, Mp)>) -> Mp {
        Mp::Map(kv.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    pub fn str(s: impl Into<String>) -> Mp {
        Mp::Str(s.into())
    }

    /// Encodes like Python's `msgpack.packb` (smallest representation).
    pub fn encode(&self, out: &mut Vec<u8>) {
        match self {
            Mp::Bool(b) => out.push(if *b { 0xc3 } else { 0xc2 }),
            Mp::UInt(n) => match *n {
                0..=0x7f => out.push(*n as u8),
                0x80..=0xff => out.extend([0xcc, *n as u8]),
                0x100..=0xffff => {
                    out.push(0xcd);
                    out.extend((*n as u16).to_be_bytes());
                }
                0x1_0000..=0xffff_ffff => {
                    out.push(0xce);
                    out.extend((*n as u32).to_be_bytes());
                }
                _ => {
                    out.push(0xcf);
                    out.extend(n.to_be_bytes());
                }
            },
            Mp::Str(s) => {
                let b = s.as_bytes();
                match b.len() {
                    0..=31 => out.push(0xa0 | b.len() as u8),
                    32..=0xff => out.extend([0xd9, b.len() as u8]),
                    0x100..=0xffff => {
                        out.push(0xda);
                        out.extend((b.len() as u16).to_be_bytes());
                    }
                    _ => {
                        out.push(0xdb);
                        out.extend((b.len() as u32).to_be_bytes());
                    }
                }
                out.extend(b);
            }
            Mp::Array(v) => {
                match v.len() {
                    0..=15 => out.push(0x90 | v.len() as u8),
                    16..=0xffff => {
                        out.push(0xdc);
                        out.extend((v.len() as u16).to_be_bytes());
                    }
                    _ => {
                        out.push(0xdd);
                        out.extend((v.len() as u32).to_be_bytes());
                    }
                }
                v.iter().for_each(|x| x.encode(out));
            }
            Mp::Map(kv) => {
                match kv.len() {
                    0..=15 => out.push(0x80 | kv.len() as u8),
                    16..=0xffff => {
                        out.push(0xde);
                        out.extend((kv.len() as u16).to_be_bytes());
                    }
                    _ => {
                        out.push(0xdf);
                        out.extend((kv.len() as u32).to_be_bytes());
                    }
                }
                for (k, v) in kv {
                    Mp::Str(k.clone()).encode(out);
                    v.encode(out);
                }
            }
        }
    }

    /// The same value as JSON, for the request body (key order doesn't matter there).
    pub fn to_json(&self) -> serde_json::Value {
        use serde_json::Value;
        match self {
            Mp::Bool(b) => Value::Bool(*b),
            Mp::UInt(n) => Value::from(*n),
            Mp::Str(s) => Value::String(s.clone()),
            Mp::Array(v) => Value::Array(v.iter().map(Mp::to_json).collect()),
            Mp::Map(kv) => Value::Object(kv.iter().map(|(k, v)| (k.clone(), v.to_json())).collect()),
        }
    }
}

pub fn keccak(data: &[u8]) -> [u8; 32] {
    Keccak256::digest(data).into()
}

/// `action_hash` from the SDK: msgpack(action) ‖ nonce (u64 BE) ‖ vault flag [‖ vault].
pub fn action_hash(action: &Mp, vault: Option<[u8; 20]>, nonce: u64) -> [u8; 32] {
    let mut data = Vec::new();
    action.encode(&mut data);
    data.extend(nonce.to_be_bytes());
    match vault {
        None => data.push(0),
        Some(v) => {
            data.push(1);
            data.extend(v);
        }
    }
    keccak(&data)
}

/// EIP-712 digest of `Agent { source, connectionId }` in the "Exchange" domain.
pub fn agent_digest(connection_id: [u8; 32], mainnet: bool) -> [u8; 32] {
    let domain_type = keccak(b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)");
    let mut chain = [0u8; 32];
    chain[30..].copy_from_slice(&1337u16.to_be_bytes());
    let mut domain = Vec::with_capacity(160);
    domain.extend(domain_type);
    domain.extend(keccak(b"Exchange"));
    domain.extend(keccak(b"1"));
    domain.extend(chain);
    domain.extend([0u8; 32]); // verifyingContract = 0x0
    let domain_sep = keccak(&domain);

    let agent_type = keccak(b"Agent(string source,bytes32 connectionId)");
    let mut st = Vec::with_capacity(96);
    st.extend(agent_type);
    st.extend(keccak(if mainnet { b"a" } else { b"b" }));
    st.extend(connection_id);
    let struct_hash = keccak(&st);

    let mut msg = Vec::with_capacity(66);
    msg.extend([0x19, 0x01]);
    msg.extend(domain_sep);
    msg.extend(struct_hash);
    keccak(&msg)
}

/// `{r, s, v}` as the exchange expects it (hex without leading zeros, like the SDK's `to_hex`).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Signature {
    pub r: String,
    pub s: String,
    pub v: u8,
}

fn to_hex_trimmed(b: &[u8]) -> String {
    let h = hex::encode(b);
    let t = h.trim_start_matches('0');
    format!("0x{}", if t.is_empty() { "0" } else { t })
}

pub struct Signer {
    key: SigningKey,
    /// The agent wallet's address (lowercase 0x hex).
    pub address: String,
}

impl Signer {
    pub fn from_hex(private_key: &str) -> Result<Self> {
        let raw = hex::decode(private_key.trim().trim_start_matches("0x")).context("private key is not hex")?;
        if raw.len() != 32 {
            bail!("private key must be 32 bytes, got {}", raw.len());
        }
        let key = SigningKey::from_slice(&raw).context("invalid secp256k1 private key")?;
        let point = key.verifying_key().to_encoded_point(false);
        let addr = keccak(&point.as_bytes()[1..]);
        Ok(Self { key, address: format!("0x{}", hex::encode(&addr[12..])) })
    }

    pub fn sign_digest(&self, digest: [u8; 32]) -> Result<Signature> {
        let (sig, rec) = self.key.sign_prehash_recoverable(&digest).context("signing")?;
        let (r, s) = sig.split_bytes();
        Ok(Signature { r: to_hex_trimmed(&r), s: to_hex_trimmed(&s), v: 27 + rec.to_byte() })
    }

    pub fn sign_l1_action(&self, action: &Mp, vault: Option<[u8; 20]>, nonce: u64, mainnet: bool) -> Result<Signature> {
        self.sign_digest(agent_digest(action_hash(action, vault, nonce), mainnet))
    }
}

/// `float_to_wire` from the SDK: at most 8 decimals, trailing zeros stripped, no exponent.
pub fn float_to_wire(x: f64) -> Result<String> {
    let rounded = format!("{x:.8}");
    let back: f64 = rounded.parse()?;
    if (back - x).abs() >= 1e-12 {
        bail!("{x} has more than 8 decimals");
    }
    let mut s = rounded;
    if s.contains('.') {
        s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    if s == "-0" || s.is_empty() {
        s = "0".into();
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "0x0123456789012345678901234567890123456789012345678901234567890123";

    fn order(cloid: Option<&str>, t: Mp) -> Mp {
        let mut o = vec![("a", Mp::UInt(1)), ("b", Mp::Bool(true)), ("p", Mp::str("100")), ("s", Mp::str("100")), ("r", Mp::Bool(false)), ("t", t)];
        if let Some(c) = cloid {
            o.push(("c", Mp::str(c)));
        }
        Mp::map(vec![("type", Mp::str("order")), ("orders", Mp::Array(vec![Mp::map(o)])), ("grouping", Mp::str("na"))])
    }

    fn gtc() -> Mp {
        Mp::map(vec![("limit", Mp::map(vec![("tif", Mp::str("Gtc"))]))])
    }

    fn check(action: &Mp, vault: Option<[u8; 20]>, mainnet: bool, r: &str, s: &str, v: u8) {
        let sig = Signer::from_hex(KEY).unwrap().sign_l1_action(action, vault, 0, mainnet).unwrap();
        assert_eq!(sig, Signature { r: r.into(), s: s.into(), v }, "mainnet={mainnet}");
    }

    // Vectors from hyperliquid-python-sdk tests/signing_test.py.
    #[test]
    fn phantom_agent_matches_production() {
        let o = Mp::map(vec![
            ("a", Mp::UInt(4)),
            ("b", Mp::Bool(true)),
            ("p", Mp::str(float_to_wire(1670.1).unwrap())),
            ("s", Mp::str(float_to_wire(0.0147).unwrap())),
            ("r", Mp::Bool(false)),
            ("t", Mp::map(vec![("limit", Mp::map(vec![("tif", Mp::str("Ioc"))]))])),
        ]);
        let action = Mp::map(vec![("type", Mp::str("order")), ("orders", Mp::Array(vec![o])), ("grouping", Mp::str("na"))]);
        assert_eq!(hex::encode(action_hash(&action, None, 1677777606040)), "0fcbeda5ae3c4950a548021552a4fea2226858c4453571bf3f24ba017eac2908");
    }

    #[test]
    fn l1_dummy_action() {
        let a = Mp::map(vec![("type", Mp::str("dummy")), ("num", Mp::UInt(100_000_000_000))]);
        check(&a, None, true, "0x53749d5b30552aeb2fca34b530185976545bb22d0b3ce6f62e31be961a59298", "0x755c40ba9bf05223521753995abb2f73ab3229be8ec921f350cb447e384d8ed8", 27);
        check(&a, None, false, "0x542af61ef1f429707e3c76c5293c80d01f74ef853e34b76efffcb57e574f9510", "0x17b8b32f086e8cdede991f1e2c529f5dd5297cbe8128500e00cbaf766204a613", 28);
        let vault: [u8; 20] = hex::decode("1719884eb866cb12b2287399b15f7db5e7d775ea").unwrap().try_into().unwrap();
        check(&a, Some(vault), true, "0x3c548db75e479f8012acf3000ca3a6b05606bc2ec0c29c50c515066a326239", "0x4d402be7396ce74fbba3795769cda45aec00dc3125a984f2a9f23177b190da2c", 28);
    }

    #[test]
    fn l1_order_actions() {
        check(&order(None, gtc()), None, true, "0xd65369825a9df5d80099e513cce430311d7d26ddf477f5b3a33d2806b100d78e", "0x2b54116ff64054968aa237c20ca9ff68000f977c93289157748a3162b6ea940e", 28);
        check(&order(None, gtc()), None, false, "0x82b2ba28e76b3d761093aaded1b1cdad4960b3af30212b343fb2e6cdfa4e3d54", "0x6b53878fc99d26047f4d7e8c90eb98955a109f44209163f52d8dc4278cbbd9f5", 27);
        let c = Some("0x00000000000000000000000000000001");
        check(&order(c, gtc()), None, true, "0x41ae18e8239a56cacbc5dad94d45d0b747e5da11ad564077fcac71277a946e3", "0x3c61f667e747404fe7eea8f90ab0e76cc12ce60270438b2058324681a00116da", 27);
        check(&order(c, gtc()), None, false, "0xeba0664bed2676fc4e5a743bf89e5c7501aa6d870bdb9446e122c9466c5cd16d", "0x7f3e74825c9114bc59086f1eebea2928c190fdfbfde144827cb02b85bbe90988", 28);
        let sl = Mp::map(vec![("trigger", Mp::map(vec![("isMarket", Mp::Bool(true)), ("triggerPx", Mp::str("103")), ("tpsl", Mp::str("sl"))]))]);
        check(&order(None, sl), None, true, "0x98343f2b5ae8e26bb2587daad3863bc70d8792b09af1841b6fdd530a2065a3f9", "0x6b5bb6bb0633b710aa22b721dd9dee6d083646a5f8e581a20b545be6c1feb405", 27);
    }

    #[test]
    fn wire_floats() {
        assert_eq!(float_to_wire(100.0).unwrap(), "100");
        assert_eq!(float_to_wire(1670.1).unwrap(), "1670.1");
        assert_eq!(float_to_wire(0.0147).unwrap(), "0.0147");
        assert_eq!(float_to_wire(0.0).unwrap(), "0");
        assert!(float_to_wire(0.123456789).is_err());
    }

    #[test]
    fn agent_address_from_key() {
        // Well-known address of this test key (eth_account).
        assert_eq!(Signer::from_hex(KEY).unwrap().address, "0x14791697260e4c9a71f18484c9f997b308e59325");
    }
}
