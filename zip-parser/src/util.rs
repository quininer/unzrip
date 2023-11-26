use memutils::Buf;

pub struct Eof;

#[inline]
pub fn take(input: Buf<'_>, n: usize) -> Result<(Buf<'_>, Buf<'_>), Eof> {
    if input.len() >= n {
        let (prefix, suffix) = input.split_at(n);
        Ok((suffix, prefix))
    } else {
        Err(Eof)
    }
}

#[inline]
pub fn read_u16(input: Buf<'_>) -> Result<(Buf<'_>, u16), Eof> {
    let mut buf = [0; 2];
    let (input, output) = take(input, buf.len())?;
    memutils::slice::copy_from_slice(&mut buf, output);
    let output = u16::from_le_bytes(buf);
    Ok((input, output))
}

#[inline]
pub fn read_u32(input: Buf<'_>) -> Result<(Buf<'_>, u32), Eof> {
    let mut buf = [0; 4];
    let (input, output) = take(input, buf.len())?;
    memutils::slice::copy_from_slice(&mut buf, output);
    let output = u32::from_le_bytes(buf);
    Ok((input, output))
}

#[inline]
pub fn read_u64(input: Buf<'_>) -> Result<(Buf<'_>, u64), Eof> {
    let mut buf = [0; 8];
    let (input, output) = take(input, buf.len())?;
    memutils::slice::copy_from_slice(&mut buf, output);
    let output = u64::from_le_bytes(buf);
    Ok((input, output))
}

pub fn rfind(haystack: Buf<'_>, needle: &[u8]) -> Option<usize> {
    let first_byte = needle.first().copied()?;

    for (i, _) in haystack.iter()
        .enumerate()
        .rev()
        .filter(|(_, b)| b.get() == first_byte)
    {
        if let Some(subset) = haystack[i..].get(..needle.len()) {
            if subset == needle {
                return Some(i);
            }
        }
    }

    None
}
