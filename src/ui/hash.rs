use std::hash::{BuildHasher, Hash};

#[doc(hidden)]
pub fn hash_value<T: Hash + ?Sized>(value: &T) -> u64 {
    foldhash::fast::FixedState::default().hash_one(value)
}

#[macro_export]
#[doc(hidden)]
macro_rules! hash {
    ($s:expr) => {{
        let id = $s;
        $crate::ui::hash_value(&id)
    }};
    () => {{
        let id = concat!(file!(), line!(), column!());
        hash!(id)
    }};
    ($($s:expr),*) => {{
        let mut s: u128 = 0;
        $(s += $crate::hash!($s) as u128;)*
        $crate::hash!(s)
    }};
}

#[cfg(test)]
mod tests {
    #[test]
    fn identical_values_have_identical_hashes() {
        assert_eq!(hash!("widget"), hash!("widget"));
        assert_eq!(hash!(42_u64, "widget"), hash!(42_u64, "widget"));
    }

    #[test]
    fn distinct_values_have_distinct_hashes() {
        assert_ne!(hash!("widget-a"), hash!("widget-b"));
        assert_ne!(hash!(1_u64, "widget"), hash!(2_u64, "widget"));
    }
}
