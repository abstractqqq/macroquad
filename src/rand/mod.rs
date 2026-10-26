//! Fast, lightweight random number generation and slice sampling.
//!
//! Vendored and modernized replacement for `quad-rand`.

use core::sync::atomic::{AtomicU64, Ordering};

const DEFAULT_INC: u64 = 1442695040888963407;
const MULTIPLIER: u64 = 6364136223846793005;

/// Thread-safe pseudo-random number generator (PCG32).
#[derive(Debug)]
pub struct RandGenerator {
    state: AtomicU64,
}

impl Default for RandGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl RandGenerator {
    /// Creates a new unseeded random generator.
    pub const fn new() -> Self {
        Self {
            state: AtomicU64::new(0),
        }
    }

    /// Seeds the generator with the specified seed value.
    pub fn srand(&self, seed: u64) {
        self.state.store(0, Ordering::Relaxed);
        self.rand();
        let oldstate = self.state.load(Ordering::Relaxed);
        self.state
            .store(oldstate.wrapping_add(seed), Ordering::Relaxed);
        self.rand();
    }

    /// Returns a pseudo-random `u32` in `0..=u32::MAX`.
    pub fn rand(&self) -> u32 {
        let mut oldstate = self.state.load(Ordering::Relaxed);
        loop {
            let next_state = oldstate.wrapping_mul(MULTIPLIER).wrapping_add(DEFAULT_INC);
            match self.state.compare_exchange_weak(
                oldstate,
                next_state,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => oldstate = actual,
            }
        }
        let xorshifted = (((oldstate >> 18) ^ oldstate) >> 27) as u32;
        let rot = (oldstate >> 59) as u32;
        xorshifted.rotate_right(rot)
    }

    /// Returns a pseudo-random `u64` in `0..=u64::MAX`.
    #[inline]
    pub fn rand_u64(&self) -> u64 {
        ((self.rand() as u64) << 32) | (self.rand() as u64)
    }

    /// Generates a random value of type `T` within `[low, high)`.
    #[inline]
    pub fn gen_range<T>(&self, low: T, high: T) -> T
    where
        T: RandomRange,
    {
        T::gen_range_with_state(self, low, high)
    }
}

static GLOBAL_STATE: RandGenerator = RandGenerator::new();

/// Seeds the global pseudo-random number generator with the value `seed`.
#[inline]
pub fn srand(seed: u64) {
    GLOBAL_STATE.srand(seed);
}

/// Returns a pseudo-random `u32` in `0..=u32::MAX`.
#[inline]
pub fn rand() -> u32 {
    GLOBAL_STATE.rand()
}

/// Trait for types that can be sampled uniformly within a range `[low, high)`.
pub trait RandomRange: Sized {
    /// Generates a random value in `[low, high)` using the global generator.
    fn gen_range(low: Self, high: Self) -> Self;

    /// Generates a random value in `[low, high)` using the provided generator state.
    fn gen_range_with_state(state: &RandGenerator, low: Self, high: Self) -> Self;
}

macro_rules! impl_int_range_32 {
    ($($ty:ty),* $(,)?) => {
        $(
            impl RandomRange for $ty {
                #[inline]
                fn gen_range(low: Self, high: Self) -> Self {
                    Self::gen_range_with_state(&GLOBAL_STATE, low, high)
                }

                #[inline]
                fn gen_range_with_state(gen: &RandGenerator, low: Self, high: Self) -> Self {
                    if low >= high {
                        return low;
                    }
                    // Bounded integer multiplication via Lemire reduction
                    let range = (high as i64 - low as i64) as u64;
                    let offset = ((gen.rand() as u64 * range) >> 32) as i64;
                    (low as i64 + offset) as Self
                }
            }
        )*
    };
}

impl_int_range_32!(u8, u16, u32, i8, i16, i32);

#[cfg(target_pointer_width = "32")]
impl_int_range_32!(usize, isize);

