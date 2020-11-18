use std::{
    collections::{
        BTreeMap, BTreeSet, hash_map::Entry, HashMap,
    },
    io::self,
    sync::{
        Arc, Condvar, Mutex,
    },
    thread,
    time,
};

use pnet::{
    datalink::{
        self, Channel::Ethernet, DataLinkReceiver, DataLinkSender,
    },
    packet::{
        ethernet::{
            MutableEthernetPacket, EtherType, EthernetPacket,
        },
        Packet,
    },
    util::MacAddr,
};

pub mod packet;

use super::general;
use super::utils;

#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq)]
struct Tri {
    src: MacAddr,
    dst: MacAddr,
    fileid: u16,
}

#[derive(Default)]
struct InternalInterfaceRecvModeHandle {
    recv_manager: Mutex<RecvConnectionManager>,
    rcv_cv: Condvar,
}

type InterfaceRecvModeHandle = Arc<InternalInterfaceRecvModeHandle>;

#[derive(Clone)]
pub struct InterfaceRecvMode {
    ih: InterfaceRecvModeHandle,
    dst: MacAddr,
    // jh: thread::JoinHandle<io::Result<()>>, TODO
}

impl InterfaceRecvMode {
    pub fn stream(&mut self, fileid: u16, src: MacAddr) -> io::Result<RecvStream> {
        let mut cm = self.ih.recv_manager.lock().unwrap();
        let tri = Tri {
            src: src,
            dst: self.dst,
            fileid: fileid,
        };
        cm.connections.insert(
            tri, 
            RecvConnection {
                // ih: self.ih.clone(),
                buffer: vec![Default::default(); general::MAX_OFFSET_LENGTH],
                flag4buffer: utils::Flags::new(),
                cnt: 0,
            }
        );
        Ok(RecvStream{
            tri: tri,
            ih: self.ih.clone(),
        })
    }
}

#[derive(Default)]
struct InternalInterfaceSendModeHandle {
    send_manager: Mutex<SendConnectionManager>,
}

type InterfaceSendModeHandle = Arc<InternalInterfaceSendModeHandle>;

#[derive(Clone)]
pub struct InterfaceSendMode {
    ih: InterfaceSendModeHandle,
    src: MacAddr,
    // jh: thread::JoinHandle<io::Result<()>>, TODO
}

impl InterfaceSendMode {
    pub fn stream(&mut self, fileid: u16, dst: MacAddr) -> io::Result<SendStream> {
        let mut cm = self.ih.send_manager.lock().unwrap();
        let tri = Tri {
            src: self.src,
            dst: dst,
            fileid: fileid,
        };
        cm.connections.insert(
            tri, 
            SendConnection {
                tri: tri,
                // ih: self.ih.clone(),
                buffer: Vec::new(),
                flag4buffer: utils::Flags::new(),
                timers: Timers { send_timers: BTreeMap::new(), rto: 0, },
                retransmissions: BTreeSet::new(),
                cnt: 0,
            }
        );
        Ok(SendStream{
            tri: tri,
            ih: self.ih.clone(),
        })
    }
}

pub struct Interface {}

impl Interface {
    pub fn bind_sendmode(interface_name: &str) -> io::Result<InterfaceSendMode> {
        let interface = datalink::interfaces()
            .into_iter()
            .find(|iface| iface.name == *interface_name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "failed to get interface"))?;

        let src = interface.mac.ok_or_else(|| io::Error::new(io::ErrorKind::Other, "failed to get mac addr"))?;

        let (tx, rx) = if let Ok(Ethernet(tx, rx)) = datalink::channel(&interface, Default::default()) {
            (tx, rx)
        } else {
            return Err(io::Error::new(io::ErrorKind::Other, "failed to create channel"));
        };

        let ih: InterfaceSendModeHandle = Arc::default();

        {
            let ih = ih.clone();
            thread::spawn(move || packet_rack_loop(rx, ih.clone()));
        }
        {
            let ih = ih.clone();
            thread::spawn(move || packet_send_loop(tx, ih.clone()));
        }

