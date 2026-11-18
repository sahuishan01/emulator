//! `csrng` GenerateRandomBytes over a real B buffer.
mod common;
use common::Rig;
use emulator_memory::Permissions;
use emulator_services::csrng::RandomService;

#[test]
fn random_bytes_fill_the_whole_buffer_and_differ() {
    let mut rig = Rig::new("csrng", Box::new(RandomService));
    rig.m.map(0x30000, vec![0; 0x1000], Permissions::READ_WRITE).unwrap();
    let mut draws = Vec::new();
    for _ in 0..2 {
        rig.call(0, &[], &[(0x30000, 0x100)], None, &[]);
        assert_eq!(rig.result(), 0);
        let mut bytes = vec![0u8; 0x101];
        rig.m.read(0x30000, &mut bytes).unwrap();
        assert_eq!(bytes[0x100], 0, "nothing past the buffer");
        bytes.truncate(0x100);
        assert!(bytes.iter().any(|&b| b != 0));
        draws.push(bytes);
    }
    assert_ne!(draws[0], draws[1]);
    assert!(rig.try_call(0, &[], &[], None, &[]).is_err(), "output buffer required");
    assert!(rig.try_call(1, &[], &[], None, &[]).is_err());
}
