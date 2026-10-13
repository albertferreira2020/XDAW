//! MIDI message parsing, input ports (midir), computer-keyboard note mapping
//! and Standard MIDI File import/export.

use xdaw_dsp::NoteKind;

pub mod file;
pub mod input;
pub mod keyboard;

/// Parsed channel-voice message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MidiMessage {
    NoteOn { channel: u8, note: u8, velocity: u8 },
    NoteOff { channel: u8, note: u8, velocity: u8 },
    ControlChange { channel: u8, cc: u8, value: u8 },
    PitchBend { channel: u8, value: u16 },
    Other,
}

pub fn parse(bytes: &[u8]) -> MidiMessage {
    let Some(&status) = bytes.first() else { return MidiMessage::Other };
    let channel = status & 0x0F;
    let d1 = bytes.get(1).copied().unwrap_or(0) & 0x7F;
    let d2 = bytes.get(2).copied().unwrap_or(0) & 0x7F;
    match status & 0xF0 {
        0x90 if bytes.len() >= 3 && d2 > 0 => MidiMessage::NoteOn { channel, note: d1, velocity: d2 },
        // Note-on with velocity 0 is a note-off by specification.
        0x90 | 0x80 if bytes.len() >= 3 => MidiMessage::NoteOff { channel, note: d1, velocity: d2 },
        0xB0 if bytes.len() >= 3 => MidiMessage::ControlChange { channel, cc: d1, value: d2 },
        0xE0 if bytes.len() >= 3 => MidiMessage::PitchBend { channel, value: (d2 as u16) << 7 | d1 as u16 },
        _ => MidiMessage::Other,
    }
}

pub const CC_SUSTAIN: u8 = 64;
pub const CC_ALL_SOUND_OFF: u8 = 120;
pub const CC_ALL_NOTES_OFF: u8 = 123;

/// Maps a raw message to an instrument event (notes, sustain, all-notes-off).
pub fn to_note_kind(bytes: &[u8]) -> Option<NoteKind> {
    match parse(bytes) {
        MidiMessage::NoteOn { note, velocity, .. } => Some(NoteKind::On { pitch: note, velocity }),
        MidiMessage::NoteOff { note, .. } => Some(NoteKind::Off { pitch: note }),
        MidiMessage::ControlChange { cc: CC_SUSTAIN, value, .. } => Some(NoteKind::Sustain(value >= 64)),
        MidiMessage::ControlChange { cc: CC_ALL_NOTES_OFF | CC_ALL_SOUND_OFF, .. } => Some(NoteKind::AllOff),
        _ => None,
    }
}

/// Note name with C3 = 60 (the convention used by Live's UI).
pub fn note_name(note: u8) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}", NAMES[note as usize % 12], note as i32 / 12 - 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_running_messages() {
        assert_eq!(parse(&[0x91, 60, 100]), MidiMessage::NoteOn { channel: 1, note: 60, velocity: 100 });
        assert_eq!(parse(&[0x90, 60, 0]), MidiMessage::NoteOff { channel: 0, note: 60, velocity: 0 });
        assert_eq!(parse(&[0x80, 61, 10]), MidiMessage::NoteOff { channel: 0, note: 61, velocity: 10 });
        assert_eq!(parse(&[0xE0, 0, 64]), MidiMessage::PitchBend { channel: 0, value: 8192 });
        assert_eq!(parse(&[0x90, 60]), MidiMessage::Other);
        assert_eq!(parse(&[]), MidiMessage::Other);
    }

    #[test]
    fn note_kinds_include_sustain_and_panic() {
        assert_eq!(to_note_kind(&[0xB0, 64, 127]), Some(NoteKind::Sustain(true)));
        assert_eq!(to_note_kind(&[0xB0, 64, 0]), Some(NoteKind::Sustain(false)));
        assert_eq!(to_note_kind(&[0xB0, 123, 0]), Some(NoteKind::AllOff));
        assert_eq!(to_note_kind(&[0xB0, 1, 3]), None);
    }

    #[test]
    fn names_use_c3_middle_c() {
        assert_eq!(note_name(60), "C3");
        assert_eq!(note_name(36), "C1");
        assert_eq!(note_name(0), "C-2");
    }
}
