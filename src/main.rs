use std::env;
use std::fs::File;
use anyhow::Context;
use camino::{ Utf8Path as Path, Utf8PathBuf as PathBuf };
use argh::FromArgs;
use rayon::prelude::*;
use memmap2::MmapOptions;
use piz::read::{ ZipArchive, FileMetadata };


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

    let zip = ZipArchive::new(&buf)?;

    zip.entries()
        .par_iter()
        .try_for_each(|metadata| read_entrie(&zip, &metadata, target))?;

    Ok(())
}

fn read_entrie(zip: &ZipArchive<'_>, metadata: &FileMetadata<'_>, target: &Path) -> anyhow::Result<()> {
    if metadata.path.is_absolute() {
        anyhow::bail!("absolute not supported");
    }

    if metadata.encrypted {
        anyhow::bail!("encrypt not supported");
    }


    let entrie = zip.read(metadata)?;



    Ok(())
}
