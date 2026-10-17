use qrcodegen::{Mask, QrCode, QrCodeEcc, QrSegment, Version};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let data = args.first().expect("payload required");
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
        QrCode::encode_segments(&segments, QrCodeEcc::Medium).unwrap()
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
