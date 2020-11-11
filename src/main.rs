use std::{
    env,
    fs::File,
    io::{
        BufWriter,
        Write,
    },
    net::{
        IpAddr,
        Ipv4Addr,
        SocketAddr,
    },
    // thread,
};

mod general;
mod utils;
mod uft;

#[allow(unused_must_use)]
fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        panic!("args error");
    }

    let mut uft = uft::Uft::new(args[1].parse::<usize>().unwrap());

    let role: &str = &args[2];
    match role {
        "sender" => {
            // serial
            for id in 0..1000 {
                let filepath: String = format!("./data/data{}", id);
                uft.send(
                    &filepath,
                    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8888),
                    id
                );
            }
            // parallel
            // let mut threads: Vec<thread::JoinHandle<_>> = Vec::new();
            // for id in 0..1000 {
            //     let filepath: String = format!("./data/data{}", id);
            //     let uft = uft.clone();
            //     threads.push(
            //         thread::spawn(move || {
            //             uft.send(
            //                 &filepath,
            //                 SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8888),
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
                let data = if let Ok(d) = uft.receive(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8888)) {
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
