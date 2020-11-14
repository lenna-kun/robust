use std::{
    io,
    mem,
};

use crate::general;

// 0                   1                   2                   3   
// 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |      Type     |     Length    |          Total Length         |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |         Identification        |             Offset            |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//
//                 Example UDP File Transfer Header

pub enum UftType {
    Data = 0,
    DataEnd = 1,
    Ack = 2,
}

#[derive(Debug, Copy, Clone)]
#[repr(C)]
pub struct UftPacketHeader {
    pub uft_type: u8,
    pub length: u8,
    pub total_length: u16,
    pub id: u16,
    pub offset: u16,
}

impl UftPacketHeader {
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

pub struct UftPacket {
    pub header: UftPacketHeader,
    pub payload: Vec<u8>,
}

impl UftPacket {
    pub fn from_raw(raw_packet: &[u8]) -> io::Result<Self> {
        let header: UftPacketHeader = UftPacketHeader::from_raw(raw_packet)?;
        
        if raw_packet.len() != header.total_length as usize {
            return Err(io::Error::new(io::ErrorKind::Other, "length error"));
        }

        Ok(Self {
            header: header,
            payload: raw_packet[general::UFT_HEADER_LENGTH..].to_vec(),
        })
    }

    pub fn raw(&self) -> Vec<u8> {
        let mut raw_packet: Vec<u8> = self.header.raw().to_vec();
        raw_packet.extend_from_slice(&self.payload);
        raw_packet
    }
}