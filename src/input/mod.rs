//! Cross-platform mouse, keyboard (and gamepads soon) module.

use crate::prelude::screen_height;
use crate::prelude::screen_width;
use crate::Vec2;
use crate::{get_context, DroppedFile};
pub use miniquad::{KeyCode, MouseButton};

macro_rules! define_key_indices {
    ($($index:literal => $key:ident),+ $(,)?) => {
        const KEY_COUNT: usize = 1 + *[$($index),+].last().unwrap();

        fn key_index(key: KeyCode) -> usize {
            match key {
                $(KeyCode::$key => $index,)+
            }
        }

        #[cfg(test)]
        const ALL_KEYS: [KeyCode; KEY_COUNT] = [$(KeyCode::$key,)+];
    };
}

define_key_indices! {
      0 => Space,
      1 => Apostrophe,
      2 => Comma,
      3 => Minus,
      4 => Period,
      5 => Slash,
      6 => Key0,
      7 => Key1,
      8 => Key2,
      9 => Key3,
     10 => Key4,
     11 => Key5,
     12 => Key6,
     13 => Key7,
     14 => Key8,
     15 => Key9,
     16 => Semicolon,
     17 => Equal,
     18 => A,
     19 => B,
     20 => C,
     21 => D,
     22 => E,
     23 => F,
     24 => G,
     25 => H,
     26 => I,
     27 => J,
     28 => K,
     29 => L,
     30 => M,
     31 => N,
     32 => O,
     33 => P,
     34 => Q,
     35 => R,
     36 => S,
     37 => T,
     38 => U,
     39 => V,
     40 => W,
     41 => X,
     42 => Y,
     43 => Z,
     44 => LeftBracket,
     45 => Backslash,
     46 => RightBracket,
     47 => GraveAccent,
     48 => World1,
     49 => World2,
     50 => Escape,
     51 => Enter,
     52 => Tab,
     53 => Backspace,
     54 => Insert,
     55 => Delete,
     56 => Right,
     57 => Left,
     58 => Down,
     59 => Up,
     60 => PageUp,
     61 => PageDown,
     62 => Home,
     63 => End,
     64 => CapsLock,
     65 => ScrollLock,
     66 => NumLock,
     67 => PrintScreen,
     68 => Pause,
     69 => F1,
     70 => F2,
     71 => F3,
     72 => F4,
     73 => F5,
     74 => F6,
     75 => F7,
     76 => F8,
     77 => F9,
     78 => F10,
     79 => F11,
     80 => F12,
     81 => F13,
     82 => F14,
     83 => F15,
     84 => F16,
     85 => F17,
     86 => F18,
     87 => F19,
     88 => F20,
     89 => F21,
     90 => F22,
     91 => F23,
     92 => F24,
     93 => F25,
     94 => Kp0,
     95 => Kp1,
     96 => Kp2,
     97 => Kp3,
     98 => Kp4,
     99 => Kp5,
    100 => Kp6,
    101 => Kp7,
    102 => Kp8,
    103 => Kp9,
    104 => KpDecimal,
    105 => KpDivide,
    106 => KpMultiply,
    107 => KpSubtract,
    108 => KpAdd,
    109 => KpEnter,
    110 => KpEqual,
    111 => LeftShift,
    112 => LeftControl,
    113 => LeftAlt,
    114 => LeftSuper,
    115 => RightShift,
    116 => RightControl,
    117 => RightAlt,
    118 => RightSuper,
    119 => Menu,
    120 => Back,
    121 => Unknown,
}

const KEY_WORDS: usize = KEY_COUNT.div_ceil(u64::BITS as usize);

#[derive(Default)]
struct KeyBits([u64; KEY_WORDS]);

impl KeyBits {
    fn contains(&self, key: KeyCode) -> bool {
        let index = key_index(key);
        self.0[index / u64::BITS as usize] & (1 << (index % u64::BITS as usize)) != 0
    }

    fn insert(&mut self, key: KeyCode) -> bool {
        let index = key_index(key);
        let bit = 1 << (index % u64::BITS as usize);
        let word = &mut self.0[index / u64::BITS as usize];
        let inserted = *word & bit == 0;
        *word |= bit;
        inserted
    }

    fn remove(&mut self, key: KeyCode) -> bool {
        let index = key_index(key);
        let bit = 1 << (index % u64::BITS as usize);
        let word = &mut self.0[index / u64::BITS as usize];
        let removed = *word & bit != 0;
        *word &= !bit;
        removed
    }

