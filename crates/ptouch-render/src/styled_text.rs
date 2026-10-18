// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Portable inline styles. Offsets count Unicode scalar values, not UTF-8 bytes.

use crate::{RenderError, Result};
use serde::{Deserialize, Serialize};
use std::ops::Range;

fn is_false(value: &bool) -> bool {
    !value
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TextStyle {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_name: Option<String>,
    /// Point size; None inherits the text object's base size.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
    #[serde(skip_serializing_if = "is_false")]
    pub bold: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub italic: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub underline: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub strikethrough: bool,
}

impl TextStyle {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }

    pub fn validate(&self) -> Result<()> {
        if self
            .font_name
            .as_ref()
            .is_some_and(|s| s.trim().is_empty() || s.len() > 256)
        {
            return Err(RenderError::Layout(
                "inline font name must contain 1–256 bytes".into(),
            ));
        }
        if self
            .font_size
            .is_some_and(|s| !s.is_finite() || !(0.1..=512.0).contains(&s))
        {
            return Err(RenderError::Layout(
                "inline font size must be between 0.1 and 512 points".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StyleSpan {
    pub start: usize,
    pub end: usize,
    pub style: TextStyle,
}

/// The text remains the single source of content; omitted ranges inherit defaults.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StyledText {
    pub text: String,
    pub spans: Vec<StyleSpan>,
}

#[derive(Debug, Clone, Default)]
pub struct StylePatch {
    pub font_name: Option<Option<String>>,
    pub font_size: Option<Option<f32>>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strikethrough: Option<bool>,
}

impl StylePatch {
    pub fn apply(&self, style: &mut TextStyle) {
        if let Some(value) = &self.font_name {
            style.font_name = value.clone();
        }
        if let Some(value) = self.font_size {
            style.font_size = value;
        }
        if let Some(value) = self.bold {
            style.bold = value;
        }
        if let Some(value) = self.italic {
            style.italic = value;
        }
        if let Some(value) = self.underline {
            style.underline = value;
        }
        if let Some(value) = self.strikethrough {
            style.strikethrough = value;
        }
    }
}

/// First reader that can retain the styles in these ranges.
pub(crate) fn minimum_version(spans: &[StyleSpan]) -> u32 {
    if spans.iter().any(|span| span.style.strikethrough) {
        8
    } else if !spans.is_empty() {
        6
    } else {
        1
    }
}

pub(crate) fn validate_version(spans: &[StyleSpan], version: u32) -> Result<()> {
    let required = minimum_version(spans);
    if version < required {
        return Err(RenderError::Layout(format!(
            "inline text styles require layout version {required}"
        )));
    }
    Ok(())
}

pub fn validate_spans(text: &str, spans: &[StyleSpan]) -> Result<()> {
    if spans.is_empty() {
        return Ok(());
    }
    if text.len() > 1_000_000 || spans.len() > 8192 {
        return Err(RenderError::Layout(
            "styled text exceeds the text or style-count limit".into(),
        ));
    }
    let length = text.chars().count();
    let mut end = 0;
    for span in spans {
        if span.start < end || span.start >= span.end || span.end > length {
            return Err(RenderError::Layout(
                "inline styles must be ordered, non-overlapping character ranges inside the text"
                    .into(),
            ));
        }
        span.style.validate()?;
        end = span.end;
    }
    Ok(())
}

pub fn style_at(spans: &[StyleSpan], index: usize) -> TextStyle {
    spans
        .iter()
        .find(|s| s.start <= index && index < s.end)
        .map_or_else(TextStyle::default, |s| s.style.clone())
}

/// All style regions, including gaps that inherit the object's defaults.
pub fn style_regions(text: &str, spans: &[StyleSpan]) -> Vec<(Range<usize>, TextStyle)> {
    let mut regions = Vec::new();
    let mut cursor = 0;
    for span in spans {
        if cursor < span.start {
            regions.push((cursor..span.start, TextStyle::default()));
        }
        regions.push((span.start..span.end, span.style.clone()));
        cursor = span.end;
    }
    let end = text.chars().count();
    if cursor < end {
        regions.push((cursor..end, TextStyle::default()));
    }
    regions
}

fn push_span(spans: &mut Vec<StyleSpan>, span: StyleSpan) {
    if span.start >= span.end || span.style.is_default() {
        return;
    }
    if let Some(last) = spans.last_mut()
        && last.end == span.start
        && last.style == span.style
    {
        last.end = span.end;
    } else {
        spans.push(span);
    }
}

impl StyledText {
    pub fn len(&self) -> usize {
        self.text.chars().count()
    }
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
    pub fn validate(&self) -> Result<()> {
        validate_spans(&self.text, &self.spans)
    }

    pub fn style_at(&self, index: usize) -> TextStyle {
        style_at(&self.spans, index)
    }

    /// Preserve unaffected attributes while applying only the supplied fields.
    pub fn format(&mut self, range: Range<usize>, patch: &StylePatch) -> bool {
        if range.start >= range.end || range.end > self.len() {
            return false;
        }
        let mut spans = Vec::new();
        for (region, style) in style_regions(&self.text, &self.spans) {
            if region.end <= range.start || range.end <= region.start {
                push_span(
                    &mut spans,
                    StyleSpan {
                        start: region.start,
                        end: region.end,
                        style,
                    },
                );
                continue;
            }
            push_span(
                &mut spans,
                StyleSpan {
                    start: region.start,
                    end: range.start.min(region.end),
                    style: style.clone(),
                },
            );
            let mut changed = style.clone();
            patch.apply(&mut changed);
            push_span(
                &mut spans,
                StyleSpan {
                    start: region.start.max(range.start),
                    end: region.end.min(range.end),
                    style: changed,
                },
            );
            push_span(
                &mut spans,
                StyleSpan {
                    start: region.start.max(range.end),
                    end: region.end,
                    style,
                },
            );
        }
        let changed = spans != self.spans;
        self.spans = spans;
        changed
    }

    /// Edit in character coordinates and preserve style ranges on either side.
    pub fn replace(&mut self, range: Range<usize>, inserted: &str, style: &TextStyle) -> bool {
        if range.start > range.end || range.end > self.len() {
            return false;
        }
        let byte = |at| {
            self.text
                .char_indices()
                .nth(at)
                .map_or(self.text.len(), |p| p.0)
        };
        let bytes = byte(range.start)..byte(range.end);
        let count = inserted.chars().count();
        let shift = |at: usize| at - range.end + range.start + count;
        let mut spans = Vec::new();
        for span in &self.spans {
            if span.start < range.start {
                push_span(
                    &mut spans,
                    StyleSpan {
                        start: span.start,
                        end: span.end.min(range.start),
                        style: span.style.clone(),
                    },
                );
            }
        }
        push_span(
            &mut spans,
            StyleSpan {
                start: range.start,
                end: range.start + count,
                style: style.clone(),
            },
        );
        for span in &self.spans {
            if span.end > range.end {
                push_span(
                    &mut spans,
                    StyleSpan {
                        start: shift(span.start.max(range.end)),
                        end: shift(span.end),
                        style: span.style.clone(),
                    },
                );
            }
        }
        self.text.replace_range(bytes, inserted);
        self.spans = spans;
        true
    }

    /// Keep normalized styles for clipboard selections, starting at zero.
    pub fn fragment(&self, range: Range<usize>) -> Self {
        let text = self
            .text
            .chars()
            .skip(range.start)
            .take(range.end.saturating_sub(range.start))
            .collect();
        let spans = self
            .spans
            .iter()
            .filter_map(|span| {
                let start = span.start.max(range.start);
                let end = span.end.min(range.end);
                (start < end).then(|| StyleSpan {
                    start: start - range.start,
                    end: end - range.start,
                    style: span.style.clone(),
                })
            })
            .collect();
        Self { text, spans }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_edits_keep_styles_and_merge_equal_neighbors() {
        let mut text = StyledText {
            text: "端口🙂AB".into(),
            spans: vec![],
        };
        text.format(
            1..4,
            &StylePatch {
                bold: Some(true),
                ..Default::default()
            },
        );
        let bold = text.style_at(2);
        assert!(text.replace(2..3, "汉字", &bold));
        assert_eq!(text.text, "端口汉字AB");
        assert_eq!((text.spans[0].start, text.spans[0].end), (1, 5));
        text.format(
            2..4,
            &StylePatch {
                font_size: Some(Some(18.0)),
                ..Default::default()
            },
        );
        assert_eq!(text.spans.len(), 3);
        assert!(text.spans[1].style.bold);
        assert_eq!(text.spans[1].style.font_size, Some(18.0));
        assert_eq!(text.fragment(2..4).text, "汉字");
        text.validate().unwrap();
        assert!(!text.replace(9..10, "", &bold));
    }

    #[test]
    fn invalid_and_overlapping_styles_are_rejected() {
        let mut span = StyleSpan {
            start: 0,
            end: 3,
            style: TextStyle::default(),
        };
        assert!(validate_spans("端口", &[span.clone()]).is_err());
        span.end = 2;
        span.style.font_size = Some(f32::NAN);
        assert!(validate_spans("端口", &[span.clone()]).is_err());
        span.style.font_size = Some(12.0);
        assert!(validate_spans("端口", &[span.clone(), span]).is_err());
    }
}
