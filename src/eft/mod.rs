use std::{
    collections::{
        BTreeMap, hash_map::Entry, HashMap,
    },
    io::self,
    sync::{
        Arc, Condvar, Mutex, mpsc,
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
    #[allow(dead_code)]
    pub fn send(&mut self, fileid: u16, dst: MacAddr, filepath: String, mtu: usize) -> io::Result<()> {
        let mut cm = self.ih.send_manager.lock().unwrap();
        let tri = Tri {
            src: self.src,
            dst: dst,
            fileid: fileid,
        };
        
        let data_fragments = utils::split_file(&filepath, mtu)?;
        let mut buffer: Vec<packet::EftPacket> = Vec::new();
        let mut send_timers: Vec<time::Instant> = Vec::new();
        let timer_init = time::Instant::now() - time::Duration::new(5, 0);
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
                    id: tri.fileid,
                    offset: offset as u16,
                },
                payload: data_fragment.to_vec(),
            };
            buffer.push(packet);
            send_timers.push(timer_init);
        }

        cm.connections.insert(
            tri, 
            SendConnection {
                tri: tri,
                // ih: self.ih.clone(),
                buffer: buffer,
                flag4buffer: utils::Flags::new(),
                timers: Timers { send_timers: send_timers, rto: 20, },
                cnt: 0,
            }
        );
        Ok(())
    }

    #[allow(dead_code)]
    pub fn send_files(&mut self, fileids: Vec<u16>, dst: MacAddr, filepaths: Vec<String>, mtu: usize) -> io::Result<()> {
        let mut cm = self.ih.send_manager.lock().unwrap();
        for i in 0..fileids.len() {
            let tri = Tri {
                src: self.src,
                dst: dst,
                fileid: fileids[i],
            };
            let data_fragments = utils::split_file(&filepaths[i], mtu)?;
            let mut buffer: Vec<packet::EftPacket> = Vec::new();
            let mut send_timers: Vec<time::Instant> = Vec::new();
            let timer_init = time::Instant::now() - time::Duration::new(5, 0);
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
                        id: tri.fileid,
                        offset: offset as u16,
                    },
                    payload: data_fragment.to_vec(),
                };
                buffer.push(packet);
                send_timers.push(timer_init);
            }

            cm.connections.insert(
                tri, 
                SendConnection {
                    tri: tri,
                    // ih: self.ih.clone(),
                    buffer: buffer,
                    flag4buffer: utils::Flags::new(),
                    timers: Timers { send_timers: send_timers, rto: 20, },
                    cnt: 0,
                }
            );
        }
        Ok(())
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
        let (mpsc_tx, mpsc_rx) = mpsc::channel();
        {
            thread::spawn(move || packet_rack_loop(rx, mpsc_tx));
        }
        {
            let ih = ih.clone();
            thread::spawn(move || packet_send_loop(tx, ih.clone(), mpsc_rx));
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

struct Message {
    tri: Tri,
    offset: u16,
}

#[allow(unused_must_use)]
fn packet_rack_loop(mut rx: Box<dyn DataLinkReceiver + 'static>, mpsc_tx: mpsc::Sender<Message>) -> io::Result<()> {
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

                let t = Tri {
                    src: frame.get_destination(),
                    dst: frame.get_source(),
                    fileid: packet.header.id,
                };

                mpsc_tx.send(Message { tri: t, offset: packet.header.offset, });
            },
            Err(_) => continue,
        }
    }
}

#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq)]
struct EndPoint {
    src: MacAddr,
    dst: MacAddr,
}

