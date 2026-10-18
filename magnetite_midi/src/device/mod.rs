//! The device layer behind a trait: [`MidirBackend`] talks to real ports through `midir`; [`MockMidi`] is a set of fake ports a
//! test can plug, unplug, play into and listen to, so everything above this module is tested with no hardware.
//!
//! A backend is meant to live on one worker thread: it is created there, lists ports there and holds its connections
//! there, which is why connections need not be `Send`. Ports are identified by **name**: an index changes when a device comes
//! or goes.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

/// Where a connection delivers what it receives: the device's own timestamp in microseconds (its epoch is the driver's, so only
/// differences mean anything) and the bytes of one message. Called on a driver thread: do little, never block.
pub type InputSink = Box<dyn FnMut(u64, &[u8]) + Send + 'static>;

/// An open input. Dropping it closes the port.
pub trait Connection {}

/// An open output. Dropping it closes the port.
pub trait OutputConnection {
    fn send(&mut self, bytes: &[u8]) -> Result<(), String>;
}

pub trait MidiBackend: Send + 'static {
    fn input_ports(&mut self) -> Vec<String>;
    fn output_ports(&mut self) -> Vec<String>;
    fn open_input(&mut self, port: &str, sink: InputSink) -> Result<Box<dyn Connection>, String>;
    fn open_output(&mut self, port: &str) -> Result<Box<dyn OutputConnection>, String>;
}

// ------------------------------------------------------------------------------------------------------------------- midir

/// The real devices (WinMM on Windows, CoreMIDI, ALSA). Listing the ports can take seconds the first time after boot (measured:
/// 10 s on Windows while the MIDI service started), which is why a caller with a frame to keep should list ports on a thread of its own.
pub struct MidirBackend {
    client: String,
}

impl MidirBackend {
    /// `client_name` names this program to the system's MIDI service; its connections are named `<client_name>-in` and `<client_name>-out`.
    pub fn new(client_name: &str) -> Self {
        Self {
            client: client_name.to_string(),
        }
    }
}

struct MidirInput {
    _connection: midir::MidiInputConnection<()>,
}
impl Connection for MidirInput {}

struct MidirOutput {
    connection: midir::MidiOutputConnection,
}
impl OutputConnection for MidirOutput {
    fn send(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.connection.send(bytes).map_err(|e| e.to_string())
    }
}

