use super::*;

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

#[test]
fn parses_the_input_and_options() {
    let parsed = parse(&args(&[
        "https://www.youtube.com/watch?v=test",
        "--title",
        "Battle",
        "--out",
        "/tmp/chip",
        "--keep-work",
    ]))
    .unwrap();
    assert_eq!(parsed.input, "https://www.youtube.com/watch?v=test");
    assert_eq!(parsed.title.as_deref(), Some("Battle"));
    assert_eq!(parsed.out, Some(PathBuf::from("/tmp/chip")));
    assert!(parsed.keep_work);
}

#[test]
fn a_local_file_needs_no_title() {
    let parsed = parse(&args(&["song.wav"])).unwrap();
    assert_eq!(parsed.input, "song.wav");
    assert!(parsed.title.is_none());
    assert!(!parsed.keep_work);
}

#[test]
fn rejects_missing_or_bad_arguments() {
    assert!(parse(&[]).is_err());
    assert!(parse(&args(&["a.wav", "--nope"])).is_err());
    assert!(parse(&args(&["a.wav", "b.wav"])).is_err());
    assert!(parse(&args(&["--title"])).is_err());
}