macro_rules! impl_int_range_64 {
    ($($ty:ty),* $(,)?) => {
        $(
            impl RandomRange for $ty {
                #[inline]
                fn gen_range(low: Self, high: Self) -> Self {
                    Self::gen_range_with_state(&GLOBAL_STATE, low, high)
                }

                #[inline]
                fn gen_range_with_state(gen: &RandGenerator, low: Self, high: Self) -> Self {
                    if low >= high {
                        return low;
                    }
                    let range = (high as i128 - low as i128) as u128;
                    let offset = ((gen.rand_u64() as u128 * range) >> 64) as i128;
                    (low as i128 + offset) as Self
                }
            }
        )*
    };
}

impl_int_range_64!(u64, i64);

#[cfg(target_pointer_width = "64")]
impl_int_range_64!(usize, isize);

impl RandomRange for f32 {
    #[inline]
    fn gen_range(low: Self, high: Self) -> Self {
        Self::gen_range_with_state(&GLOBAL_STATE, low, high)
    }

    #[inline]
    fn gen_range_with_state(gen: &RandGenerator, low: Self, high: Self) -> Self {
        if low >= high {
            return low;
        }
        let r = gen.rand() as f32 / (u32::MAX as f32 + 1.0);
        low + (high - low) * r
    }
}

impl RandomRange for f64 {
    #[inline]
    fn gen_range(low: Self, high: Self) -> Self {
        Self::gen_range_with_state(&GLOBAL_STATE, low, high)
    }

    #[inline]
    fn gen_range_with_state(gen: &RandGenerator, low: Self, high: Self) -> Self {
        if low >= high {
            return low;
        }
        let r = gen.rand() as f64 / (u32::MAX as f64 + 1.0);
        low + (high - low) * r
    }
}

/// Generates a random value in `[low, high)` using the global generator.
#[inline]
pub fn gen_range<T>(low: T, high: T) -> T
where
    T: RandomRange,
{
    GLOBAL_STATE.gen_range(low, high)
}

/// An iterator yielding randomly selected elements from a slice.
pub struct SliceChooseIter<'a, T> {
    source: &'a [T],
    indices: std::vec::IntoIter<usize>,
}

impl<'a, T> Iterator for SliceChooseIter<'a, T> {
    type Item = &'a T;

    #[inline]
    fn next(&mut self) -> Option<&'a T> {
        self.indices.next().map(|ix| &self.source[ix])
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.indices.size_hint()
    }
}

impl<'a, T> ExactSizeIterator for SliceChooseIter<'a, T> {}

/// Random selection and shuffling helper methods on slices.
pub trait ChooseRandom<T> {
    /// In-place shuffle of slice elements using the global generator.
    #[inline]
    fn shuffle(&mut self) {
        self.shuffle_with_state(&GLOBAL_STATE)
    }

    /// Randomly picks an element reference, or `None` if the slice is empty.
    #[inline]
    fn choose(&self) -> Option<&T> {
        self.choose_with_state(&GLOBAL_STATE)
    }

    /// Randomly picks a mutable element reference, or `None` if the slice is empty.
    #[inline]
    fn choose_mut(&mut self) -> Option<&mut T> {
        self.choose_mut_with_state(&GLOBAL_STATE)
    }

    /// Yields up to `amount` randomly selected elements from the slice.
    #[inline]
    fn choose_multiple(&self, amount: usize) -> SliceChooseIter<'_, T> {
        self.choose_multiple_with_state(&GLOBAL_STATE, amount)
    }

    /// In-place shuffle using a specific generator state.
    fn shuffle_with_state(&mut self, state: &RandGenerator);

    /// Randomly picks an element using a specific generator state.
    fn choose_with_state(&self, state: &RandGenerator) -> Option<&T>;

    /// Randomly picks a mutable element using a specific generator state.
    fn choose_mut_with_state(&mut self, state: &RandGenerator) -> Option<&mut T>;

    /// Yields up to `amount` randomly selected elements using a specific generator state.
    fn choose_multiple_with_state(
        &self,
        state: &RandGenerator,
        amount: usize,
    ) -> SliceChooseIter<'_, T>;
}

