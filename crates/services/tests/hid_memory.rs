//! HID shared memory follows Horizon's layout: one handheld controller with a
//! single style bit, every other npad and device disconnected.
use emulator_services::hid::*;

fn u32_at(mem: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(mem[at..at + 4].try_into().unwrap())
}

fn u64_at(mem: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(mem[at..at + 8].try_into().unwrap())
}

#[test]
fn layout_offsets_are_consistent() {
    assert_eq!(NPAD_COMMON_LIFOS + 7 * NPAD_COMMON_LIFO_SIZE, NPAD_SIX_AXIS_LIFOS);
    assert_eq!(NPAD_SIX_AXIS_LIFOS + 6 * NPAD_SIX_AXIS_LIFO_SIZE, NPAD_DEVICE_TYPE);
    assert_eq!(LIFO_HEADER_SIZE + 17 * (8 + 0x28), NPAD_COMMON_LIFO_SIZE);
    assert_eq!(LIFO_HEADER_SIZE + 17 * (8 + 0x60), NPAD_SIX_AXIS_LIFO_SIZE);
    assert!(NPAD_OFFSET + NPAD_COUNT * NPAD_ENTRY_SIZE <= HID_SHARED_MEMORY_SIZE);
    assert_eq!(npad_entry(NPAD_ID_HANDHELD), Some(HANDHELD_ENTRY));
    assert_eq!((npad_entry(0), npad_entry(7), npad_entry(0x10), npad_entry(8)), (Some(0), Some(7), Some(9), None));
}

#[test]
fn only_the_handheld_npad_is_connected_with_one_style() {
    let mem = create_hid_shared_memory();
    let mem = mem.borrow();
    for entry in 0..NPAD_COUNT {
        let base = NPAD_OFFSET + entry * NPAD_ENTRY_SIZE;
        let style = u32_at(&mem, base + NPAD_STYLE_SET);
        if entry == HANDHELD_ENTRY {
            assert_eq!(style, NPAD_STYLE_HANDHELD);
            assert_eq!(style.count_ones(), 1, "GetVibrationDeviceHandles requires one style bit");
            assert_eq!(u32_at(&mem, base + NPAD_DEVICE_TYPE), 0x0c);
        } else {
            assert_eq!(style, 0, "npad entry {entry} is disconnected");
        }
        for lifo in 0..7 {
            let at = base + NPAD_COMMON_LIFOS + lifo * NPAD_COMMON_LIFO_SIZE;
            assert_eq!((u64_at(&mem, at + 8), u64_at(&mem, at + 0x10), u64_at(&mem, at + 0x18)), (17, 0, 1));
            let attributes = u32_at(&mem, at + LIFO_HEADER_SIZE + 8 + 0x20);
            let expected = if entry == HANDHELD_ENTRY && lifo == 1 { NPAD_ATTRIBUTES_HANDHELD } else { 0 };
            assert_eq!(attributes, expected, "entry {entry} lifo {lifo}");
            assert_eq!(u64_at(&mem, at + LIFO_HEADER_SIZE + 8 + 8), 0, "no buttons");
        }
        for lifo in 0..6 {
            let at = base + NPAD_SIX_AXIS_LIFOS + lifo * NPAD_SIX_AXIS_LIFO_SIZE;
            assert_eq!(u64_at(&mem, at + 8), 17);
            let attributes = u32_at(&mem, at + LIFO_HEADER_SIZE + 8 + 0x58);
            assert_eq!(attributes, u32::from(entry == HANDHELD_ENTRY && lifo == 1));
        }
    }
    for device in [DEBUG_PAD_OFFSET, TOUCH_SCREEN_OFFSET, MOUSE_OFFSET, KEYBOARD_OFFSET] {
        assert_eq!((u64_at(&mem, device + 8), u64_at(&mem, device + 0x18)), (17, 1));
    }
}
