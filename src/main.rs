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
use zip_parser::{ compress, ZipArchive, CentralFileHeader };
use memutils::Buf;
use util::{
    ReadOnlyReader, Crc32Checker, FilenameEncoding,
    to_tiny_vec, dos2time, path_join, path_open,
};


/// unzrip - extract compressed files in a ZIP archive
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

    /// overwrite files WITHOUT prompting
    #[argh(switch, short = 'o')]
    overwrite: bool,

    /// try a faster method, but it may not be stable
    #[argh(switch)]
    fast: bool,

    /// try to keep the original filename,
    /// which will ignore the charset.
    #[argh(switch)]
    keep_origin_filename: bool,
}

fn main() -> anyhow::Result<()> {
    let options: Options = argh::from_env();

    let target_dir = if let Some(exdir) = options.exdir.clone() {
        exdir
    } else {
        env::current_dir()?
    };
    let encoding = if options.keep_origin_filename {
        FilenameEncoding::Os
    } else if let Some(label) = options.charset.clone() {
        let encoding = Encoding::for_label(label.as_bytes()).context("invalid encoding label")?;
        FilenameEncoding::Charset(encoding)
    } else {
        FilenameEncoding::Auto
    };

    for file in options.file.iter() {
        unzip(&options, encoding, &target_dir, file)?;
    }

    Ok(())
}

fn unzip(options: &Options, encoding: FilenameEncoding, target_dir: &Path, path: &Path)
    -> anyhow::Result<()>
{
    println!("Archive: {}", path.display());

    let fd = fs::File::open(path)?;

    // # Safety
    //
    // mmap operation
    let buf = unsafe {
        MmapOptions::new().map_copy_read_only(&fd)?
    };
    let buf = memutils::slice::from_slice(&buf);

    let zip = ZipArchive::parse(&buf)?;
    let len: usize = zip.eocdr().cd_entries().context("cd entries overwrite")?;
    let len = cmp::min(len, 128);

    zip.entries()?
        .try_fold(Vec::with_capacity(len), |mut acc, e| e.map(|e| {
            acc.push(e);
            acc
        }))?
        .par_iter()
        .try_for_each(|cfh| do_entry(options, encoding, &zip, &cfh, target_dir))?;

    Ok(())
}

fn do_entry(
    options: &Options,
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

    if (name.ends_with_str("/") || name.ends_with_str("\\"))
        && cfh.method == compress::STORE
        && buf.is_empty()
    {
        #[cfg(unix)]
        let name = name.trim_end_with(|c| c == '\\');
        let path = encoding.decode(&name)?;
        do_dir(target_dir, &path)?
    } else {
        let path = encoding.decode(&name)?;
        do_file(options, cfh, target_dir, &path, buf)?;
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
    options: &Options,
    cfh: &CentralFileHeader,
    target_dir: &Path,
    path: &Path,
    buf: Buf<'_>
) -> anyhow::Result<()> {
    let target = path_join(target_dir, path)?;

    let reader = ReadOnlyReader(buf);
    let reader: Box<dyn Read> = if options.fast {
        use flate2::bufread::DeflateDecoder;
        #[cfg(feature = "zstd-sys")]
        use zstd::stream::read::Decoder as ZstdDecoder;

        // # Safety
        //
        // Assume that the file is stable and will not be modified
        let reader = unsafe {
            memutils::slice::as_slice(reader.0)
        };

        match cfh.method {
            compress::STORE => Box::new(reader),
            compress::DEFLATE => Box::new(DeflateDecoder::new(reader)),
            #[cfg(feature = "zstd-sys")]
            compress::ZSTD => Box::new(ZstdDecoder::with_buffer(reader)?),
            _ => anyhow::bail!("compress method is not supported: {}", cfh.method)
        }
    } else {
        use flate2::read::DeflateDecoder;
        #[cfg(feature = "zstd-sys")]
        use zstd::stream::read::Decoder as ZstdDecoder;

        match cfh.method {
            compress::STORE => Box::new(reader),
            compress::DEFLATE => Box::new(DeflateDecoder::new(reader)),
            #[cfg(feature = "zstd-sys")]
            compress::ZSTD => Box::new(ZstdDecoder::new(reader)?),
            _ => anyhow::bail!("compress method is not supported: {}", cfh.method)
        }
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

    let mut fd = path_open(&target, options.overwrite).with_context(|| path.display().to_string())?;

    io::copy(&mut reader, &mut fd)?;

    filetime::set_file_handle_times(&fd, None, Some(mtime))?;

    #[cfg(unix)]
    if cfh.ext_attrs != 0 && cfh.made_by_ver >> 8 == zip_parser::system::UNIX {
        use std::os::unix::fs::PermissionsExt;

        let perm = fs::Permissions::from_mode(cfh.ext_attrs >> 16);
        fd.set_permissions(util::sanitize_setuid(perm))?;
    }

    println!("  inflating: {}", path.display());

    Ok(())
}
