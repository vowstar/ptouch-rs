use qrcodegen::{QrSegment, QrSegmentMode, Version};

#[derive(Clone, Copy)]
enum SegmentKind {
    Numeric,
    Alphanumeric,
    Byte,
}

#[derive(Clone, Copy)]
struct Parent {
    start: usize,
    kind: SegmentKind,
}

/// Split text into the shortest numeric, alphanumeric, and byte segments for
/// one QR version group. Character-count widths are constant inside each group.
pub(crate) fn make_segments(text: &str, version: Version) -> Vec<QrSegment> {
    if text.is_empty() {
        return Vec::new();
    }
    if QrSegment::is_numeric(text) {
        return vec![QrSegment::make_numeric(text)];
    }
    if QrSegment::is_alphanumeric(text) && !text.bytes().any(|byte| byte.is_ascii_digit()) {
        return vec![QrSegment::make_alphanumeric(text)];
    }
    if !text
        .chars()
        .any(|character| character.is_ascii_digit() || is_alphanumeric(character))
    {
        return vec![QrSegment::make_bytes(text.as_bytes())];
    }

    let chars: Vec<char> = text.chars().collect();
    let offsets: Vec<usize> = text
        .char_indices()
        .map(|(offset, _)| offset)
        .chain(std::iter::once(text.len()))
        .collect();
    let count_bits = |kind| match kind {
        SegmentKind::Numeric => count_width(QrSegmentMode::Numeric, version),
        SegmentKind::Alphanumeric => count_width(QrSegmentMode::Alphanumeric, version),
        SegmentKind::Byte => count_width(QrSegmentMode::Byte, version),
    };
    let mut costs = vec![None; chars.len() + 1];
    let mut parents = vec![None; chars.len() + 1];
    costs[0] = Some(0usize);

    for start in 0..chars.len() {
        let Some(base_cost) = costs[start] else {
            continue;
        };
        for kind in [
            SegmentKind::Numeric,
            SegmentKind::Alphanumeric,
            SegmentKind::Byte,
        ] {
            let width = usize::from(count_bits(kind));
            let max_count = (1usize << width) - 1;
            let mut data_bits = 0usize;
            let mut char_count = 0usize;
            let mut byte_count = 0usize;
            for end in (start + 1)..=chars.len() {
                let character = chars[end - 1];
                let increment = match kind {
                    SegmentKind::Numeric if character.is_ascii_digit() => {
                        char_count += 1;
                        match char_count % 3 {
                            1 => 4,
                            2 => 3,
                            _ => 3,
                        }
                    }
                    SegmentKind::Alphanumeric if is_alphanumeric(character) => {
                        char_count += 1;
                        if char_count % 2 == 1 { 6 } else { 5 }
                    }
                    SegmentKind::Byte => {
                        byte_count += character.len_utf8();
                        char_count = byte_count;
                        8 * character.len_utf8()
                    }
                    _ => break,
                };
                if char_count > max_count {
                    break;
                }
                data_bits += increment;
                let segment_bits = 4 + width + data_bits;
                let candidate = base_cost + segment_bits;
                let destination = &mut costs[end];
                if destination.is_none_or(|best| candidate < best) {
                    *destination = Some(candidate);
                    parents[end] = Some(Parent { start, kind });
                }
            }
        }
    }

    let mut ranges = Vec::new();
    let mut end = chars.len();
    while end > 0 {
        let Some(parent) = parents[end] else {
            return vec![QrSegment::make_bytes(text.as_bytes())];
        };
        ranges.push((parent.start, end, parent.kind));
        end = parent.start;
    }
    ranges.reverse();
    ranges
        .into_iter()
        .map(|(start, end, kind)| {
            let slice = &text[offsets[start]..offsets[end]];
            match kind {
                SegmentKind::Numeric => QrSegment::make_numeric(slice),
                SegmentKind::Alphanumeric => QrSegment::make_alphanumeric(slice),
                SegmentKind::Byte => QrSegment::make_bytes(slice.as_bytes()),
            }
        })
        .collect()
}

fn count_width(mode: QrSegmentMode, version: Version) -> u8 {
    let group = usize::from((version.value() + 7) / 17);
    match mode {
        QrSegmentMode::Numeric => [10, 12, 14][group],
        QrSegmentMode::Alphanumeric => [9, 11, 13][group],
        QrSegmentMode::Byte => [8, 16, 16][group],
        QrSegmentMode::Kanji | QrSegmentMode::Eci => unreachable!(),
    }
}

fn is_alphanumeric(character: char) -> bool {
    "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:".contains(character)
}
