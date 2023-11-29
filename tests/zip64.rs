use std::fs;
use std::io::{ self, Read };
use tempfile::tempdir;
use zip::ZipWriter;
use assert_cmd::cmd::Command;


#[test]
fn test_zip64_tmpfs() -> anyhow::Result<()> {
    let dir = tempdir()?;
    let dir = dir.path();

    let path = dir.join("testbig1.zip");

    // create zip
    {
        let fd = fs::File::create(&path)?;
        let mut writer = ZipWriter::new(fd);

        writer.start_file(
            "bigfile",
            zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .large_file(true)
        )?;
        let len = 4 * 1024 * 1024 * 1024 + 1024;
        let mut reader = io::repeat(0).take(len);
        io::copy(&mut reader, &mut writer)?;

        writer.finish()?;
    }

    Command::cargo_bin("unzrip")?
        .arg(&path)
        .arg("-d")
        .arg(dir)
        .assert()
        .success();

    Ok(())
}
