//! DMC samples for the cover drums: a short kick, a snare and a hat, encoded
//! as 1-bit delta streams. FamiStudio stores the processed DMC bytes.

use std::f64::consts::PI;

const RATE: f64 = 33544.0;

pub fn samples() -> Vec<(&'static str, String)> {
    vec![
        ("Kick", encode(&kick())),
        ("Snare", encode(&noise(0.12, 0.03, 0.9))),
        ("Hat", encode(&noise(0.06, 0.01, 0.7))),
    ]
}

fn kick() -> Vec<f64> {
    let len = (0.14 * RATE) as usize;
    let mut phase = 0.0;
    (0..len)
        .map(|index| {
            let t = index as f64 / RATE;
            let frequency = 160.0 * (-t / 0.05).exp() + 45.0;
            phase += 2.0 * PI * frequency / RATE;
            phase.sin() * (-t / 0.04).exp()
        })
        .collect()
}

fn noise(seconds: f64, decay: f64, amplitude: f64) -> Vec<f64> {
    let len = (seconds * RATE) as usize;
    let mut seed = 7u32;
    (0..len)
        .map(|index| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let t = index as f64 / RATE;
            let value = (seed as f64 / u32::MAX as f64) * 2.0 - 1.0;
            value * (-t / decay).exp() * amplitude
        })
        .collect()
}

pub fn encode(wave: &[f64]) -> String {
    let mut accumulator = 32i32;
    let mut bits = Vec::with_capacity(wave.len());
    for value in wave {
        let target = (value.clamp(-1.0, 1.0) * 35.0 + 64.0) as i32;
        if target >= accumulator {
            bits.push(1u8);
            accumulator = (accumulator + 2).min(127);
        } else {
            bits.push(0u8);
            accumulator = (accumulator - 2).max(0);
        }
    }
    let mut out = String::with_capacity(bits.len() / 4);
    for chunk in bits.chunks(8) {
        let mut byte = 0u8;
        for (index, bit) in chunk.iter().enumerate() {
            byte |= bit << index;
        }
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
#[path = "../tests/chip_dmc.rs"]
mod tests;
