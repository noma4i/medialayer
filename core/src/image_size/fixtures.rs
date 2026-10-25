pub fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
    bytes.extend(width.to_be_bytes());
    bytes.extend(height.to_be_bytes());
    bytes.extend([8, 6, 0, 0, 0]);
    bytes
}

pub fn gif(width: u16, height: u16) -> Vec<u8> {
    let mut bytes = b"GIF89a".to_vec();
    bytes.extend(width.to_le_bytes());
    bytes.extend(height.to_le_bytes());
    bytes
}

fn riff(chunk: &[u8], body: &[u8]) -> Vec<u8> {
    let mut bytes = b"RIFF\0\0\0\0WEBP".to_vec();
    bytes.extend(chunk);
    bytes.extend((body.len() as u32).to_le_bytes());
    bytes.extend(body);
    bytes
}

pub fn webp_lossy(width: u16, height: u16) -> Vec<u8> {
    let mut body = vec![0, 0, 0, 0x9d, 0x01, 0x2a];
    body.extend(width.to_le_bytes());
    body.extend(height.to_le_bytes());
    riff(b"VP8 ", &body)
}

pub fn webp_lossless(width: u32, height: u32) -> Vec<u8> {
    let mut body = vec![0x2f];
    body.extend(((width - 1) | ((height - 1) << 14)).to_le_bytes());
    riff(b"VP8L", &body)
}

pub fn webp_extended(width: u32, height: u32) -> Vec<u8> {
    let mut body = vec![0, 0, 0, 0];
    body.extend(&(width - 1).to_le_bytes()[..3]);
    body.extend(&(height - 1).to_le_bytes()[..3]);
    riff(b"VP8X", &body)
}

fn segment(marker: u8, payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0xff, marker];
    bytes.extend((payload.len() as u16 + 2).to_be_bytes());
    bytes.extend(payload);
    bytes
}

fn exif(orientation: u16, little: bool) -> Vec<u8> {
    let short = |value: u16| if little { value.to_le_bytes() } else { value.to_be_bytes() };
    let long = |value: u32| if little { value.to_le_bytes() } else { value.to_be_bytes() };
    let mut payload = b"Exif\0\0".to_vec();
    payload.extend(if little { b"II" } else { b"MM" });
    payload.extend(short(42));
    payload.extend(long(8));
    payload.extend(short(1));
    payload.extend(short(0x0112));
    payload.extend(short(3));
    payload.extend(long(1));
    payload.extend(short(orientation));
    payload.extend([0, 0]);
    payload.extend(long(0));
    segment(0xe1, &payload)
}

pub fn jpeg(width: u16, height: u16, turn: Option<(u16, bool)>) -> Vec<u8> {
    let mut bytes = vec![0xff, 0xd8];
    bytes.extend(segment(0xe0, b"JFIF\0filler"));
    if let Some((orientation, little)) = turn {
        bytes.extend(exif(orientation, little));
    }
    let mut frame = vec![8];
    frame.extend(height.to_be_bytes());
    frame.extend(width.to_be_bytes());
    frame.extend([1, 1, 0x11, 0]);
    bytes.extend(segment(0xc0, &frame));
    bytes
}

pub fn padded(mut head: Vec<u8>, length: usize) -> Vec<u8> {
    head.resize(length, 0);
    head
}
