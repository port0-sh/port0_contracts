pub fn pack_u32_pair(lo: u32, hi: u32) -> u64 {
    (lo as u64) | ((hi as u64) << 32)
}
pub fn unpack_u32_pair(packed: u64) -> (u32, u32) {
    let lo = packed as u32;
    let hi = (packed >> 32) as u32;
    (lo, hi)
}