impl MidiBackend for MidirBackend {
    fn input_ports(&mut self) -> Vec<String> {
        match midir::MidiInput::new(&self.client) {
            Ok(input) => input
                .ports()
                .iter()
                .filter_map(|p| input.port_name(p).ok())
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    fn output_ports(&mut self) -> Vec<String> {
        match midir::MidiOutput::new(&self.client) {
            Ok(output) => output
                .ports()
                .iter()
                .filter_map(|p| output.port_name(p).ok())
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    fn open_input(
        &mut self,
        port: &str,
        mut sink: InputSink,
    ) -> Result<Box<dyn Connection>, String> {
        let mut input = midir::MidiInput::new(&self.client).map_err(|e| e.to_string())?;
        input.ignore(midir::Ignore::ActiveSense); // keep clock, start/stop and sysex: bindings and clock-in need them
        let found = input
            .ports()
            .into_iter()
            .find(|p| input.port_name(p).is_ok_and(|n| n == port))
            .ok_or_else(|| format!("no input port named {port:?}"))?;
        let connection = input
            .connect(
                &found,
                &format!("{}-in", self.client),
                move |micros, bytes, _| sink(micros, bytes),
                (),
            )
            .map_err(|e| e.to_string())?;
        Ok(Box::new(MidirInput {
            _connection: connection,
        }))
    }

    fn open_output(&mut self, port: &str) -> Result<Box<dyn OutputConnection>, String> {
        let output = midir::MidiOutput::new(&self.client).map_err(|e| e.to_string())?;
        let found = output
            .ports()
            .into_iter()
            .find(|p| output.port_name(p).is_ok_and(|n| n == port))
            .ok_or_else(|| format!("no output port named {port:?}"))?;
        let connection = output
            .connect(&found, &format!("{}-out", self.client))
            .map_err(|e| e.to_string())?;
        Ok(Box::new(MidirOutput { connection }))
    }
}

// -------------------------------------------------------------------------------------------------------------------- mock

#[derive(Default)]
struct MockState {
    inputs: Vec<String>,
    outputs: Vec<String>,
    sinks: HashMap<String, InputSink>,
    open_outputs: HashSet<String>,
    sent: HashMap<String, Vec<Vec<u8>>>,
    clock: u64,
}

/// Fake ports. Clone the handle to keep it in a test while the backend (made with [`MockMidi::backend`]) runs on the worker.
#[derive(Clone, Default)]
pub struct MockMidi(Arc<Mutex<MockState>>);

impl MockMidi {
    pub fn new() -> Self {
        Self::default()
    }

    /// The backend half, to hand to whatever drives a [`MidiBackend`].
    pub fn backend(&self) -> MockBackend {
        MockBackend(self.clone())
    }

    /// Plug an input port in (a keyboard, a controller).
    pub fn add_input(&self, name: &str) {
        let mut s = self.0.lock().unwrap();
        if !s.inputs.iter().any(|n| n == name) {
            s.inputs.push(name.to_string());
        }
    }

    pub fn add_output(&self, name: &str) {
        let mut s = self.0.lock().unwrap();
        if !s.outputs.iter().any(|n| n == name) {
            s.outputs.push(name.to_string());
        }
    }

    /// Unplug an input: it disappears from the list and an open connection stops delivering.
    pub fn remove_input(&self, name: &str) {
        let mut s = self.0.lock().unwrap();
        s.inputs.retain(|n| n != name);
        s.sinks.remove(name);
    }

    pub fn remove_output(&self, name: &str) {
        let mut s = self.0.lock().unwrap();
        s.outputs.retain(|n| n != name);
        s.open_outputs.remove(name);
    }

    /// Play bytes into an input port, as the device would; `false` if nothing is connected to it.
    pub fn inject(&self, port: &str, bytes: &[u8]) -> bool {
        let mut s = self.0.lock().unwrap();
        s.clock += 1000;
        let clock = s.clock;
        match s.sinks.get_mut(port) {
            Some(sink) => {
                sink(clock, bytes);
                true
            }
            None => false,
        }
    }

    /// Everything sent to an output port so far.
    pub fn sent(&self, port: &str) -> Vec<Vec<u8>> {
        self.0
            .lock()
            .unwrap()
            .sent
            .get(port)
            .cloned()
            .unwrap_or_default()
    }

    pub fn input_is_open(&self, port: &str) -> bool {
        self.0.lock().unwrap().sinks.contains_key(port)
    }

    pub fn output_is_open(&self, port: &str) -> bool {
        self.0.lock().unwrap().open_outputs.contains(port)
    }
}

pub struct MockBackend(MockMidi);

struct MockInput {
    state: MockMidi,
    port: String,
}
impl Connection for MockInput {}
impl Drop for MockInput {
    fn drop(&mut self) {
        self.state.0.lock().unwrap().sinks.remove(&self.port);
    }
}

struct MockOutput {
    state: MockMidi,
    port: String,
}
impl OutputConnection for MockOutput {
    fn send(&mut self, bytes: &[u8]) -> Result<(), String> {
        let mut s = self.state.0.lock().unwrap();
        if !s.outputs.contains(&self.port) {
            return Err(format!("{} is unplugged", self.port));
        }
        s.sent
            .entry(self.port.clone())
            .or_default()
            .push(bytes.to_vec());
        Ok(())
    }
}
impl Drop for MockOutput {
    fn drop(&mut self) {
        self.state.0.lock().unwrap().open_outputs.remove(&self.port);
    }
}

impl MidiBackend for MockBackend {
    fn input_ports(&mut self) -> Vec<String> {
        self.0 .0.lock().unwrap().inputs.clone()
    }

    fn output_ports(&mut self) -> Vec<String> {
        self.0 .0.lock().unwrap().outputs.clone()
    }

    fn open_input(&mut self, port: &str, sink: InputSink) -> Result<Box<dyn Connection>, String> {
        let mut s = self.0 .0.lock().unwrap();
        if !s.inputs.iter().any(|n| n == port) {
            return Err(format!("no input port named {port:?}"));
        }
        s.sinks.insert(port.to_string(), sink);
        Ok(Box::new(MockInput {
            state: self.0.clone(),
            port: port.to_string(),
        }))
    }

    fn open_output(&mut self, port: &str) -> Result<Box<dyn OutputConnection>, String> {
        let mut s = self.0 .0.lock().unwrap();
        if !s.outputs.iter().any(|n| n == port) {
            return Err(format!("no output port named {port:?}"));
        }
        s.open_outputs.insert(port.to_string());
        Ok(Box::new(MockOutput {
            state: self.0.clone(),
            port: port.to_string(),
        }))
    }
}