    fn clear(&mut self) {
        self.0.fill(0);
    }

    fn is_empty(&self) -> bool {
        self.0.iter().all(|word| *word == 0)
    }
}

pub(crate) struct KeyboardState {
    down: KeyBits,
    pressed: KeyBits,
    released: KeyBits,
    down_order: Vec<KeyCode>,
    pressed_order: Vec<KeyCode>,
    released_order: Vec<KeyCode>,
    last_pressed: Option<KeyCode>,
}

impl KeyboardState {
    pub(crate) fn new() -> Self {
        Self {
            down: KeyBits::default(),
            pressed: KeyBits::default(),
            released: KeyBits::default(),
            down_order: Vec::with_capacity(8),
            pressed_order: Vec::with_capacity(8),
            released_order: Vec::with_capacity(8),
            last_pressed: None,
        }
    }

    pub(crate) fn press(&mut self, key: KeyCode, repeat: bool) {
        if self.down.insert(key) {
            self.down_order.push(key);
        }
        if !repeat && self.pressed.insert(key) {
            self.pressed_order.push(key);
        }
        if !repeat {
            self.last_pressed = Some(key);
        }
    }

    pub(crate) fn release(&mut self, key: KeyCode) {
        if self.down.remove(key) {
            self.down_order.retain(|down| *down != key);
        }
        if self.released.insert(key) {
            self.released_order.push(key);
        }
    }

    pub(crate) fn release_all(&mut self) {
        for index in 0..self.down_order.len() {
            let key = self.down_order[index];
            self.down.remove(key);
            if self.released.insert(key) {
                self.released_order.push(key);
            }
        }
        self.down_order.clear();
    }

    pub(crate) fn clear_pressed(&mut self) {
        self.pressed.clear();
        self.pressed_order.clear();
        self.last_pressed = None;
    }

    pub(crate) fn clear_frame(&mut self) {
        self.clear_pressed();
        self.released.clear();
        self.released_order.clear();
    }

    pub(crate) fn is_down(&self, key: KeyCode) -> bool {
        self.down.contains(key)
    }

    pub(crate) fn is_pressed(&self, key: KeyCode) -> bool {
        self.pressed.contains(key)
    }

    pub(crate) fn is_released(&self, key: KeyCode) -> bool {
        self.released.contains(key)
    }

    pub(crate) fn is_any_down(&self) -> bool {
        !self.down.is_empty()
    }

    pub(crate) fn last_pressed(&self) -> Option<KeyCode> {
        self.last_pressed
    }

    pub(crate) fn pressed_keys(&self) -> Vec<KeyCode> {
        self.pressed_order.clone()
    }

    pub(crate) fn down_keys(&self) -> Vec<KeyCode> {
        self.down_order.clone()
    }

    pub(crate) fn released_keys(&self) -> Vec<KeyCode> {
        self.released_order.clone()
    }
}

fn mouse_button_bit(button: MouseButton) -> u8 {
    1 << match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::Unknown => 3,
    }
}

#[derive(Default)]
pub(crate) struct MouseButtonState {
    down: u8,
    pressed: u8,
    released: u8,
}

impl MouseButtonState {
    pub(crate) fn press(&mut self, button: MouseButton) {
        let bit = mouse_button_bit(button);
        self.down |= bit;
        self.pressed |= bit;
    }

    pub(crate) fn release(&mut self, button: MouseButton) {
        let bit = mouse_button_bit(button);
        self.down &= !bit;
        self.released |= bit;
    }

    pub(crate) fn release_all(&mut self) {
        self.released |= self.down;
        self.down = 0;
    }

    pub(crate) fn clear_pressed(&mut self) {
        self.pressed = 0;
    }

    pub(crate) fn clear_frame(&mut self) {
        self.pressed = 0;
        self.released = 0;
    }

    pub(crate) fn is_down(&self, button: MouseButton) -> bool {
        self.down & mouse_button_bit(button) != 0
    }

    pub(crate) fn is_pressed(&self, button: MouseButton) -> bool {
        self.pressed & mouse_button_bit(button) != 0
    }

