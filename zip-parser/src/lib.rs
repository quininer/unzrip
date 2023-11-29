//! https://www.hanshq.net/zip.html#zip
//! https://pkware.cachefly.net/webdocs/casestudies/APPNOTE.TXT

mod util;

use thiserror::Error;
use util::{ Eof, take, read_u16, read_u32, read_u64, rfind };
use memutils::Buf;


pub mod compress {
    pub const STORE: u16   = 0;
    pub const DEFLATE: u16 = 8;
    pub const ZSTD: u16    = 93;
}

pub mod system {
    pub const DOS: u16 = 0;
    pub const UNIX: u16 = 3;
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("eof")]
    Eof,
    #[error("bad eocdr magic number")]
    BadEocdr,
    #[error("bad cfh magic number")]
    BadCfh,
    #[error("bad lfh magic number")]
    BadLfh,
    #[error("not supported")]
    Unsupported,
    #[error("offset overflow")]
    OffsetOverflow,
}

impl From<Eof> for Error {
    #[inline]
    fn from(_err: Eof) -> Error {
        Error::Eof
    }
}

pub enum EocdRecord<'buf> {
    Zip(EocdRecord32<'buf>),
    Zip64(EocdRecord64<'buf>)
}

impl EocdRecord<'_> {
    pub fn cd_offset(&self) -> Option<usize> {
        match self {
            EocdRecord::Zip(eocdr) => eocdr.cd_offset.try_into().ok(),
            EocdRecord::Zip64(eocdr) => eocdr.cd_offset.try_into().ok()
        }
    }

    pub fn cd_entries(&self) -> Option<usize> {
        match self {
            EocdRecord::Zip(eocdr) => eocdr.cd_entries.try_into().ok(),
            EocdRecord::Zip64(eocdr) => eocdr.cd_entries.try_into().ok()
        }
    }
}

/*
      end of central dir signature    4 bytes  (0x06054b50)
      number of this disk             2 bytes
      number of the disk with the start of the central directory  2 bytes
      total number of entries in the central directory on this disk  2 bytes
      total number of entries in the central directory           2 bytes
      size of the central directory   4 bytes
      offset of start of central directory with respect to the starting disk number        4 bytes
      .ZIP file comment length        2 bytes
      .ZIP file comment       (variable size)
 */
pub struct EocdRecord32<'a> {
    pub sig: u32,
    pub disk_nbr: u16,
    pub cd_start_disk: u16,
    pub disk_cd_entries: u16,
    pub cd_entries: u16,
    pub cd_size: u32,
    pub cd_offset: u32,
    pub comment: Buf<'a>
}

impl EocdRecord32<'_> {
    fn find(buf: Buf<'_>) -> Result<(EocdRecord32<'_>, usize), Error> {
        const EOCDR_SIGNATURE: &[u8; 4] = &[b'P', b'K', 5, 6];
        const MAX_BACK_OFFSET: usize = 1024 * 128; // > eocdr size + u16::MAX

        debug_assert_eq!(EOCDR_SIGNATURE.len(), std::mem::size_of::<u32>());

        let eocdr_buf = {
            let max_back_buf = buf.len()
                .checked_sub(MAX_BACK_OFFSET)
                .map(|pos| &buf[pos..])
                .unwrap_or(buf);

            let eocdr_offset = rfind(max_back_buf, EOCDR_SIGNATURE)
                .ok_or(Error::BadEocdr)?;
            &max_back_buf[eocdr_offset..]
        };

        let input = eocdr_buf;
        let (input, sig) = read_u32(input)?;
        let (input, disk_nbr) = read_u16(input)?;
        let (input, cd_start_disk) = read_u16(input)?;
        let (input, disk_cd_entries) = read_u16(input)?;
        let (input, cd_entries) = read_u16(input)?;
        let (input, cd_size) = read_u32(input)?;
        let (input, cd_offset) = read_u32(input)?;
        let (input, comment_len) = read_u16(input)?;
        let (_input, comment) = take(input, comment_len.into())?;

        let eocdr = EocdRecord32 {
            sig,
            disk_nbr,
            cd_start_disk,
            disk_cd_entries,
            cd_entries,
            cd_size,
            cd_offset,
            comment
        };
        let offset = buf.len() - eocdr_buf.len();

        Ok((eocdr, offset))
    }
}

