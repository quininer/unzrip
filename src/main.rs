use std::{ cmp, env, io };
use std::fs::File;
use anyhow::Context;
use camino::{ Utf8Path as Path, Utf8PathBuf as PathBuf };
use argh::FromArgs;
use rayon::prelude::*;
use memmap2::MmapOptions;
use flate2::bufread::DeflateDecoder;
use zip_parser::{ compress_method, ZipArchive, CentralFileHeader };


/// UnPiz - list, test and extract compressed files in a ZIP archive
#[derive(FromArgs)]
struct Options {
    /// path of the ZIP archive(s).
    #[argh(positional)]
    file: Vec<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    let options: Options = argh::from_env();

    let target = env::current_dir()?;
    let target = Path::from_path(&target)
        .context("must utf8 path")?;

    for file in options.file.iter() {
        unpiz(file, &target)?;
    }

    Ok(())
}

fn unpiz(path: &Path, target: &Path) -> anyhow::Result<()> {
    let fd = File::open(path)?;
    let buf = unsafe {
        MmapOptions::new().map_copy_read_only(&fd)?
    };

    let zip = ZipArchive::parse(&buf)?;
    let len: usize = zip.eocdr().cd_entries.try_into()?;
    let len = cmp::min(len, 128);

    zip.entries()?
        .try_fold(Vec::with_capacity(len), |mut acc, e| e.map(|e| {
            acc.push(e);
            acc
        }))?
        .par_iter()
        .try_for_each(|cfh| do_entry(&zip, &cfh, target))?;

    Ok(())
}

fn do_entry(zip: &ZipArchive<'_>, cfh: &CentralFileHeader<'_>, target: &Path) -> anyhow::Result<()> {
    let (_lfh, buf) = zip.read(cfh)?;

    if cfh.gp_flag & 1 != 0 {
        anyhow::bail!("encrypt is not supported");
    }

    let target = {
        let (name, ..) = encoding_rs::UTF_8.decode(cfh.name);
        let path = Path::new(&*name);

        if !path.is_relative() {
            anyhow::bail!("must relative path");
        }

        target.join(path)
    };


    let mut reader = match cfh.method {
        compress_method::STORE => Reader::None(buf),
        compress_method::DEFLATE => Reader::Deflate(DeflateDecoder::new(buf)),
        _ => anyhow::bail!("compress method is not supported: {}", cfh.method)
    };

    let mut target = File::options()
        .write(true)
        .append(true)
        .create_new(true)
        .open(&target)?;

    io::copy(&mut reader, &mut target)?;

    Ok(())
}

enum Reader<'a> {
    None(&'a [u8]),
    Deflate(DeflateDecoder<&'a [u8]>)
}

impl io::Read for Reader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Reader::None(reader) => io::Read::read(reader, buf),
            Reader::Deflate(reader) => io::Read::read(reader, buf)
        }

    }
}
