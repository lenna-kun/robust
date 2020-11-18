use std::{
    env,
    fs::File,
    io::{
        BufWriter,
        Write,
    },
    // thread,
};

use pnet::util::MacAddr;

mod general;
mod utils;
mod eft;

#[allow(unused_must_use)]
fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 4 {
        panic!("args error");
    }

    let role: &str = &args[3];
    match role {
        "sender" => {
            let mut interface = eft::Interface::bind_sendmode(&args[2]).unwrap();
            // serial
            for id in 0..1000 {
                let filepath: String = format!("./data/data{}", id);
                let mut stream = interface.stream(id, MacAddr::new(0xff, 0xff, 0xff, 0xff, 0xff, 0xff)).unwrap();
                stream.send(&filepath, args[1].parse::<usize>().unwrap());
            }
        },
        "receiver" => {
            let mut interface = eft::Interface::bind_recvmode(&args[2]).unwrap();
            for id in 0.. {
                let filepath: String = format!("./received/data{}", id);
                let mut stream = interface.stream(id, MacAddr::new(0xff, 0xff, 0xff, 0xff, 0xff, 0xff)).unwrap();
                let data = if let Ok(d) = stream.read() {
                    d
                } else {
                    continue
                };
                let mut f = BufWriter::new(
                    if let Ok(f) = File::create(&filepath) {
                        f
                    } else {
                        continue
                    }
                );
                f.write(&data);
            }
        },
        _ => panic!("args error"),
    }
}
