//! The two string hashes the game turns into ZDO keys.
//!
//! `stable_hash` is Valheim's own `GetStableHashCode`, used for prefab ids and
//! most property keys. It hashes UTF-16 code units two at a time with wrapping
//! 32-bit arithmetic, so the result must be reproduced bit for bit.
//!
//! `animator_key` covers the keys `ZSyncAnimation` writes. Those are not
//! string hashes at all: the class stores animator parameters under
//! `438569 + Animator.StringToHash(name)`, and Unity's `Animator.StringToHash`
//! is CRC-32. Without this, every synced animation parameter on every creature
//! shows up as an unnameable hash and can never be attributed to a component.

pub fn stable_hash(s: &str) -> i32 {
    let units: Vec<u16> = s.encode_utf16().collect();
    let mut a: i32 = 5381;
    let mut b: i32 = 5381;
    let mut i = 0;
    while i < units.len() && units[i] != 0 {
        a = ((a << 5).wrapping_add(a)) ^ i32::from(units[i]);
        if i == units.len() - 1 || units[i + 1] == 0 {
            break;
        }
        b = ((b << 5).wrapping_add(b)) ^ i32::from(units[i + 1]);
        i += 2;
    }
    a.wrapping_add(b.wrapping_mul(1_566_083_941))
}

/// Offset `ZSyncAnimation` adds to a parameter hash to get its ZDO key.
const ANIMATOR_KEY_BASE: i32 = 438_569;

/// Unity `Animator.StringToHash`: CRC-32 (IEEE) of the name's bytes.
pub fn animator_hash(name: &str) -> i32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for &b in name.as_bytes() {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc as i32
}

/// The ZDO key `ZSyncAnimation` stores a synced parameter under.
pub fn animator_key(name: &str) -> i32 {
    ANIMATOR_KEY_BASE.wrapping_add(animator_hash(name))
}

#[cfg(test)]
mod tests {
    use super::{animator_hash, animator_key, stable_hash};

    #[test]
    fn empty_string_is_the_seed_combination() {
        assert_eq!(
            stable_hash(""),
            5381_i32.wrapping_add(5381_i32.wrapping_mul(1_566_083_941))
        );
    }

    #[test]
    fn distinct_keys_hash_differently() {
        assert_ne!(stable_hash("items"), stable_hash("item"));
        assert_ne!(stable_hash("tamed"), stable_hash("Tamed"));
    }

    /// Both vectors come from keys observed in real saves: `anim_speed` is
    /// written raw (`ZSyncAnimation.s_animSpeedID`), everything else through
    /// the offset.
    #[test]
    fn animator_hashes_match_observed_keys() {
        assert_eq!(animator_hash("anim_speed"), 1_477_933_170);
        assert_eq!(animator_key("flapping"), 23_215_934);
        assert_eq!(animator_key("forward_speed"), -1_489_121_593);
        assert_eq!(animator_key("onGround"), -1_493_684_636);
        assert_ne!(animator_key("alert"), stable_hash("alert"));
    }
}
