pub mod packet;

use std::{
    collections::HashMap,
    io,
    net::{
        SocketAddr,
        UdpSocket,
    },
    sync::mpsc::{
        channel,
        Sender,
    },
    thread,
};

use super::general;
use super::utils;

#[derive(Clone)]
pub struct Uft {
    mtu: usize,
    rto: u32,
    received_files_buffer: HashMap<u16, Vec<Vec<u8>>>,
    received_files_buffer_flag: HashMap<u16, utils::Flags>,
}

impl Uft {
    pub fn new(mtu: usize) -> Self {
        Self {
            mtu: mtu,
            rto: 40, // ms
            received_files_buffer: HashMap::new(),
            received_files_buffer_flag: HashMap::new(),
        }
    }

    fn receive_ack_task(socket: UdpSocket, tx: Sender<usize>, id: u16, flags_length: usize) -> io::Result<()> {
        let mut flags = utils::Flags::new();
        flags.set_length(flags_length)?;
        while !flags.isallset() {
            let packet = {
                let mut packet = vec![0u8; 2048];
                let bytes = {
                    if let Ok(b) = socket.recv(&mut packet) {
                        b
                    } else {
                        continue
                    }
                };
                packet.resize(bytes, 0);
                if let Ok(p) = packet::UftPacket::from_raw(packet) {
                    p
                } else {
                    continue
                }
            };
            if packet.header.id == id && packet.header.uft_type == packet::UftType::Ack as u8 {
                if let Ok(_) = flags.set(packet.header.offset as usize) {
                    if let Err(_) = tx.send(packet.header.offset as usize) {
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    fn update_rto(&mut self, rtt: u32) {
        self.rto = (7 * self.rto + 3 * rtt) / 10;
    }

    #[allow(unused_must_use)]
    pub fn send(&mut self, filepath: &str, address: SocketAddr, id: u16) -> io::Result<()> {
        let socket = UdpSocket::bind("0.0.0.0:0")?;

        let data_fragments = utils::split_file_into_mtu_size(filepath, self.mtu)?;
        let mut timeouts: Vec<utils::Time> = {
            let now = utils::Time::now();
            vec![now; data_fragments.len()]
        };
        let mut flags = utils::Flags::new();
        flags.set_length(data_fragments.len())?;
        let (tx, rx) = channel();
        {
            let flags_length = flags.get_length().unwrap();
            let socket
                = match socket.try_clone() {
                    Ok(s) => s,
                    Err(e) => return Err(e),
                };
            thread::spawn(move || {
                Uft::receive_ack_task(socket, tx, id, flags_length).unwrap();
            });
        }

        while !flags.isallset() {
            for (offset, data_fragment) in data_fragments.iter().enumerate() {
                if let Ok(s) = flags.isset(offset) {
                    if s {
                        continue;
                    }
                } else {
                    return Err(io::Error::new(io::ErrorKind::Other, "data too long"));
                }

                let now = utils::Time::now(); 
                if now < timeouts[offset].add_millis(self.rto) {
                    continue;
                }

                self.update_rto(now.millis_sub(&timeouts[offset])); // calculate rto from rtt

                let packet = packet::UftPacket {
                    header: packet::UftPacketHeader {
                        uft_type:
                            if data_fragments.len() - 1 == offset {
                                packet::UftType::DataEnd as u8
                            } else {
                                packet::UftType::Data as u8
                            },
                        length: 8,
                        total_length: data_fragment.len() as u16 + 8,
                        id: id,
                        offset: offset as u16,
                    },
                    payload: data_fragment.to_vec(),
                };

                timeouts[offset] = utils::Time::now();

                socket.send_to(&packet.raw(), address);

                loop {
                    let offset: usize = if let Ok(o) = rx.try_recv() {
                        o
                    } else {
                        break
                    };
                    flags.set(offset as usize);
                }
            }
        }
        Ok(())
    }

    fn send_ack(socket: &UdpSocket, id: u16, offset: u16, address: &SocketAddr) -> io::Result<()> {
        let packet = packet::UftPacket {
            header: packet::UftPacketHeader {
                uft_type: packet::UftType::Ack as u8,
                length: 8,
                total_length: 8,
                id: id,
                offset: offset,
            },
            payload: vec![],
        };
        socket.send_to(&packet.raw(), address)?;
        Ok(())
    }

    #[allow(unused_must_use)]
    pub fn receive(&mut self, server_addr: SocketAddr) -> io::Result<Vec<u8>> {
        let socket = UdpSocket::bind(server_addr)?;
        loop {
            let mut packet = vec![0u8; 2048];
            let (bytes, client_addr) = if let Ok(r) = socket.recv_from(&mut packet) {
                r
            } else {
                continue
            };
            packet.resize(bytes, 0);
            let packet: packet::UftPacket = if let Ok(p) = packet::UftPacket::from_raw(packet) {
                p
            } else  {
                continue
            };

            let file_buffer = self.received_files_buffer
                .entry(packet.header.id)
                .or_insert(vec![Vec::new(); general::MAX_OFFSET_LENGTH]);
            let file_buffer_flag = self.received_files_buffer_flag.entry(packet.header.id).or_insert(utils::Flags::new());
            if let Err(_) = file_buffer_flag.set(packet.header.offset as usize) {
                continue;
            }
            file_buffer[packet.header.offset as usize] = packet.payload;

            if packet.header.uft_type == packet::UftType::DataEnd as u8 {
                file_buffer_flag.set_length(packet.header.offset as usize + 1).unwrap();
            }

            Uft::send_ack(&socket, packet.header.id, packet.header.offset, &client_addr);

            if file_buffer_flag.isallset() {
                let buffer_length = if let Ok(length) = file_buffer_flag.get_length() {
                    length
                } else {
                    continue
                };
                let raw_file: Vec<u8> = file_buffer[0..buffer_length].iter().fold(Vec::new(),
                    |mut acc, f| {
                        acc.extend_from_slice(f);
                        acc
                    }
                );
                self.received_files_buffer.remove(&packet.header.id);
                self.received_files_buffer_flag.remove(&packet.header.id);
                return Ok(raw_file);
            }
        }
    }
}