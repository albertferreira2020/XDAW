//! Standard MIDI File import/export for clip notes (beats = quarter notes).

use midly::num::{u4, u7, u15, u28};
use midly::{Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FileNote {
    pub pitch: u8,
    pub start: f64,
    pub length: f64,
    pub velocity: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportedMidi {
    pub notes: Vec<FileNote>,
    /// First tempo found (BPM), if any.
    pub bpm: Option<f64>,
    /// Length rounded up to whole bars of 4 beats.
    pub length_beats: f64,
}

pub fn import(bytes: &[u8]) -> Result<ImportedMidi, String> {
    let smf = Smf::parse(bytes).map_err(|e| e.to_string())?;
    let ppq = match smf.header.timing {
        Timing::Metrical(t) => t.as_int() as f64,
        Timing::Timecode(..) => return Err("SMPTE-timed MIDI files are not supported".into()),
    };
    let mut notes = vec![];
    let mut bpm = None;
    for track in &smf.tracks {
        let mut tick: u64 = 0;
        let mut open: [Option<(u64, u8)>; 128] = [None; 128];
        for ev in track {
            tick += ev.delta.as_int() as u64;
            match ev.kind {
                TrackEventKind::Meta(MetaMessage::Tempo(us)) if bpm.is_none() => {
                    bpm = Some(60_000_000.0 / us.as_int() as f64);
                }
                TrackEventKind::Midi { message, .. } => {
                    let (key, on_vel) = match message {
                        MidiMessage::NoteOn { key, vel } => (key.as_int(), vel.as_int()),
                        MidiMessage::NoteOff { key, .. } => (key.as_int(), 0),
                        _ => continue,
                    };
                    if let Some((start, vel)) = open[key as usize].take() {
                        notes.push(FileNote {
                            pitch: key,
                            start: start as f64 / ppq,
                            length: ((tick - start) as f64 / ppq).max(1.0 / 64.0),
                            velocity: vel,
                        });
                    }
                    if on_vel > 0 {
                        open[key as usize] = Some((tick, on_vel));
                    }
                }
                _ => {}
            }
        }
        for (key, o) in open.iter().enumerate() {
            if let Some((start, vel)) = o {
                notes.push(FileNote { pitch: key as u8, start: *start as f64 / ppq, length: ((tick - start) as f64 / ppq).max(0.25), velocity: *vel });
            }
        }
    }
    notes.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
    let end = notes.iter().map(|n| n.start + n.length).fold(0.0, f64::max);
    Ok(ImportedMidi { notes, bpm, length_beats: ((end / 4.0).ceil() * 4.0).max(4.0) })
}

pub const EXPORT_PPQ: u16 = 480;

pub fn export(notes: &[FileNote], bpm: f64) -> Vec<u8> {
    let to_tick = |b: f64| (b * EXPORT_PPQ as f64).round().max(0.0) as u64;
    let mut timed: Vec<(u64, bool, u8, u8)> = vec![];
    for n in notes {
        let s = to_tick(n.start);
        let e = to_tick(n.start + n.length).max(s + 1);
        timed.push((s, true, n.pitch.min(127), n.velocity.clamp(1, 127)));
        timed.push((e, false, n.pitch.min(127), 0));
    }
    // Offs before ons at the same tick.
    timed.sort_by_key(|&(t, on, p, _)| (t, on, p));
    let mut track: Vec<TrackEvent> = vec![TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::Tempo(midly::num::u24::new((60_000_000.0 / bpm.max(1.0)) as u32))),
    }];
    let mut last = 0;
    for (t, on, p, v) in timed {
        let message = if on { MidiMessage::NoteOn { key: u7::new(p), vel: u7::new(v) } } else { MidiMessage::NoteOff { key: u7::new(p), vel: u7::new(0) } };
        track.push(TrackEvent { delta: u28::new((t - last) as u32), kind: TrackEventKind::Midi { channel: u4::new(0), message } });
        last = t;
    }
    track.push(TrackEvent { delta: u28::new(0), kind: TrackEventKind::Meta(MetaMessage::EndOfTrack) });
    let smf = Smf { header: Header::new(Format::SingleTrack, Timing::Metrical(u15::new(EXPORT_PPQ))), tracks: vec![track] };
    let mut out = vec![];
    smf.write_std(&mut out).expect("writing to a Vec cannot fail");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_notes_and_tempo() {
        let notes = vec![
            FileNote { pitch: 60, start: 0.0, length: 1.0, velocity: 100 },
            FileNote { pitch: 64, start: 0.5, length: 0.25, velocity: 80 },
            FileNote { pitch: 60, start: 1.0, length: 2.0, velocity: 90 },
        ];
        let bytes = export(&notes, 128.0);
        let back = import(&bytes).unwrap();
        assert_eq!(back.notes, notes);
        assert!((back.bpm.unwrap() - 128.0).abs() < 0.01);
        assert_eq!(back.length_beats, 4.0);
    }

    #[test]
    fn rejects_garbage() {
        assert!(import(b"not midi").is_err());
    }
}
