//! Bounded parser for user-supplied `name = hex` key files.
//!
//! The parser does not locate, persist, or log key files. It keeps key bytes in
//! memory only and redacts them from `Debug` output. Zeroing on drop is best
//! effort; callers must still protect the process and its memory.

use std::collections::BTreeMap;
use std::fmt;

pub const MAX_KEY_FILE_SIZE: usize = 64 * 1024;
pub const MAX_KEY_ENTRIES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyFileError {
    TooLarge,
    InvalidUtf8,
    TooManyEntries,
    InvalidLine(usize),
    InvalidName(usize),
    InvalidHex(usize),
    UnsupportedKeyLength(usize),
    DuplicateName(usize),
}

impl fmt::Display for KeyFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => write!(f, "key file exceeds size limit"),
            Self::InvalidUtf8 => write!(f, "key file contains invalid UTF-8"),
            Self::TooManyEntries => write!(f, "key file contains too many entries"),
            Self::InvalidLine(line) => write!(f, "invalid line at {line}"),
            Self::InvalidName(line) => write!(f, "invalid key name at {line}"),
            Self::InvalidHex(line) => write!(f, "invalid hex at {line}"),
            Self::UnsupportedKeyLength(line) => write!(f, "unsupported key length at {line}"),
            Self::DuplicateName(line) => write!(f, "duplicate key name at {line}"),
        }
    }
}

impl std::error::Error for KeyFileError {}

impl Default for KeyFile {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }
}

struct SecretKey(Vec<u8>);

impl Drop for SecretKey {
    fn drop(&mut self) {
        self.0.fill(0);
        std::hint::black_box(&mut self.0);
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// In-memory key/value file with redacted debug formatting.
pub struct KeyFile {
    entries: BTreeMap<String, SecretKey>,
}

impl fmt::Debug for KeyFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyFile")
            .field("entry_count", &self.entries.len())
            .field("key_bytes", &"<redacted>")
            .finish()
    }
}

impl KeyFile {
    /// Parse a bounded UTF-8 file containing one `name = hex` entry per line.
    /// Blank lines and full-line `#` comments are ignored. Keys must be 16 or
    /// 32 bytes. Errors identify line numbers without echoing input data.
    pub fn parse(input: &[u8]) -> Result<Self, KeyFileError> {
        if input.len() > MAX_KEY_FILE_SIZE {
            return Err(KeyFileError::TooLarge);
        }
        let text = std::str::from_utf8(input).map_err(|_| KeyFileError::InvalidUtf8)?;
        let mut entries = BTreeMap::new();
        for (line_index, line) in text.lines().enumerate() {
            let line_number = line_index + 1;
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if entries.len() == MAX_KEY_ENTRIES {
                return Err(KeyFileError::TooManyEntries);
            }
            let Some((name, value)) = line.split_once('=') else {
                return Err(KeyFileError::InvalidLine(line_number));
            };
            let name = name.trim();
            let value = value.trim();
            if name.is_empty()
                || name.len() > 128
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
            {
                return Err(KeyFileError::InvalidName(line_number));
            }
            if value.len() % 2 != 0 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(KeyFileError::InvalidHex(line_number));
            }
            if value.len() != 32 && value.len() != 64 {
                return Err(KeyFileError::UnsupportedKeyLength(line_number));
            }
            let mut decoded = Vec::with_capacity(value.len() / 2);
            for pair in value.as_bytes().chunks_exact(2) {
                let high = hex_nibble(pair[0]).ok_or(KeyFileError::InvalidHex(line_number))?;
                let low = hex_nibble(pair[1]).ok_or(KeyFileError::InvalidHex(line_number))?;
                decoded.push((high << 4) | low);
            }
            if entries
                .insert(name.to_ascii_lowercase(), SecretKey(decoded))
                .is_some()
            {
                return Err(KeyFileError::DuplicateName(line_number));
            }
        }
        Ok(Self { entries })
    }

    /// Return a key by its normalized lower-case name.
    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.entries
            .get(&name.to_ascii_lowercase())
            .map(|key| key.0.as_slice())
    }

    /// Add or update a key by name. Key must be 16 or 32 bytes.
    pub fn add_key(&mut self, name: &str, key: &[u8]) -> Result<(), KeyFileError> {
        if key.len() != 16 && key.len() != 32 {
            return Err(KeyFileError::UnsupportedKeyLength(0));
        }
        if name.is_empty()
            || name.len() > 128
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        {
            return Err(KeyFileError::InvalidName(0));
        }
        self.entries
            .insert(name.to_ascii_lowercase(), SecretKey(key.to_vec()));
        Ok(())
    }

    /// Explicitly register a 16-byte title key under its 16-byte rights ID.
    pub fn add_title_key(&mut self, rights_id: &[u8; 16], title_key: [u8; 16]) {
        let id = encode_hex(rights_id);
        self.entries
            .insert(format!("titlekey_{id}"), SecretKey(title_key.to_vec()));
        self.entries
            .insert(id, SecretKey(title_key.to_vec()));
    }

    /// Merge all entries from another KeyFile into this one.
    pub fn merge(&mut self, other: &KeyFile) {
        for (name, key) in &other.entries {
            self.entries
                .insert(name.clone(), SecretKey(key.0.clone()));
        }
    }

    /// Number of distinct key entries currently stored.
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Load and parse a key file from disk.
    pub fn load_file<P: AsRef<std::path::Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let data = std::fs::read(path)?;
        let key_file = Self::parse(&data)?;
        Ok(key_file)
    }

    pub fn header_key(&self) -> Option<&[u8; 32]> {
        self.get("header_key")?.try_into().ok()
    }

    pub fn master_key(&self, generation: u8) -> Option<&[u8; 16]> {
        let name = format!("master_key_{generation:02x}");
        self.get(&name)?.try_into().ok()
    }

    pub fn key_area_key(&self, area: KeyArea, generation: u8) -> Option<&[u8; 16]> {
        let name = format!("key_area_key_{}_{generation:02x}", area.as_str());
        self.get(&name)?.try_into().ok()
    }

    pub fn title_kek(&self, generation: u8) -> Option<&[u8; 16]> {
        let name = format!("titlekek_{generation:02x}");
        self.get(&name)?.try_into().ok()
    }

    /// Look up a title key named by a 128-bit rights ID, accepting either a
    /// bare hex ID or the explicit `titlekey_<id>` form.
    pub fn title_key(&self, rights_id: &[u8; 16]) -> Option<&[u8; 16]> {
        let id = encode_hex(rights_id);
        self.get(&format!("titlekey_{id}"))
            .or_else(|| self.get(&id))?
            .try_into()
            .ok()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyArea {
    Application,
    Ocean,
    System,
}

impl KeyArea {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Application => "application",
            Self::Ocean => "ocean",
            Self::System => "system",
        }
    }
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}
