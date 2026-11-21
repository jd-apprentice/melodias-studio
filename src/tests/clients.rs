use super::*;

#[test]
fn glowing_eyes_are_sanitized() {
    let prompt = sanitize_eye_prompt("a girl with glowing red eyes in a forest");
    assert_eq!(prompt, "a girl with soft green eyes in a forest");
}

#[test]
fn image_extra_args_carry_seed_and_steps() {
    let prompt = with_extra_args("pixel art", 42, 12, "glowing eyes");
    assert!(prompt.starts_with("pixel art <sd_cpp_extra_args>"));
    assert!(prompt.contains("\"seed\":42"));
    assert!(prompt.contains("\"sample_steps\":12"));
    assert!(prompt.contains("\"negative_prompt\":\"glowing eyes\""));
    assert!(prompt.ends_with("</sd_cpp_extra_args>"));
}
