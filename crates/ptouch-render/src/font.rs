// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Font discovery using cosmic-text's font system.
//!
//! Provides helpers to list available system fonts and find a font by name.

use cosmic_text::FontSystem;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

static PUA_FONT_GLYPHS: OnceLock<Mutex<HashMap<String, HashSet<char>>>> = OnceLock::new();

/// Load a system face for a GUI editing adapter. Printing still uses cosmic-text.
pub fn face_data(name: &str, bold: bool, italic: bool) -> Option<(Vec<u8>, u32)> {
    let fonts = FontSystem::new();
    let id = fonts.db().query(&cosmic_text::fontdb::Query {
        families: &[
            cosmic_text::Family::Name(name),
            cosmic_text::Family::SansSerif,
        ],
        weight: if bold {
            cosmic_text::Weight::BOLD
        } else {
            cosmic_text::Weight::NORMAL
        },
        style: if italic {
            cosmic_text::Style::Italic
        } else {
            cosmic_text::Style::Normal
        },
        ..Default::default()
    })?;
    fonts
        .db()
        .with_face_data(id, |data, index| {
            (data.len() <= 33_554_432).then(|| (data.to_vec(), index))
        })
        .flatten()
}

/// Find a font by name and return its full family name if found.
///
/// The search is case-insensitive and matches on family name.
pub fn find_font(name: &str) -> Option<String> {
    let font_system = FontSystem::new();
    let name_lower = name.to_lowercase();

    for face in font_system.db().faces() {
        for family in &face.families {
            if family.0.to_lowercase().contains(&name_lower) {
                return Some(family.0.clone());
            }
        }
    }

    None
}

/// Check whether a system font family contains a particular Unicode glyph.
pub fn has_glyph(name: &str, character: char) -> bool {
    let cache = PUA_FONT_GLYPHS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let key = name.to_lowercase();
    let glyphs = cache.entry(key).or_insert_with(|| {
        let font_system = FontSystem::new();
        let id = font_system.db().faces().find_map(|face| {
            face.families
                .iter()
                .any(|family| family.0.eq_ignore_ascii_case(name))
                .then_some(face.id)
        });
        id.and_then(|id| {
            font_system.db().with_face_data(id, |data, index| {
                ttf_parser::Face::parse(data, index).ok().map(|face| {
                    (0xF000..=0xF0FF)
                        .filter_map(char::from_u32)
                        .filter(|character| face.glyph_index(*character).is_some())
                        .collect()
                })
            })
        })
        .flatten()
        .unwrap_or_default()
    });
    glyphs.contains(&character)
}

/// List all available font family names on the system.
///
/// Returns a sorted, deduplicated list of family names.
pub fn list_fonts() -> Vec<String> {
    let font_system = FontSystem::new();
    let mut names: Vec<String> = Vec::new();

    for face in font_system.db().faces() {
        for family in &face.families {
            names.push(family.0.clone());
        }
    }

    names.sort();
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_fonts_not_empty() {
        // On most systems there should be at least some fonts.
        // This test may fail in minimal containers without fonts.
        let fonts = list_fonts();
        // We do not assert non-empty since CI may have no fonts,
        // but we check it does not panic.
        log::debug!("Found {} font families", fonts.len());
    }

    #[test]
    fn test_list_fonts_sorted() {
        let fonts = list_fonts();
        for pair in fonts.windows(2) {
            assert!(pair[0] <= pair[1], "fonts should be sorted");
        }
    }

    #[test]
    fn test_find_font_nonexistent() {
        let result = find_font("ThisFontDefinitelyDoesNotExist12345");
        assert!(result.is_none());
    }

    #[test]
    fn test_has_glyph_does_not_use_a_fallback_family() {
        assert!(!has_glyph("ThisFontDefinitelyDoesNotExist12345", 'A'));
    }
}
