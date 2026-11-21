//! Arrangement for the chip cover: turns transcription notes and drum hits
//! into a FamiStudio text project with section textures, arpeggios, portamento
//! and DPCM drums.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result};

use super::{audio, dmc, midi};

pub const FRAME_RATE: f64 = 60.0988;
const BEAT_LENGTH: i64 = 4;
const PATTERN_ROWS: i64 = 64;
const SECTION_ROWS: i64 = 128;
const NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

pub struct Request<'a> {
    pub melody: &'a [midi::Note],
    pub bass: &'a [midi::Note],
    pub source: &'a Path,
    pub drums_stem: &'a Path,
    pub title: &'a str,
    pub out: &'a Path,
}

pub struct Summary {
    pub patterns: usize,
    pub lead: usize,
    pub harmony: usize,
    pub bass: usize,
    pub dpcm: usize,
    pub noise: usize,
    pub tempo: f64,
    pub note_length: i64,
    pub textures: [usize; 3],
}

struct Data<'a> {
    melody: &'a [midi::Note],
    bass: &'a [midi::Note],
    hits: &'a [(f64, u8)],
    tempo: f64,
    energies: &'a [f64],
    title: &'a str,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Texture {
    Sparse,
    Normal,
    Full,
}

struct Event {
    row: i64,
    value: String,
    duration: i64,
    instrument: &'static str,
    volume: Option<u8>,
    vibrato: bool,
    slide: Option<String>,
}

pub fn build(request: Request<'_>) -> Result<Summary> {
    let source = audio::decode_mono(request.source, 22050)?;
    let tempo = audio::tempo(&source, 22050);
    let note_length = note_length_for(tempo);

    let drum_audio = audio::decode_mono(request.drums_stem, 22050)?;
    let hits = audio::drums(&drum_audio, 22050);

    let total_rows = total_rows(request.melody, request.bass, &hits, note_length);
    let sections = ((total_rows + SECTION_ROWS - 1) / SECTION_ROWS).max(1) as usize;
    let seconds_per_section = SECTION_ROWS as f64 * note_length as f64 / FRAME_RATE;
    let energies = audio::section_energies(&source, 22050, sections, seconds_per_section);

    let (text, summary) = compose(Data {
        melody: request.melody,
        bass: request.bass,
        hits: &hits,
        tempo,
        energies: &energies,
        title: request.title,
    });
    std::fs::write(request.out, text)
        .with_context(|| format!("writing {}", request.out.display()))?;
    Ok(summary)
}