/*

        zip64 end of central dir
        signature                       4 bytes  (0x06064b50)
        size of zip64 end of central
        directory record                8 bytes
        version made by                 2 bytes
        version needed to extract       2 bytes
        number of this disk             4 bytes
        number of the disk with the start of the central directory  4 bytes
        total number of entries in the central directory on this disk  8 bytes
        total number of entries in the central directory               8 bytes
        size of the central directory   8 bytes
        offset of start of central directory with respect to the starting disk number        8 bytes
        zip64 extensible data sector    (variable size)
 */
pub struct EocdRecord64<'buf> {
    pub sig: u32,
    pub size: usize,
    pub made_by_ver: u16,
    pub extract_ver: u16,
    pub disk_nbr: u32,
    pub cd_start_disk: u32,
    pub disk_cd_entries: u64,
    pub cd_entries: u64,
    pub cd_size: u64,
    pub cd_offset: u64,
    pub ext_data_sector: Buf<'buf>
}

/*
   4.3.15 Zip64 end of central directory locator

      zip64 end of central dir locator
      signature                       4 bytes  (0x07064b50)
      number of the disk with the
      start of the zip64 end of
      central directory               4 bytes
      relative offset of the zip64
      end of central directory record 8 bytes
      total number of disks           4 bytes
 */
#[allow(dead_code)]
struct EocdRecord64Locator {
    sig: u32,
    start_disk_nbr: u32,
    offset: u64,
    total_disk_nbr: u32
}

impl<'buf> EocdRecord64<'buf> {
    fn find(buf: Buf<'buf>, eocdr_offset: usize) -> Result<EocdRecord64<'buf>, Error> {
        const EOCDR_SIGNATURE: &[u8; 4] = &[b'P', b'K', 6, 6];
        const LOCATOR_SIGNATURE: &[u8; 4] = &[b'P', b'K', 6, 7];
        const LOCATOR_LEN: usize =
            std::mem::size_of::<u32>() +
            std::mem::size_of::<u32>() +
            std::mem::size_of::<u64>() +
            std::mem::size_of::<u32>();

        let locator_buf = {
            let locator_offset = eocdr_offset.checked_sub(LOCATOR_LEN)
                .ok_or(Error::OffsetOverflow)?;

            buf.get(locator_offset..)
                .filter(|buf| &buf[..LOCATOR_SIGNATURE.len()] == LOCATOR_SIGNATURE)
                .ok_or(Error::BadEocdr)?
        };

        let locator = {
            let input = locator_buf;
            let (input, sig) = read_u32(input)?;
            let (input, start_disk_nbr) = read_u32(input)?;
            let (input, offset) = read_u64(input)?;
            let (_, total_disk_nbr) = read_u32(input)?;

            EocdRecord64Locator { sig, start_disk_nbr, offset, total_disk_nbr }
        };

        let eocdr_buf = {
            let offset: usize = locator.offset.try_into()
                .map_err(|_| Error::OffsetOverflow)?;
            buf.get(offset..)
                .filter(|buf| &buf[..EOCDR_SIGNATURE.len()] == EOCDR_SIGNATURE)
                .ok_or(Error::BadEocdr)?
        };

        let input = eocdr_buf;
        let (input, sig) = read_u32(input)?;
        let (input, size) = read_u64(input)?;
        let size: usize = size.try_into()
            .map_err(|_| Error::OffsetOverflow)?;
        let input = input.get(..size)
            .ok_or(Error::BadEocdr)?;
        let (input, made_by_ver) = read_u16(input)?;
        let (input, extract_ver) = read_u16(input)?;
        let (input, disk_nbr) = read_u32(input)?;
        let (input, cd_start_disk) = read_u32(input)?;
        let (input, disk_cd_entries) = read_u64(input)?;
        let (input, cd_entries) = read_u64(input)?;
        let (input, cd_size) = read_u64(input)?;
        let (input, cd_offset) = read_u64(input)?;

        Ok(EocdRecord64 {
            sig,
            size,
            made_by_ver,
            extract_ver,
            disk_nbr,
            cd_start_disk,
            disk_cd_entries,
            cd_entries,
            cd_size,
            cd_offset,
            ext_data_sector: input
        })
    }
}

struct ExtensibleDataField<'a> {
    id: u16,
    data: Buf<'a>
}

