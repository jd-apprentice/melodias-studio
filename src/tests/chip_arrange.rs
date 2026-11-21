use super::*;

fn note(start: f64, end: f64, pitch: u8) -> midi::Note {
    midi::Note { start, end, pitch }
}

#[test]
fn note_names_use_famistudio_octaves() {
    assert_eq!(note_name(60), "C3");
    assert_eq!(note_name(61), "C#3");
    assert_eq!(note_name(72), "C4");
}

#[test]
fn textures_split_by_energy_tertiles() {
    let kinds = textures(&[0.1, 0.5, 0.9]);
    assert_eq!(kinds.len(), 3);
    assert!(kinds[0] == Texture::Sparse);
    assert!(kinds[1] == Texture::Normal);
    assert!(kinds[2] == Texture::Full);
}

#[test]
fn builds_a_project_with_three_channels_and_dpcm() {
    let melody = vec![
        note(0.0, 0.5, 60),
        note(0.0, 0.5, 64),
        note(0.0, 0.5, 67),
        note(1.0, 1.5, 62),
    ];
    let bass = vec![note(0.0, 0.5, 36)];
    let hits = vec![(0.0, 36), (0.5, 38)];
    let energies = [0.9];
    let (text, summary) = compose(Data {
        melody: &melody,
        bass: &bass,
        hits: &hits,
        tempo: 120.0,
        energies: &energies,
        title: "Test",
    });
    assert!(text.contains("Channel Type=\"Square1\""));
    assert!(text.contains("Channel Type=\"Triangle\""));
    assert!(text.contains("Channel Type=\"DPCM\""));
    assert!(text.contains("PatternInstance"));
    assert!(text.contains("DPCMSample"));
    assert!(summary.patterns >= 1);
    assert_eq!(summary.dpcm, 2);
    assert!(summary.lead >= 1);
}
