use emulator_crypto::aes128::{aes128_ctr_xor, aes128_xts_decrypt, aes128_xts_encrypt, Aes128};
use emulator_crypto::CryptoError;

fn hex<const N: usize>(text: &str) -> [u8; N] {
    assert_eq!(text.len(), N * 2);
    let mut bytes = [0; N];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).unwrap();
    }
    bytes
}

#[test]
fn fips_197_aes128_encrypt_and_inverse_cipher_vector() {
    let aes = Aes128::new(&hex("000102030405060708090a0b0c0d0e0f"));
    let mut block = hex("00112233445566778899aabbccddeeff");
    aes.encrypt_block(&mut block);
    assert_eq!(block, hex("69c4e0d86a7b0430d8cdb78070b4c55a"));
    aes.decrypt_block(&mut block);
    assert_eq!(block, hex("00112233445566778899aabbccddeeff"));
}

#[test]
fn nist_sp800_38a_ctr_vector_and_involution() {
    let key = hex("2b7e151628aed2a6abf7158809cf4f3c");
    let counter = hex("f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff");
    let plaintext = hex::<16>("6bc1bee22e409f96e93d7e117393172a");
    let expected = hex::<16>("874d6191b620e3261bef6864990db6ce");
    let mut data = plaintext;
    aes128_ctr_xor(&key, counter, &mut data).unwrap();
    assert_eq!(data, expected);
    aes128_ctr_xor(&key, counter, &mut data).unwrap();
    assert_eq!(data, plaintext);
}

#[test]
fn xts_round_trip_two_blocks_and_rejects_partial_units() {
    let key = hex("2718281828459045235360287471352662497757247093699959574966967627");
    let tweak = [0; 16];
    let plaintext = [0xA5; 32];
    let mut ciphertext = plaintext;
    aes128_xts_encrypt(&key, tweak, &mut ciphertext).unwrap();
    assert_ne!(ciphertext, plaintext);
    aes128_xts_decrypt(&key, tweak, &mut ciphertext).unwrap();
    assert_eq!(ciphertext, plaintext);
    assert_eq!(
        aes128_xts_decrypt(&key, tweak, &mut [0; 15]),
        Err(CryptoError::XtsInputMustContainWholeBlocks)
    );
    assert_eq!(
        aes128_xts_decrypt(&key, tweak, &mut []),
        Err(CryptoError::XtsInputMustContainWholeBlocks)
    );
}

#[test]
fn xts_ieee_1619_single_block_known_answer() {
    let key = hex("2718281828459045235360287471352631415926535897932384626433832795");
    let tweak = [0; 16];
    let mut block = hex::<16>("000102030405060708090a0b0c0d0e0f");
    aes128_xts_encrypt(&key, tweak, &mut block).unwrap();
    assert_eq!(block, hex::<16>("27a7479befa1d476489f308cd4cfa6e2"));
    aes128_xts_decrypt(&key, tweak, &mut block).unwrap();
    assert_eq!(block, hex::<16>("000102030405060708090a0b0c0d0e0f"));
}

#[test]
fn xts_ieee_1619_two_block_tweak_advance_vector() {
    let key = [0; 32];
    let tweak = [0; 16];
    let mut data = [0; 32];
    aes128_xts_encrypt(&key, tweak, &mut data).unwrap();
    assert_eq!(
        data,
        hex("917cf69ebd68b2ec9b9fe9a3eadda692cd43d2f59598ed858c02c2652fbf922e")
    );
    aes128_xts_decrypt(&key, tweak, &mut data).unwrap();
    assert_eq!(data, [0; 32]);
}

#[test]
fn ctr_checks_wrap_before_modifying_data() {
    let key = [0; 16];
    let mut data = [0x5A; 17];
    let original = data;
    assert_eq!(
        aes128_ctr_xor(&key, [0xff; 16], &mut data),
        Err(CryptoError::CounterWouldWrap)
    );
    assert_eq!(data, original);
}
