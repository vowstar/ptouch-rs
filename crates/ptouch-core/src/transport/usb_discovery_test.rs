use super::*;

#[test]
fn endpoint_pair_cannot_cross_alternate_settings() {
    assert!(matches!(
        select_bulk_endpoints([(0, vec![0x01]), (1, vec![0x82])]),
        Err(PtouchError::InvalidUsbInterface)
    ));
    assert_eq!(
        select_bulk_endpoints([(1, vec![0x02, 0x83]), (0, vec![0x01, 0x82])]).unwrap(),
        BulkEndpoints {
            alternate: 0,
            out: 1,
            input: 0x82,
            requires_selection: true,
        }
    );
    assert!(select_bulk_endpoints([(0, vec![1, 2, 0x81])]).is_err());
}

#[test]
fn sole_default_setting_does_not_send_set_interface() {
    let endpoints = select_bulk_endpoints([(0, vec![0x02, 0x81])]).unwrap();
    endpoints
        .activate(|_| panic!("PT-D610BT must not receive SET_INTERFACE"))
        .unwrap();
}

#[test]
fn multiple_settings_explicitly_activate_default_setting() {
    for other_endpoints in [vec![], vec![0x03], vec![0x03, 0x84]] {
        let endpoints =
            select_bulk_endpoints([(0, vec![0x02, 0x81]), (1, other_endpoints)]).unwrap();
        let mut requests = Vec::new();
        endpoints
            .activate(|alternate| {
                requests.push(alternate);
                Ok(())
            })
            .unwrap();
        assert_eq!(requests, [0]);
    }
}

#[test]
fn nondefault_setting_is_always_activated() {
    for settings in [
        vec![(1, vec![0x02, 0x81])],
        vec![(0, vec![]), (1, vec![0x02, 0x81])],
    ] {
        let endpoints = select_bulk_endpoints(settings).unwrap();
        let mut requests = Vec::new();
        endpoints
            .activate(|alternate| {
                requests.push(alternate);
                Ok(())
            })
            .unwrap();
        assert_eq!(requests, [1]);
    }
}

#[test]
fn alternate_selection_errors_are_preserved() {
    let endpoints = select_bulk_endpoints([(0, vec![0x02, 0x81]), (1, vec![])]).unwrap();
    for error in [
        rusb::Error::Timeout,
        rusb::Error::Pipe,
        rusb::Error::NoDevice,
    ] {
        assert_eq!(endpoints.activate(|_| Err(error)), Err(error));
    }
}

#[test]
fn duplicate_printers_require_explicit_selection() {
    assert!(matches!(
        only_candidate(vec![1, 2]),
        Err(PtouchError::AmbiguousDevice)
    ));
    assert!(matches!(
        only_candidate(Vec::<u8>::new()),
        Err(PtouchError::DeviceNotFound)
    ));
    assert_eq!(only_candidate(vec![7]).unwrap(), 7);
    assert_eq!(
        "2:17".parse::<UsbLocation>().unwrap(),
        UsbLocation {
            bus: 2,
            address: 17
        }
    );
    for value in ["2", "2:17:3", "256:1", "1:-1"] {
        assert!(value.parse::<UsbLocation>().is_err());
    }
}

#[test]
fn usb_facade_preserves_send_and_sync() {
    fn assert_traits<T: Send + Sync>() {}
    assert_traits::<PtouchDevice>();
}
