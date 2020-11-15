use chrono::{
    DateTime,
    Utc
};

use std::{
    cmp::Ordering,
    fs::File,
    io::{
        self,
        Read,
        BufReader,
    },
};

use crate::general;

#[derive(Eq, Copy, Clone)]
pub struct Time {
    secs: i64,
    millis: u32,
}

impl Time {
    pub fn new() -> Self {
        Self {
            secs: 0,
            millis: 0,
        }
    }

    pub fn now() -> Self {
        let dt: DateTime<Utc> = Utc::now();
        Self {
            secs: dt.timestamp(),
            millis: dt.timestamp_subsec_millis(),
        }
    }

    pub fn add_millis(&self, millis: u32) -> Self {
        let millis: u32 = millis + self.millis;
        Self {
            secs: self.secs + (millis as i64 / 1000),
            millis: millis % 1000,
        }
    }

    pub fn millis_sub(&self, other: &Self) -> u32 {
        ((self.secs as u64 * 1000 + self.millis as u64) - (other.secs as u64 * 1000 + other.millis as u64)) as u32
    }
}

impl Ord for Time {
    fn cmp(&self, other: &Self) -> Ordering {
        if self.secs == other.secs && self.millis == other.millis {
            Ordering::Equal
        } else if self.secs < other.secs || (self.secs == other.secs && self.millis < other.millis) {
            Ordering::Less
        } else {
            Ordering::Greater
        }
    }
}

impl PartialOrd for Time {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Time {
    fn eq(&self, other: &Self) -> bool {
        self.secs == other.secs && self.millis == other.millis
    }
}

#[derive(Debug, Copy, Clone)]
pub struct Flags {
    flags: [u32; 10],
    length: Option<usize>,
}

impl Flags {
    pub fn new() -> Self {
        Self {
            flags: [0; 10],
            length: None,
        }
    }

    pub fn set_length(&mut self, length: usize) -> io::Result<()> {
        if length > general::MAX_OFFSET_LENGTH {
            return Err(io::Error::new(io::ErrorKind::Other, "offset error"));
        }
        self.length = Some(length);
        Ok(())
    }

    pub fn get_length(&self) -> io::Result<usize> {
        match self.length {
            Some(length) => Ok(length),
            None => Err(io::Error::new(io::ErrorKind::Other, "flags error")),
        }
    }
    
    pub fn set(&mut self, access: usize) -> io::Result<()> {
        if access >= general::MAX_OFFSET_LENGTH {
            return Err(io::Error::new(io::ErrorKind::Other, "offset error"));
        }
        self.flags[access / 32] |= 1 << (access % 32);
        Ok(())
    }

    pub fn isset(&self, access: usize) -> io::Result<bool> {
        if access >= general::MAX_OFFSET_LENGTH {
            return Err(io::Error::new(io::ErrorKind::Other, "offset error"));
        }
        Ok(((self.flags[access / 32] >> (access % 32)) & 0b1) == 0b1)
    }

    pub fn isallset(&self) -> bool {
        if let Some(length) = self.length {
            (0..length).fold(true, |acc, access| acc && self.isset(access).unwrap())
        } else {
            false
        }
    }
}

pub fn split_file_into_mtu_size(filepath: &str, mtu: usize) -> io::Result<Vec<Vec<u8>>> {
    let mut data_fragments: Vec<Vec<u8>> = Vec::new();
    let mut f = BufReader::new(File::open(filepath)?);
    loop {
        let mut data_fragment: Vec<u8> = vec![0; mtu - general::UFT_HEADER_LENGTH]; //  - general::IP_HEADER_LENGTH - general::UDP_HEADER_LENGTH - general::UFT_HEADER_LENGTH
        let data_length: usize = f.read(&mut data_fragment)?;
        if data_length == 0 {
            return Ok(data_fragments);
        }
        data_fragment.resize(data_length, 0);
        data_fragments.push(data_fragment);
    }
}