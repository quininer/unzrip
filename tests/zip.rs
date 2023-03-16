use std::{ fs, io };
use std::path::{ Path, PathBuf };
use bstr::ByteSlice;
use tempfile::tempdir;
use zip::ZipWriter;
use assert_cmd::cmd::Command;


fn hash_file(path: &Path) -> anyhow::Result<u64> {
    use std::hash::Hasher;
    use std::collections::hash_map::DefaultHasher;

    struct HashWriter(DefaultHasher);

    impl io::Write for HashWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.write(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let mut fd = fs::File::open(path)?;
    let mut hasher = HashWriter(DefaultHasher::new());

    io::copy(&mut fd, &mut hasher)?;

    Ok(hasher.0.finish())
}

fn list_dir(path: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut list = Vec::new();

    for entry in walkdir::WalkDir::new(path)
        .max_depth(2)
    {
        let entry = entry?;

        if entry.path() != path {
            let path = entry.path().strip_prefix(path)?;
            list.push(path.into());
        }
    }

    Ok(list)
}

#[test]
fn test_simple_zip_file() -> anyhow::Result<()> {
    let dir = tempdir()?;
    let dir = dir.path();

    let path = dir.join("test1.zip");

    // create zip
    {
        let fd = fs::File::create(&path)?;
        let mut writer = ZipWriter::new(fd);

        writer.start_file("Cargo.toml", Default::default())?;
        io::copy(&mut fs::File::open("Cargo.toml")?, &mut writer)?;

        writer.add_directory("lock/", Default::default())?;
        writer.start_file("lock/Cargo.lock", Default::default())?;
        io::copy(&mut fs::File::open("Cargo.lock")?, &mut writer)?;

        writer.add_symlink("lock/Cargo.toml", "Cargo.toml", Default::default())?;
        writer.add_directory("lock2\\", Default::default())?;

        writer.finish()?;
    }

    Command::cargo_bin("unzrip")?
        .arg(&path)
        .arg("-d")
        .arg(dir)
        .assert()
        .success();

    assert_eq!(hash_file(Path::new("Cargo.toml"))?, hash_file(&dir.join("Cargo.toml"))?);
    assert_eq!(hash_file(Path::new("Cargo.lock"))?, hash_file(&dir.join("lock/Cargo.lock"))?);

    let mut list = list_dir(dir)?;
    list.sort();

    assert_eq!(list, vec![
        Path::new("Cargo.toml"),
        Path::new("lock"),
        Path::new("lock/Cargo.lock"),
        Path::new("lock/Cargo.toml"),
        Path::new("lock2"),
        Path::new("test1.zip"),
    ]);

    Ok(())
}


#[test]
fn test_encoding_filename() -> anyhow::Result<()> {
    let dir = tempdir()?;
    let dir = dir.path();

    let path = dir.join("test2.zip");

    // create zip
    {
        let fd = fs::File::create(&path)?;
        let mut writer = ZipWriter::new(fd);

        let name = "中文漢字";
        let (name2, _, _) = encoding_rs::GBK.encode(name);
        let name2 = name2.into_owned();
        assert_ne!(name.as_bytes(), &name2);

        // Just test :(
        let bad_name = unsafe {
            String::from_utf8_unchecked(name2)
        };

        writer.start_file(bad_name, Default::default())?;

        let name = "かんじ";
        let (name2, _, _) = encoding_rs::SHIFT_JIS.encode(name);
        let name2 = name2.into_owned();
        assert_ne!(name.as_bytes(), &name2);

        // Just test :(
        let bad_name = unsafe {
            String::from_utf8_unchecked(name2)
        };

        writer.start_file(bad_name, Default::default())?;

        writer.finish()?;
    }

    Command::cargo_bin("unzrip")?
        .arg(&path)
        .arg("-d")
        .arg(dir)
        .assert()
        .success();

    let mut list = list_dir(dir)?;
    list.sort();

    assert_eq!(list, vec![
        Path::new("test2.zip"),
        Path::new("かんじ"),
        Path::new("中文漢字"),
    ]);

    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn test_unix_filename() -> anyhow::Result<()> {
    let dir = tempdir()?;
    let dir = dir.path();

    let path = dir.join("test3.zip");
    let name = vec![0x12, 0x23, 0x34, 0x45, 0x56, 0x67, 0x78, 0x89, 0x90];

    // create zip
    {
        let fd = fs::File::create(&path)?;
        let mut writer = ZipWriter::new(fd);

        // Just test :(
        let bad_name = unsafe {
            String::from_utf8_unchecked(name.clone())
        };

        writer.start_file(bad_name, Default::default())?;

        writer.finish()?;
    }

    Command::cargo_bin("unzrip")?
        .arg(&path)
        .arg("--keep-origin-filename")
        .arg("-d")
        .arg(dir)
        .assert()
        .success();

    let mut list = list_dir(dir)?;
    list.sort();

    assert_eq!(list, vec![
        name.to_path().unwrap(),
        Path::new("test3.zip"),
    ]);

    Ok(())
}

#[test]
fn test_evil_path() -> anyhow::Result<()> {
    let dir = tempdir()?;
    let dir = dir.path();

    let path = dir.join("test4.zip");

    // create zip
    {
        let fd = fs::File::create(&path)?;
        let mut writer = ZipWriter::new(fd);

        writer.start_file("/home/user/.bashrc", Default::default())?;
        writer.finish()?;
    }


    let assert = Command::cargo_bin("unzrip")?
        .arg(&path)
        .arg("-d")
        .arg(dir)
        .assert()
        .failure();
    assert!(assert.get_output().stderr.contains_str("must relative path"));

    Ok(())
}


#[test]
fn test_evil_path2() -> anyhow::Result<()> {
    let dir = tempdir()?;
    let dir = dir.path();

    let path = dir.join("test5.zip");

    // create zip
    {
        let fd = fs::File::create(&path)?;
        let mut writer = ZipWriter::new(fd);

        writer.start_file("../../../../../../../../.bashrc", Default::default())?;
        writer.finish()?;
    }


    let assert = Command::cargo_bin("unzrip")?
        .arg(&path)
        .arg("-d")
        .arg(dir)
        .assert()
        .failure();
    assert!(assert.get_output().stderr.contains_str("filename over the path limit"));

    Ok(())
}