fn compose(data: Data<'_>) -> (String, Summary) {
    let note_length = note_length_for(data.tempo);
    let melody = merge(data.melody);
    let bass = merge(data.bass);
    let melody_rows = rows(&melody, note_length);
    let bass_rows = rows(&bass, note_length);

    let mut max_row = 0;
    for (row, candidates) in &melody_rows {
        max_row = max_row.max(*row);
        for (_, end) in candidates {
            max_row = max_row.max(*end);
        }
    }
    for (row, candidates) in &bass_rows {
        max_row = max_row.max(*row);
        for (_, end) in candidates {
            max_row = max_row.max(*end);
        }
    }
    for (time, _) in data.hits {
        max_row = max_row.max(row_of(*time, note_length));
    }
    let total = max_row + 1;
    let pattern_count = (((total + PATTERN_ROWS - 1) / PATTERN_ROWS).max(1)) as usize;
    let textures = textures(data.energies);

    let mut square1 = Vec::new();
    let mut square2 = Vec::new();
    for (row, candidates) in &melody_rows {
        let mut candidates = candidates.clone();
        candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.0));
        let kind = texture(*row, &textures);
        let volume = match kind {
            Texture::Sparse => 10,
            Texture::Normal => 12,
            Texture::Full => 14,
        };
        let (pitch, end) = candidates[0];
        square1.push(Event {
            row: *row,
            value: note_name(pitch),
            duration: (end - *row).max(1) * note_length,
            instrument: "Lead",
            volume: Some(volume),
            vibrato: kind == Texture::Full && end - *row >= 3,
            slide: None,
        });
        if kind != Texture::Sparse && candidates.len() > 1 {
            let (pitch, end) = candidates[1];
            square2.push(Event {
                row: *row,
                value: note_name(pitch),
                duration: (end - *row).max(1) * note_length,
                instrument: "Harmony",
                volume: Some(if kind == Texture::Full { 9 } else { 8 }),
                vibrato: false,
                slide: None,
            });
        }
        if kind == Texture::Full && candidates.len() > 2 {
            let end_row = (candidates[0].1 - *row).max(2);
            let mut cursor = *row + 1;
            let mut extra = 2;
            while cursor < *row + end_row.min(8) {
                let pitch = candidates[extra % candidates.len()].0;
                square2.push(Event {
                    row: cursor,
                    value: note_name(pitch),
                    duration: note_length,
                    instrument: "Harmony",
                    volume: Some(7),
                    vibrato: false,
                    slide: None,
                });
                cursor += 1;
                extra += 1;
            }
        }
    }

    let mut bass_events = Vec::new();
    let mut bass_pitches = Vec::new();
    for (row, candidates) in &bass_rows {
        let mut candidates = candidates.clone();
        candidates.sort();
        let (pitch, end) = candidates[0];
        bass_pitches.push(pitch);
        bass_events.push(Event {
            row: *row,
            value: note_name(pitch),
            duration: (end - *row).max(1) * note_length,
            instrument: "Bass",
            volume: Some(match texture(*row, &textures) {
                Texture::Sparse => 12,
                Texture::Normal => 14,
                Texture::Full => 15,
            }),
            vibrato: false,
            slide: None,
        });
    }
    for index in 0..bass_events.len().saturating_sub(1) {
        let a = bass_pitches[index] as i32;
        let b = bass_pitches[index + 1] as i32;
        if (a - b).abs() <= 2 && (a - b) != 0 && bass_events[index].duration >= note_length * 2 {
            bass_events[index].slide = Some(bass_events[index + 1].value.clone());
        }
    }

    let mut dpcm = Vec::new();
    let mut noise = Vec::new();
    let mut seen = BTreeSet::new();
    for (time, pitch) in data.hits {
        let row = row_of(*time, note_length);
        if !seen.insert(row) {
            continue;
        }
        let kind = texture(row, &textures);
        if kind == Texture::Sparse {
            continue;
        }
        match *pitch {
            36 => dpcm.push(Event {
                row,
                value: "C4".to_string(),
                duration: note_length,
                instrument: "Kick",
                volume: None,
                vibrato: false,
                slide: None,
            }),
            38 => dpcm.push(Event {
                row,
                value: "C4".to_string(),
                duration: note_length,
                instrument: "Snare",
                volume: None,
                vibrato: false,
                slide: None,
            }),
            _ if kind == Texture::Full => noise.push(Event {
                row,
                value: "A#3".to_string(),
                duration: note_length,
                instrument: "Hat",
                volume: None,
                vibrato: false,
                slide: None,
            }),
            _ => {}
        }
    }

    let mut counts = [0usize; 3];
    for kind in &textures {
        counts[match kind {
            Texture::Sparse => 0,
            Texture::Normal => 1,
            Texture::Full => 2,
        }] += 1;
    }

    let mut lines = vec![format!(
        "Project Version=\"4.5.4\" TempoMode=\"FamiStudio\" Name=\"{}\" Author=\"melodias\"",
        escape(data.title)
    )];
    for (name, data) in dmc::samples() {
        lines.push(format!(
            "\tDPCMSample Name=\"{name}\" DmcInitialValue=\"16\" Data=\"{data}\""
        ));
    }
    for (name, envelopes, dpcm_name) in [
        (
            "Lead",
            vec![("Volume", 6, "12,12,11,10,9,8"), ("DutyCycle", 1, "1")],
            None,
        ),
        (
            "Harmony",
            vec![("Volume", 5, "8,7,6,5,4"), ("DutyCycle", 1, "2")],
            None,
        ),
        ("Bass", vec![("Volume", 1, "15")], None),
        ("Hat", vec![("Volume", 6, "9,6,4,2,1,1")], None),
        ("Kick", vec![], Some("Kick")),
        ("Snare", vec![], Some("Snare")),
    ] {
        lines.push(format!("\tInstrument Name=\"{name}\""));
        for (kind, length, values) in envelopes {
            lines.push(format!(
                "\t\tEnvelope Type=\"{kind}\" Length=\"{length}\" Values=\"{values}\""
            ));
        }
        if let Some(sample) = dpcm_name {
            lines.push(format!(
                "\t\tDPCMMapping Note=\"C4\" Sample=\"{sample}\" Pitch=\"15\" Loop=\"False\""
            ));
        }
    }
    lines.push(format!(
        "\tSong Name=\"{}\" Length=\"{pattern_count}\" LoopPoint=\"0\" PatternLength=\"{PATTERN_ROWS}\" \
         BeatLength=\"{BEAT_LENGTH}\" NoteLength=\"{note_length}\" Groove=\"{note_length}\" \
         GroovePaddingMode=\"Middle\"",
        escape(data.title)
    ));

    for (channel, events) in [
        ("Square1", &square1),
        ("Square2", &square2),
        ("Triangle", &bass_events),
        ("Noise", &noise),
        ("DPCM", &dpcm),
    ] {
        lines.push(format!("\t\tChannel Type=\"{channel}\""));
        let split = split_patterns(events, pattern_count);
        for (index, notes) in split.iter().enumerate() {
            lines.push(format!("\t\t\tPattern Name=\"{channel} P{index}\""));
            for event in notes {
                let mut extras = String::new();
                if let Some(volume) = event.volume {
                    extras.push_str(&format!(" Volume=\"{volume}\""));
                }
                if event.vibrato {
                    extras.push_str(" VibratoSpeed=\"3\" VibratoDepth=\"2\"");
                }
                if let Some(slide) = &event.slide {
                    extras.push_str(&format!(" SlideTarget=\"{slide}\""));
                }
                lines.push(format!(
                    "\t\t\t\tNote Time=\"{}\" Value=\"{}\" Duration=\"{}\" Instrument=\"{}\"{}",
                    (event.row % PATTERN_ROWS) * note_length,
                    event.value,
                    event.duration,
                    event.instrument,
                    extras
                ));
            }
            lines.push(format!(
                "\t\t\tPatternInstance Time=\"{index}\" Pattern=\"{channel} P{index}\""
            ));
        }
    }

    let summary = Summary {
        patterns: pattern_count,
        lead: square1.len(),
        harmony: square2.len(),
        bass: bass_events.len(),
        dpcm: dpcm.len(),
        noise: noise.len(),
        tempo: data.tempo,
        note_length,
        textures: counts,
    };
    (lines.join("\n") + "\n", summary)
}

