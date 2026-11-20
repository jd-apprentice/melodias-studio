use super::*;

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

#[test]
fn parses_fetch_flags_and_defaults() {
    let parsed = parse_fetch(&args(&[
        "--source",
        "opengameart",
        "--concept",
        "castle",
        "--concept",
        "battle",
        "--count",
        "3",
        "--delay",
        "0.5",
        "--size",
        "512",
        "--dry-run",
    ]))
    .unwrap();
    assert_eq!(parsed.source, "opengameart");
    assert_eq!(parsed.concepts, vec!["castle", "battle"]);
    assert_eq!(parsed.count, 3);
    assert_eq!(parsed.size, 512);
    assert!(parsed.dry_run);
    assert_eq!(parsed.delay, Duration::from_millis(500));

    let default = fetch::Options::default();
    assert_eq!(default.source, "pixabay");
    assert_eq!(default.count, 40);
    assert_eq!(default.max_files_per_pack, 60);
    assert!(!default.packs);
    assert!(parse_fetch(&args(&["--packs"])).unwrap().packs);
}

#[test]
fn rejects_bad_fetch_arguments() {
    assert!(parse_fetch(&args(&["--nope"])).is_err());
    assert!(parse_fetch(&args(&["--count"])).is_err());
    assert!(parse_fetch(&args(&["--count", "many"])).is_err());
    assert!(parse_fetch(&args(&["--delay", "-1"])).is_err());
    assert!(parse_fetch(&args(&["stray"])).is_err());
}

#[test]
fn parses_import_sources_in_order() {
    let parsed = parse_import(&args(&[
        "--concept",
        "castle",
        "--license",
        "CC0-1.0",
        "--author",
        "Kenney",
        "one.zip",
        "two/",
    ]))
    .unwrap();
    assert_eq!(parsed.concept, "castle");
    assert_eq!(parsed.license, "CC0-1.0");
    assert_eq!(parsed.author, "Kenney");
    assert_eq!(
        parsed.sources,
        vec![PathBuf::from("one.zip"), PathBuf::from("two/")]
    );
}
