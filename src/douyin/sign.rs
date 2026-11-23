//! Douyin request signatures, ported from douyinLive (MIT, jwwsjlm).
//!
//! - WebSocket `signature`: `internal/webcastsign/native.go`
//! - HTTP `a_bogus`: `sign/ab_sign.go`, including its SM3
//!
//! This file depends only on std and `md-5`, so the recording tools in
//! `scripts/douyin/` compile it unchanged and check it against
//! what the Go original returned.

use md5::{Digest, Md5};

/// Error text is the Go original's.
pub const INVALID_STUB: &str = "X-MS-STUB 必须是 32 位十六进制 MD5";

const WEBSOCKET_ALPHABET: &[u8; 64] =
    b"Dkdpgh4ZKsQB80/Mfvw36XI1R25+WUAlEi7NLboqYTOPuzmFjJnryx9HVGcaStCe";
const UA_ALPHABET: &[u8; 64] = b"ckdp1h4ZKsUB80/Mfvw36XIgR25+WQAlEi7NLboqYTOPuzmFjJnryx9HVGDaStCe";
const BOGUS_ALPHABET: &[u8; 64] =
    b"Dkdpgh2ZmsQB80/MfvV36XI1R45-WUAlEixNLwoqYTOPuzKFjJnry79HbGcaStCe";
const WINDOW_ENV: &str = "1920|1080|1920|1040|0|30|0|0|1872|92|1920|1040|1857|92|1|24|Win32";

/// Base64 without padding over a custom alphabet, as Go's `base64.NewEncoding(..).WithPadding(NoPadding)`.
fn encode(alphabet: &[u8; 64], data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..=chunk.len() {
            out.push(char::from(alphabet[((n >> (18 - 6 * i)) & 63) as usize]));
        }
    }
    out
}

