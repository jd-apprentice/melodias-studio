use std::fs;

use super::*;

fn scratch(name: &str) -> PathBuf {
    util::temp_dir(name).unwrap()
}

fn request<'a>(hash: &'a str, bytes: &'a [u8], concept: &'a str) -> WriteRequest<'a> {
    WriteRequest {
        concept,
        source: "pixabay",
        hash,
        extension: "jpg",
        bytes,
        entry: Entry::new(concept, "pixabay"),
    }
}

#[test]
fn writes_a_file_and_skips_the_same_hash() {
    let root = scratch("store");
    let mut library = Library::open(root.clone()).unwrap();
    let bytes = b"pretend this is an image";
    let hash = util::sha256_bytes(bytes);
    let first = library.write(request(&hash, bytes, "battle")).unwrap();
    let Stored::Added(path) = first else {
        panic!("the first write should add a file");
    };
    assert!(path.is_file());
    assert!(path.ends_with(format!("pixabay-{}.jpg", &hash[..8])));
    let manifest = fs::read_to_string(root.join("sources.jsonl")).unwrap();
    assert!(manifest.contains(&hash));
    assert!(manifest.contains("battle/pixabay-"));

    let second = library.write(request(&hash, bytes, "battle")).unwrap();
    assert!(matches!(second, Stored::Skipped));
    let lines = fs::read_to_string(root.join("sources.jsonl"))
        .unwrap()
        .lines()
        .count();
    assert_eq!(lines, 1);
    fs::remove_dir_all(root).ok();
}

#[test]
fn entries_omit_empty_fields() {
    let line = serde_json::to_string(&Entry::new("forest", "import")).unwrap();
    assert!(line.contains("\"concept\":\"forest\""));
    assert!(!line.contains("author"));
    assert!(!line.contains("page_url"));
    assert!(!line.contains("fetched_at"));
}

#[test]
fn short_hashes_are_refused() {
    let root = scratch("store-short");
    let mut library = Library::open(root.clone()).unwrap();
    assert!(library.write(request("abc", b"x", "castle")).is_err());
    fs::remove_dir_all(root).ok();
}
