//! Dense native relationships compiled from names by the offline importer.
use pumpkin_data::BlockId;

const HEADER: usize = 14;
const STRIDE: usize = 6;
const DATA: &[u8; HEADER + BlockId::COUNT as usize * STRIDE] =
    include_bytes!("block_relations.bin");

// Catch stale tables even when a registry update keeps the same number of blocks.
const _: () = {
    assert!(DATA[0] == b'H' && DATA[1] == b'B' && DATA[2] == b'R' && DATA[3] == 1);
    assert!(u16::from_le_bytes([DATA[4], DATA[5]]) == BlockId::COUNT);
    let expected = u64::from_le_bytes([
        DATA[6], DATA[7], DATA[8], DATA[9], DATA[10], DATA[11], DATA[12], DATA[13],
    ]);
    let mut fingerprint = 0xcbf29ce484222325u64;
    let mut id = 0;
    while id < BlockId::COUNT {
        let name = BlockId::new_or_air(id).to_block().name.as_bytes();
        let mut index = 0;
        while index < name.len() {
            fingerprint = (fingerprint ^ name[index] as u64).wrapping_mul(0x100000001b3);
            index += 1;
        }
        fingerprint = fingerprint.wrapping_mul(0x100000001b3);
        id += 1;
    }
    assert!(
        fingerprint == expected,
        "Stale block relationships; run scripts/generate_block_relations.py"
    );
};

fn lookup(id: BlockId, field: usize) -> Option<BlockId> {
    let offset = HEADER + id.as_u16() as usize * STRIDE + field * 2;
    BlockId::new(u16::from_le_bytes([DATA[offset], DATA[offset + 1]]))
}
pub(super) fn canonical(id: BlockId) -> BlockId {
    lookup(id, 0).expect("Native canonical block identity")
}
pub(super) fn wall(id: BlockId) -> Option<BlockId> {
    lookup(id, 1)
}
pub(super) fn material(id: BlockId) -> Option<BlockId> {
    lookup(id, 2)
}