/// RC4 keystream XOR, as Go's `crypto/rc4`. `key` must be 1 to 256 bytes.
fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut s: [u8; 256] = std::array::from_fn(|i| i as u8);
    let mut j = 0u8;
    for i in 0..256 {
        j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len()]);
        s.swap(i, j as usize);
    }
    let (mut i, mut j) = (0u8, 0u8);
    data.iter()
        .map(|b| {
            i = i.wrapping_add(1);
            j = j.wrapping_add(s[i as usize]);
            s.swap(i as usize, j as usize);
            b ^ s[s[i as usize].wrapping_add(s[j as usize]) as usize]
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// MD5 of the bytes, as lowercase hex.
pub fn md5_hex(data: &[u8]) -> String {
    hex(&Md5::digest(data))
}

/// The WebSocket `signature` for an X-MS-STUB, with every random choice given.
/// Port of `SignWithValues`.
pub fn websocket_signature_with(
    stub: &str,
    counter: u8,
    random_flag: bool,
    payload_random: u8,
    key_random: u8,
) -> Result<String, String> {
    // Go's hex.DecodeString takes hex digits only; from_str_radix alone would also take "+".
    let bytes = (stub.len() == 32 && stub.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| {
            (0..16)
                .map(|i| u8::from_str_radix(stub.get(2 * i..2 * i + 2)?, 16).ok())
                .collect::<Option<Vec<u8>>>()
        })
        .flatten()
        .ok_or_else(|| INVALID_STUB.to_owned())?;
    let stub_digest = Md5::digest(&bytes);
    let empty_digest = Md5::digest(Md5::digest([]));
    let mut payload = [
        counter & 0x3f,
        0,
        1,
        14,
        empty_digest[14],
        empty_digest[15],
        stub_digest[14],
        stub_digest[15],
        payload_random,
        0,
    ];
    payload[9] = payload[..9].iter().fold(0, |x, b| x ^ b);
    let mut flags = 1u8 << 6;
    if random_flag {
        flags |= 1 << 4;
    }
    let mut result = vec![flags, key_random];
    result.extend(rc4(&[key_random], &payload));
    Ok(encode(WEBSOCKET_ALPHABET, &result))
}

/// Keeps the per-session counter the WebSocket signature carries. Port of `Generator`.
#[derive(Default)]
pub struct WebsocketSigner {
    counter: u8,
}
impl WebsocketSigner {
    /// `random` must return uniformly random bytes.
    pub fn sign(&mut self, stub: &str, mut random: impl FnMut() -> u8) -> Result<String, String> {
        // Payload and key bytes stay in 0..=254, like Math.floor(255 * Math.random()).
        fn below_ff(random: &mut impl FnMut() -> u8) -> u8 {
            loop {
                let b = random();
                if b != 0xff {
                    break b;
                }
            }
        }
        let flag = random();
        self.counter = self.counter.wrapping_add(1);
        let payload = below_ff(&mut random);
        let key = below_ff(&mut random);
        websocket_signature_with(stub, self.counter & 0x3f, flag & 1 == 1, payload, key)
    }
}

/// SM3 as the Go original computes it.
struct Sm3 {
    reg: [u32; 8],
    chunk: Vec<u8>,
    size: usize,
}
impl Sm3 {
    const IV: [u32; 8] = [
        1937774191, 1226093241, 388252375, 3666478592, 2842636476, 372324522, 3817729613,
        2969243214,
    ];
    fn new() -> Self {
        Self {
            reg: Self::IV,
            chunk: Vec::with_capacity(64),
            size: 0,
        }
    }
    fn write(&mut self, data: &[u8]) {
        self.size += data.len();
        self.chunk.extend_from_slice(data);
        let full = self.chunk.len() / 64 * 64;
        let blocks = self.chunk.drain(..full).collect::<Vec<_>>();
        for block in blocks.chunks(64) {
            self.compress(block);
        }
    }
    fn compress(&mut self, data: &[u8]) {
        let mut w = [0u32; 132];
        for (t, word) in w.iter_mut().take(16).enumerate() {
            *word = u32::from_be_bytes([
                data[4 * t],
                data[4 * t + 1],
                data[4 * t + 2],
                data[4 * t + 3],
            ]);
        }
        for j in 16..68 {
            let a = w[j - 16] ^ w[j - 9] ^ w[j - 3].rotate_left(15);
            let a = a ^ a.rotate_left(15) ^ a.rotate_left(23);
            w[j] = a ^ w[j - 13].rotate_left(7) ^ w[j - 6];
        }
        for j in 0..64 {
            w[j + 68] = w[j] ^ w[j + 4];
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.reg;
        for j in 0..64 {
            let (tj, ff, gg) = if j < 16 {
                (0x79CC4519u32, a ^ b ^ c, e ^ f ^ g)
            } else {
                (0x7A879D8A, (a & b) | (a & c) | (b & c), (e & f) | (!e & g))
            };
            let ss1 = a
                .rotate_left(12)
                .wrapping_add(e)
                .wrapping_add(tj.rotate_left(j as u32 % 32))
                .rotate_left(7);
            let ss2 = ss1 ^ a.rotate_left(12);
            let tt1 = ff.wrapping_add(d).wrapping_add(ss2).wrapping_add(w[j + 68]);
            let tt2 = gg.wrapping_add(h).wrapping_add(ss1).wrapping_add(w[j]);
            d = c;
            c = b.rotate_left(9);
            b = a;
            a = tt1;
            h = g;
            g = f.rotate_left(19);
            f = e;
            e = tt2 ^ tt2.rotate_left(9) ^ tt2.rotate_left(17);
        }
        for (r, v) in self.reg.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *r ^= v;
        }
    }
    fn sum(data: &[u8]) -> [u8; 32] {
        let mut s = Self::new();
        s.write(data);
        let bits = (s.size as u64) * 8;
        let mut tail = std::mem::take(&mut s.chunk);
        tail.push(0x80);
        while tail.len() % 64 != 56 {
            tail.push(0);
        }
        tail.extend_from_slice(&bits.to_be_bytes());
        for block in tail.chunks(64) {
            s.compress(block);
        }
        let mut out = [0u8; 32];
        for (i, v) in s.reg.iter().enumerate() {
            out[4 * i..4 * i + 4].copy_from_slice(&v.to_be_bytes());
        }
        out
    }
}

/// SM3 digest as lowercase hex.
#[allow(
    dead_code,
    reason = "used by the tests and the recording crate, not by the app"
)]
pub fn sm3_hex(data: &[u8]) -> String {
    hex(&Sm3::sum(data))
}

/// Four bytes from a 16-bit value mixed with two option bytes. Port of `generRandom`.
fn gener_random(random: u32, option: [u8; 2]) -> [u8; 4] {
    let (b1, b2) = ((random & 255) as u8, ((random >> 8) & 255) as u8);
    [
        (b1 & 170) | (option[0] & 85),
        (b1 & 85) | (option[0] & 170),
        (b2 & 170) | (option[1] & 85),
        (b2 & 85) | (option[1] & 170),
    ]
}

