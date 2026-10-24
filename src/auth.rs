use std::{fs::File, io::Read};

use crate::Error;

/// A random secret a viewer must present to connect.
pub struct AccessToken {
    value: String,
}

impl AccessToken {
    /// Generates a new random token.
    pub fn generate() -> Result<Self, Error> {
        let mut bytes = [0u8; 16];
        File::open("/dev/urandom")?.read_exact(&mut bytes)?;
        let value = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        Ok(Self { value })
    }

    /// Returns the token as text.
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// Returns true only for an exact match, compared in constant time.
    pub fn verify(&self, candidate: &str) -> bool {
        let (expected, candidate) = (self.value.as_bytes(), candidate.as_bytes());
        if expected.len() != candidate.len() {
            return false;
        }
        expected
            .iter()
            .zip(candidate)
            .fold(0, |diff, (a, b)| diff | (a ^ b))
            == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_32_hex_characters() {
        let token = AccessToken::generate().unwrap();
        assert_eq!(token.as_str().len(), 32);
        assert!(token.as_str().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn tokens_are_different_each_time() {
        let first = AccessToken::generate().unwrap();
        let second = AccessToken::generate().unwrap();
        assert_ne!(first.as_str(), second.as_str());
    }

    #[test]
    fn accepts_only_the_exact_token() {
        let token = AccessToken::generate().unwrap();
        assert!(token.verify(token.as_str()));
        assert!(!token.verify(""));
        assert!(!token.verify("not-the-token"));
        assert!(!token.verify(&token.as_str().to_uppercase()));
        assert!(!token.verify(&format!("{}0", token.as_str())));
    }
}