impl ExtensibleDataField<'_> {
    fn read(input: Buf<'_>) -> Result<(Buf<'_>, ExtensibleDataField<'_>), Error> {
        let (input, id) = read_u16(input)?;
        let (input, len) = read_u16(input)?;
        let (input, data) = take(input, len.into())?;
        Ok((input, ExtensibleDataField { id, data }))
    }
}

pub struct CentralFileHeader<'a> {
    pub made_by_ver: u16,
    pub extract_ver: u16,
    pub gp_flag: u16,
    pub method: u16,
    pub mod_time: u16,
    pub mod_date: u16,
    pub crc32: u32,
    pub comp_size: u64,
    pub uncomp_size: u64,
    pub disk_nbr_start: u16,
    pub int_attrs: u16,
    pub ext_attrs: u32,
    pub lfh_offset: u64,
    pub name: Buf<'a>,
    pub extra: Buf<'a>,
    pub comment: Buf<'a>
}

impl CentralFileHeader<'_> {
    fn parse(input: Buf<'_>, is_zip64: bool)
        -> Result<(Buf<'_>, CentralFileHeader<'_>), Error>
    {
        const CFH_SIGNATURE: &[u8; 4] = &[b'P', b'K', 1, 2];
        const ID_ZIP64_EXTENDED_INFO: u16 = 0x0001;

        let (input, expect_sig) = take(input, CFH_SIGNATURE.len())?;
        if expect_sig != CFH_SIGNATURE {
            return Err(Error::BadCfh);
        }

        let (input, made_by_ver) = read_u16(input)?;
        let (input, extract_ver) = read_u16(input)?;
        let (input, gp_flag) = read_u16(input)?;
        let (input, method) = read_u16(input)?;
        let (input, mod_time) = read_u16(input)?;
        let (input, mod_date) = read_u16(input)?;
        let (input, crc32) = read_u32(input)?;
        let (input, comp_size) = read_u32(input)?;
        let (input, uncomp_size) = read_u32(input)?;
        let (input, name_len) = read_u16(input)?;
        let (input, extra_len) = read_u16(input)?;
        let (input, comment_len) = read_u16(input)?;
        let (input, disk_nbr_start) = read_u16(input)?;
        let (input, int_attrs) = read_u16(input)?;
        let (input, ext_attrs) = read_u32(input)?;
        let (input, lfh_offset) = read_u32(input)?;
        let (input, name) = take(input, name_len.into())?;
        let (input, extra) = take(input, extra_len.into())?;
        let (input, comment) = take(input, comment_len.into())?;

        let mut zip64_extbuf = {
            let mut input = extra;
            loop {
                if input.is_empty() {
                    break None
                }

                let (input2, field) = ExtensibleDataField::read(extra)?;
                input = input2;

                if field.id == ID_ZIP64_EXTENDED_INFO {
                    break Some(field.data);
                }
            }
        };

        macro_rules! checkext {
            ( $value:expr ) => {
                match (zip64_extbuf, $value) {
                    (Some(input), u32::MAX) if is_zip64 => {
                        let (input, n) = read_u64(input)?;
                        zip64_extbuf = Some(input);
                        n
                    },
                    (_, n) => n.into()
                }
            }
        }

        let uncomp_size = checkext!(uncomp_size);
        let comp_size = checkext!(comp_size);
        let lfh_offset = checkext!(lfh_offset);

        // more ?
        let _zip64_extbuf = zip64_extbuf;

        let header = CentralFileHeader {
            made_by_ver,
            extract_ver,
            gp_flag,
            method,
            mod_time,
            mod_date,
            crc32,
            comp_size,
            uncomp_size,
            disk_nbr_start,
            int_attrs,
            ext_attrs,
            lfh_offset,
            name,
            extra,
            comment
        };

        Ok((input, header))
    }
}