    pub(crate) fn is_released(&self, button: MouseButton) -> bool {
        self.released & mouse_button_bit(button) != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TouchPhase {
    Started,
    Stationary,
    Moved,
    Ended,
    Cancelled,
}

impl From<miniquad::TouchPhase> for TouchPhase {
    fn from(miniquad_phase: miniquad::TouchPhase) -> TouchPhase {
        match miniquad_phase {
            miniquad::TouchPhase::Started => TouchPhase::Started,
            miniquad::TouchPhase::Moved => TouchPhase::Moved,
            miniquad::TouchPhase::Ended => TouchPhase::Ended,
            miniquad::TouchPhase::Cancelled => TouchPhase::Cancelled,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Touch {
    pub id: u64,
    pub phase: TouchPhase,
    pub position: Vec2,
}

/// Constrain mouse to window
pub fn set_cursor_grab(grab: bool) {
    let context = get_context();
    context.cursor_grabbed = grab;
    miniquad::window::set_cursor_grab(grab);
}

/// Set mouse cursor visibility
pub fn show_mouse(shown: bool) {
    miniquad::window::show_mouse(shown);
}

/// Return mouse position in pixels.
pub fn mouse_position() -> (f32, f32) {
    let context = get_context();

    (
        context.mouse_position.x / miniquad::window::dpi_scale(),
        context.mouse_position.y / miniquad::window::dpi_scale(),
    )
}

/// Return mouse position in range [-1; 1].
pub fn mouse_position_local() -> Vec2 {
    let (pixels_x, pixels_y) = mouse_position();

    convert_to_local(Vec2::new(pixels_x, pixels_y))
}

/// Returns the difference between the current mouse position and the mouse position on the previous frame.
pub fn mouse_delta_position() -> Vec2 {
    let context = get_context();

    let current_position = mouse_position_local();
    let last_position = context.last_mouse_position.unwrap_or(current_position);

    // Calculate the delta
    last_position - current_position
}

/// This is set to true by default, meaning touches will raise mouse events in addition to raising touch events.
/// If set to false, touches won't affect mouse events.
pub fn is_simulating_mouse_with_touch() -> bool {
    get_context().simulate_mouse_with_touch
}

/// This is set to true by default, meaning touches will raise mouse events in addition to raising touch events.
/// If set to false, touches won't affect mouse events.
pub fn simulate_mouse_with_touch(option: bool) {
    get_context().simulate_mouse_with_touch = option;
}

/// Return touches with positions in pixels.
pub fn touches() -> Vec<Touch> {
    get_context().touches.values().cloned().collect()
}

/// Return touches with positions in range [-1; 1].
pub fn touches_local() -> Vec<Touch> {
    get_context()
        .touches
        .values()
        .map(|touch| {
            let mut touch = touch.clone();
            touch.position = convert_to_local(touch.position);
            touch
        })
        .collect()
}

pub fn mouse_wheel() -> (f32, f32) {
    let context = get_context();

    (context.mouse_wheel.x, context.mouse_wheel.y)
}

/// Detect if the key has been pressed once
pub fn is_key_pressed(key_code: KeyCode) -> bool {
    let context = get_context();

    context.keyboard.is_pressed(key_code)
}

/// Detect if the key is being pressed
pub fn is_key_down(key_code: KeyCode) -> bool {
    let context = get_context();

    context.keyboard.is_down(key_code)
}

/// Detect if the key has been released this frame
pub fn is_key_released(key_code: KeyCode) -> bool {
    let context = get_context();

    context.keyboard.is_released(key_code)
}

/// Detect if any key is being pressed
pub fn is_any_key_down() -> bool {
    let context = get_context();
    context.keyboard.is_any_down()
}

/// Return the last pressed char.
/// Each "get_char_pressed" call will consume a character from the input queue.
pub fn get_char_pressed() -> Option<char> {
    let context = get_context();

    context.chars_pressed_queue.pop_front()
}

/// Return the last key pressed during this frame.
pub fn last_key_pressed() -> Option<KeyCode> {
    let context = get_context();
    context.keyboard.last_pressed()
}

/// Return keys pressed during this frame in event order.
pub fn keys_pressed() -> Vec<KeyCode> {
    let context = get_context();
    context.keyboard.pressed_keys()
}

/// Return held keys in the order they were pressed.
pub fn keys_down() -> Vec<KeyCode> {
    let context = get_context();
    context.keyboard.down_keys()
}

/// Return keys released during this frame in event order.
pub fn keys_released() -> Vec<KeyCode> {
    let context = get_context();
    context.keyboard.released_keys()
}

/// Clears input queue
pub fn clear_input_queue() {
    let context = get_context();
    context.chars_pressed_queue.clear();
    context.mouse_buttons.clear_pressed();
    context.keyboard.clear_pressed();
}

/// Detect if the button is being pressed
pub fn is_mouse_button_down(btn: MouseButton) -> bool {
    let context = get_context();

    context.mouse_buttons.is_down(btn)
}

/// Detect if the button has been pressed once
pub fn is_mouse_button_pressed(btn: MouseButton) -> bool {
    let context = get_context();

    context.mouse_buttons.is_pressed(btn)
}

/// Detect if the button has been released this frame
pub fn is_mouse_button_released(btn: MouseButton) -> bool {
    let context = get_context();

    context.mouse_buttons.is_released(btn)
}

/// Convert a position in pixels to a position in the range [-1; 1].
fn convert_to_local(pixel_pos: Vec2) -> Vec2 {
    Vec2::new(pixel_pos.x / screen_width(), pixel_pos.y / screen_height()) * 2.0
        - Vec2::new(1.0, 1.0)
}

/// Prevents quit
pub fn prevent_quit() {
    get_context().prevent_quit_event = true;
}

/// Detect if quit has been requested
pub fn is_quit_requested() -> bool {
    get_context().quit_requested
}

/// Gets the files which have been dropped on the window.
pub fn get_dropped_files() -> Vec<DroppedFile> {
    get_context().dropped_files()
}

/// Functions for advanced input processing.
///
/// Functions in this module should be used by external tools that uses miniquad system, like different UI libraries. User shouldn't use this function.
pub mod utils {
    use crate::get_context;

    /// Register input subscriber. Returns subscriber identifier that must be used in `repeat_all_miniquad_input`.
    pub fn register_input_subscriber() -> usize {
        let context = get_context();

        context.input_events.push(vec![]);

        context.input_events.len() - 1
    }

    /// Repeats all events that came since last call of this function with current value of `subscriber`. This function must be called at each frame.
    pub fn repeat_all_miniquad_input<T: miniquad::EventHandler>(t: &mut T, subscriber: usize) {
        let context = get_context();

        for event in &context.input_events[subscriber] {
            event.repeat(t);
        }
        context.input_events[subscriber].clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::HashSet,
        hint::black_box,
        time::{Duration, Instant},
    };

    #[test]
    fn every_key_has_a_unique_dense_index() {
        let mut seen = [false; KEY_COUNT];
        for key in ALL_KEYS {
            let index = key_index(key);
            assert!(index < KEY_COUNT);
            assert!(!seen[index], "duplicate index {index} for {key:?}");
            seen[index] = true;
        }
        assert!(seen.into_iter().all(|value| value));
    }

    #[test]
    fn key_bits_cover_every_key() {
        let mut bits = KeyBits::default();
        for key in ALL_KEYS {
            assert!(bits.insert(key));
            assert!(bits.contains(key));
            assert!(!bits.insert(key));
        }
        assert!(!bits.is_empty());

        for key in ALL_KEYS {
            assert!(bits.remove(key));
            assert!(!bits.contains(key));
            assert!(!bits.remove(key));
        }
        assert!(bits.is_empty());
    }

    #[test]
    fn keyboard_tracks_transitions_and_order() {
        let mut keyboard = KeyboardState::new();
        keyboard.press(KeyCode::A, false);
        keyboard.press(KeyCode::B, false);
        keyboard.press(KeyCode::A, false);

        assert!(keyboard.is_down(KeyCode::A));
        assert!(keyboard.is_pressed(KeyCode::A));
        assert_eq!(keyboard.down_keys(), [KeyCode::A, KeyCode::B]);
        assert_eq!(keyboard.pressed_keys(), [KeyCode::A, KeyCode::B]);
        assert_eq!(keyboard.last_pressed(), Some(KeyCode::A));

        keyboard.release(KeyCode::A);
        keyboard.press(KeyCode::A, false);
        assert!(keyboard.is_down(KeyCode::A));
        assert!(keyboard.is_released(KeyCode::A));
        assert_eq!(keyboard.down_keys(), [KeyCode::B, KeyCode::A]);
        assert_eq!(keyboard.released_keys(), [KeyCode::A]);
        assert_eq!(keyboard.pressed_keys(), [KeyCode::A, KeyCode::B]);
        assert_eq!(keyboard.last_pressed(), Some(KeyCode::A));
    }

    #[test]
    fn repeat_does_not_create_a_pressed_transition() {
        let mut keyboard = KeyboardState::new();
        keyboard.press(KeyCode::A, true);

        assert!(keyboard.is_down(KeyCode::A));
        assert!(!keyboard.is_pressed(KeyCode::A));
        assert!(keyboard.pressed_keys().is_empty());
    }

    #[test]
    fn frame_reset_keeps_held_keys_and_vector_capacity() {
        let mut keyboard = KeyboardState::new();
        keyboard.press(KeyCode::A, false);
        keyboard.release(KeyCode::B);
        let pressed_capacity = keyboard.pressed_order.capacity();
        let released_capacity = keyboard.released_order.capacity();

        keyboard.clear_frame();

        assert!(keyboard.is_down(KeyCode::A));
        assert!(!keyboard.is_pressed(KeyCode::A));
        assert!(!keyboard.is_released(KeyCode::B));
        assert_eq!(keyboard.pressed_order.capacity(), pressed_capacity);
        assert_eq!(keyboard.released_order.capacity(), released_capacity);
    }

    #[test]
    fn releasing_all_keys_is_ordered_and_retains_capacity() {
        let mut keyboard = KeyboardState::new();
        keyboard.press(KeyCode::B, false);
        keyboard.press(KeyCode::A, false);
        let down_capacity = keyboard.down_order.capacity();

        keyboard.release_all();

        assert!(!keyboard.is_any_down());
        assert_eq!(keyboard.released_keys(), [KeyCode::B, KeyCode::A]);
        assert_eq!(keyboard.down_order.capacity(), down_capacity);
    }

    #[test]
    fn mouse_tracks_transitions_and_releases_all_buttons() {
        let mut mouse = MouseButtonState::default();
        mouse.press(MouseButton::Left);
        mouse.press(MouseButton::Right);
        assert!(mouse.is_down(MouseButton::Left));
        assert!(mouse.is_pressed(MouseButton::Right));

        mouse.release(MouseButton::Left);
        assert!(!mouse.is_down(MouseButton::Left));
        assert!(mouse.is_released(MouseButton::Left));

        mouse.release_all();
        assert!(!mouse.is_down(MouseButton::Right));
        assert!(mouse.is_released(MouseButton::Right));

        mouse.clear_frame();
        assert!(!mouse.is_pressed(MouseButton::Right));
        assert!(!mouse.is_released(MouseButton::Right));
    }

    fn median(mut samples: Vec<Duration>) -> Duration {
        samples.sort_unstable();
        samples[samples.len() / 2]
    }

    #[test]
    #[ignore = "microbenchmark; run explicitly in release mode"]
    fn input_state_benchmark() {
        const ITERATIONS: usize = 10_000_000;
        const SAMPLES: usize = 7;

        let mut keyboard = KeyboardState::new();
        keyboard.press(KeyCode::Space, false);
        let mut hash_set = HashSet::new();
        hash_set.insert(KeyCode::Space);

        let mut bitset_samples = Vec::with_capacity(SAMPLES);
        let mut hash_set_samples = Vec::with_capacity(SAMPLES);
        for sample in 0..SAMPLES {
            let time_bitset = || {
                let start = Instant::now();
                for _ in 0..ITERATIONS {
                    black_box(keyboard.is_down(black_box(KeyCode::Space)));
                }
                start.elapsed()
            };
            let time_hash_set = || {
                let start = Instant::now();
                for _ in 0..ITERATIONS {
                    black_box(hash_set.contains(black_box(&KeyCode::Space)));
                }
                start.elapsed()
            };

            if sample % 2 == 0 {
                bitset_samples.push(time_bitset());
                hash_set_samples.push(time_hash_set());
            } else {
                hash_set_samples.push(time_hash_set());
                bitset_samples.push(time_bitset());
            }
        }

        let bitset = median(bitset_samples).as_secs_f64() * 1e9 / ITERATIONS as f64;
        let hash_set = median(hash_set_samples).as_secs_f64() * 1e9 / ITERATIONS as f64;
        println!(
            "is_key_down: bitset {bitset:.2} ns, HashSet {hash_set:.2} ns, speedup {:.2}x",
            hash_set / bitset
        );
    }
}
