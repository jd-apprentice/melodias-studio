//! Minimal MIDI reader for the chip cover: notes in seconds.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use midly::{MetaMessage, MidiMessage, Smf, Timing, TrackEventKind};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Note {
    pub start: f64,
    pub end: f64,
    pub pitch: u8,
}

pub fn read(path: &Path) -> Result<Vec<Note>> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let smf = Smf::parse(&bytes).with_context(|| format!("parsing {}", path.display()))?;
    let ticks_per_beat = match smf.header.timing {
        Timing::Metrical(ticks) => ticks.as_int() as f64,
        Timing::Timecode(_, _) => 480.0,
    };

    let mut notes = Vec::new();
    for track in &smf.tracks {
        let mut tick = 0u64;
        let mut tempo_us = 500_000u32;
        let mut active: HashMap<u8, Vec<f64>> = HashMap::new();
        let mut last_time = 0.0;
        for event in track {
            tick += event.delta.as_int() as u64;
            let seconds = tick as f64 / ticks_per_beat * (tempo_us as f64 / 1_000_000.0);
            last_time = seconds;
            if let TrackEventKind::Meta(MetaMessage::Tempo(tempo)) = event.kind {
                tempo_us = tempo.as_int();
            }
            if let TrackEventKind::Midi { message, .. } = event.kind {
                match message {
                    MidiMessage::NoteOn { key, vel } => {
                        if vel.as_int() > 0 {
                            active.entry(key.as_int()).or_default().push(seconds);
                        } else if let Some(start) =
                            active.get_mut(&key.as_int()).and_then(|stack| stack.pop())
                        {
                            notes.push(Note {
                                start,
                                end: seconds,
                                pitch: key.as_int(),
                            });
                        }
                    }
                    MidiMessage::NoteOff { key, .. } => {
                        if let Some(start) =
                            active.get_mut(&key.as_int()).and_then(|stack| stack.pop())
                        {
                            notes.push(Note {
                                start,
                                end: seconds,
                                pitch: key.as_int(),
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        for (pitch, stack) in active {
            for start in stack {
                notes.push(Note {
                    start,
                    end: last_time.max(start + 0.05),
                    pitch,
                });
            }
        }
    }
    notes.sort_by(|a, b| a.start.total_cmp(&b.start));
    Ok(notes)
}
