//! Ogg Opus (Telegram voice notes) to CAF, without decoding.
//!
//! macOS decodes Opus natively, but only from a CAF container; Core Audio
//! can't read Ogg. The Opus packets move over unchanged: CAF just needs the
//! stream description, a table of packet sizes and frame counts, and the
//! encoder's priming (Ogg's pre-skip).

use std::{fs::File, io::BufReader, path::Path};

use anyhow::{Context, Result, bail, ensure};

/// Opus always decodes at 48 kHz.
const RATE: u32 = 48_000;

pub fn ogg_to_caf(input: &Path, output: &Path) -> Result<()> {
    let mut reader = ogg::PacketReader::new(BufReader::new(File::open(input)?));
    let head = reader.read_packet()?.context("empty Ogg file")?;
    ensure!(head.data.starts_with(b"OpusHead"), "not an Ogg Opus file");
    ensure!(head.data.len() >= 19, "short OpusHead");
    let channels = u32::from(head.data[9]);
    let pre_skip = u32::from(u16::from_le_bytes([head.data[10], head.data[11]]));
    ensure!(channels > 0, "no channels");
    // OpusTags, then audio.
    reader.read_packet()?.context("missing OpusTags")?;

    let mut packets: Vec<Vec<u8>> = Vec::new();
    let mut frames: Vec<u32> = Vec::new();
    let mut last_granule = 0;
    while let Some(packet) = reader.read_packet()? {
        if packet.data.is_empty() {
            continue;
        }
        frames.push(packet_frames(&packet.data)?);
        last_granule = packet.absgp_page();
        packets.push(packet.data);
    }
    ensure!(!packets.is_empty(), "no audio");

    let total: u64 = frames.iter().map(|&f| u64::from(f)).sum();
    // The last page's granule position is the true end, counting pre-skip.
    let valid = last_granule
        .saturating_sub(u64::from(pre_skip))
        .min(total.saturating_sub(u64::from(pre_skip)));
    let remainder = total - valid - u64::from(pre_skip);
    let fixed = frames.iter().all(|&f| f == frames[0]).then_some(frames[0]);

    let mut caf = Vec::new();
    caf.extend_from_slice(b"caff");
    caf.extend_from_slice(&1u16.to_be_bytes());
    caf.extend_from_slice(&0u16.to_be_bytes());

    // Stream description.
    let mut desc = Vec::new();
    desc.extend_from_slice(&f64::from(RATE).to_be_bytes());
    desc.extend_from_slice(b"opus");
    desc.extend_from_slice(&0u32.to_be_bytes()); // format flags
    desc.extend_from_slice(&0u32.to_be_bytes()); // bytes per packet: variable
    desc.extend_from_slice(&fixed.unwrap_or(0).to_be_bytes()); // frames per packet
    desc.extend_from_slice(&channels.to_be_bytes());
    desc.extend_from_slice(&0u32.to_be_bytes()); // bits per channel
    chunk(&mut caf, b"desc", &desc);

    // Decoder cookie in the layout Apple's encoder writes: application
    // (VoIP), sample rate, frames per packet, bitrate (auto), channels.
    let mut cookie = Vec::new();
    for value in [
        2048,
        RATE as i32,
        fixed.unwrap_or(960) as i32,
        -1000,
        channels as i32,
        0,
        0,
    ] {
        cookie.extend_from_slice(&value.to_be_bytes());
    }
    chunk(&mut caf, b"kuki", &cookie);

    // Packet table: sizes (and frame counts when they vary), as varints.
    let mut table = Vec::new();
    table.extend_from_slice(&(packets.len() as i64).to_be_bytes());
    table.extend_from_slice(&(valid as i64).to_be_bytes());
    table.extend_from_slice(&(pre_skip as i32).to_be_bytes());
    table.extend_from_slice(&(remainder as i32).to_be_bytes());
    for (packet, &count) in packets.iter().zip(&frames) {
        varint(&mut table, packet.len() as u64);
        if fixed.is_none() {
            varint(&mut table, u64::from(count));
        }
    }
    chunk(&mut caf, b"pakt", &table);

    let mut data = 0u32.to_be_bytes().to_vec(); // edit count
    for packet in &packets {
        data.extend_from_slice(packet);
    }
    chunk(&mut caf, b"data", &data);

    let partial = output.with_extension("partial");
    std::fs::write(&partial, caf)?;
    std::fs::rename(partial, output)?;
    Ok(())
}

/// Samples (at 48 kHz) in one Opus packet, from its TOC byte (RFC 6716 §3.1).
fn packet_frames(packet: &[u8]) -> Result<u32> {
    let toc = packet[0];
    let config = toc >> 3;
    let per_frame = match config {
        0..=11 => [480, 960, 1920, 2880][usize::from(config % 4)],
        12..=15 => [480, 960][usize::from(config % 2)],
        _ => [120, 240, 480, 960][usize::from(config % 4)],
    };
    let count = match toc & 3 {
        0 => 1,
        1 | 2 => 2,
        _ => match packet.get(1) {
            Some(byte) => u32::from(byte & 0x3f),
            None => bail!("truncated Opus packet"),
        },
    };
    Ok(per_frame * count)
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(kind);
    out.extend_from_slice(&(body.len() as i64).to_be_bytes());
    out.extend_from_slice(body);
}

/// CAF's variable-length integers: 7 bits per byte, most significant first.
fn varint(out: &mut Vec<u8>, mut value: u64) {
    let mut bytes = vec![(value & 0x7f) as u8];
    value >>= 7;
    while value > 0 {
        bytes.push((value & 0x7f) as u8 | 0x80);
        value >>= 7;
    }
    bytes.reverse();
    out.extend_from_slice(&bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The demo voice note converts, and Core Audio reads the result.
    #[test]
    fn demo_voice_note() -> Result<()> {
        let input = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/demo/voice.oga");
        let output = std::env::temp_dir().join("kite-opus-test.caf");
        ogg_to_caf(&input, &output)?;
        let status = std::process::Command::new("afconvert")
            .args(["-f", "WAVE", "-d", "LEI16"])
            .arg(&output)
            .arg(output.with_extension("wav"))
            .status()?;
        ensure!(status.success(), "Core Audio couldn't decode the CAF");
        Ok(())
    }
}
