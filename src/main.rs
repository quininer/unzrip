use std::{ cmp, env };
use std::fs::File;
use anyhow::Context;
use camino::{ Utf8Path as Path, Utf8PathBuf as PathBuf };
use argh::FromArgs;
use rayon::prelude::*;
use memmap2::MmapOptions;
use zip_parser::{ ZipArchive, CentralFileHeader };


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
        .try_for_each(|metadata| do_entry(&zip, &metadata, target))?;

    Ok(())
}

fn do_entry(zip: &ZipArchive<'_>, metadata: &CentralFileHeader<'_>, target: &Path) -> anyhow::Result<()> {
    let (entrie, buf) = zip.read(metadata)?;

    Ok(())
}