fn merge(notes: &[midi::Note]) -> Vec<midi::Note> {
    let mut sorted = notes.to_vec();
    sorted.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut out: Vec<midi::Note> = Vec::new();
    for note in sorted {
        if let Some(last) = out.last_mut()
            && last.pitch == note.pitch
            && note.start <= last.end + 0.12
        {
            last.end = last.end.max(note.end);
            continue;
        }
        out.push(note);
    }
    out
}

fn rows(notes: &[midi::Note], note_length: i64) -> BTreeMap<i64, Vec<(u8, i64)>> {
    let mut out: BTreeMap<i64, Vec<(u8, i64)>> = BTreeMap::new();
    for note in notes {
        let row = row_of(note.start, note_length);
        let end = row_of(note.end, note_length).max(row + 1);
        out.entry(row).or_default().push((note.pitch, end));
    }
    out
}

fn total_rows(
    melody: &[midi::Note],
    bass: &[midi::Note],
    hits: &[(f64, u8)],
    note_length: i64,
) -> i64 {
    let mut max = 0;
    for note in melody.iter().chain(bass) {
        max = max.max(row_of(note.start, note_length));
        max = max.max(row_of(note.end, note_length));
    }
    for (time, _) in hits {
        max = max.max(row_of(*time, note_length));
    }
    max + 1
}

fn note_length_for(tempo: f64) -> i64 {
    (60.0 * FRAME_RATE / (tempo * BEAT_LENGTH as f64))
        .round()
        .clamp(3.0, 15.0) as i64
}

fn row_of(seconds: f64, note_length: i64) -> i64 {
    ((seconds * FRAME_RATE / note_length as f64).round() as i64).max(0)
}

fn note_name(pitch: u8) -> String {
    format!("{}{}", NAMES[pitch as usize % 12], pitch as i32 / 12 - 2)
}

fn textures(energies: &[f64]) -> Vec<Texture> {
    let mut sorted = energies.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    if sorted.is_empty() {
        return Vec::new();
    }
    let low = sorted[sorted.len() / 3];
    let high = sorted[(sorted.len() * 2) / 3];
    energies
        .iter()
        .map(|value| {
            if *value < low {
                Texture::Sparse
            } else if *value < high {
                Texture::Normal
            } else {
                Texture::Full
            }
        })
        .collect()
}

fn texture(row: i64, textures: &[Texture]) -> Texture {
    let section = (row / SECTION_ROWS) as usize;
    textures.get(section).copied().unwrap_or(Texture::Normal)
}

fn split_patterns(events: &[Event], pattern_count: usize) -> Vec<Vec<&Event>> {
    let mut patterns: Vec<Vec<&Event>> = vec![Vec::new(); pattern_count];
    for event in events {
        let slot = (event.row / PATTERN_ROWS) as usize;
        if slot < pattern_count {
            patterns[slot].push(event);
        }
    }
    for pattern in &mut patterns {
        pattern.sort_by_key(|event| event.row);
    }
    patterns
}

fn escape(value: &str) -> String {
    value.replace('"', "\"\"")
}

#[cfg(test)]
#[path = "../tests/chip_arrange.rs"]
mod tests;
