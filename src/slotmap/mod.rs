// Vendored and adapted from slotmap 1.1.1.
// The original implementation is Copyright (c) 2021 Orson Peters.
// See LICENSE for the complete Zlib license notice.

mod basic;
mod util;

use core::fmt::{self, Debug, Formatter};
use core::hash::{Hash, Hasher};
use core::num::NonZeroU32;

pub(crate) use basic::SlotMap;

/// The actual data stored in a key.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct KeyData {
    idx: u32,
    version: NonZeroU32,
}

impl KeyData {
    const fn new(idx: u32, version: u32) -> Self {
        debug_assert!(version > 0);

        Self {
            idx,
            version: unsafe { NonZeroU32::new_unchecked(version | 1) },
        }
    }

    fn null() -> Self {
        Self::new(u32::MAX, 1)
    }

    fn is_null(self) -> bool {
        self.idx == u32::MAX
    }

    pub(crate) fn as_ffi(self) -> u64 {
        (u64::from(self.version.get()) << 32) | u64::from(self.idx)
    }

    pub(crate) const fn from_ffi(value: u64) -> Self {
        let idx = value & 0xffff_ffff;
        let version = (value >> 32) | 1;
        Self::new(idx as u32, version as u32)
    }
}

impl Debug for KeyData {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        if self.is_null() {
            f.write_str("null")
        } else {
            write!(f, "{}v{}", self.idx, self.version.get())
        }
    }
}

impl Default for KeyData {
    fn default() -> Self {
        Self::null()
    }
}

impl Hash for KeyData {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.as_ffi())
    }
}

pub(crate) unsafe trait Key:
    From<KeyData>
    + Copy
    + Clone
    + Default
    + Eq
    + PartialEq
    + Ord
    + PartialOrd
    + core::hash::Hash
    + core::fmt::Debug
{
    fn null() -> Self {
        KeyData::null().into()
    }

    fn is_null(&self) -> bool {
        self.data().is_null()
    }

    fn data(&self) -> KeyData;
}

#[derive(Copy, Clone, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
#[repr(transparent)]
pub(crate) struct DefaultKey(KeyData);

impl From<KeyData> for DefaultKey {
    fn from(k: KeyData) -> Self {
        DefaultKey(k)
    }
}

unsafe impl Key for DefaultKey {
    fn data(&self) -> KeyData {
        self.0
    }
}