        Ok(InterfaceSendMode {
            ih: ih,
            src: src,
        })
    }

    pub fn bind_recvmode(interface_name: &str) -> io::Result<InterfaceRecvMode> {
        let interface = datalink::interfaces()
            .into_iter()
            .find(|iface| iface.name == *interface_name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "failed to get interface"))?;

        let dst = interface.mac.ok_or_else(|| io::Error::new(io::ErrorKind::Other, "failed to get mac addr"))?;

        let (tx, rx) = if let Ok(Ethernet(tx, rx)) = datalink::channel(&interface, Default::default()) {
            (tx, rx)
        } else {
            return Err(io::Error::new(io::ErrorKind::Other, "failed to create channel"));
        };

        let ih: InterfaceRecvModeHandle = Arc::default();

        {
            let ih = ih.clone();
            thread::spawn(move || packet_recv_loop(tx, rx, ih.clone()));
        }

        Ok(InterfaceRecvMode {
            ih: ih,
            dst: dst,
        })
    }
}

#[derive(Default)]
struct SendConnectionManager {
    connections: HashMap<Tri, SendConnection>,
}

#[derive(Default)]
struct RecvConnectionManager {
    connections: HashMap<Tri, RecvConnection>,
}

fn send_ack(tx: &mut Box<dyn DataLinkSender + 'static>, src_address: MacAddr, dst_address: MacAddr, id: u16, offset: u16) -> io::Result<()> {
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
    tx.build_and_send(1, 14+packet.len(),
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

fn packet_rack_loop(mut rx: Box<dyn DataLinkReceiver + 'static>, ih: InterfaceSendModeHandle) -> io::Result<()> {
    loop {
        match rx.next() {
            Ok(frame) => {
                let frame = EthernetPacket::new(frame).unwrap();
                if frame.get_ethertype() != EtherType(0xEF7) {
                    continue;
                }

                let packet = if let Ok(p) = packet::EftPacket::from_raw(frame.payload().to_vec()) {
                    p
                } else {
                    continue
                };

                let mut cmg = ih.send_manager.lock().unwrap();
                let cm = &mut *cmg;
                let t = Tri {
                    src: frame.get_destination(),
                    dst: frame.get_source(),
                    fileid: packet.header.id,
                };

                match cm.connections.entry(t) {
                    Entry::Occupied(mut s) => {
                        if packet.header.packet_type != packet::EftType::Ack as u8 {
                            continue;
                        }
                        // received ack
                        if let Ok(b) = s.get_mut().on_packet(packet.header.offset) {
                            if b {
                                cm.connections.remove(&t); // ConnectionManagerから削除
                            }
                        }
                    },
                    _ => continue,
                }
            },
            Err(_) => continue,
        }
    }
}

fn packet_send_loop(mut tx: Box<dyn DataLinkSender + 'static>, ih: InterfaceSendModeHandle) {
    loop {
        let mut cmg = ih.send_manager.lock().unwrap();
        for connection in cmg.connections.values_mut() {
            connection.on_tick(&mut tx);
            connection.on_fast_retransmission(&mut tx);
        }
    }
}

struct Timers {
    send_timers: BTreeMap<u16, time::Instant>,
    rto: u32,
}

struct SendConnection {
    tri: Tri,
    // ih: InterfaceSendModeHandle,
    buffer: Vec<packet::EftPacket>,
    flag4buffer: utils::Flags,
    timers: Timers,
    retransmissions: BTreeSet<u16>,
    cnt: usize,
}

impl SendConnection {
    fn on_packet<'a>(&mut self, offset: u16) -> io::Result<bool> { // ファイルの送信が完了したか
        if self.flag4buffer.isset(offset as usize)? {
            return Ok(false);
        }
        self.flag4buffer.set(offset as usize)?;

        self.cnt += 1;

        // 高速再転送
        for access in 0..offset {
            if !self.flag4buffer.isset(access as usize)? {
                self.retransmissions.insert(access);
            }
        }

        if self.flag4buffer.get_length()? == self.cnt {
            return Ok(true);
        }

        Ok(false)
    }

    #[allow(unused_must_use)]
    fn on_tick(&mut self, tx: &mut Box<dyn DataLinkSender + 'static>) {
        for (offset, timer) in self.timers.send_timers.clone().into_iter() {
            if timer.elapsed().as_millis() > self.timers.rto as u128 {
                self.write(tx, offset);
            }
        }
    }

    #[allow(unused_must_use)]
    fn on_fast_retransmission(&mut self, tx: &mut Box<dyn DataLinkSender + 'static>) {
        for offset in self.retransmissions.clone().into_iter() {
            self.retransmissions.remove(&offset);
            self.write(tx, offset);
        }
    }

    fn write(&mut self, tx: &mut Box<dyn DataLinkSender + 'static>, offset: u16) -> io::Result<()> {
        if self.flag4buffer.isset(offset as usize)? {
            return Ok(())
        }
        let packet = self.buffer[offset as usize].raw();
        tx.build_and_send(1, 14+packet.len(),
            &mut |new_packet| {
                let mut new_packet = MutableEthernetPacket::new(new_packet).unwrap();

                new_packet.set_source(self.tri.src);
                new_packet.set_destination(self.tri.dst);
                new_packet.set_ethertype(EtherType(0xEF7));
                new_packet.set_payload(&packet);
            }
        );
        self.timers.send_timers.insert(offset, time::Instant::now());
        Ok(())
    }
}

#[allow(unused_must_use)]
fn packet_recv_loop(mut tx: Box<dyn DataLinkSender + 'static>, mut rx: Box<dyn DataLinkReceiver + 'static>, ih: InterfaceRecvModeHandle) -> io::Result<()> {
    loop {
        match rx.next() {
            Ok(frame) => {
                let frame = EthernetPacket::new(frame).unwrap();
                if frame.get_ethertype() != EtherType(0xEF7) {
                    continue;
                }

                let packet = if let Ok(p) = packet::EftPacket::from_raw(frame.payload().to_vec()) {
                    p
                } else {
                    continue
                };

                let mut cmg = ih.recv_manager.lock().unwrap();
                let cm = &mut *cmg;
                let t = Tri {
                    src: frame.get_source(),
                    dst: frame.get_destination(),
                    fileid: packet.header.id,
                };

                match cm.connections.entry(t) {
                    Entry::Occupied(mut s) => {
                        if packet.header.packet_type == packet::EftType::Ack as u8 {
                            continue;
                        }
                        if let Ok(b) = s.get_mut().on_packet(packet.header.offset, packet.header.packet_type, &packet.payload) {
                            send_ack(&mut tx, t.dst, t.src, packet.header.id, packet.header.offset);
                            if b {
                                ih.rcv_cv.notify_all() // ファイル受信完了
                            }
                        }
                    },
                    _ => continue,
                }
            },
            Err(_) => continue,
        }
    }
}

struct RecvConnection {
    // ih: InterfaceRecvModeHandle,
    buffer: Vec<Vec<u8>>,
    flag4buffer: utils::Flags,
    cnt: usize,
}

impl RecvConnection {
    fn on_packet<'a>(&mut self, offset: u16, packet_type: u8, data: &'a [u8]) -> io::Result<bool> { // ファイルが完成したか
        if self.flag4buffer.isset(offset as usize)? {
            return Ok(false);
        }
        self.flag4buffer.set(offset as usize)?;
        self.buffer[offset as usize] = data.to_vec();

        self.cnt += 1;

        if packet_type == packet::EftType::DataEnd as u8 {
            self.flag4buffer.set_length(offset as usize + 1)?;
        }

        if let Ok(l) = self.flag4buffer.get_length() {
            if self.cnt == l {
                return Ok(true);
            }
        }

        Ok(false)
    }
}