#[allow(unused_must_use)]
fn packet_send_loop(mut tx: Box<dyn DataLinkSender + 'static>, ih: InterfaceSendModeHandle, mpsc_rx: mpsc::Receiver<Message>) {
    let mut cmg = ih.send_manager.lock().unwrap();
    let cm = &mut *cmg;
    let mut fast_retransmissions: HashMap<EndPoint, BTreeMap<u16, BTreeMap<u16, bool>>> = HashMap::new();
    let mut timeout_retransmissions: HashMap<EndPoint, BTreeMap<u16, BTreeMap<u16, bool>>> = HashMap::new();
    loop {
        loop { // get fast_retransmissions
            if let Ok(m) = mpsc_rx.try_recv() {
                let c = if let Some(c) = cm.connections.get_mut(&m.tri) {
                    c
                } else {
                    continue
                };
                if let Ok((fin, fr)) = c.on_packet(m.offset) {
                    if fin { // file sent
                        cm.connections.remove(&m.tri); // ConnectionManagerから削除
                        continue;
                    }
                    if let Some(offsets) = fr {
                        for offset in offsets {
                            let e = fast_retransmissions
                                .entry(EndPoint { src: m.tri.src, dst: m.tri.dst, })
                                .or_insert(BTreeMap::new());
                            let e = e
                                .entry(m.tri.fileid)
                                .or_insert(BTreeMap::new());
                            e.insert(offset, true);
                        }
                    }
                }
            } else {
                break
            }
        }
        for connection in cm.connections.values_mut() { // get timeout packets
            for offset in connection.timeouts() {
                let e = timeout_retransmissions
                    .entry(EndPoint { src: connection.tri.src, dst: connection.tri.dst, })
                    .or_insert(BTreeMap::new());
                let e = e
                    .entry(connection.tri.fileid)
                    .or_insert(BTreeMap::new());
                e.insert(offset, true);
            }
        }
        // それぞれのインターフェースにおける先頭パケットを送信
        for fast_retransmission in fast_retransmissions.iter_mut() {
            'outer1: loop {
                for (fileid, next) in fast_retransmission.1.iter_mut() {
                    if let Some((offset, b)) = next.iter_mut().next() {
                        if *b {
                            let tri = Tri { src: fast_retransmission.0.src, dst: fast_retransmission.0.dst, fileid: *fileid, };
                            if let Some(c) = cm.connections.get_mut(&tri) {
                                c.write(&mut tx, *offset);
                                *b = false;
                                break 'outer1;
                            } else {
                                continue; // ?
                            };
                        } else {
                            continue;
                        }
                    }
                    break 'outer1;
                }
            }
        }
        for timeout_retransmission in timeout_retransmissions.iter_mut() {
            'outer2: loop {
                for (fileid, next) in timeout_retransmission.1.iter_mut() {
                    if let Some((offset, b)) = next.iter_mut().next() {
                        if *b {
                            let tri = Tri { src: timeout_retransmission.0.src, dst: timeout_retransmission.0.dst, fileid: *fileid, };
                            if let Some(c) = cm.connections.get_mut(&tri) {
                                c.write(&mut tx, *offset);
                                *b = false;
                                break 'outer2;
                            } else {
                                continue; // ?
                            };
                        } else {
                            continue;
                        }
                    }
                    break 'outer2;
                }
            }
        }
    }
}

struct Timers {
    send_timers: Vec<time::Instant>,
    rto: u32,
}

struct SendConnection {
    tri: Tri,
    // ih: InterfaceSendModeHandle,
    buffer: Vec<packet::EftPacket>,
    flag4buffer: utils::Flags,
    timers: Timers,
    cnt: usize,
}

impl SendConnection {
    fn on_packet<'a>(&mut self, offset: u16) -> io::Result<(bool, Option<Vec<u16>>)> { // ファイルの送信が完了したか
        if self.flag4buffer.isset(offset as usize)? {
            return Ok((false, None));
        }
        self.flag4buffer.set(offset as usize)?;

        self.cnt += 1;

        let mut fast_retransmissions: Vec<u16> = Vec::new();
        // 高速再転送
        for access in 0..offset {
            if !self.flag4buffer.isset(access as usize)? {
                fast_retransmissions.push(access);
            }
        }

        if self.flag4buffer.get_length()? == self.cnt {
            return Ok((true, None));
        }

        Ok((false, Some(fast_retransmissions)))
    }

    // #[allow(unused_must_use)]
    fn timeouts(&self) -> Vec<u16> {
        let mut timeouts: Vec<u16> = Vec::new();
        for (offset, timer) in self.timers.send_timers.iter().enumerate() {
            if timer.elapsed().as_millis() > self.timers.rto as u128 {
                timeouts.push(offset as u16);
            }
        }
        timeouts
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
        self.timers.send_timers[offset as usize] = time::Instant::now();
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
                                eprintln!("file received");
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

pub struct RecvStream {
    tri: Tri,
    ih: InterfaceRecvModeHandle,
}

impl RecvStream {
    pub fn read(&mut self) -> io::Result<Vec<u8>> {
        loop {
            let mut cm = self.ih.recv_manager.lock().unwrap();
            cm = self.ih.rcv_cv.wait(cm).unwrap();
            let c = cm.connections.get_mut(&self.tri).ok_or_else(|| {
                io::Error::new(io::ErrorKind::Other, "stream was terminated unexpectedly")
            })?;
            if let Ok(l) = c.flag4buffer.get_length() {
                if l != c.cnt {
                    continue;
                }
            } else {
                continue;
            }
            let raw_file: Vec<u8> = c.buffer[0..c.cnt].iter().fold(Vec::new(),
                |mut acc, f| {
                    acc.extend_from_slice(f);
                    acc
                }
            );
            // cm.connections.remove(&self.tri);
            return Ok(raw_file);
        }
    }
}