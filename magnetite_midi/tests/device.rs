//! The mock ports: what a test driving a [`MidiBackend`] relies on.

use magnetite_midi::device::{MidiBackend, MockMidi};
use std::sync::{Arc, Mutex};

#[test]
fn an_input_delivers_what_is_played_into_it_until_it_is_unplugged() {
    let mock = MockMidi::new();
    mock.add_input("keys");
    let mut backend = mock.backend();
    assert_eq!(backend.input_ports(), ["keys"]);
    let got = Arc::new(Mutex::new(Vec::new()));
    let sink = got.clone();
    let connection = backend
        .open_input(
            "keys",
            Box::new(move |micros, bytes| sink.lock().unwrap().push((micros, bytes.to_vec()))),
        )
        .unwrap();
    assert!(mock.inject("keys", &[0x90, 60, 100]));
    assert!(mock.inject("keys", &[0x80, 60, 0]));
    {
        let got = got.lock().unwrap();
        assert_eq!(
            got.iter().map(|g| g.1.clone()).collect::<Vec<_>>(),
            [vec![0x90, 60, 100], vec![0x80, 60, 0]]
        );
        assert!(got[1].0 > got[0].0, "the device clock only goes forward");
    }
    drop(connection);
    assert!(
        !mock.input_is_open("keys") && !mock.inject("keys", &[0xF8]),
        "a closed port delivers nothing"
    );
    assert!(
        backend.open_input("pads", Box::new(|_, _| {})).is_err(),
        "a port that is not plugged in cannot be opened"
    );
}

#[test]
fn an_output_records_what_is_sent_and_refuses_once_unplugged() {
    let mock = MockMidi::new();
    mock.add_output("synth");
    let mut backend = mock.backend();
    let mut out = backend.open_output("synth").unwrap();
    out.send(&[0xC0, 5]).unwrap();
    assert_eq!(mock.sent("synth"), [vec![0xC0, 5]]);
    assert!(mock.output_is_open("synth"));
    mock.remove_output("synth");
    assert!(out.send(&[0xC0, 6]).is_err(), "an unplugged port says so");
}
