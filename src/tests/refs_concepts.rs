use std::collections::HashSet;

use super::*;

#[test]
fn concept_names_are_unique_and_safe() {
    assert!(CONCEPTS.len() >= 15);
    let mut seen = HashSet::new();
    for concept in CONCEPTS {
        assert!(
            crate::refs::util::valid_concept(concept.name),
            "invalid concept name: {}",
            concept.name
        );
        assert!(
            seen.insert(concept.name),
            "duplicate concept: {}",
            concept.name
        );
        assert!(!concept.term.is_empty());
    }
}

#[test]
fn pixabay_gets_a_pixel_art_query_or_an_override() {
    for concept in CONCEPTS {
        let query = pixabay_query(concept);
        assert!(!query.is_empty());
        if concept.pixabay.is_none() {
            assert!(
                query.starts_with("pixel art "),
                "{} should use the pixel art prefix, got {query}",
                concept.name
            );
        }
    }
    let forest = CONCEPTS.iter().find(|item| item.name == "forest").unwrap();
    assert_eq!(pixabay_query(forest), "pixel art forest background");
}

#[test]
fn opengameart_uses_the_term_unless_overridden() {
    let jrpg = CONCEPTS.iter().find(|item| item.name == "jrpg").unwrap();
    assert_eq!(opengameart_query(jrpg), "rpg icons");
    let castle = CONCEPTS.iter().find(|item| item.name == "castle").unwrap();
    assert_eq!(opengameart_query(castle), "castle");
}

#[test]
fn defaults_cover_every_concept() {
    let defaults = concepts_for(pixabay_query);
    assert_eq!(defaults.concepts.len(), CONCEPTS.len());
    for concept in CONCEPTS {
        assert!(defaults.queries.contains_key(concept.name));
    }
}
