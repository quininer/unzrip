use std::{ io, fs };
use anyhow::Context;
use flate2::bufread::DeflateDecoder;
use zstd::stream::read::Decoder as ZstdDecoder;
use camino::{ Utf8Path as Path, Utf8PathBuf as PathBuf, Utf8Component as Component };


pub enum Decoder<R: io::BufRead> {
    None(R),
    Deflate(DeflateDecoder<R>),
    Zstd(ZstdDecoder<'static, R>)
}

impl<R: io::BufRead> io::Read for Decoder<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Decoder::None(reader) => io::Read::read(reader, buf),
            Decoder::Deflate(reader) => io::Read::read(reader, buf),
            Decoder::Zstd(reader) => io::Read::read(reader, buf)
        }
    }
}

pub struct Crc32Checker<R> {
    reader: R,
    expect: u32,
    hasher: crc32fast::Hasher,
}

impl<R> Crc32Checker<R> {
    pub fn new(reader: R, expect: u32) -> Crc32Checker<R> {
        Crc32Checker {
            reader, expect,
            hasher: crc32fast::Hasher::new()
        }
    }
}

impl<R: io::Read> io::Read for Crc32Checker<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = io::Read::read(&mut self.reader, buf)?;

        if n == 0 {
            let crc = self.hasher.clone().finalize();
            if crc != self.expect {
                let msg = format!("crc32 check failed. expect: {}, got: {}",
                    self.expect,
                    crc
                );
                return Err(io::Error::new(io::ErrorKind::InvalidData, msg))
            }
        } else {
            self.hasher.update(&buf[..n]);
        }

        Ok(n)
    }
}

pub fn dos2time(dos_date: u16, dos_time: u16)
    -> anyhow::Result<time::PrimitiveDateTime>
{
    let sec = (dos_time & 0x1f) * 2;
    let min = (dos_time >> 5) & 0x3f;
    let hour = dos_time >> 11;

    let day = dos_date & 0x1f;
    let mon = (dos_date >> 5) & 0xf;
    let year = (dos_date >> 9) + 1980;

    let mon: u8 = mon.try_into().context("mon cast")?;
    let mon: time::Month = mon.try_into()?;

    let time = time::Time::from_hms(
        hour.try_into().context("hour cast")?,
        min.try_into().context("min cast")?,
        sec.try_into().context("sec cast")?
    )?;
    let date = time::Date::from_calendar_date(
        year.try_into().context("year cast")?,
        mon,
        day.try_into().context("day cast")?
    )?;

    Ok(date.with_time(time))
}

pub fn path_join(base: &Path, path: &Path) -> PathBuf {
    let (_, path) = path.components()
        .fold((0, base.to_path_buf()), |(mut depth, mut sum), next| {
            match next {
                Component::Normal(p) => {
                    sum.push(p);
                    depth += 1;
                },
                Component::ParentDir if depth > 0 => if sum.pop() {
                    depth -= 1;
                },
                _ => ()
            };
            (depth, sum)
        });
    path
}

pub fn path_open(path: &Path) -> io::Result<fs::File> {
    let mut open_options = fs::File::options();
    open_options.write(true).append(true).create_new(true);

    match open_options.open(path) {
        Ok(fd) => Ok(fd),
        Err(err) => {
            // parent dir not found
            if err.kind() == io::ErrorKind::NotFound {
                if let Some(dir) = path.parent() {
                    fs::create_dir_all(dir)
                        .or_else(|err| if err.kind() == io::ErrorKind::AlreadyExists {
                            Ok(())
                        } else {
                            Err(err)
                        })?;
                    return open_options.open(path);
                }
            }

            Err(err)
        }
    }
}