/// The bb byte string, RC4-encrypted with key "y". Port of `generateRc4BbStr`
/// with the arguments the original always passes: suffix "cus", arguments [0, 1, 14].
fn bb_bytes(params: &str, user_agent: &str, start: u32) -> Vec<u8> {
    let url_hash = Sm3::sum(&Sm3::sum(format!("{params}cus").as_bytes()));
    let cus_hash = Sm3::sum(&Sm3::sum(b"cus"));
    let ua = encode(UA_ALPHABET, &rc4(&[0, 1, 14], user_agent.as_bytes()));
    let ua_hash = Sm3::sum(ua.as_bytes());
    let end = start.wrapping_add(100);
    let [s0, s1, s2, s3] = start.to_be_bytes();
    let [e0, e1, e2, e3] = end.to_be_bytes();
    let [p0, p1, p2, p3] = 110624u32.to_be_bytes();
    let [a0, a1, a2, a3] = 6383u32.to_le_bytes();
    let env = WINDOW_ENV.as_bytes();
    let (env_lo, env_hi) = ((env.len() & 255) as u8, ((env.len() >> 8) & 255) as u8);
    // Indexes into the original's b table; constant entries are written as values.
    let (b18, b26, b27, b28, b29, b30, b31, b32, b33) = (44u8, 0, 0, 0, 0, 0, 1, 0, 0);
    let (b34, b35, b36, b37, b48) = (0u8, 0, 0, 14, 3);
    let (b24, b25, b49, b50, b70, b71) = (0u8, 0, 0, 0, 0, 0);
    let (b38, b39, b40, b41, b42, b43) = (
        url_hash[21],
        url_hash[22],
        cus_hash[21],
        cus_hash[22],
        ua_hash[23],
        ua_hash[24],
    );
    let checksum = [
        b18, s0, b26, b30, b38, b40, b42, s1, b27, b31, b35, b39, b41, b43, s2, b28, b32, b36, s3,
        b29, b33, b37, e0, e1, e2, e3, b48, b49, b50, b24, b25, p0, p1, p2, p3, a0, a1, a2, a3,
        env_lo, env_hi, b70, b71,
    ]
    .iter()
    .fold(0, |x, b| x ^ b);
    let mut bb = vec![
        b18, s0, p0, b26, b30, b34, a1, b38, b40, p1, b42, s1, b27, p2, p3, b31, b35, a0, b39, b41,
        b43, s2, b28, b32, a3, b36, s3, b29, b33, b37, e0, e1, a2, e2, e3, b48, b49, b50, b24, b25,
        env_lo, env_hi, b70, b71,
    ];
    bb.extend_from_slice(env);
    bb.push(checksum);
    rc4(b"y", &bb)
}

/// The `a_bogus` value for a query string, at a start time in milliseconds
/// (only the low 32 bits count). Port of `AbSign` with the clock made explicit.
pub fn a_bogus_at(params: &str, user_agent: &str, start_ms: i64) -> String {
    let mut data = Vec::new();
    for (value, option) in [
        (0.123456789f64, [3, 45]),
        (0.987654321, [1, 0]),
        (0.555555555, [1, 5]),
    ] {
        data.extend(gener_random((value * 10000.0) as u32, option));
    }
    data.extend(bb_bytes(params, user_agent, start_ms as u32));
    encode(BOGUS_ALPHABET, &data) + "="
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sm3_matches_the_standard_test_vector() {
        // GB/T 32905-2016, example 1.
        assert_eq!(
            sm3_hex(b"abc"),
            "66c7f0f462eeedd9d1f2d46bdc10e4e24167c4875cf2f7a2297da02b8f4ba8e0"
        );
    }

    #[test]
    fn signer_skips_ff_for_payload_and_key() {
        let mut bytes = [1u8, 0xff, 7, 0xff, 9].into_iter();
        let mut signer = WebsocketSigner::default();
        let stub = md5_hex(b"x");
        let got = signer.sign(&stub, || bytes.next().unwrap_or(0)).unwrap();
        assert_eq!(got, websocket_signature_with(&stub, 1, true, 7, 9).unwrap());
    }
}
