use std::{
    fs::File,
    io::{
        self,
        Read,
        BufReader,
    },
};

use crate::general;

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

    #[allow(dead_code)]
    pub fn isallset(&self) -> bool {
        if let Some(length) = self.length {
            (0..length).fold(true, |acc, access| acc && self.isset(access).unwrap())
        } else {
            false
        }
    }
}

pub fn split_file(filepath: &str, size: usize) -> io::Result<Vec<Vec<u8>>> {
    let mut data_fragments: Vec<Vec<u8>> = Vec::new();
    let mut f = BufReader::new(File::open(filepath)?);
    loop {
        let mut data_fragment: Vec<u8> = vec![0; size - general::EFT_HEADER_LENGTH];
        let data_length: usize = f.read(&mut data_fragment)?;
        if data_length == 0 {
            return Ok(data_fragments);
        }
        data_fragment.resize(data_length, 0);
        data_fragments.push(data_fragment);
    }
}

pub fn rstrip_null(bytes: &mut Vec<u8>) {
    while let Some(b) = bytes.last() {
        if *b == 0 {
            bytes.pop().unwrap();
        } else {
            break;
        }
    }
}