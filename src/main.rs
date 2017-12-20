extern crate jack;

use std::iter::FromIterator;
use std::num::ParseIntError;
use std::sync::Mutex;
use std::cell::RefCell;
use std::rc::Rc;
use std::io::Write;
use std::io::stdout;
use std::io::stdin;
use std::thread;
use std::sync::mpsc::channel;
use std::sync::mpsc::Sender;
use std::sync::mpsc::Receiver;

use jack::prelude::MidiOutPort;
use jack::prelude::Port;
use jack::prelude::MidiOutSpec;
use jack::prelude::MidiInSpec;
use jack::prelude::NotificationHandler;
use jack::prelude::*;

macro_rules! midi {
    ( $( $x:expr ),* ) => {
        {
            let mut temp_vec = Vec::new();
            $(
                temp_vec.push($x);
            )*
            RawMidi {
                time: 0,
                bytes: &temp_vec,
            }
        }
    };
}

struct MidiMessage {
    status: u8,
    data_1: u8,
    data_2: u8,
}

impl MidiMessage {
    // Construct MidiMessage from 3 bytes.  The actual number of bytes that
    // will compose the midi message is 4 (last being 0), except when data_2 is
    // 0.
    //
    // This cannot fail currently, however it can generate invalid midi.
    //
    // TODO: Put in sanity checking.
    fn from_bytes(status: u8, data_1: u8, data_2: u8) -> Result<MidiMessage,String> {
        Ok(MidiMessage { status, data_1, data_2 })
    }

    // Construct a MidiMessage from status selected from:
    //
    //     note_off
    //     note_on
    //     poly_key_pressure
    //     controller_change
    //     program_change
    //     channel_pressure
    //     pitch_bend
    //
    // If invalid status, channel is not in the range [0..15], or data_2 is
    // not 0 for program_change or channel_pressure, Err(String) is returned.
    fn from_description(status: String, channel: u8, data_1: u8, data_2: u8) -> Result<MidiMessage,String> {
        if channel > 15 {
            return Err(format!("Channel is not within [0..15]."));
        }
        let status_byte;
        if status == "note_off" {
            status_byte = 128 + channel;
        }
        else if status == "note_on" {
            status_byte = 128+16 + channel;
        }
        else if status == "poly_key_pressure" {
            status_byte = 128+32 + channel;
        }
        else if status == "controller_change" {
            status_byte = 128+32+16 + channel;
        }
        else if status == "program_change" {
            if data_2 != 0 {
                return Err(format!("data_2 must be 0 for {}.", status));
            }
            status_byte = 128+64 + channel;
        }
        else if status == "channel_pressure" {
            if data_2 != 0 {
                return Err(format!("data_2 must be 0 for {}.", status));
            }
            status_byte = 128+64+16 + channel;
        }
        else if status == "pitch_blend" {
            status_byte = 128+64+32 + channel;
        }
        else {
            return Err(format!("Invalid status: \"{}\".", status));
        }
        let status = status_byte;
        Ok(MidiMessage {
            status,
            data_1,
            data_2,
        })
    }

    //fn to_raw<'a>(&'a self) -> RawMidi<'a> {
    //    //let mut v = Vec::new();
    //    let mut v = [0,0,0,0];
    //    //if self.data_2 != 0 {
    //        return (RawMidi {
    //            time: 0,
    //            bytes: &v,
    //        });
    //    //}
    //    //else {
    //    //    v.push(self.status);
    //    //    v.push(self.data_1);
    //    //    v.push(0);
    //    //    return (RawMidi {
    //    //        time: 0,
    //    //        bytes: &v,
    //    //    });
    //    //}
    //}
}

struct NHandler {
}

impl NHandler {
    fn new() -> NHandler {
        NHandler {}
    }
}

