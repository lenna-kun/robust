pub mod packet;
use std::{
    collections::{
        HashMap,
        HashSet,
    },
    io,
    sync::{
        Mutex,
        mpsc,
    },
    thread,
};

use pnet::{
    datalink::{
        self, 
        Channel::Ethernet,
        DataLinkReceiver,
        DataLinkSender,
    },
    packet::{
        ethernet::{
            MutableEthernetPacket,
            EtherType,
            EthernetPacket,
        },
        Packet,
    },
    util::MacAddr,
};

use super::general;
use super::utils;

lazy_static! {
    static ref TX: Mutex<Option<Box<dyn DataLinkSender + 'static>>> = Mutex::new(None);
    static ref RX: Mutex<Option<Box<dyn DataLinkReceiver + 'static>>> = Mutex::new(None);
}

#[derive(Clone)]
pub struct Uft {
    address: MacAddr,
    mtu: usize,
    rto: u32,
    received_files_flag: HashSet<u16>,
    received_files_buffer: HashMap<u16, Vec<Vec<u8>>>,
    received_files_buffer_flag: HashMap<u16, utils::Flags>,
}

impl Uft {
    pub fn new(mtu: usize, interface_name: &str, address: MacAddr) -> io::Result<Self> {
        let interface = datalink::interfaces()
            .into_iter()
            .find(|iface| iface.name == *interface_name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "failed to get interface"))?;

        if let Ok(Ethernet(tx_local, rx_local)) = datalink::channel(&interface, Default::default()) {
            *TX.lock().unwrap() = Some(tx_local);
            *RX.lock().unwrap() = Some(rx_local);
        } else {
            return Err(io::Error::new(io::ErrorKind::Other, "failed to create channel"));
        }

        Ok(Self {
            address: address,
            mtu: mtu,
            rto: 40, // ms
            received_files_flag: HashSet::new(),
            received_files_buffer: HashMap::new(),
            received_files_buffer_flag: HashMap::new(),
        })
    }

    fn receive_ack_task(src_address: MacAddr, dst_address: MacAddr, mpsc_tx: mpsc::Sender<usize>, id: u16, flags_length: usize) -> io::Result<()> {
        let mut flags = utils::Flags::new();
        flags.set_length(flags_length)?;
        let mut datalink_rx = RX.lock().unwrap();
        while !flags.isallset() {
            match datalink_rx.as_mut().unwrap().next() {
                Ok(frame) => {
                    let frame = EthernetPacket::new(frame).unwrap();
                    if frame.get_ethertype() != EtherType(0xFF) || frame.get_source() != src_address || frame.get_destination() != dst_address {
                        continue;
                    }
                    let packet = if let Ok(p) = packet::UftPacket::from_raw(frame.payload().to_vec()) {
                        p
                    } else {
                        continue
                    };
                    if packet.header.id == id && packet.header.uft_type == packet::UftType::Ack as u8 {
                        if let Ok(_) = flags.set(packet.header.offset as usize) {
                            if let Err(_) = mpsc_tx.send(packet.header.offset as usize) {
                                break;
                            }
                        }
                    }
                },
                Err(_) => continue,
            }
        }
        Ok(())
    }

    // fn update_rto(&mut self, rtt: u32) {
    //     self.rto = (7 * self.rto + 3 * rtt) / 10;
    // }

    #[allow(unused_must_use)]
    pub fn send(&mut self, filepath: &str, dst_address: MacAddr, id: u16) -> io::Result<()> {
        let data_fragments = utils::split_file_into_mtu_size(filepath, self.mtu)?;
        let mut timeouts: Vec<utils::Time> = vec![utils::Time::new(); data_fragments.len()];
        let mut flags = utils::Flags::new();
        flags.set_length(data_fragments.len())?;
        let (mpsc_tx, mpsc_rx) = mpsc::channel();
        let mut datalink_tx = TX.lock().unwrap();
        {
            let flags_length = flags.get_length().unwrap();
            let src_address = self.address;
            thread::spawn(move || {
                Uft::receive_ack_task(dst_address.clone(), src_address, mpsc_tx, id, flags_length).unwrap();
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

                if utils::Time::now() < timeouts[offset].add_millis(self.rto) {
                    continue;
                }

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

                // let ether_packet = vec![0u8; ];
                // ether_packet.extend_from_slice(&packet.raw());
                let packet = packet.raw();

                datalink_tx.as_mut().unwrap().build_and_send(1, 14+packet.len(),
                    &mut |new_packet| {
                        let mut new_packet = MutableEthernetPacket::new(new_packet).unwrap();

                        // new_packet.clone_from(&ether_packet);

                        new_packet.set_source(self.address);
                        new_packet.set_destination(dst_address);

                        new_packet.set_ethertype(EtherType(0xFF));
                        new_packet.set_payload(&packet);
                    }
                );

                loop {
                    let offset: usize = if let Ok(o) = mpsc_rx.try_recv() {
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

    fn send_ack(src_address: MacAddr, dst_address: MacAddr, id: u16, offset: u16) -> io::Result<()> {
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
        
        let packet = packet.raw();
        TX.lock().unwrap().as_mut().unwrap().build_and_send(1, 14+packet.len(),
            &mut |new_packet| {
                let mut new_packet = MutableEthernetPacket::new(new_packet).unwrap();

                // new_packet.clone_from(&ether_packet);

                new_packet.set_source(src_address);
                new_packet.set_destination(dst_address);

                new_packet.set_ethertype(EtherType(0xFF));
                new_packet.set_payload(&packet);
            }
        ).ok_or_else(|| io::Error::new(io::ErrorKind::Other, "failed to send ack"))??;
        Ok(())
    }

    #[allow(unused_must_use)]
    pub fn receive_from(&mut self, src_address: MacAddr) -> io::Result<Vec<u8>> {
        let mut datalink_rx = RX.lock().unwrap();
        loop {
            match datalink_rx.as_mut().unwrap().next() {
                Ok(frame) => {
                    let frame = EthernetPacket::new(frame).unwrap();
                    if frame.get_ethertype() != EtherType(0xFF) || frame.get_source() != src_address || frame.get_destination() != self.address {
                        continue;
                    }
                    let packet = if let Ok(p) = packet::UftPacket::from_raw(frame.payload().to_vec()) {
                        p
                    } else {
                        continue
                    };

                    if self.received_files_flag.contains(&packet.header.id) {
                        Uft::send_ack(self.address, src_address, packet.header.id, packet.header.offset);
                        continue;
                    }

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

                    Uft::send_ack(self.address, src_address, packet.header.id, packet.header.offset);
                    
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
                        self.received_files_flag.insert(packet.header.id);
                        return Ok(raw_file);
                    }
                },
                Err(_) => continue,
            }
        }
    }
}