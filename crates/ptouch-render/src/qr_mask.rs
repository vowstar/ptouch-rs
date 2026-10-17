use qrcodegen::{Mask, QrCode, QrCodeEcc, QrSegment, Version};

/// Select a QR mask with the scoring behavior that matched Brother's captures.
///
/// The finder-pattern rule adds one N3 penalty when either adjacent light run
/// reaches four modules. It does not add a second penalty when both sides do.
pub(crate) fn encode_with_brother_mask_score(
    segments: &[QrSegment],
    correction: QrCodeEcc,
    version: Version,
) -> std::result::Result<QrCode, qrcodegen::DataTooLong> {
    let mut best: Option<(i32, QrCode)> = None;
    for mask in 0..8 {
        let qr = QrCode::encode_segments_advanced(
            segments,
            correction,
            version,
            version,
            Some(Mask::new(mask)),
            false,
        )?;
        let score = penalty_score(&qr);
        if best
            .as_ref()
            .is_none_or(|(best_score, _)| score < *best_score)
        {
            best = Some((score, qr));
        }
    }
    Ok(best.expect("eight QR masks are evaluated").1)
}

fn penalty_score(qr: &QrCode) -> i32 {
    let size = qr.size();
    let mut score = 0;

    for y in 0..size - 1 {
        for x in 0..size - 1 {
            let module = qr.get_module(x, y);
            if qr.get_module(x + 1, y) == module
                && qr.get_module(x, y + 1) == module
                && qr.get_module(x + 1, y + 1) == module
            {
                score += 3;
            }
        }
    }

    for y in 0..size {
        let row: Vec<_> = (0..size).map(|x| qr.get_module(x, y)).collect();
        score += line_penalty(&row);
    }
    for x in 0..size {
        let column: Vec<_> = (0..size).map(|y| qr.get_module(x, y)).collect();
        score += line_penalty(&column);
    }

    let dark_modules = (0..size)
        .flat_map(|y| (0..size).map(move |x| qr.get_module(x, y)))
        .filter(|dark| *dark)
        .count() as i32;
    let total_modules = size * size;
    let rounded_dark_percent = (200 * dark_modules + total_modules) / total_modules / 2;
    score + ((rounded_dark_percent - 50).abs() / 5) * 10
}

fn line_penalty(line: &[bool]) -> i32 {
    let mut runs = Vec::with_capacity(line.len() + 1);
    if line[0] {
        runs.push(-1);
    }
    let mut current = line[0];
    let mut run_length = 1;
    for &module in &line[1..] {
        if module == current {
            run_length += 1;
        } else {
            runs.push(run_length);
            current = module;
            run_length = 1;
        }
    }
    runs.push(run_length);

    let mut score = 0;
    for (index, &run) in runs.iter().enumerate() {
        if run >= 5 {
            score += 3 + run - 5;
        }
        if index & 1 == 0 || index < 3 || index >= runs.len() - 2 || run % 3 != 0 {
            continue;
        }
        let unit = run / 3;
        if runs[index - 2] == unit
            && runs[index - 1] == unit
            && runs[index + 1] == unit
            && runs[index + 2] == unit
            && (index == 3
                || runs[index - 3] >= 4 * unit
                || index + 4 >= runs.len()
                || runs[index + 3] >= 4 * unit)
        {
            score += 40;
        }
    }
    score
}
