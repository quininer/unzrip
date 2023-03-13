mod util;

use std::{ cmp, env, fs };
use std::io::{ self, Read };
use std::path::{ Path, PathBuf };
use argh::FromArgs;
use anyhow::Context;
use bstr::ByteSlice;
use encoding_rs::Encoding;
use rayon::prelude::*;
use memmap2::MmapOptions;
use flate2::read::DeflateDecoder;
use zstd::stream::read::Decoder as ZstdDecoder;
use zip_parser::{ compress, system, ZipArchive, CentralFileHeader };
use memutils::Buf;
use util::{
    ReadOnlyReader, Decoder, Crc32Checker, FilenameEncoding,
    to_tiny_vec, dos2time, path_join, path_open, sanitize_setuid
};


/// unzipx - extract compressed files in a ZIP archive
#[derive(FromArgs)]
struct Options {
    /// path of the ZIP archive(s).
    #[argh(positional)]
    file: Vec<PathBuf>,

    /// an optional directory to which to extract files.
    #[argh(option, short = 'd')]
    exdir: Option<PathBuf>,

    /// specify character set used to decode filename,
    /// which will be automatically detected by default.
    #[argh(option, short = 'O')]
    charset: Option<String>,

    /// try to keep the original filename,
    /// which will ignore the charset.
    #[argh(switch)]
    keep_origin_filename: bool
}

fn main() -> anyhow::Result<()> {
    let options: Options = argh::from_env();

    let target_dir = if let Some(exdir) = options.exdir {
        exdir
    } else {
        env::current_dir()?
    };
    let encoding = if options.keep_origin_filename {
        FilenameEncoding::Os
    } else if let Some(label) = options.charset {
        let encoding = Encoding::for_label(label.as_bytes()).context("invalid encoding label")?;
        FilenameEncoding::Charset(encoding)
    } else {
        FilenameEncoding::Auto
    };

    for file in options.file.iter() {
        unzip(encoding, &target_dir, file)?;
    }

    Ok(())
}

fn unzip(encoding: FilenameEncoding, target_dir: &Path, path: &Path) -> anyhow::Result<()> {
    println!("Archive: {}", path.display());

    let fd = fs::File::open(path)?;
    let buf = unsafe {
        MmapOptions::new().map_copy_read_only(&fd)?
    };
    let buf = memutils::slice::from_slice(&buf);

    let zip = ZipArchive::parse(&buf)?;
    let len: usize = zip.eocdr().cd_entries.try_into()?;
    let len = cmp::min(len, 128);

    zip.entries()?
        .try_fold(Vec::with_capacity(len), |mut acc, e| e.map(|e| {
            acc.push(e);
            acc
        }))?
        .par_iter()
        .try_for_each(|cfh| do_entry(encoding, &zip, &cfh, target_dir))?;

    Ok(())
}

fn do_entry(
    encoding: FilenameEncoding,
    zip: &ZipArchive<'_>,
    cfh: &CentralFileHeader<'_>,
    target_dir: &Path
) -> anyhow::Result<()> {
    let (_lfh, buf) = zip.read(cfh)?;

    if cfh.gp_flag & 1 != 0 {
        anyhow::bail!("encrypt is not supported");
    }

    let name = to_tiny_vec(cfh.name);
    let path = encoding.decode(&name)?;

    if name.ends_with_str("/")
        && cfh.method == compress::STORE
        && buf.is_empty()
    {
        do_dir(target_dir, &path)?
    } else {
        do_file(cfh, target_dir, &path, buf)?;
    }

    Ok(())
}

fn do_dir(target_dir: &Path, path: &Path) -> anyhow::Result<()> {
    let target = path_join(target_dir, path)?;

    fs::create_dir_all(target)
        .or_else(|err| if err.kind() == io::ErrorKind::AlreadyExists {
            Ok(())
        } else {
            Err(err)
        })
        .with_context(|| path.display().to_string())?;

    println!("   creating: {}", path.display());

    Ok(())
}

fn do_file(
    cfh: &CentralFileHeader,
    target_dir: &Path,
    path: &Path,
    buf: Buf<'_>
) -> anyhow::Result<()> {
    let target = path_join(target_dir, path)?;

    let reader = ReadOnlyReader(buf);
    let reader = match cfh.method {
        compress::STORE => Decoder::None(reader),
        compress::DEFLATE => Decoder::Deflate(DeflateDecoder::new(reader)),
        compress::ZSTD => Decoder::Zstd(ZstdDecoder::new(reader)?),
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

    let mut fd = path_open(&target).with_context(|| path.display().to_string())?;

    io::copy(&mut reader, &mut fd)?;

    filetime::set_file_handle_times(&fd, None, Some(mtime))?;

    #[cfg(unix)]
    if cfh.ext_attrs != 0 && cfh.made_by_ver >> 8 == system::UNIX {
        use std::os::unix::fs::PermissionsExt;

        let perm = fs::Permissions::from_mode(cfh.ext_attrs >> 16);
        fd.set_permissions(sanitize_setuid(perm))?;
    }

    println!("  inflating: {}", path.display());

    Ok(())
}