impl<T> ChooseRandom<T> for [T] {
    #[inline]
    fn shuffle_with_state(&mut self, state: &RandGenerator) {
        let len = self.len();
        if len <= 1 {
            return;
        }
        for i in (1..len).rev() {
            let j = ((state.rand_u64() as u128 * (i as u128 + 1)) >> 64) as usize;
            self.swap(i, j);
        }
    }

    #[inline]
    fn choose_with_state(&self, state: &RandGenerator) -> Option<&T> {
        if self.is_empty() {
            return None;
        }
        let ix = ((state.rand_u64() as u128 * self.len() as u128) >> 64) as usize;
        self.get(ix)
    }

    #[inline]
    fn choose_mut_with_state(&mut self, state: &RandGenerator) -> Option<&mut T> {
        if self.is_empty() {
            return None;
        }
        let ix = ((state.rand_u64() as u128 * self.len() as u128) >> 64) as usize;
        self.get_mut(ix)
    }

    #[inline]
    fn choose_multiple_with_state(
        &self,
        state: &RandGenerator,
        amount: usize,
    ) -> SliceChooseIter<'_, T> {
        let amount = amount.min(self.len());
        let mut indices: Vec<usize> = (0..self.len()).collect();
        for i in 0..amount {
            let j = i + ((state.rand_u64() as u128 * (self.len() - i) as u128) >> 64) as usize;
            indices.swap(i, j);
        }
        indices.truncate(amount);

        SliceChooseIter {
            source: self,
            indices: indices.into_iter(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_srand_determinism() {
        let gen = RandGenerator::new();
        gen.srand(12345);
        let a = gen.rand();
        let b = gen.rand();
        gen.srand(12345);
        assert_eq!(a, gen.rand());
        assert_eq!(b, gen.rand());
    }

    #[test]
    fn test_gen_range_integers() {
        let gen = RandGenerator::new();
        gen.srand(42);
        for _ in 0..1000 {
            let val = gen.gen_range(10i32, 20i32);
            assert!(val >= 10 && val < 20);

            let val_u8 = gen.gen_range(5u8, 15u8);
            assert!(val_u8 >= 5 && val_u8 < 15);

            let val_neg = gen.gen_range(-50i32, -10i32);
            assert!(val_neg >= -50 && val_neg < -10);

            let val_u64 = gen.gen_range(1_000_000_000_000u64, 2_000_000_000_000u64);
            assert!(val_u64 >= 1_000_000_000_000 && val_u64 < 2_000_000_000_000);
        }
    }

    #[test]
    fn test_gen_range_floats() {
        let gen = RandGenerator::new();
        gen.srand(42);
        for _ in 0..1000 {
            let val = gen.gen_range(-250.0f32, 250.0f32);
            assert!(val >= -250.0 && val < 250.0);

            let val_f64 = gen.gen_range(0.0f64, 1.0f64);
            assert!(val_f64 >= 0.0 && val_f64 < 1.0);
        }
    }

    #[test]
    fn test_empty_or_single_ranges() {
        let gen = RandGenerator::new();
        assert_eq!(gen.gen_range(5, 5), 5);
        assert_eq!(gen.gen_range(10, 5), 10);
        assert_eq!(gen.gen_range(5.0f32, 5.0f32), 5.0);
    }

    #[test]
    fn test_choose_and_shuffle() {
        let gen = RandGenerator::new();
        let mut data = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let original = data;
        data.shuffle_with_state(&gen);
        assert_eq!(data.len(), original.len());
        assert!(data.iter().all(|x| original.contains(x)));

        let chosen = original.choose_with_state(&gen);
        assert!(chosen.is_some());
        assert!(original.contains(chosen.unwrap()));

        let empty: [i32; 0] = [];
        assert_eq!(empty.choose_with_state(&gen), None);

        let sampled: Vec<_> = original
            .choose_multiple_with_state(&gen, 3)
            .copied()
            .collect();
        assert_eq!(sampled.len(), 3);
        for item in &sampled {
            assert!(original.contains(item));
        }

        // Also test global convenience functions
        let _ = rand();
        let _ = gen_range(0, 10);
    }
}
