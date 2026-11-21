use super::*;

#[test]
fn a_two_hz_click_track_reads_as_120_bpm() {
    let rate = 22050;
    let mut samples = vec![0.0f32; rate as usize * 10];
    for (index, sample) in samples.iter_mut().enumerate() {
        let seconds = index as f64 / rate as f64;
        if (seconds * 2.0).fract() < 0.02 {
            *sample = 1.0;
        }
    }
    let bpm = tempo(&samples, rate);
    assert!((bpm - 120.0).abs() < 12.0, "expected about 120, got {bpm}");
}

#[test]
fn section_energy_follows_the_signal() {
    let rate = 1000;
    let mut samples = vec![0.1f32; 4000];
    for sample in samples.iter_mut().skip(2000) {
        *sample = 0.8;
    }
    let energies = section_energies(&samples, rate, 2, 2.0);
    assert_eq!(energies.len(), 2);
    assert!(energies[0] < energies[1]);
}
