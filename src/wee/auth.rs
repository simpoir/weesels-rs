use super::messages::Handshake;
use openssl::hash::{Hasher, MessageDigest};

/// A colon-separated list of hash algo supported.
pub const SUPPORTED_HASHES: &'static str = "plain:sha256:sha512"; // pbkdf2+sha256:pbkdf2+sha512

pub enum Algo<'a> {
    Plain,
    Sha { nonce: &'a str, size: &'a str },
    // Pbkdf2Sha256 { nonce: &'a str, iterations: u32 },
    // Pbkdf2Sha512 { nonce: &'a str, iterations: u32 },
}

impl<'a> std::convert::From<&'a Handshake> for Algo<'a> {
    fn from(h: &'a Handshake) -> Self {
        match h.password_hash_algo.as_str() {
            "plain" => Algo::Plain,
            "sha256" | "sha512" => Algo::Sha {
                nonce: h.nonce.as_str(),
                size: if h.password_hash_algo.as_str() == "sha256" {
                    "256"
                } else {
                    "512"
                },
            },
            _ => unimplemented!(),
        }
    }
}

/// Create an authentication options chunk usable for init messages.
pub fn create_auth(algo: Algo, password: &str) -> String {
    _create_auth(algo, password, openssl::rand::rand_bytes)
}

type _RandFunc = fn(&mut [u8]) -> Result<(), openssl::error::ErrorStack>;

fn _create_auth(algo: Algo, password: &str, rand: _RandFunc) -> String {
    match algo {
        Algo::Plain => format!("password={}", password),
        Algo::Sha { nonce, size } => {
            let mut c_nonce = [0u8; 7];
            rand(&mut c_nonce).unwrap();
            let mut hasher = Hasher::new(if "256" == size {
                MessageDigest::sha256()
            } else {
                MessageDigest::sha512()
            })
            .unwrap();
            let hash = {
                hasher
                    .update(hex::decode(nonce).unwrap().as_slice())
                    .unwrap();
                hasher.update(&c_nonce).unwrap();
                hasher.update(password.as_bytes()).unwrap();
                hasher.finish().unwrap()
            };

            format!(
                "password_hash=sha{}:{}{}:{}",
                size,
                nonce,
                hex::encode(c_nonce),
                hex::encode(hash)
            )
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::borrow::Borrow;

    fn handshake(algo: &'static str) -> Handshake {
        Handshake {
            password_hash_algo: String::from(algo),
            password_hash_iterations: String::new(),
            compression: String::new(),
            totp: String::new(),
            nonce: String::new(),
        }
    }

    impl Handshake {
        fn with_nonce(mut self, nonce: &'static str) -> Self {
            self.nonce = String::from(nonce);
            self
        }
    }

    fn not_random(buf: &mut [u8]) -> Result<(), openssl::error::ErrorStack> {
        println!("{}", buf.len());
        buf.clone_from_slice(b"\xa4\xb7\x32\x07\xf5\xaa\xe4");
        Ok(())
    }

    #[test]
    fn test_auth_plain() {
        let res = create_auth(handshake("plain").borrow().into(), "foobar");
        assert_eq!("password=foobar", res)
    }

    #[test]
    fn test_sha256() {
        let res = _create_auth(
            handshake("sha256")
                .with_nonce("85b1ee00695a5b254e14f4885538df0d")
                .borrow()
                .into(),
            "test",
            not_random,
        );
        assert_eq!(
            "password_hash=sha256:85b1ee00695a5b254e14f4885538df0da4b73207f5aae4:\
             2c6ed12eb0109fca3aedc03bf03d9b6e804cd60a23e1731fd17794da423e21db",
            res
        )
    }

    #[test]
    fn test_sha512() {
        let res = _create_auth(
            handshake("sha512")
                .with_nonce("85b1ee00695a5b254e14f4885538df0d")
                .borrow()
                .into(),
            "test",
            not_random,
        );
        let expected = "\
            password_hash=sha512:85b1ee00695a5b254e14f4885538df0da4b73207f5aae4:\
            0a1f0172a542916bd86e0cbceebc1c38ed791f6be246120452825f0d74ef1078c79e\
            9812de8b0ab3dfaf598b6ca14522374ec6a8653a46df3f96a6b54ac1f0f8";
        assert_eq!(expected, res)
    }
}
