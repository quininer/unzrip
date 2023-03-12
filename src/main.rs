mod util;

use std::{ cmp, env, fs };
use std::io::{ self, Read };
use std::borrow::Cow;
use anyhow::Context;
use camino::{ Utf8Path as Path, Utf8PathBuf as PathBuf };
use argh::FromArgs;
use rayon::prelude::*;
use memmap2::MmapOptions;
use flate2::bufread::DeflateDecoder;
use zstd::stream::read::Decoder as ZstdDecoder;
use encoding_rs::Encoding;
use chardetng::EncodingDetector;
use zip_parser::{ compress, system, ZipArchive, CentralFileHeader };
use util::{ Decoder, Crc32Checker, dos2time, path_join, path_open, sanitize_setuid };


/// unzipx - extract compressed files in a ZIP archive
#[derive(FromArgs)]
struct Options {
    /// path of the ZIP archive(s).
    #[argh(positional)]
    file: Vec<PathBuf>,

    /// an optional directory to which to extract files.
    #[argh(option, short = 'd')]
    exdir: Option<PathBuf>,

    /// specify character set used to decode filename, which will be automatically detected by default.
    #[argh(option, short = 'O')]
    charset: Option<String>
}

fn main() -> anyhow::Result<()> {
    let options: Options = argh::from_env();

    let target_dir = if let Some(exdir) = options.exdir {
        exdir
    } else {
        let path = env::current_dir()?;
        PathBuf::from_path_buf(path).ok().context("must utf8 path")?
    };
    let charset = if let Some(label) = options.charset {
        Some(Encoding::for_label(label.as_bytes()).context("invalid encoding label")?)
    } else {
        None
    };

    for file in options.file.iter() {
        unzip(charset, &target_dir, file)?;
    }

    Ok(())
}

fn unzip(charset: Option<&'static Encoding>, target_dir: &Path, path: &Path) -> anyhow::Result<()> {
    println!("Archive: {}", path);

    let fd = fs::File::open(path)?;
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
        .try_for_each(|cfh| do_entry(charset, &zip, &cfh, target_dir))?;

    Ok(())
}

fn do_entry(
    charset: Option<&'static Encoding>,
    zip: &ZipArchive<'_>,
    cfh: &CentralFileHeader<'_>,
    target_dir: &Path
) -> anyhow::Result<()> {
    let (_lfh, buf) = zip.read(cfh)?;

    if cfh.gp_flag & 1 != 0 {
        anyhow::bail!("encrypt is not supported");
    }

    let name = if let Some(encoding) = charset {
        let (name, ..) = encoding.decode(cfh.name);
        name
    } else if let Ok(name) = std::str::from_utf8(cfh.name) {
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

    if name.ends_with('/')
        && cfh.method == compress::STORE
        && buf.is_empty()
    {
        do_dir(target_dir, path)?
    } else {
        do_file(cfh, target_dir, path, buf)?;
    }

    Ok(())
}

fn do_dir(target_dir: &Path, path: &Path) -> anyhow::Result<()> {
    let target = path_join(target_dir, path);

    fs::create_dir_all(target)
        .or_else(|err| if err.kind() == io::ErrorKind::AlreadyExists {
            Ok(())
        } else {
            Err(err)
        })
        .with_context(|| path.to_owned())?;

    println!("   creating: {}", path);

    Ok(())
}

fn do_file(
    cfh: &CentralFileHeader,
    target_dir: &Path,
    path: &Path,
    buf: &[u8]
) -> anyhow::Result<()> {
    let target = path_join(target_dir, path);

    let reader = match cfh.method {
        compress::STORE => Decoder::None(buf),
        compress::DEFLATE => Decoder::Deflate(DeflateDecoder::new(buf)),
        compress::ZSTD => Decoder::Zstd(ZstdDecoder::with_buffer(buf)?),
        _ => anyhow::bail!("compress method is not supported: {}", cfh.method)
    };
    // prevent zipbomb
    let reader = reader.take(cfh.uncomp_size.into());
    let mut reader = Crc32Checker::new(reader, cfh.crc32);

    let mtime = {
        let time = dos2time(cfh.mod_date, cfh.mod_time)?.assume_utc();
        let unix_timestamp = time.unix_timestamp();
        let nanos = time.nanosecond();
        filetime::FileTime::from_unix_time(unix_timestamp, nanos)
    };

    let mut fd = path_open(&target).with_context(|| path.to_owned())?;

    io::copy(&mut reader, &mut fd)?;

    filetime::set_file_handle_times(&fd, None, Some(mtime))?;

    #[cfg(unix)]
    if cfh.ext_attrs != 0 && cfh.made_by_ver >> 8 == system::UNIX {
        use std::os::unix::fs::PermissionsExt;

        let perm = fs::Permissions::from_mode(cfh.ext_attrs >> 16);
        fd.set_permissions(sanitize_setuid(perm))?;
    }

    println!("  inflating: {}", path);

    Ok(())
}