// TODO: Implement latency callback because we will be a driver that might have
// inequal inputs and outputs.
impl NotificationHandler for NHandler {
    fn thread_init(&self, c: &Client) {
        println!("self.thread_init({})",c.name());
    }
    fn ports_connected(&mut self,
    c: &Client,
    port_a: JackPortId,
    port_b: JackPortId,
    are_connected: bool) {
        let port_a = c.port_by_id(port_a);
        let port_b = c.port_by_id(port_b);
        if port_a.is_some() && c.is_mine(&port_a.unwrap()) {
            println!("Port a is mine!");
        }
        else if port_b.is_some() && c.is_mine(&port_b.unwrap()) {
            println!("Port b is mine!");
        }
        else {
            println!("Neither are mine.");
            return;
        }
        if are_connected {
            println!("Connected!");
        }
        else {
            println!("Not connected!");
        }
    }
}

struct Handler<'a> {
    buffer: &'a Mutex<RefCell<Vec<String>>>,
    in_port: &'a mut Port<MidiInSpec>,
    out_port: &'a mut Port<MidiOutSpec>,
    send_null: bool,
    need_init: Mutex<RefCell<bool>>,
    messenger: Sender<&'static str>,
}

impl<'a> Handler<'a> {
    fn new(buffer: &'a Mutex<RefCell<Vec<String>>>,
    in_port: &'a mut Port<MidiInSpec>,
    out_port: &'a mut Port<MidiOutSpec>,
    init_signal: Mutex<RefCell<bool>>,
    messenger: Sender<&'static str>) -> Handler<'a> {
        //let buffer = &RefCell::new(buffer);
        Handler {
            buffer,
            in_port,
            out_port,
            send_null: false,
            need_init: init_signal,
            messenger,
        }
    }

}

unsafe impl<'a> Send for Handler<'a> {}

// Compose a RawMidi message given a Vec of bytes.
fn c<'a>(v: &'a Vec<u8>) -> RawMidi<'a> {
    RawMidi {
        time: 0,
        bytes: &v,
    }
}

fn init (god: &mut Handler) {
}

const IN_CONTROL_ON: RawMidi = RawMidi {
    time: 0,
    bytes: &[143,15,127],
};

