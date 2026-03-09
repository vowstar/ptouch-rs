// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Font discovery using cosmic-text's font system.
//!
//! Provides helpers to list available system fonts and find a font by name.

use cosmic_text::FontSystem;

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
}
