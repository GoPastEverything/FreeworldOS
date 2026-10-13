//! M5-Q strict, deliberately tiny ELF64 FreeWorldOS loader.
//! Exactly one executable PT_LOAD and one PT_NOTE; not a general ELF loader.
const CODE: u64 = 0x0000_5000_3000_0000;
const HEADER: usize = 64;
const PH_SIZE: usize = 56;
const NOTE_AT: usize = HEADER + 2 * PH_SIZE; // 0xb0
const CODE_AT: usize = 0x100;
pub const IMAGE_BYTES: usize = CODE_AT + super::super::arch::process_ring3::M5P_STUB.len();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Truncated, InvalidElf, UnsupportedLayout, MissingNote, BadAbiVersion }

fn get16(bytes: &[u8], offset: usize) -> Result<u16, Error> {
    let x = bytes.get(offset..offset.checked_add(2).ok_or(Error::Truncated)?).ok_or(Error::Truncated)?;
    Ok(u16::from_le_bytes([x[0], x[1]]))
}
fn get32(bytes: &[u8], offset: usize) -> Result<u32, Error> {
    let x = bytes.get(offset..offset.checked_add(4).ok_or(Error::Truncated)?).ok_or(Error::Truncated)?;
    Ok(u32::from_le_bytes(x.try_into().map_err(|_| Error::Truncated)?))
}
fn get64(bytes: &[u8], offset: usize) -> Result<u64, Error> {
    let x = bytes.get(offset..offset.checked_add(8).ok_or(Error::Truncated)?).ok_or(Error::Truncated)?;
    Ok(u64::from_le_bytes(x.try_into().map_err(|_| Error::Truncated)?))
}
fn put16(b: &mut [u8], at: usize, n: u16) { b[at..at+2].copy_from_slice(&n.to_le_bytes()); }
fn put32(b: &mut [u8], at: usize, n: u32) { b[at..at+4].copy_from_slice(&n.to_le_bytes()); }
fn put64(b: &mut [u8], at: usize, n: u64) { b[at..at+8].copy_from_slice(&n.to_le_bytes()); }

pub fn test_image(version: u32) -> [u8; IMAGE_BYTES] {
    let mut b = [0u8; IMAGE_BYTES];
    b[0..4].copy_from_slice(b"\x7fELF");
    b[4] = 2; // 64-bit
    b[5] = 1; // little endian
    b[6] = 1; // ELF version
    put16(&mut b, 16, 2); // ET_EXEC
    put16(&mut b, 18, 62); // EM_X86_64
    put32(&mut b, 20, 1);
    put64(&mut b, 24, CODE);
    put64(&mut b, 32, HEADER as u64);
    put16(&mut b, 52, HEADER as u16);
    put16(&mut b, 54, PH_SIZE as u16);
    put16(&mut b, 56, 2);
    // PHDR 0: single non-writable executable PT_LOAD
    let p = HEADER;
    put32(&mut b, p, 1);
    put32(&mut b, p+4, 5); // PF_R | PF_X
    put64(&mut b, p+8, CODE_AT as u64);
    put64(&mut b, p+16, CODE);
    put64(&mut b, p+24, CODE);
    put64(&mut b, p+32, crate::arch::process_ring3::M5P_STUB.len() as u64);
    put64(&mut b, p+40, crate::arch::process_ring3::M5P_STUB.len() as u64);
    put64(&mut b, p+48, 0x100); // congruent offset/address
    // PHDR 1: FreeWorldOS PT_NOTE
    let p = HEADER + PH_SIZE;
    put32(&mut b, p, 4);
    put64(&mut b, p+8, NOTE_AT as u64);
    put64(&mut b, p+32, 40);
    put64(&mut b, p+40, 40);
    put64(&mut b, p+48, 4);
    put32(&mut b, NOTE_AT, 12);
    put32(&mut b, NOTE_AT+4, 16);
    put32(&mut b, NOTE_AT+8, 1);
    b[NOTE_AT+12..NOTE_AT+24].copy_from_slice(b"FreeWorldOS\0");
    put32(&mut b, NOTE_AT+24, version);
    put32(&mut b, NOTE_AT+28, 0); // environment: FreeWorld
    put32(&mut b, NOTE_AT+32, 0); // flags: none
    put32(&mut b, NOTE_AT+36, 0); // reserved: must be zero
    b[CODE_AT..].copy_from_slice(&crate::arch::process_ring3::M5P_STUB);
    b
}

#[cfg(feature = "m5r-ci-self-test")]
pub const HELLO_IMAGE_BYTES: usize = CODE_AT + 0x80 + 20;

/// Native ELF v1 with one executable PT_LOAD containing code plus embedded
/// read-only message bytes at virtual CODE+0x80. Nothing is hardcoded into the
/// kernel's ELF-to-page staging path.
#[cfg(feature = "m5r-ci-self-test")]
pub fn hello_image() -> [u8; HELLO_IMAGE_BYTES] {
    let base = test_image(1);
    let mut image = [0u8; HELLO_IMAGE_BYTES];
    image[..CODE_AT].copy_from_slice(&base[..CODE_AT]);
    put64(&mut image, HEADER+32, 0x80+20);
    put64(&mut image, HEADER+40, 0x80+20);
    image[CODE_AT..CODE_AT+crate::arch::process_ring3::M5R_CODE.len()]
        .copy_from_slice(&crate::arch::process_ring3::M5R_CODE);
    image[CODE_AT+0x80..CODE_AT+0x80+20]
        .copy_from_slice(crate::arch::process_ring3::M5R_TEXT);
    image
}

