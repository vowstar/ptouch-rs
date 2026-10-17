use qrcodegen::{Mask, QrCode, QrCodeEcc, QrSegment, Version};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let data = args.first().expect("payload required");
    if args.get(1).is_some_and(|mode| mode == "production") {
        let version = args.get(2).and_then(|value| value.parse().ok());
        let bitmap = ptouch_render::barcode::render_qr(
            data,
            ptouch_render::barcode::QrErrorCorrection::Medium,
            version,
        )
        .unwrap();
        let modules = bitmap.width() / 8 - 8;
        println!("modules={modules} size={modules} ecc=Medium quiet_zone=4 cell=8");
        for y in 0..modules {
            for x in 0..modules {
                print!(
                    "{}",
                    if bitmap.get_pixel((x + 4) * 8 + 4, (y + 4) * 8 + 4) {
                        '#'
                    } else {
                        '.'
                    }
                );
            }
            println!();
        }
        return;
    }
    let segments = if args
        .get(1)
        .is_some_and(|mode| mode == "split-trailing-numeric")
    {
        let split = data
            .rfind(|character: char| !character.is_ascii_digit())
            .map_or(0, |index| index + 1);
        vec![
            QrSegment::make_alphanumeric(&data[..split]),
            QrSegment::make_numeric(&data[split..]),
        ]
    } else if args
        .get(1)
        .is_some_and(|mode| mode == "split-trailing-numeric-byte")
    {
        let split = data
            .rfind(|character: char| !character.is_ascii_digit())
            .map_or(0, |index| index + 1);
        vec![
            QrSegment::make_bytes(&data.as_bytes()[..split]),
            QrSegment::make_numeric(&data[split..]),
        ]
    } else if let Some(specification) = args.get(1).and_then(|mode| mode.strip_prefix("manual:")) {
        specification
            .split(',')
            .map(|part| {
                let (mode, range) = part.split_once(':').expect("mode:start-end required");
                let (start, end) = range.split_once('-').expect("start-end required");
                let start: usize = start.parse().expect("invalid segment start");
                let end: usize = end.parse().expect("invalid segment end");
                let text = &data[start..end];
                match mode {
                    "byte" => QrSegment::make_bytes(text.as_bytes()),
                    "numeric" => QrSegment::make_numeric(text),
                    "alphanumeric" => QrSegment::make_alphanumeric(text),
                    _ => panic!("unsupported segment mode: {mode}"),
                }
            })
            .collect()
    } else {
        QrSegment::make_segments(data)
    };
    let qr = if let Some(version) = args.get(2) {
        let version = Version::new(version.parse().expect("invalid version"));
        let mask = args
            .get(3)
            .map(|value| Mask::new(value.parse().expect("invalid mask")));
        QrCode::encode_segments_advanced(
            &segments,
            QrCodeEcc::Medium,
            version,
            version,
            mask,
            false,
        )
        .unwrap()
    } else {
        QrCode::encode_segments_advanced(
            &segments,
            QrCodeEcc::Medium,
            Version::MIN,
            Version::MAX,
            None,
            false,
        )
        .unwrap()
    };
    println!(
        "version={} ecc={:?} mask={} size={}",
        qr.version().value(),
        qr.error_correction_level(),
        qr.mask().value(),
        qr.size()
    );
    for y in 0..qr.size() {
        for x in 0..qr.size() {
            print!("{}", if qr.get_module(x, y) { '#' } else { '.' });
        }
        println!();
    }
}
