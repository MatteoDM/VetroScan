// SPDX-License-Identifier: GPL-2.0-or-later
//! Minimal reader for classic libpcap files.

use crate::{Error, Result};

/// Link type written by USBPcap.
pub const LINKTYPE_USBPCAP: u32 = 249;

const GLOBAL_HEADER_LEN: usize = 24;
const RECORD_HEADER_LEN: usize = 16;

/// A parsed pcap file, borrowing its bytes.
#[derive(Debug)]
pub struct Pcap<'a> {
    bytes: &'a [u8],
    big_endian: bool,
    nanos: bool,
    /// Link-layer type of every record.
    pub link_type: u32,
}

/// One captured packet.
#[derive(Debug, Clone, Copy)]
pub struct Record<'a> {
    /// Capture time, in seconds since the Unix epoch.
    pub timestamp: f64,
    /// Captured bytes (may be shorter than the original packet).
    pub data: &'a [u8],
}

impl<'a> Pcap<'a> {
    /// Parses the global header.
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        let head = bytes
            .get(..GLOBAL_HEADER_LEN)
            .ok_or_else(|| Error::Format("file shorter than the pcap header".into()))?;
        let magic = [head[0], head[1], head[2], head[3]];
        let (big_endian, nanos) = match magic {
            [0xd4, 0xc3, 0xb2, 0xa1] => (false, false),
            [0xa1, 0xb2, 0xc3, 0xd4] => (true, false),
            [0x4d, 0x3c, 0xb2, 0xa1] => (false, true),
            [0xa1, 0xb2, 0x3c, 0x4d] => (true, true),
            [0x0a, 0x0d, 0x0d, 0x0a] => {
                return Err(Error::Format(
                    "pcapng is not supported, save as classic pcap".into(),
                ))
            }
            _ => return Err(Error::Format(format!("unknown magic {magic:02x?}"))),
        };
        let mut pcap = Self {
            bytes,
            big_endian,
            nanos,
            link_type: 0,
        };
        pcap.link_type = pcap.u32_at(20);
        Ok(pcap)
    }

    fn u32_at(&self, offset: usize) -> u32 {
        let b = [
            self.bytes[offset],
            self.bytes[offset + 1],
            self.bytes[offset + 2],
            self.bytes[offset + 3],
        ];
        if self.big_endian {
            u32::from_be_bytes(b)
        } else {
            u32::from_le_bytes(b)
        }
    }

    /// Iterates over the records. A truncated last record ends the iteration
    /// with an error.
    pub fn records(&self) -> Records<'_, 'a> {
        Records {
            pcap: self,
            offset: GLOBAL_HEADER_LEN,
        }
    }
}

/// Iterator over the records of a [`Pcap`].
#[derive(Debug)]
pub struct Records<'p, 'a> {
    pcap: &'p Pcap<'a>,
    offset: usize,
}

impl<'a> Iterator for Records<'_, 'a> {
    type Item = Result<Record<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        let bytes = self.pcap.bytes;
        if self.offset >= bytes.len() {
            return None;
        }
        if self.offset + RECORD_HEADER_LEN > bytes.len() {
            self.offset = bytes.len();
            return Some(Err(Error::Format("truncated record header".into())));
        }
        let secs = self.pcap.u32_at(self.offset);
        let frac = self.pcap.u32_at(self.offset + 4);
        let incl = self.pcap.u32_at(self.offset + 8) as usize;
        let start = self.offset + RECORD_HEADER_LEN;
        let Some(data) = bytes.get(start..start + incl) else {
            self.offset = bytes.len();
            return Some(Err(Error::Format("truncated record data".into())));
        };
        self.offset = start + incl;
        let scale = if self.pcap.nanos { 1e-9 } else { 1e-6 };
        Some(Ok(Record {
            timestamp: f64::from(secs) + f64::from(frac) * scale,
            data,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(records: &[(u32, u32, &[u8])]) -> Vec<u8> {
        let mut v = vec![0xd4, 0xc3, 0xb2, 0xa1, 2, 0, 4, 0];
        v.extend_from_slice(&[0; 8]);
        v.extend_from_slice(&65535u32.to_le_bytes());
        v.extend_from_slice(&LINKTYPE_USBPCAP.to_le_bytes());
        for (s, us, data) in records {
            v.extend_from_slice(&s.to_le_bytes());
            v.extend_from_slice(&us.to_le_bytes());
            v.extend_from_slice(&(data.len() as u32).to_le_bytes());
            v.extend_from_slice(&(data.len() as u32).to_le_bytes());
            v.extend_from_slice(data);
        }
        v
    }

    #[test]
    fn reads_records() {
        let bytes = file(&[(10, 500_000, &[1, 2, 3]), (11, 0, &[])]);
        let pcap = Pcap::parse(&bytes).unwrap();
        assert_eq!(pcap.link_type, LINKTYPE_USBPCAP);
        let recs: Vec<_> = pcap.records().collect::<Result<_>>().unwrap();
        assert_eq!(recs.len(), 2);
        assert!((recs[0].timestamp - 10.5).abs() < 1e-9);
        assert_eq!(recs[0].data, &[1, 2, 3]);
        assert!(recs[1].data.is_empty());
    }

    #[test]
    fn truncated_record_is_an_error() {
        let mut bytes = file(&[(1, 0, &[1, 2, 3, 4])]);
        bytes.truncate(bytes.len() - 2);
        let pcap = Pcap::parse(&bytes).unwrap();
        let recs: Vec<_> = pcap.records().collect();
        assert_eq!(recs.len(), 1);
        assert!(recs[0].is_err());
    }

    #[test]
    fn rejects_pcapng() {
        let mut bytes = vec![0x0a, 0x0d, 0x0d, 0x0a];
        bytes.resize(24, 0);
        assert!(Pcap::parse(&bytes).is_err());
    }
}
