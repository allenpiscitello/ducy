//! Writing information-set keys to bytes, for checkpoints.

/// An information-set key that can be saved and loaded.
pub trait Key: Clone + Eq + std::hash::Hash + Send + Sync {
    fn write(&self, out: &mut Vec<u8>);
    /// Reads a key written by [`Key::write`], advancing `input`.
    fn read(input: &mut &[u8]) -> Option<Self>;
}

pub(crate) fn take<'a>(input: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
    if input.len() < n {
        return None;
    }
    let (head, rest) = input.split_at(n);
    *input = rest;
    Some(head)
}

pub(crate) fn read_u64(input: &mut &[u8]) -> Option<u64> {
    take(input, 8).map(|b| u64::from_le_bytes(b.try_into().expect("8 bytes")))
}

impl Key for String {
    fn write(&self, out: &mut Vec<u8>) {
        out.extend((self.len() as u64).to_le_bytes());
        out.extend(self.as_bytes());
    }
    fn read(input: &mut &[u8]) -> Option<Self> {
        let n = read_u64(input)? as usize;
        String::from_utf8(take(input, n)?.to_vec()).ok()
    }
}

impl Key for u64 {
    fn write(&self, out: &mut Vec<u8>) {
        out.extend(self.to_le_bytes());
    }
    fn read(input: &mut &[u8]) -> Option<Self> {
        read_u64(input)
    }
}

impl Key for usize {
    fn write(&self, out: &mut Vec<u8>) {
        (*self as u64).write(out);
    }
    fn read(input: &mut &[u8]) -> Option<Self> {
        read_u64(input).map(|v| v as usize)
    }
}

impl Key for () {
    fn write(&self, _: &mut Vec<u8>) {}
    fn read(_: &mut &[u8]) -> Option<Self> {
        Some(())
    }
}

impl Key for Vec<u8> {
    fn write(&self, out: &mut Vec<u8>) {
        out.extend((self.len() as u64).to_le_bytes());
        out.extend(self);
    }
    fn read(input: &mut &[u8]) -> Option<Self> {
        let n = read_u64(input)? as usize;
        take(input, n).map(<[u8]>::to_vec)
    }
}
