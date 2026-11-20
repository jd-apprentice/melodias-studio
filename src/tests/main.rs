use super::*;

#[test]
fn writes_a_clean_log() {
    let path = std::env::temp_dir().join(format!("melodias-studio-{}.log", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut out = Reporter::new(Some(path.clone())).unwrap();
    out.stage(0, State::Running);
    out.stage(0, State::Ok);
    out.log_line("image prompt: pixel art castle");

    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("▶ 1/11 Image prompt"));
    assert!(text.contains("✔ 1/11 Image prompt"));
    assert!(text.contains("   · image prompt: pixel art castle"));
    let _ = std::fs::remove_file(path);
}
