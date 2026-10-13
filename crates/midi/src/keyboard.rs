//! Computer-keyboard MIDI: two rows of keys play a chromatic octave; Z/X shift
//! octaves and C/V change velocity (the layout follows common DAW practice).

/// Returns the semitone offset from the octave base for a key name.
pub fn key_semitone(key: &str) -> Option<u8> {
    Some(match key {
        "A" => 0,
        "W" => 1,
        "S" => 2,
        "E" => 3,
        "D" => 4,
        "F" => 5,
        "T" => 6,
        "G" => 7,
        "Y" => 8,
        "H" => 9,
        "U" => 10,
        "J" => 11,
        "K" => 12,
        "O" => 13,
        "L" => 14,
        "P" => 15,
        _ => return None,
    })
}

#[derive(Clone, Copy, Debug)]
pub struct ComputerKeyboard {
    pub octave: i32,
    pub velocity: u8,
}

impl Default for ComputerKeyboard {
    fn default() -> Self {
        Self { octave: 3, velocity: 100 }
    }
}

impl ComputerKeyboard {
    /// MIDI note for a key (octave 3 → C3 = 60).
    pub fn note_for(&self, key: &str) -> Option<u8> {
        let semi = key_semitone(key)? as i32;
        let n = (self.octave + 2) * 12 + semi;
        (0..=127).contains(&n).then_some(n as u8)
    }

    pub fn octave_down(&mut self) {
        self.octave = (self.octave - 1).max(-2);
    }

    pub fn octave_up(&mut self) {
        self.octave = (self.octave + 1).min(8);
    }

    pub fn velocity_down(&mut self) {
        self.velocity = self.velocity.saturating_sub(20).max(1);
    }

    pub fn velocity_up(&mut self) {
        self.velocity = (self.velocity + 20).min(127);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_a_is_c3() {
        let mut k = ComputerKeyboard::default();
        assert_eq!(k.note_for("A"), Some(60));
        assert_eq!(k.note_for("K"), Some(72));
        k.octave_down();
        assert_eq!(k.note_for("A"), Some(48));
        assert_eq!(k.note_for("Q"), None);
    }
}
