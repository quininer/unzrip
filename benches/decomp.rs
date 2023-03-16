use std::cmp;
use std::hint::black_box;
use std::io::{ self, Read };
use std::hash::{ BuildHasher, Hasher };
use std::collections::hash_map::RandomState;
use flate2::{ read, bufread };
use criterion::{criterion_group, criterion_main, Criterion};
use memutils::Buf;


struct ReadOnlyReader<'a>(pub Buf<'a>);

impl<'a> Read for ReadOnlyReader<'a> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let len = cmp::min(self.0.len(), buf.len());
        let (x, y) = self.0.split_at(len);
        memutils::slice::copy_from_slice(&mut buf[..len], x);
        self.0 = y;
        Ok(len)
    }
}

#[cfg(unzrip_i_am_nightly_and_i_want_fast)]
struct ReadOnlyNightlyReader<'a>(pub Buf<'a>);

#[cfg(unzrip_i_am_nightly_and_i_want_fast)]
impl<'a> Read for ReadOnlyNightlyReader<'a> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let len = cmp::min(self.0.len(), buf.len());
        let (x, y) = self.0.split_at(len);
        memutils::slice::copy_from_slice_nightly(&mut buf[..len], x);
        self.0 = y;
        Ok(len)
    }
}

fn bench_decomp(c: &mut Criterion) {
    macro_rules! bench {
        ( $name:expr, $input:expr ) => {
            c.bench_function(format!("deflate-buf-{}", $name).as_str(), |b| {
                b.iter(|| {
                    let input = black_box($input);
                    let mut input = bufread::DeflateDecoder::new(input);
                    let mut output = io::sink();
                    io::copy(&mut input, &mut output).unwrap();
                });
            });

            c.bench_function(format!("deflate-nobuf-{}", $name).as_str(), |b| {
                b.iter(|| {
                    let input = black_box($input);
                    let mut input = read::DeflateDecoder::new(input);
                    let mut output = io::sink();
                    io::copy(&mut input, &mut output).unwrap();
                });
            });

            c.bench_function(format!("deflate-volatile-{}", $name).as_str(), |b| {
                b.iter(|| {
                    let input = black_box($input);
                    let input = ReadOnlyReader(memutils::slice::from_slice(input));
                    let mut input = read::DeflateDecoder::new(input);
                    let mut output = io::sink();
                    io::copy(&mut input, &mut output).unwrap();
                });
            });

            #[cfg(unzrip_i_am_nightly_and_i_want_fast)]
            c.bench_function(format!("deflate-volatile-nightly-{}", $name).as_str(), |b| {
                b.iter(|| {
                    let input = black_box($input);
                    let input = ReadOnlyNightlyReader(memutils::slice::from_slice(input));
                    let mut input = read::DeflateDecoder::new(input);
                    let mut output = io::sink();
                    io::copy(&mut input, &mut output).unwrap();
                });
            });
        }
    }

    let mut output = Vec::new();

    let data = include_bytes!("../Cargo.lock");
    bufread::DeflateEncoder::new(&data[..], flate2::Compression::best())
        .read_to_end(&mut output).unwrap();
    assert!(!output.is_empty());

    bench!("text", &output[..]);


    let mut hasher = RandomState::new().build_hasher();
    let size = 3 * 1024 * 1024 + 123;
    let data: Vec<u8> = (0..size)
        .map(|n| {
            hasher.write_usize(n);
            hasher.finish().to_le_bytes()
        })
        .fold(Vec::with_capacity(size * std::mem::size_of::<u64>()), |mut buf, next| {
            buf.extend_from_slice(&next);
            buf
        });
    output.clear();
    bufread::DeflateEncoder::new(&data[..], flate2::Compression::best())
        .read_to_end(&mut output).unwrap();
    assert!(!output.is_empty());

    bench!("rand", &output[..]);
}

criterion_group!(decomp, bench_decomp);
criterion_main!(decomp);
