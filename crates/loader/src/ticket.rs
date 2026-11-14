//! Clean-room Nintendo Switch Ticket (.tik) parser and title key extractor.
//!
//! Nintendo Switch submissions and packages (NSP) can contain ticket files (`.tik`)
//! containing the Rights ID and encrypted title keys. For common tickets, the title
//! key is encrypted via AES-128-ECB using the firmware generation `titlekek`.

use crate::LoadError;
use emulator_crypto::aes128::aes128_ecb_decrypt;
use emulator_crypto::keys::KeyFile;

pub const TICKET_MIN_DATA_SIZE: usize = 0x170;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub signature_type: u32,
    pub issuer: String,
    pub title_key_block: [u8; 256],
    pub title_key_type: u8,
    pub master_key_revision: u8,
    pub ticket_id: u64,
    pub device_id: u64,
    pub rights_id: [u8; 16],
    pub account_id: u32,
}

impl Ticket {
    /// Parse a ticket binary file (`.tik`).
    pub fn parse(bytes: &[u8]) -> Result<Self, LoadError> {
        if bytes.len() < 4 {
            return Err(LoadError::Invalid("ticket header is truncated"));
        }

        let sig_type = u32::from_be_bytes(bytes[0..4].try_into().unwrap());
        let (sig_len, pad_len) = match sig_type {
            0x010000 | 0x010003 => (0x200, 0x3C), // RSA-4096 (SHA1 / SHA256)
            0x010001 | 0x010004 => (0x100, 0x3C), // RSA-2048 (SHA1 / SHA256)
            0x010002 | 0x010005 => (0x3C, 0x40),  // ECDSA (SHA1 / SHA256)
            0x010006 => (0x14, 0x28),             // HMAC-SHA1
            _ => return Err(LoadError::Invalid("unrecognized ticket signature type")),
        };

        let data_offset = 4 + sig_len + pad_len;
        if bytes.len() < data_offset + TICKET_MIN_DATA_SIZE {
            return Err(LoadError::Invalid("ticket data payload is truncated"));
        }

        let data = &bytes[data_offset..];

        // 0x000..0x040: Issuer string (null-terminated UTF-8)
        let issuer_end = data[..0x40]
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(0x40);
        let issuer = std::str::from_utf8(&data[..issuer_end])
            .unwrap_or("")
            .to_string();

        // 0x040..0x140: Title key block (256 bytes)
        let mut title_key_block = [0u8; 256];
        title_key_block.copy_from_slice(&data[0x40..0x140]);

        // 0x140: Format version (u8)
        // 0x141: Title key type (0 = Common / AES-128, 1 = Personalized / RSA)
        let title_key_type = data[0x141];

        // 0x145: Master key revision / crypto generation
        let master_key_revision = data[0x145];

        // 0x150..0x158: Ticket ID (u64 LE)
        let ticket_id = u64::from_le_bytes(data[0x150..0x158].try_into().unwrap());

        // 0x158..0x160: Device ID (u64 LE)
        let device_id = u64::from_le_bytes(data[0x158..0x160].try_into().unwrap());

        // 0x160..0x170: Rights ID (16 bytes)
        let mut rights_id = [0u8; 16];
        rights_id.copy_from_slice(&data[0x160..0x170]);

        // 0x170..0x174: Account ID (u32 LE)
        let account_id = u32::from_le_bytes(data[0x170..0x174].try_into().unwrap());

        Ok(Self {
            signature_type: sig_type,
            issuer,
            title_key_block,
            title_key_type,
            master_key_revision,
            ticket_id,
            device_id,
            rights_id,
            account_id,
        })
    }

    /// Whether this ticket contains a Common AES-128 title key.
    pub fn is_common(&self) -> bool {
        self.title_key_type == 0
    }

    /// Decrypt the 16-byte title key from a common ticket using the provided KeyFile.
    pub fn decrypt_title_key(&self, keys: &KeyFile) -> Result<[u8; 16], LoadError> {
        if !self.is_common() {
            return Err(LoadError::Invalid(
                "ticket is not a common ticket; personalized RSA title keys require console-unique certs",
            ));
        }

        let titlekek = keys
            .title_kek(self.master_key_revision)
            .ok_or(LoadError::Invalid(
                "titlekek for ticket master key revision is missing from key file",
            ))?;

        let mut title_key = [0u8; 16];
        title_key.copy_from_slice(&self.title_key_block[..16]);

        aes128_ecb_decrypt(titlekek, &mut title_key)
            .map_err(|_| LoadError::Invalid("ticket AES-128-ECB title key decryption failed"))?;

        Ok(title_key)
    }

    /// Decrypt the title key if common, and register it directly into the KeyFile.
    pub fn import_into(&self, keys: &mut KeyFile) -> Result<[u8; 16], LoadError> {
        let title_key = self.decrypt_title_key(keys)?;
        keys.add_title_key(&self.rights_id, title_key);
        Ok(title_key)
    }
}
