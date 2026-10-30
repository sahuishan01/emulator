use emulator_gpu::*;
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

#[test]
fn literal_packets_have_expected_subchannels_methods_and_payloads() {
    // Independently authored wire vectors, not constructed with decoder helpers.
    let input = bytes(&[
        0,
        0x2003_2040,
        10,
        20,
        30,
        0x6002_4050,
        40,
        50,
        0x8123_6060,
        0xa003_8070,
        60,
        70,
        80,
    ]);
    let result = decode_pushbuffer(&input, 9).unwrap();
    let expected = [
        (1, 0x40, 10),
        (1, 0x41, 20),
        (1, 0x42, 30),
        (2, 0x50, 40),
        (2, 0x50, 50),
        (3, 0x60, 0x123),
        (4, 0x70, 60),
        (4, 0x71, 70),
        (4, 0x71, 80),
    ];
    assert_eq!(result.len(), expected.len());
    for (actual, (subchannel, method_word, data)) in result.into_iter().zip(expected) {
        assert_eq!(
            actual,
            MethodWrite {
                subchannel,
                method_word,
                data
            }
        );
        assert_eq!(actual.method_byte_offset(), u32::from(method_word) * 4);
    }
}
#[test]
fn gpfifo_entry_uses_all_40_address_bits_and_exact_declared_capture_length() {
    let entry = GpFifoEntry::parse(&[0x00, 0x10, 0x00, 0x00, 0x12, 0x04, 0x00, 0x00]).unwrap();
    assert_eq!(entry.address, GpuVa(0x12_0000_1000));
    assert_eq!(entry.word_count, 1);
    assert_eq!(
        entry.decode(&0x8000_0000_u32.to_le_bytes(), 1).unwrap(),
        [MethodWrite {
            subchannel: 0,
            method_word: 0,
            data: 0
        }]
    );
    assert_eq!(entry.decode(&[], 1), Err(DecodeError::LengthMismatch));
    assert_eq!(entry.decode(&[0; 8], 1), Err(DecodeError::LengthMismatch));
    for n in [0, 1, 7, 9] {
        assert_eq!(
            GpFifoEntry::parse(&vec![0; n]),
            Err(DecodeError::InvalidEntrySize)
        );
    }
}
#[test]
fn unsupported_entry_control_flags_and_address_overflow_fail_explicitly() {
    for (low, high) in [
        (1, 0x400),
        (2, 0x400),
        (0, 0x500),
        (0, 0x600),
        (0, 0x80000400),
        (0, 0),
    ] {
        assert_eq!(
            GpFifoEntry::parse(&bytes(&[low, high])),
            Err(DecodeError::UnsupportedEntry)
        );
    }
    assert_eq!(
        GpFifoEntry::parse(&bytes(&[0xffff_fffc, 0x8ff])),
        Err(DecodeError::AddressOverflow)
    );
    let forged = GpFifoEntry {
        address: GpuVa(u64::MAX),
        word_count: 1,
    };
    assert_eq!(forged.decode(&[0; 4], 1), Err(DecodeError::AddressOverflow));
}
#[test]
fn malformed_packet_prefixes_and_unsupported_forms_never_decode_as_success() {
    let valid = bytes(&[0x2003_2040, 10, 20, 30]);
    for length in 1..valid.len() {
        assert!(decode_pushbuffer(&valid[..length], 10).is_err());
    }
    for header in [
        0x0001_0000,
        0x4001_0000,
        0xc000_0000,
        0xe000_0000,
        0x2001_1000,
    ] {
        assert!(matches!(
            decode_pushbuffer(&bytes(&[header, 0]), 2),
            Err(DecodeError::UnsupportedPacket { word_offset: 0, .. })
        ));
    }
    assert_eq!(decode_pushbuffer(&[0; 3], 0), Err(DecodeError::PartialWord));
}
#[test]
fn input_output_budgets_and_method_boundaries_are_checked_before_publication() {
    assert_eq!(
        decode_pushbuffer(&vec![0; (MAX_PUSHBUFFER_WORDS + 1) * 4], 0),
        Err(DecodeError::TooLarge)
    );
    assert_eq!(
        decode_pushbuffer(&[], MAX_METHOD_WRITES + 1),
        Err(DecodeError::InvalidBudget)
    );
    assert!(matches!(
        decode_pushbuffer(&bytes(&[0x2002_0000, 1, 2]), 1),
        Err(DecodeError::WriteBudget { word_offset: 0 })
    ));
    for header in [0x2002_0fff, 0xa002_0fff] {
        assert_eq!(
            decode_pushbuffer(&bytes(&[header, 1, 2]), 2),
            Err(DecodeError::MethodOverflow { word_offset: 0 })
        );
    }
    let result = decode_pushbuffer(&bytes(&[0x6002_0fff, 1, 2]), 2).unwrap();
    assert_eq!(result[1].method_word, 0xfff);
    assert_eq!(decode_pushbuffer(&[0; 4], 0).unwrap(), []);
    assert!(matches!(
        decode_pushbuffer(&bytes(&[0x8001_0000, 0xe000_0000]), 2),
        Err(DecodeError::UnsupportedPacket { word_offset: 1, .. })
    ));
}
