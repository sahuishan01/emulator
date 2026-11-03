use crate::{DecodeError as E, MAX_DECODED_BYTES};

#[derive(Clone, Copy, Debug)]
pub enum StorageLayout {
    Pitch { pitch: usize },
    BlockLinear { height_log2: u8 },
}
pub fn storage_size(row_bytes: usize, rows: usize, layout: StorageLayout) -> Result<usize, E> {
    if row_bytes == 0 || rows == 0 {
        return Err(E::InvalidDimensions);
    }
    let (pitch, height) = match layout {
        StorageLayout::Pitch { pitch } if pitch >= row_bytes => (pitch, rows),
        StorageLayout::BlockLinear { height_log2 } if height_log2 <= 5 => {
            let height = 8usize << height_log2;
            (
                row_bytes.checked_add(63).ok_or(E::SizeOverflow)? / 64 * 64,
                rows.checked_add(height - 1).ok_or(E::SizeOverflow)? / height * height,
            )
        }
        _ => return Err(E::InvalidLayout),
    };
    let size = pitch.checked_mul(height).ok_or(E::SizeOverflow)?;
    if size > MAX_DECODED_BYTES {
        return Err(E::AllocationLimit);
    }
    Ok(size)
}
fn offset(x: usize, y: usize, row_bytes: usize, height_log2: u8) -> usize {
    let bh = 1usize << height_log2;
    let blocks_x = row_bytes.div_ceil(64);
    let block_y = y / (8 * bh);
    let gob_y = y % (8 * bh) / 8;
    let gob = (block_y * blocks_x + x / 64) * bh + gob_y;
    // Generic 16Bx2 GOB: byte bits x[3:0],y[0],x[4],y[2:1],x[5].
    gob * 512 + (x & 15) + ((y & 1) << 4) + ((x & 16) << 1) + ((y & 6) << 5) + ((x & 32) << 3)
}
/// Unpack byte rows (RGBA texels or compressed blocks); strip all pitch/GOB padding.
pub fn unpack_2d(
    input: &[u8],
    row_bytes: usize,
    rows: usize,
    layout: StorageLayout,
) -> Result<Vec<u8>, E> {
    let expected = storage_size(row_bytes, rows, layout)?;
    if input.len() != expected {
        return Err(E::InputLength {
            expected,
            actual: input.len(),
        });
    }
    let size = row_bytes.checked_mul(rows).ok_or(E::SizeOverflow)?;
    let mut out = Vec::new();
    out.try_reserve_exact(size)
        .map_err(|_| E::AllocationFailed)?;
    out.resize(size, 0);
    for y in 0..rows {
        for x in 0..row_bytes {
            let from = match layout {
                StorageLayout::Pitch { pitch } => y * pitch + x,
                StorageLayout::BlockLinear { height_log2 } => offset(x, y, row_bytes, height_log2),
            };
            out[y * row_bytes + x] = input[from];
        }
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_gob_bit_locations_and_cross_block_boundaries() {
        for (x, y, expected) in [
            (0, 0, 0),
            (15, 0, 15),
            (16, 0, 32),
            (32, 0, 256),
            (0, 1, 16),
            (0, 2, 64),
            (0, 4, 128),
            (63, 7, 511),
            (64, 0, 1024),
            (0, 8, 512),
            (0, 16, 2048),
        ] {
            assert_eq!(offset(x, y, 128, 1), expected);
        }
        for log in 0..=5 {
            let mut seen = std::collections::BTreeSet::new();
            for y in 0..(8 << log) * 2 {
                for x in 0..128 {
                    assert!(seen.insert(offset(x, y, 128, log)));
                }
            }
            assert_eq!(seen.len(), 128 * (8 << log) * 2);
            assert_eq!(*seen.last().unwrap(), seen.len() - 1);
        }
    }
    #[test]
    fn invalid_layout_and_allocation_are_rejected_before_allocation() {
        assert!(storage_size(0, 1, StorageLayout::Pitch { pitch: 32 }).is_err());
        assert!(storage_size(33, 1, StorageLayout::Pitch { pitch: 32 }).is_err());
        assert!(storage_size(1, 1, StorageLayout::BlockLinear { height_log2: 6 }).is_err());
        assert!(
            storage_size(usize::MAX, 2, StorageLayout::BlockLinear { height_log2: 0 }).is_err()
        );
        assert!(storage_size(
            MAX_DECODED_BYTES,
            2,
            StorageLayout::Pitch {
                pitch: MAX_DECODED_BYTES
            }
        )
        .is_err());
        assert!(unpack_2d(&[0; 31], 4, 1, StorageLayout::Pitch { pitch: 32 }).is_err());
        assert_eq!(
            unpack_2d(
                &[1, 2, 3, 4, 99, 99, 99, 99],
                4,
                1,
                StorageLayout::Pitch { pitch: 8 }
            )
            .unwrap(),
            [1, 2, 3, 4]
        );
    }
}
