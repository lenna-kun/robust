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

struct Message {
    offset: u16,
    time: utils::Time,
}

#[derive(Clone)]
pub struct Eft {
    address: MacAddr,
    mtu: usize,
    rto: u32,
    flags_for_files: HashSet<u16>,
    buffers: HashMap<u16, Vec<Vec<u8>>>,
    flags_for_buffers: HashMap<u16, utils::Flags>,
}

impl Eft {
    pub fn new(mtu: usize, interface_name: &str) -> io::Result<Self> {
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
            address: interface.mac.ok_or_else(|| io::Error::new(io::ErrorKind::Other, "failed to get mac addr"))?,
            mtu: mtu,
            rto: 30, // ms
            flags_for_files: HashSet::new(),
            buffers: HashMap::new(),
            flags_for_buffers: HashMap::new(),
        })
    }

    #[allow(unused_must_use)]
    fn receive_ack_task(src_address: MacAddr, dst_address: MacAddr, mpsc_tx: mpsc::Sender<Message>, id: u16, flags_length: usize) -> io::Result<()> {
        let mut flags = utils::Flags::new();
        flags.set_length(flags_length)?;
        let mut datalink_rx = RX.lock().unwrap();
        while !flags.isallset() {
            match datalink_rx.as_mut().unwrap().next() {
                Ok(frame) => {
                    let frame = EthernetPacket::new(frame).unwrap();
                    if frame.get_ethertype() != EtherType(0xEF7) || frame.get_source() != src_address || frame.get_destination() != dst_address {
                        continue;
                    }
                    let packet = if let Ok(p) = packet::EftPacket::from_raw({
                        let mut raw = frame.payload().to_vec();
                        raw.resize(8, 0);
                        raw
                    }) {
                        p
                    } else {
                        continue
                    };

                    if packet.header.id == id && packet.header.packet_type == packet::EftType::Ack as u8 {
                        if let Ok(_) = flags.set(packet.header.offset as usize) {
                            mpsc_tx.send(Message { offset: packet.header.offset, time: utils::Time::now() });
                        }
                    }
                },
                Err(_) => continue,
            }
        }
        Ok(())
    }

    #[allow(unused_must_use)]
    pub fn send(&mut self, filepath: &str, dst_address: MacAddr, id: u16) -> io::Result<()> {
        let data_fragments = utils::split_file(filepath, self.mtu)?;
        let mut timeouts: Vec<utils::Time> = vec![utils::Time::new(); data_fragments.len()];
        let mut flags = utils::Flags::new();
        flags.set_length(data_fragments.len())?;
        let (mpsc_tx, mpsc_rx) = mpsc::channel();
        let mut datalink_tx = TX.lock().unwrap();

        {
            let flags_length = flags.get_length().unwrap();
            let src_address = self.address;
            thread::spawn(move || {
                Eft::receive_ack_task(dst_address.clone(), src_address, mpsc_tx, id, flags_length).unwrap();
            });
        }

        while !flags.isallset() {
            for (offset, data_fragment) in data_fragments.iter().enumerate() {
                if flags.isset(offset)? || utils::Time::now() < timeouts[offset].add_millis(self.rto) {
                    continue;
                }

                let packet = packet::EftPacket {
                    header: packet::EftPacketHeader {
                        packet_type:
                            if data_fragments.len() - 1 == offset {
                                packet::EftType::DataEnd as u8
                            } else {
                                packet::EftType::Data as u8
                            },
                        length: 8,
                        total_length: data_fragment.len() as u16 + 8,
                        id: id,
                        offset: offset as u16,
                    },
                    payload: data_fragment.to_vec(),
                };

                timeouts[offset] = utils::Time::now();

                let packet = packet.raw();
                datalink_tx.as_mut().unwrap().build_and_send(1, 14+packet.len(),
                    &mut |new_packet| {
                        let mut new_packet = MutableEthernetPacket::new(new_packet).unwrap();

                        new_packet.set_source(self.address);
                        new_packet.set_destination(dst_address);
                        new_packet.set_ethertype(EtherType(0xEF7));
                        new_packet.set_payload(&packet);
                    }
                );

                {
                    let mut wrap: Option<Message> = None;
                    loop {
                        wrap = if let Ok(w) = mpsc_rx.try_recv() {
                            flags.set(offset as usize);
                            Some(w)
                        } else {
                            break
                        };
                    }
                    if let Some(w) = wrap {
                        self.rto = w.time.millis_sub(&timeouts[w.offset as usize]);
                    }
                }
            }
        }
        Ok(())
    }

    fn send_ack(src_address: MacAddr, dst_address: MacAddr, id: u16, offset: u16) -> io::Result<()> {
        let packet = packet::EftPacket {
            header: packet::EftPacketHeader {
                packet_type: packet::EftType::Ack as u8,
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

                new_packet.set_source(src_address);
                new_packet.set_destination(dst_address);
                new_packet.set_ethertype(EtherType(0xEF7));
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
                    if frame.get_ethertype() != EtherType(0xEF7) || frame.get_source() != src_address || frame.get_destination() != self.address {
                        continue;
                    }
                    let packet = if let Ok(p) = packet::EftPacket::from_raw(utils::rstrip_null(frame.payload().to_vec())) {
                        p
                    } else {
                        continue
                    };

                    if self.flags_for_files.contains(&packet.header.id) {
                        Eft::send_ack(self.address, src_address, packet.header.id, packet.header.offset);
                        continue;
                    }

                    let buf = self.buffers
                        .entry(packet.header.id)
                        .or_insert(vec![Vec::new(); general::MAX_OFFSET_LENGTH]);
                    let flg4buf = self.flags_for_buffers.entry(packet.header.id).or_insert(utils::Flags::new());
                    if let Err(_) = flg4buf.set(packet.header.offset as usize) {
                        continue;
                    }
                    buf[packet.header.offset as usize] = packet.payload;

                    if packet.header.packet_type == packet::EftType::DataEnd as u8 {
                        flg4buf.set_length(packet.header.offset as usize + 1).unwrap();
                    }

                    Eft::send_ack(self.address, src_address, packet.header.id, packet.header.offset);
                    
                    if flg4buf.isallset() {
                        let buflen = if let Ok(length) = flg4buf.get_length() {
                            length
                        } else {
                            continue
                        };
                        let raw_file: Vec<u8> = buf[0..buflen].iter().fold(Vec::new(),
                            |mut acc, f| {
                                acc.extend_from_slice(f);
                                acc
                            }
                        );
                        self.flags_for_files.insert(packet.header.id);
                        return Ok(raw_file);
                    }
                },
                Err(_) => continue,
            }
        }
    }
}