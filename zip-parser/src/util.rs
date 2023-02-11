pub struct Eof;


#[inline]
pub fn take(input: &[u8], n: usize) -> Result<(&[u8], &[u8]), Eof> {
    if input.len() >= n {
        let (prefix, suffix) = input.split_at(n);
        Ok((suffix, prefix))
    } else {
        Err(Eof)
    }
}

#[inline]
pub fn read_u16(input: &[u8]) -> Result<(&[u8], u16), Eof> {
    let mut buf = [0; 2];
    let (input, output) = take(input, buf.len())?;
    buf.copy_from_slice(output);
    let output = u16::from_le_bytes(buf);
    Ok((input, output))
}

#[inline]
pub fn read_u32(input: &[u8]) -> Result<(&[u8], u32), Eof> {
    let mut buf = [0; 4];
    let (input, output) = take(input, buf.len())?;
    buf.copy_from_slice(output);
    let output = u32::from_le_bytes(buf);
    Ok((input, output))
}
