use super::*;

#[test]
fn extensions_come_from_urls_and_paths() {
    assert_eq!(url_extension("https://x.test/a/b/photo.JPG?itok=1"), "jpg");
    assert_eq!(url_extension("https://x.test/a/b/file.png#frag"), "png");
    assert_eq!(url_extension("https://x.test/no-extension"), "");
    assert!(is_image_extension("png"));
    assert!(is_image_extension("jpeg"));
    assert!(!is_image_extension("zip"));
}

#[test]
fn concepts_are_safe_folder_names() {
    assert!(valid_concept("battle"));
    assert!(valid_concept("jrpg-2"));
    assert!(valid_concept("a_b.c"));
    assert!(!valid_concept(""));
    assert!(!valid_concept("."));
    assert!(!valid_concept(".."));
    assert!(!valid_concept("two words"));
    assert!(!valid_concept("slash/inside"));
    assert!(!valid_prefix("space here"));
}

#[test]
fn html_scans_stop_where_asked() {
    let text = "href=\"/content/castle\" and license-name'>CC0</div> and https://x.test/a.png\"";
    let slugs = find_all(text, "href=\"/content/");
    assert_eq!(take_until(slugs[0], &['"']), "castle");
    let licenses = find_all(text, "license-name'>");
    assert_eq!(take_until(licenses[0], &['<']), "CC0");
    let urls = find_all(text, "https://x.test/");
    assert_eq!(take_until(urls[0], &['"']), "a.png");
}

#[test]
fn entities_are_decoded() {
    assert_eq!(decode_entities("Tom &amp; Jerry"), "Tom & Jerry");
    assert_eq!(decode_entities("&quot;hi&quot;"), "\"hi\"");
    assert_eq!(decode_entities(" &nbsp; "), "");
}

#[test]
fn hashing_is_stable() {
    assert_eq!(
        sha256_bytes(b"a"),
        "ca978112ca1bbdcafac231b39a23dc4da786eff8147c4e72b9807785afee48bb"
    );
    assert_eq!(sha256_bytes(b"").len(), 64);
    assert_ne!(sha256_bytes(b"a"), sha256_bytes(b"b"));
}

#[test]
fn temp_dirs_clean_themselves() {
    let path = {
        let scratch = Scratch::new("test").unwrap();
        assert!(scratch.path().is_dir());
        scratch.path().to_path_buf()
    };
    assert!(!path.exists());
}