#[cfg(feature = "m5s-ci-self-test")]
pub fn invalid_then_hello_image() -> [u8; HELLO_IMAGE_BYTES] {
    let mut image = hello_image();
    let len = crate::arch::process_ring3::M5S_CODE.len();
    image[CODE_AT..CODE_AT+len].copy_from_slice(&crate::arch::process_ring3::M5S_CODE);
    image
}

#[cfg(feature = "m5u-ci-self-test")]
pub const CROSS_IMAGE_BYTES: usize = CODE_AT + 4096 + 12;
#[cfg(feature = "m5u-ci-self-test")]
pub fn cross_page_hello_image() -> [u8; CROSS_IMAGE_BYTES] {
    let base = test_image(1);
    let mut image = [0u8; CROSS_IMAGE_BYTES];
    image[..CODE_AT].copy_from_slice(&base[..CODE_AT]);
    put64(&mut image, HEADER+32, 4096+12);
    put64(&mut image, HEADER+40, 4096+12);
    image[CODE_AT..CODE_AT+60].copy_from_slice(&crate::arch::process_ring3::M5U_CODE);
    image[CODE_AT+4096-8..CODE_AT+4096+12].copy_from_slice(crate::arch::process_ring3::M5R_TEXT);
    image
}

pub struct LoadSegment<'a> { pub entry: u64, pub code: &'a [u8] }

pub fn validate(bytes: &[u8]) -> Result<LoadSegment<'_>, Error> {
    if bytes.len() < HEADER { return Err(Error::Truncated); }
    if bytes.get(..7) != Some(&b"\x7fELF\x02\x01\x01"[..])
        || get16(bytes,16)? != 2 || get16(bytes,18)? != 62
        || get32(bytes,20)? != 1 || get16(bytes,52)? != HEADER as u16
        || get16(bytes,54)? != PH_SIZE as u16 || get16(bytes,56)? != 2
        || get64(bytes,32)? != HEADER as u64
    { return Err(Error::InvalidElf); }
    let entry = get64(bytes,24)?;
    if entry != CODE { return Err(Error::UnsupportedLayout); }
    let mut load: Option<&[u8]> = None;
    let mut has_note = false;
    for i in 0..2usize {
        let p = HEADER+i*PH_SIZE;
        let kind = get32(bytes,p)?;
        let offset = usize::try_from(get64(bytes,p+8)?).map_err(|_| Error::UnsupportedLayout)?;
        let size = usize::try_from(get64(bytes,p+32)?).map_err(|_| Error::UnsupportedLayout)?;
        let end = offset.checked_add(size).ok_or(Error::UnsupportedLayout)?;
        let data = bytes.get(offset..end).ok_or(Error::Truncated)?;
        match kind {
            1 if load.is_none() => {
                if get32(bytes,p+4)? != 5
                    || get64(bytes,p+16)? != CODE || get64(bytes,p+24)? != CODE
                    || get64(bytes,p+40)? != size as u64
                    || get64(bytes,p+48)? != 0x100
                    || offset % 0x100 != (CODE as usize) % 0x100
                    || data.is_empty() || data.len() > if cfg!(feature = "m5u-ci-self-test") { 4096+12 } else { 4096 }
                { return Err(Error::UnsupportedLayout); }
                load = Some(data);
            }
            4 if !has_note => {
                if size != 40 || get64(bytes,p+40)? != 40 || get32(data,0)? != 12
                    || get32(data,4)? != 16 || get32(data,8)? != 1
                    || data.get(12..24) != Some(&b"FreeWorldOS\0"[..])
                { return Err(Error::MissingNote); }
                if get32(data,24)? != 1 { return Err(Error::BadAbiVersion); }
                if get32(data,28)? != 0 || get32(data,32)? != 0 || get32(data,36)? != 0 {
                    return Err(Error::UnsupportedLayout);
                }
                has_note = true;
            }
            _ => return Err(Error::UnsupportedLayout),
        }
    }
    if !has_note { return Err(Error::MissingNote); }
    let code = load.ok_or(Error::UnsupportedLayout)?;
    if !code.contains(&0x05) { return Err(Error::UnsupportedLayout); }
    Ok(LoadSegment { entry, code })
}

#[cfg(feature = "m5q-ci-self-test")]
pub fn ci_parser_test() -> Result<(), Error> {
    let good = test_image(1);
    let load = validate(&good)?;
    if load.entry != CODE || load.code != crate::arch::process_ring3::M5P_STUB {
        return Err(Error::UnsupportedLayout);
    }
    if validate(&test_image(2)).err() != Some(Error::BadAbiVersion) {
        return Err(Error::UnsupportedLayout);
    }
    for descriptor_offset in [NOTE_AT+28, NOTE_AT+32, NOTE_AT+36] {
        let mut bad = good;
        bad[descriptor_offset] = 1;
        if validate(&bad).err() != Some(Error::UnsupportedLayout) {
            return Err(Error::UnsupportedLayout);
        }
    }
    let mut old_descriptor = good;
    put32(&mut old_descriptor, NOTE_AT+4, 4);
    if validate(&old_descriptor).err() != Some(Error::MissingNote) {
        return Err(Error::UnsupportedLayout);
    }
    let mut bad_name = good;
    bad_name[NOTE_AT+12] ^= 1;
    if validate(&bad_name).err() != Some(Error::MissingNote) {
        return Err(Error::UnsupportedLayout);
    }
    Ok(())
}