pub struct LocalFileHeader<'a> {
    pub extract_ver: u16,
    pub gp_flag: u16,
    pub method: u16,
    pub mod_time: u16,
    pub mod_date: u16,
    pub crc32: u32,
    pub comp_size: u32,
    pub uncomp_size: u32,
    pub name: Buf<'a>,
    pub extra: Buf<'a>
}

impl LocalFileHeader<'_> {
    fn parse(input: Buf<'_>) -> Result<(Buf<'_>, LocalFileHeader<'_>), Error> {
        const LFH_SIGNATURE: &[u8; 4] = &[b'P', b'K', 3, 4];

        let (input, expect_sig) = take(input, LFH_SIGNATURE.len())?;
        if expect_sig != LFH_SIGNATURE {
            return Err(Error::BadLfh);
        }

        let (input, extract_ver) = read_u16(input)?;
        let (input, gp_flag) = read_u16(input)?;
        let (input, method) = read_u16(input)?;
        let (input, mod_time) = read_u16(input)?;
        let (input, mod_date) = read_u16(input)?;
        let (input, crc32) = read_u32(input)?;
        let (input, comp_size) = read_u32(input)?;
        let (input, uncomp_size) = read_u32(input)?;
        let (input, name_len) = read_u16(input)?;
        let (input, extra_len) = read_u16(input)?;
        let (input, name) = take(input, name_len.into())?;
        let (input, extra) = take(input, extra_len.into())?;

        let header = LocalFileHeader {
            extract_ver,
            gp_flag,
            method,
            mod_time,
            mod_date,
            crc32,
            comp_size,
            uncomp_size,
            name,
            extra
        };

        Ok((input, header))
    }
}

pub struct ZipArchive<'a> {
    buf: Buf<'a>,
    eocdr: EocdRecord<'a>
}

impl ZipArchive<'_> {
    pub fn parse(buf: Buf<'_>) -> Result<ZipArchive<'_>, Error> {
        let (eocdr, eocdr_offset) = EocdRecord32::find(buf)?;

        if eocdr.disk_cd_entries != eocdr.cd_entries {
            return Err(Error::Unsupported);
        }

        let eocdr = if eocdr.cd_offset != u32::MAX {
            EocdRecord::Zip(eocdr)
        } else {
            EocdRecord64::find(buf, eocdr_offset).map(EocdRecord::Zip64)?
        };

        Ok(ZipArchive { buf, eocdr })
    }

    pub fn eocdr(&self) -> &EocdRecord<'_> {
        &self.eocdr
    }

    pub fn entries(&self) -> Result<ZipEntries<'_>, Error> {
        let offset= self.eocdr.cd_offset()
            .ok_or(Error::OffsetOverflow)?;
        let buf = self.buf.get(offset..)
            .ok_or(Error::Eof)?;
        let count = self.eocdr.cd_entries()
            .ok_or(Error::OffsetOverflow)?;
        let is_zip64 = matches!(self.eocdr, EocdRecord::Zip64(_));

        Ok(ZipEntries { buf, count, is_zip64 })
    }

    pub fn read<'a>(&'a self, cfh: &CentralFileHeader) -> Result<(LocalFileHeader<'a>, Buf<'_>), Error> {
        let offset: usize = cfh.lfh_offset.try_into()
            .map_err(|_| Error::OffsetOverflow)?;
        let buf = self.buf.get(offset..).ok_or(Error::Eof)?;

        let (input, lfh) = LocalFileHeader::parse(buf)?;

        let size: usize = cfh.comp_size.try_into()
            .map_err(|_| Error::OffsetOverflow)?;
        let (_, buf) = take(input, size)?;

        Ok((lfh, buf))
    }
}

pub struct ZipEntries<'a> {
    buf: Buf<'a>,
    count: usize,
    is_zip64: bool
}

impl<'a> Iterator for ZipEntries<'a> {
    type Item = Result<CentralFileHeader<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let new_count = self.count.checked_sub(1)?;

        let input = self.buf;
        let (input, cfh) = match CentralFileHeader::parse(input, self.is_zip64) {
            Ok(output) => output,
            Err(err) => return Some(Err(err))
        };

        self.buf = input;
        self.count = new_count;

        Some(Ok(cfh))
    }
}
