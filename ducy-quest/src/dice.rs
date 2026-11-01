//! The world's dice: a number in `0..1` from the seed and some keys, the same
//! as the page's `roll(seed, ...keys)` gave (its keys joined with `|`).

/// A key for [`roll`]: text, or a whole number written as JavaScript writes
/// it.
pub trait Key {
    /// Appends the key as text.
    fn put(&self, s: &mut String);
}

impl Key for &str {
    fn put(&self, s: &mut String) {
        s.push_str(self);
    }
}
impl Key for String {
    fn put(&self, s: &mut String) {
        s.push_str(self);
    }
}
impl Key for &String {
    fn put(&self, s: &mut String) {
        s.push_str(self);
    }
}
macro_rules! int_key {
    ($($t:ty),*) => {$(
        impl Key for $t {
            fn put(&self, s: &mut String) {
                s.push_str(&self.to_string());
            }
        }
    )*};
}
int_key!(i32, i64, u32, u64, usize);
impl Key for f64 {
    fn put(&self, s: &mut String) {
        s.push_str(&js_num(*self));
    }
}

/// A number as JavaScript's `String(x)` writes it (for the sizes the quest
/// uses: no exponents).
pub fn js_num(x: f64) -> String {
    if x == 0.0 {
        "0".to_string()
    } else if x.is_nan() {
        "NaN".to_string()
    } else if x.is_infinite() {
        if x > 0.0 { "Infinity" } else { "-Infinity" }.to_string()
    } else {
        x.to_string()
    }
}

/// The keys joined with `|`.
pub fn join(keys: &[&dyn Key]) -> String {
    let mut s = String::new();
    for (i, k) in keys.iter().enumerate() {
        if i > 0 {
            s.push('|');
        }
        k.put(&mut s);
    }
    s
}

/// A number in `0..1` from `seed` and the joined keys `key`.
pub fn roll_str(seed: u64, key: &str) -> f64 {
    // JavaScript's `seed ^ c` takes the seed's low 32 bits.
    let mut h = (seed as u32) ^ 0x9e37_79b9;
    for c in key.chars() {
        // `charCodeAt(0)` of each code point: its first UTF-16 unit.
        let mut buf = [0u16; 2];
        let unit = c.encode_utf16(&mut buf)[0] as u32;
        h = (h ^ unit).wrapping_mul(0x85eb_ca6b);
        h ^= h >> 13;
    }
    h = (h ^ (h >> 16)).wrapping_mul(0xc2b2_ae35);
    (h ^ (h >> 16)) as f64 / 4_294_967_296.0
}

/// `roll!(seed; key, key, …)`: the world's dice.
#[macro_export]
macro_rules! roll {
    ($seed:expr; $($k:expr),+ $(,)?) => {
        $crate::dice::roll_str(($seed) as u64, &$crate::dice::join(&[$(&$k as &dyn $crate::dice::Key),+]))
    };
}

/// A new person's seed from the dice: `floor(roll * 2^31) + 1`.
pub fn seed_from(r: f64) -> u64 {
    (r * 2_147_483_648.0).floor() as u64 + 1
}

#[cfg(test)]
mod tests {
    #[test]
    fn rolls_are_in_range_and_repeat() {
        let a = roll!(42u32; "deuce", "1/2", "hour", 7i64);
        assert!((0.0..1.0).contains(&a));
        assert_eq!(a, roll!(42u32; "deuce", "1/2", "hour", 7i64));
        assert_ne!(a, roll!(43u32; "deuce", "1/2", "hour", 7i64));
        // A whole float is written as JavaScript writes it.
        assert_eq!(roll!(1; "x", 7.0), roll!(1; "x", 7i64));
        assert_eq!(super::js_num(-0.0), "0");
        assert_eq!(super::js_num(8470.5), "8470.5");
    }
}