pub struct SendStream {
    tri: Tri,
    ih: InterfaceSendModeHandle,
}

impl SendStream {
    pub fn send(&mut self, filepath: &str, mtu: usize) -> io::Result<()> {
        let data_fragments = utils::split_file(filepath, mtu)?;
        let mut cm = self.ih.send_manager.lock().unwrap();
        let c = cm.connections.get_mut(&self.tri).ok_or_else(|| {
            io::Error::new(io::ErrorKind::Other, "stream was terminated unexpectedly")
        })?;

        for (offset, data_fragment) in data_fragments.iter().enumerate() {
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
                    id: self.tri.fileid,
                    offset: offset as u16,
                },
                payload: data_fragment.to_vec(),
            };
            c.buffer.push(packet);
            c.retransmissions.insert(offset as u16);
        }
        Ok(())
    }
}

pub struct RecvStream {
    tri: Tri,
    ih: InterfaceRecvModeHandle,
}

impl RecvStream {
    pub fn read(&mut self) -> io::Result<Vec<u8>> {
        let mut cm = self.ih.recv_manager.lock().unwrap();
        cm = self.ih.rcv_cv.wait(cm).unwrap();
        let c = cm.connections.get_mut(&self.tri).ok_or_else(|| {
            io::Error::new(io::ErrorKind::Other, "stream was terminated unexpectedly")
        })?;
        let raw_file: Vec<u8> = c.buffer[0..c.cnt].iter().fold(Vec::new(),
            |mut acc, f| {
                acc.extend_from_slice(f);
                acc
            }
        );
        cm.connections.remove(&self.tri);
        Ok(raw_file)
    }
}