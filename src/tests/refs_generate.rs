use super::*;

#[test]
fn prompts_use_the_concept_scene_and_style() {
    let prompt = build_prompt("volcano", "8-bit pixel art");
    assert!(prompt.contains("volcano"));
    assert!(prompt.contains("lava"));
    assert!(prompt.contains("8-bit pixel art"));
    assert!(prompt.ends_with("no watermark"));
}

#[test]
fn unknown_concepts_get_a_generic_scene() {
    assert_eq!(scene("lighthouse"), "a lighthouse scene");
}

#[test]
fn seeds_are_nonzero_and_shifted_by_index() {
    assert!(random_seed(0) >= 1);
    assert_ne!(random_seed(0), random_seed(1));
}
