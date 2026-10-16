use std::cell::Cell;

use super::*;

#[test]
fn usb_write_resumes_only_the_unwritten_suffix_after_partial_progress() {
    let mut writes = Vec::new();
    let mut results = [Ok(2), Err(rusb::Error::Timeout), Ok(4)].into_iter();
    send_usb_bytes(
        &[1, 2, 3, 4, 5, 6],
        Duration::from_secs(30),
        &CancellationToken::default(),
        |data, timeout| {
            assert_eq!(timeout, USB_WRITE_POLL_TIMEOUT);
            writes.push(data.to_vec());
            results.next().expect("unexpected replay")
        },
        || Duration::ZERO,
        |_| panic!("nonzero writes do not need pacing"),
    )
    .unwrap();
    assert_eq!(
        writes,
        [vec![1, 2, 3, 4, 5, 6], vec![3, 4, 5, 6], vec![3, 4, 5, 6]]
    );
}

#[test]
fn usb_write_checks_cancellation_before_sending_and_after_a_partial_transfer() {
    for cancel_before_write in [true, false] {
        let token = CancellationToken::default();
        if cancel_before_write {
            token.cancel();
        }
        let mut writes = 0;
        let result = send_usb_bytes(
            &[1, 2, 3],
            Duration::from_secs(30),
            &token,
            |_, _| {
                writes += 1;
                token.cancel();
                Ok(1)
            },
            || Duration::ZERO,
            |_| {},
        );
        assert!(matches!(result, Err(PtouchError::Cancelled)));
        assert_eq!(writes, usize::from(!cancel_before_write));
    }
}

#[test]
fn usb_write_uses_one_deadline_across_progress_and_timeouts() {
    let elapsed = Cell::new(Duration::ZERO);
    let mut poll_timeouts = Vec::new();
    let mut writes = Vec::new();
    let result = send_usb_bytes(
        &[1, 2, 3],
        Duration::from_millis(600),
        &CancellationToken::default(),
        |data, timeout| {
            poll_timeouts.push(timeout);
            writes.push(data.to_vec());
            elapsed.set(elapsed.get() + timeout);
            if writes.len() == 1 {
                Ok(1)
            } else {
                Err(rusb::Error::Timeout)
            }
        },
        || elapsed.get(),
        |_| {},
    );
    assert!(matches!(result, Err(PtouchError::Timeout)));
    assert_eq!(
        poll_timeouts,
        [
            Duration::from_millis(250),
            Duration::from_millis(250),
            Duration::from_millis(100)
        ]
    );
    assert_eq!(writes, [vec![1, 2, 3], vec![2, 3], vec![2, 3]]);
}

#[test]
fn usb_write_aborts_on_disconnect_or_other_non_timeout_errors() {
    for error in [
        rusb::Error::NoDevice,
        rusb::Error::Pipe,
        rusb::Error::Io,
        rusb::Error::Interrupted,
    ] {
        let mut writes = 0;
        let result = send_usb_bytes(
            &[1, 2, 3],
            Duration::from_secs(30),
            &CancellationToken::default(),
            |data, _| {
                writes += 1;
                match writes {
                    1 => Ok(1),
                    2 => {
                        assert_eq!(data, [2, 3]);
                        Err(error)
                    }
                    _ => panic!("must not retry a failed transfer"),
                }
            },
            || Duration::ZERO,
            |_| {},
        );
        assert!(matches!(result, Err(PtouchError::UsbError(actual)) if actual == error));
        assert_eq!(writes, 2);
    }
}

#[test]
fn usb_write_never_passes_a_submillisecond_timeout_to_libusb() {
    for already_elapsed in [Duration::ZERO, Duration::from_millis(1)] {
        let mut writes = 0;
        let result = send_usb_bytes(
            &[1],
            already_elapsed + Duration::from_micros(999),
            &CancellationToken::default(),
            |_, _| {
                writes += 1;
                Ok(1)
            },
            || already_elapsed,
            |_| {},
        );
        assert!(matches!(result, Err(PtouchError::Timeout)));
        assert_eq!(writes, 0);
    }
}

#[test]
fn usb_zero_length_writes_are_paced_and_still_obey_the_deadline() {
    let elapsed = Cell::new(Duration::ZERO);
    let mut pauses = Vec::new();
    let mut writes = 0;
    let result = send_usb_bytes(
        &[1],
        Duration::from_millis(25),
        &CancellationToken::default(),
        |_, _| {
            writes += 1;
            Ok(0)
        },
        || elapsed.get(),
        |duration| {
            pauses.push(duration);
            elapsed.set(elapsed.get() + duration);
        },
    );
    assert!(matches!(result, Err(PtouchError::Timeout)));
    assert_eq!(writes, 3);
    assert_eq!(
        pauses,
        [
            Duration::from_millis(10),
            Duration::from_millis(10),
            Duration::from_millis(5)
        ]
    );
}

#[test]
fn usb_write_rejects_an_impossible_byte_count() {
    let result = send_usb_bytes(
        &[1],
        Duration::from_secs(1),
        &CancellationToken::default(),
        |_, _| Ok(2),
        || Duration::ZERO,
        |_| {},
    );
    assert!(matches!(result, Err(PtouchError::SendFailed(_))));
}
