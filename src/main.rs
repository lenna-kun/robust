use std::{
    env,
    fs::File,
    io::{
        BufWriter,
        Write,
    },
    thread,
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
            // serial
            // let interface = eft::Interface::bind_sendmode(&args[2]).unwrap();
            // for id in 0..1000 {
            //     let filepath: String = format!("./data/data{}", id);
            //     interface.send(id, MacAddr::new(0xff, 0xff, 0xff, 0xff, 0xff, 0xff), filepath, args[1].parse::<usize>().unwrap()).unwrap();
            // }
            let mut interface = eft::Interface::bind_sendmode(&args[2]).unwrap();
            let mut fileids: Vec<u16> = Vec::new();
            let mut filepaths: Vec<String> = Vec::new();
            for id in 0..1000 {
                fileids.push(id);
                filepaths.push(format!("./data/data{}", id));
            }
            interface.send_files(fileids, MacAddr::new(0xff, 0xff, 0xff, 0xff, 0xff, 0xff), filepaths, args[1].parse::<usize>().unwrap()).unwrap();
        },
        "receiver" => {
            let interface = eft::Interface::bind_recvmode(&args[2]).unwrap();
            let mut threads: Vec<thread::JoinHandle<_>> = Vec::new();
            for id in 0..1000 {
                let mut interface = interface.clone();
                threads.push(thread::spawn(move || {
                    let filepath: String = format!("./received/data{}", id);
                    let mut stream = interface.stream(id, MacAddr::new(0xff, 0xff, 0xff, 0xff, 0xff, 0xff)).unwrap();
                    let data = if let Ok(d) = stream.read() {
                        d
                    } else {
                        return
                    };
                    let mut f = BufWriter::new(
                        if let Ok(f) = File::create(&filepath) {
                            f
                        } else {
                            return
                        }
                    );
                    f.write(&data);
                }));
            }
            for thread in threads {
                thread.join();
            }
        },
        _ => panic!("args error"),
    }
}
