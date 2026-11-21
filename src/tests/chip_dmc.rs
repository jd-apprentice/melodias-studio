use super::*;

#[test]
fn samples_are_hex_and_deterministic() {
    let first = samples();
    assert_eq!(first.len(), 3);
    for (_, data) in &first {
        assert!(!data.is_empty());
        assert_eq!(data.len() % 2, 0);
        assert!(data.chars().all(|character| character.is_ascii_hexdigit()));
    }
    let second = samples();
    assert_eq!(first[0].1, second[0].1);
    assert_eq!(first[1].1, second[1].1);
}

#[test]
fn a_constant_wave_encodes_one_bit_per_sample() {
    let data = encode(&[0.0; 128]);
    assert_eq!(data.len(), 32);
}