impl<'a> ProcessHandler for Handler<'a> {
    // Process a Jack step.
    //
    // This function should never block for anything, nor use relative wait.
    //
    // TODO: Use a data structure that can send messages to the main thread.
    //
    // int jack_set_process_callback(...) specifies the following should be avoided:
    //
    //     all I/O functions (disk, TTY, network), malloc, free, printf,
    //     pthread_mutex_lock, sleep, wait, poll, select, pthread_join,
    //     pthread_cond_wait, etc, etc.
    //
    // We currently violate the I/O/printf stipulation.
    //
    // If we push anything to Vec, we may cause Vec to reallocate, violating
    // malloc/free.
    fn process(&mut self, client: &Client, scope: &ProcessScope) -> JackControl {
        fn init(messenger: &Sender<&str>, port: &mut MidiOutPort) -> JackControl {
            println!("Initiating extended mode.");
            port.write(&c(&vec![159,12,127,0])).unwrap();
            // 159, 36, 1-127 (79)
            messenger.send("Coloring LEDs");
            println!("Coloring LEDs.");
            for i in 96..121 {
                //println!("Coloring LED {}", i);
                port.write(&c(&vec![159,i,47,0])).unwrap();
            }
            port.write(&c(&vec![191,59,127])).unwrap();

            JackControl::Continue
        }

        let mut midi_port = MidiOutPort::new(&mut self.out_port, scope);

        let buffer = self.buffer.try_lock();
        if buffer.is_err() {
            println!("No lock for buffer.");
            return JackControl::Continue;
        }
        let buffer = buffer.unwrap();
        let buffer = buffer.try_borrow_mut();
        if buffer.is_err() {
            println!("Cannot get mutable borrow for buffer.");
            return JackControl::Continue;
        }
        let mut buffer = buffer.unwrap();
        let s;
        if buffer.len() > 0 {
            println!("Buffer count: {}", buffer.len());
            s = buffer.pop().unwrap();
            println!("Buffer count: {}", buffer.len());
        }
        else {
            return JackControl::Continue;
        }

        if s == "init" {
            return init(&self.messenger, &mut midi_port);
        }
        let split = Vec::from_iter(s.split(" "));
        if split.len() != 3 {
            println!("\x08\x08Need length 3.");
            print!("> ");
            stdout().flush().ok().expect("Can't flush stdout...");
            return JackControl::Continue;
        }
        let split: Vec<Result<u8,ParseIntError>> = split.iter().map(move |x| x.parse::<u8>()).collect();
        if split.clone().iter().any(|x| x.is_err()) {
            println!("\x08\x08At least one of these is not u8.");
            print!("> ");
            stdout().flush().ok().expect("Can't flush stdout...");
            return JackControl::Continue;
        }
        let split: Vec<u8> = split.iter().map(|x| x.clone().unwrap()).collect();
        let bytes = [split[0],split[1],split[2],0];
        bytes.iter().for_each(|x| print!("{};",x));
        println!();
        let raw = RawMidi {
            time: 0,
            bytes: &bytes,
        };
        let r = midi_port.write(&raw);
        if let Err(e) = r {
            println!("Error writing: {:?}", e);
        }
        else {
            println!("\x08\x08Write OK.");
            stdout().flush().ok().expect("Can't flush toilet...");
            println!("Buffer count: {}", buffer.len());
            print!("> ");
        }
        JackControl::Continue
    }
}
//fn failed_borrow<'a>() {
//    let _x = 12;
//
//    // ERROR: `_x` does not live long enough
//    //let y: &'a i32 = &_x;
//    // Attempting to use the lifetime `'a` as an explicit type annotation
//    // inside the function will fail because the lifetime of `&_x` is shorter
//    // than that of `y`. A short lifetime cannot be coerced into a longer one.
//}


fn main() {
    print!("Opening client...");
    // TODO: make name parameterized.
    let c_res = Client::new("rusty_client", client_options::NO_START_SERVER);
    let client;
    
    match c_res {
        Ok((c, _status)) => {
            println!("OK.  name={}.", c.name());
            client = c;
            },
        Err(e) => {
            println!("Error: {:?}.  Bailing.", e);
            panic!("Failed to open server.");
        },
    };
    print!("Opening out port...");
    let port_status = client.register_port("rusty_port_out", MidiOutSpec);

    if let Err(e) = port_status {
        panic!("Failed to open out port: {:?}",e);
    }

    println!("OK.");
    let mut out_midi_port = port_status.unwrap();
    print!("Opening in port...");
    let port_status = client.register_port("rusty_port_in", MidiInSpec);

    if let Err(e) = port_status {
        panic!("Failed to open in port: {:?}",e);
    }
    let mut in_midi_port = port_status.unwrap();
    println!("OK.");
    let buffer = Vec::new();
    let buffer = Mutex::new(RefCell::new(buffer));
    let init_signal = Mutex::new(RefCell::new(false));

    let (tx,rx) = channel::<&str>();

    let handler = Handler::new(&buffer, &mut in_midi_port, &mut out_midi_port, init_signal, tx);
    let nhandler = NHandler::new();
    let client = AsyncClient::new(client, nhandler, handler);


    loop {
        print!("> ");
        stdout().flush().ok().expect("Could not flush stdout");
        let mut s = String::new();
        let r = stdin().read_line(&mut s);
        if let Err(e) = r {
            println!("Bad!");
            continue;
        }
        let result = buffer.lock();
        if result.is_err() {
            println!("Failed to get lock.");
            continue;
        }
        let buffer = result.unwrap();
        let mut buffer = buffer.borrow_mut();
        buffer.push(s.trim().to_string());
        println!("{}",s);
    }
}
