use super::*;

const PAGE: &str = r#"
<div class="field field-name-field-art-preview">
  <a href='https://opengameart.org/sites/default/files/styles/medium/public/castle_part1.png'>
    <img src='https://opengameart.org/sites/default/files/styles/medium/public/castle_part1.png' alt='Preview'>
  </a>
</div>
<div class="field-item even" property="dc:title"><h2>Pixel Art &amp; Castle Tileset</h2></div>
<div class="field field-name-author-submitter">
  <div class="field-label">Author:&nbsp;</div>
  <div class="field-item even"><span class='username'><a href="/users/rubberduck">rubberduck</a></span></div>
</div>
<div class='license-name'>CC0</div>
<a href="https://opengameart.org/sites/default/files/castle_sheet.png">sheet</a>
<a href='https://opengameart.org/sites/default/files/castle_pack.zip'>pack</a>
<img src="https://opengameart.org/sites/default/files/license_images/cc-by.png">
<img src="https://opengameart.org/sites/default/files/styles/icon_48x48/public/pictures/picture-1.png">
<img src="https://opengameart.org/sites/default/files/styles/icon_64x64/public/comment_type_images/oga-icon-comment.png">
<a href="/content/faq">faq</a>
<a href="/content/castle-platformer">castle</a>
<a href="/content/forest-background">forest</a>
"#;

#[test]
fn reads_cc0_and_the_metadata() {
    assert_eq!(licenses_in(PAGE), vec!["CC0".to_string()]);
    assert!(is_cc0(&licenses_in(PAGE)));
    assert_eq!(
        title_in(PAGE).as_deref(),
        Some("Pixel Art & Castle Tileset")
    );
    assert_eq!(author_in(PAGE).as_deref(), Some("rubberduck"));
    assert_eq!(
        preview_in(PAGE).as_deref(),
        Some("https://opengameart.org/sites/default/files/styles/medium/public/castle_part1.png")
    );
}

#[test]
fn lists_content_slugs_in_order() {
    let slugs = slugs_in(PAGE);
    assert_eq!(slugs.first().map(String::as_str), Some("faq"));
    assert!(slugs.contains(&"castle-platformer".to_string()));
    assert!(slugs.contains(&"forest-background".to_string()));
}

#[test]
fn keeps_real_files_and_drops_site_chrome() {
    let files = files_in(PAGE);
    assert!(
        files.contains(&"https://opengameart.org/sites/default/files/castle_sheet.png".to_string())
    );
    assert!(
        files.contains(&"https://opengameart.org/sites/default/files/castle_pack.zip".to_string())
    );
    assert!(!files.iter().any(|url| url.contains("license_images")));
    assert!(!files.iter().any(|url| url.contains("/styles/")));
    assert!(!files.iter().any(|url| url.contains("comment_type")));
}

#[test]
fn maps_a_preview_to_its_original_when_present() {
    let files = vec!["https://opengameart.org/sites/default/files/castle_part1.png".to_string()];
    assert_eq!(
        original_for(
            "https://opengameart.org/sites/default/files/styles/medium/public/castle_part1.png",
            &files
        )
        .as_deref(),
        Some("https://opengameart.org/sites/default/files/castle_part1.png")
    );
    assert_eq!(
        original_for(
            "https://opengameart.org/sites/default/files/styles/medium/public/missing.png",
            &files
        ),
        None
    );
}

#[test]
fn requires_the_query_word_in_the_title_or_slug() {
    assert!(relevant("SunnyLand Forest", "sunnyland-forest", "forest"));
    assert!(relevant("RPG Icons", "rpg-icons", "rpg icons"));
    assert!(!relevant("PixVoxel Wargame", "pixvoxel-wargame", "volcano"));
    assert!(relevant("Anything", "anything", "ab"));
}

#[test]
fn rejects_non_cc0_licenses() {
    assert!(!is_cc0(&["CC-BY 3.0".to_string()]));
    assert!(!is_cc0(&[]));
    assert!(is_cc0(&["CC-BY 3.0".to_string(), "CC0".to_string()]));
    assert!(is_cc0(&["CC0 1.0".to_string()]));
}
