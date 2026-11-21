use super::*;

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

#[test]
fn parses_the_directory_and_replace() {
    let parsed = parse(&args(&["output", "--replace"])).unwrap();
    assert_eq!(parsed.out, Some(PathBuf::from("output")));
    assert!(parsed.replace);
    let default = parse(&[]).unwrap();
    assert!(default.out.is_none());
    assert!(!default.replace);
}

#[test]
fn rejects_unknown_options_and_extra_arguments() {
    assert!(parse(&args(&["--nope"])).is_err());
    assert!(parse(&args(&["a", "b"])).is_err());
}
