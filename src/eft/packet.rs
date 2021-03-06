use std::{
    io,
    mem,
};

use crate::general;
use crate::utils;

// 0                   1                   2                   3   
// 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |      Type     |     Length    |          Total Length         |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |         Identification        |             Offset            |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//
//                Example Ethernet File Transfer Header

pub enum EftType {
    Data = 0,
    DataEnd = 1,
    Ack = 2,
}

#[derive(Debug, Copy, Clone, Default)]
#[repr(C)]
pub struct EftPacketHeader {
    pub packet_type: u8,
    pub length: u8,
    pub total_length: u16,
    pub id: u16,
    pub offset: u16,
}

impl EftPacketHeader {
    pub fn from_raw(raw_header: &[u8]) -> io::Result<Self> {
        if raw_header.len() < 8 {
            return Err(io::Error::new(io::ErrorKind::Other, "parse error"));
        }
        let mut raw_header_arr: [u8; 8] = unsafe { mem::MaybeUninit::uninit().assume_init() };
        for i in 0..8 {
            raw_header_arr[i] = raw_header[i];
        }
        
        unsafe {
            Ok(mem::transmute::<[u8; 8], Self>(raw_header_arr))
        }
    }

    pub fn raw(&self) -> [u8; 8] {
        unsafe {
            mem::transmute::<Self, [u8; 8]>(self.clone())
        }
    }
}

#[derive(Clone, Default)]
pub struct EftPacket {
    pub header: EftPacketHeader,
    pub payload: Vec<u8>,
}

impl EftPacket {
    pub fn from_raw(mut raw_packet: Vec<u8>) -> io::Result<Self> {
        let header: EftPacketHeader = EftPacketHeader::from_raw(&raw_packet)?;

        if header.packet_type == EftType::Ack as u8 {
            raw_packet.resize(8, 0);
        } else {
            utils::rstrip_null(&mut raw_packet);
        }
        
        if raw_packet.len() != header.total_length as usize {
            return Err(io::Error::new(io::ErrorKind::Other, "length error"));
        }

        Ok(Self {
            header: header,
            payload: raw_packet[general::EFT_HEADER_LENGTH..].to_vec(),
        })
    }

    pub fn raw(&self) -> Vec<u8> {
        let mut raw_packet: Vec<u8> = self.header.raw().to_vec();
        raw_packet.extend_from_slice(&self.payload);
        raw_packet
    }
}