#[macro_use]
extern crate lazy_static;

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
mod uft;

#[allow(unused_must_use)]
fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        panic!("args error");
    }

    let mut uft = uft::Uft::new(args[1].parse::<usize>().unwrap(), &args[2], MacAddr::new(0xff, 0xff, 0xff, 0xff, 0xff, 0xff)).unwrap();

    let role: &str = &args[3];
    match role {
        "sender" => {
            // serial
            for id in 0..1000 {
                let filepath: String = format!("./data/data{}", id);
                uft.send(
                    &filepath,
                    MacAddr::new(0xff, 0xff, 0xff, 0xff, 0xff, 0xff),
                    id
                );
            }
            // parallel
            // let mut threads: Vec<thread::JoinHandle<_>> = Vec::new();
            // for id in 0..1000 {
            //     let filepath: String = format!("./data/data{}", id);
            //     let mut uft = uft.clone();
            //     threads.push(
            //         thread::spawn(move || {
            //             uft.send(
            //                 &filepath,
            //                 MacAddr::new(0xff, 0xff, 0xff, 0xff, 0xff, 0xff),
            //                 id
            //             );
            //         })
            //     );
            // }
            // for thread in threads {
            //     thread.join();
            // }
        },
        "receiver" => {
            for i in 0.. {
                let filepath: String = format!("./received/data{}", i);
                let data = if let Ok(d) = uft.receive_from(MacAddr::new(0xff, 0xff, 0xff, 0xff, 0xff, 0xff)) {
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
