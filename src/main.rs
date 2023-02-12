mod util;

use std::{ cmp, env, io };
use std::fs::File;
use std::borrow::Cow;
use anyhow::Context;
use camino::{ Utf8Path as Path, Utf8PathBuf as PathBuf };
use argh::FromArgs;
use rayon::prelude::*;
use memmap2::MmapOptions;
use flate2::bufread::DeflateDecoder;
use zstd::stream::read::Decoder as ZstdDecoder;
use chardetng::EncodingDetector;
use zip_parser::{ compress, ZipArchive, CentralFileHeader };
use util::{ Decoder, Crc32Checker, dos2time, path_join };


/// unzipx - extract compressed files in a ZIP archive
#[derive(FromArgs)]
struct Options {
    /// path of the ZIP archive(s).
    #[argh(positional)]
    file: Vec<PathBuf>,

    /// an optional directory to which to extract files.
    #[argh(option, short = 'd')]
    exdir: Option<PathBuf>
}

fn main() -> anyhow::Result<()> {
    let options: Options = argh::from_env();

    let target = if let Some(exdir) = options.exdir {
        exdir
    } else {
        let path = env::current_dir()?;
        PathBuf::from_path_buf(path).ok().context("must utf8 path")?
    };

    for file in options.file.iter() {
        unzip(file, &target)?;
    }

    Ok(())
}

fn unzip(path: &Path, target: &Path) -> anyhow::Result<()> {
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

fn do_entry(
    zip: &ZipArchive<'_>,
    cfh: &CentralFileHeader<'_>,
    target: &Path
) -> anyhow::Result<()> {
    let (_lfh, buf) = zip.read(cfh)?;

    if cfh.gp_flag & 1 != 0 {
        anyhow::bail!("encrypt is not supported");
    }

    let name = if let Ok(name) = std::str::from_utf8(cfh.name) {
        Cow::Borrowed(name)
    } else {
        let mut encoding_detector = EncodingDetector::new();
        encoding_detector.feed(cfh.name, true);
        let (name, ..) = encoding_detector.guess(None, false).decode(cfh.name);
        name
    };
    let path = Path::new(&*name);

    if !path.is_relative() {
        anyhow::bail!("must relative path: {:?}", path);
    }

    let target = path_join(target, path);

    let reader = match cfh.method {
        compress::STORE => Decoder::None(buf),
        compress::DEFLATE => Decoder::Deflate(DeflateDecoder::new(buf)),
        compress::ZSTD => Decoder::Zstd(ZstdDecoder::with_buffer(buf)?),
        _ => anyhow::bail!("compress method is not supported: {}", cfh.method)
    };
    let mut reader = Crc32Checker::new(reader, cfh.crc32);

    let mut target = File::options()
        .write(true)
        .append(true)
        .create_new(true)
        .open(&target)
        .with_context(|| path.to_owned())?;

    let mtime = {
        let time = dos2time(cfh.mod_date, cfh.mod_time)?.assume_utc();
        let unix_timestamp = time.unix_timestamp();
        let nanos = time.nanosecond();
        filetime::FileTime::from_unix_time(unix_timestamp, nanos)
    };

    io::copy(&mut reader, &mut target)?;

    filetime::set_file_handle_times(&target, None, Some(mtime))?;

    println!("export: {:?}", path);

    Ok(())
}
