use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Digest {
    pub algorithm: Algorithm,
    pub hash: String,
    pub size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Algorithm {
    Blake3,
    Sha256,
}

impl fmt::Display for Algorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Blake3 => "blake3",
            Self::Sha256 => "sha256",
        })
    }
}

impl std::str::FromStr for Algorithm {
    type Err = &'static str;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "blake3" => Ok(Self::Blake3),
            "sha256" => Ok(Self::Sha256),
            _ => Err("unsupported digest algorithm"),
        }
    }
}

impl Digest {
    pub fn validate(&self) -> bool {
        self.hash.len() == 64
            && self
                .hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }

    pub fn key(&self) -> String {
        format!("{}/{}/{}", self.algorithm, self.hash, self.size)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionResultEnvelope {
    pub result: ActionResult,
    #[serde(default)]
    pub signatures: Vec<Signature>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionResult {
    pub action: Digest,
    pub metadata: Option<Digest>,
    pub output_root: Option<Digest>,
    pub version: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signature {
    pub algorithm: String,
    pub key_id: String,
    pub signature: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Directory {
    pub directories: Vec<DirectoryNode>,
    pub files: Vec<FileNode>,
    pub symlinks: Vec<SymlinkNode>,
    pub version: u8,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryNode {
    pub digest: Digest,
    pub mode: u32,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileNode {
    pub digest: Digest,
    pub executable: bool,
    pub mode: u32,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymlinkNode {
    pub mode: u32,
    pub name: String,
    pub target: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_lowercase_hex_digests() {
        let valid = Digest {
            algorithm: Algorithm::Blake3,
            hash: "a".repeat(64),
            size: 42,
        };
        assert!(valid.validate());
        let invalid = Digest {
            hash: "A".repeat(64),
            ..valid
        };
        assert!(!invalid.validate());
    }
}
