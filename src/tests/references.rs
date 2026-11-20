use std::fs;
use std::path::PathBuf;

use super::*;

fn temp_root(name: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("melodias-references-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    root
}

fn cleanup(root: PathBuf) {
    let _ = fs::remove_dir_all(root);
}

#[test]
fn selects_the_matching_concept_and_filters_images() {
    let root = temp_root("concept");
    for dir in ["battle", "jrpg-battle", "forest"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    fs::write(root.join("battle/a.png"), b"x").unwrap();
    fs::write(root.join("battle/notes.txt"), b"x").unwrap();
    fs::write(root.join("jrpg-battle/c.jpg"), b"x").unwrap();
    fs::write(root.join("forest/d.webp"), b"x").unwrap();
    fs::write(root.join("e.png"), b"x").unwrap();

    assert_eq!(
        images(&root, "8bit jrpg battle theme", "").unwrap(),
        vec![root.join("jrpg-battle/c.jpg")]
    );
    assert_eq!(
        images(&root, "epic castle", "").unwrap(),
        vec![root.join("e.png")]
    );
    assert_eq!(
        images(&root, "8bit battle", "forest").unwrap(),
        vec![root.join("forest/d.webp")]
    );
    assert_eq!(
        images(&root, "8bit battle", "missing").unwrap(),
        vec![root.join("battle/a.png")]
    );

    cleanup(root);
}

#[test]
fn rotates_subsets_and_wraps_around() {
    let files = vec![
        PathBuf::from("a.png"),
        PathBuf::from("b.png"),
        PathBuf::from("c.png"),
    ];
    assert_eq!(pick(&files, 0, 2), vec![files[0].clone(), files[1].clone()]);
    assert_eq!(pick(&files, 2, 2), vec![files[2].clone(), files[0].clone()]);
    assert!(pick(&files, 0, 0).is_empty());
    assert!(pick(&[], 0, 3).is_empty());
}

#[test]
fn a_missing_folder_is_an_error() {
    let root = temp_root("missing");
    assert!(images(&root, "anything", "").is_err());
    assert!(images(&PathBuf::new(), "anything", "").unwrap().is_empty());
}
