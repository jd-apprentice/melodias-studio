use super::*;

#[test]
fn parses_the_particle_option() {
    assert_eq!(particles_value("both").unwrap(), "both");
    assert_eq!(particles_value("BOKEH").unwrap(), "bokeh");
    assert_eq!(particles_value(" sparks ").unwrap(), "sparks");
    assert_eq!(particles_value("").unwrap(), "both");
    assert!(particles_value("neon").is_err());
}

#[test]
fn requires_the_overlays_of_the_selected_particles() {
    assert!(required_overlays("bokeh").contains(&"particles-bokeh.mp4"));
    assert!(!required_overlays("bokeh").contains(&"particles-sparks.mp4"));
    assert!(required_overlays("sparks").contains(&"particles-sparks.mp4"));
    assert!(!required_overlays("sparks").contains(&"particles-bokeh.mp4"));
    assert_eq!(required_overlays("both").len(), 3);
}

#[test]
fn resolves_a_relative_render_output_against_the_caller() {
    let abs = std::path::Path::new("/tmp/run/work/video-remotion.mp4");
    assert_eq!(absolute(abs).unwrap(), abs);
    let relative = std::path::Path::new("output/anime/work/video-remotion.mp4");
    let resolved = absolute(relative).unwrap();
    assert!(resolved.is_absolute());
    assert!(resolved.ends_with(relative));
}
