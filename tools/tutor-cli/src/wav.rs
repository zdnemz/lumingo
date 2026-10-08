//! Reading a WAV file into 16 kHz mono samples, for scripted voice turns.
//!
//! Only what a recording of speech needs: RIFF/WAVE with 16, 24 or 32 bit
//! integer PCM or 32 bit float samples, any channel count (averaged to mono) and
//! any rate (resampled to 16 kHz).

use anyhow::{Context, Result, bail};
use audio_io::MonoResampler;
use speech::SPEECH_SAMPLE_RATE;

struct Format {
    float: bool,
    channels: usize,
    rate: u32,
    bits: u16,
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn parse_format(chunk: &[u8]) -> Result<Format> {
    let tag = u16_at(chunk, 0).context("the fmt chunk is too short")?;
    let channels = u16_at(chunk, 2).context("the fmt chunk is too short")?;
    let rate = u32_at(chunk, 4).context("the fmt chunk is too short")?;
    let bits = u16_at(chunk, 14).context("the fmt chunk is too short")?;
    // WAVE_FORMAT_EXTENSIBLE names the real format in the first two bytes of its sub-format.
    let tag = if tag == 0xFFFE {
        u16_at(chunk, 24).context("the extensible fmt chunk is too short")?
    } else {
        tag
    };
    let float = match (tag, bits) {
        (1, 16 | 24 | 32) => false,
        (3, 32) => true,
        _ => bail!("only 16, 24 and 32 bit PCM and 32 bit float WAV files are supported"),
    };
    if channels == 0 || rate == 0 {
        bail!("the WAV file reports no channels or a rate of 0 Hz");
    }
    Ok(Format {
        float,
        channels: usize::from(channels),
        rate,
        bits,
    })
}

fn decode(data: &[u8], format: &Format) -> Vec<f32> {
    let width = usize::from(format.bits / 8);
    let frame_bytes = width * format.channels;
    data.chunks_exact(frame_bytes)
        .map(|frame| {
            let sum: f32 = frame.chunks_exact(width).map(|s| sample(s, format)).sum();
            sum / format.channels as f32
        })
        .collect()
}

fn sample(bytes: &[u8], format: &Format) -> f32 {
    match (format.float, format.bits) {
        (true, _) => f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        (false, 16) => f32::from(i16::from_le_bytes([bytes[0], bytes[1]])) / 32_768.0,
        (false, 24) => {
            // Sign-extend the three bytes through the top of an i32.
            let value = i32::from_le_bytes([0, bytes[0], bytes[1], bytes[2]]) >> 8;
            value as f32 / 8_388_608.0
        }
        _ => {
            let value = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            value as f32 / 2_147_483_648.0
        }
    }
}

/// Parses WAV bytes into 16 kHz mono `f32`.
pub fn parse(bytes: &[u8]) -> Result<Vec<f32>> {
    if bytes.get(0..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        bail!("this is not a RIFF/WAVE file");
    }
    let mut format = None;
    let mut data = None;
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32_at(bytes, at + 4).context("a chunk header is cut off")? as usize;
        let body = bytes
            .get(at + 8..(at + 8 + size).min(bytes.len()))
            .context("a chunk is cut off")?;
        match id {
            b"fmt " => format = Some(parse_format(body)?),
            b"data" => data = Some(body),
            _ => {}
        }
        // Chunks are padded to an even length.
        at += 8 + size + (size & 1);
    }
    let format = format.context("the WAV file has no fmt chunk")?;
    let data = data.context("the WAV file has no data chunk")?;
    let mono = decode(data, &format);
    if mono.is_empty() {
        bail!("the WAV file holds no audio");
    }
    if format.rate == SPEECH_SAMPLE_RATE {
        return Ok(mono);
    }
    let mut resampler = MonoResampler::new(format.rate, SPEECH_SAMPLE_RATE)
        .context("the sample rate cannot be converted")?;
    resampler
        .convert_clip(&mono)
        .context("the audio could not be resampled to 16 kHz")
}

/// Encodes 16 kHz mono samples as 16 bit PCM WAV. Used by the tests to make a
/// recording to feed.
pub fn encode_pcm16(samples: &[f32], rate: u32) -> Vec<u8> {
    let data_len = samples.len() * 2;
    let mut out = Vec::with_capacity(44 + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(
        &u32::try_from(36 + data_len)
            .unwrap_or(u32::MAX)
            .to_le_bytes(),
    );
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&u32::try_from(data_len).unwrap_or(u32::MAX).to_le_bytes());
    for s in samples {
        let value = (s.clamp(-1.0, 1.0) * 32_767.0) as i16;
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: u32, ms: u32) -> Vec<f32> {
        (0..u64::from(rate) * u64::from(ms) / 1000)
            .map(|i| 0.5 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / rate as f32).sin())
            .collect()
    }

    #[test]
    fn a_16_khz_mono_file_round_trips_within_quantisation() {
        let samples = tone(16_000, 100);
        let back = parse(&encode_pcm16(&samples, 16_000)).unwrap();
        assert_eq!(back.len(), samples.len());
        for (a, b) in samples.iter().zip(&back) {
            assert!((a - b).abs() < 1e-3);
        }
    }

    #[test]
    fn another_rate_is_resampled_to_16_khz() {
        let back = parse(&encode_pcm16(&tone(48_000, 200), 48_000)).unwrap();
        assert_eq!(back.len(), 3_200);
    }

    #[test]
    fn stereo_is_averaged_to_mono() {
        // Left 0.5, right -0.5 cancel; left 0.5, right 0.5 stay.
        let mut bytes = encode_pcm16(&[0.0; 0], 16_000);
        bytes.truncate(44);
        let frames: [(i16, i16); 2] = [(16_384, -16_384), (16_384, 16_384)];
        let mut data = Vec::new();
        for (l, r) in frames {
            data.extend_from_slice(&l.to_le_bytes());
            data.extend_from_slice(&r.to_le_bytes());
        }
        // Rewrite the header for two channels.
        bytes[22..24].copy_from_slice(&2_u16.to_le_bytes());
        bytes[28..32].copy_from_slice(&64_000_u32.to_le_bytes());
        bytes[32..34].copy_from_slice(&4_u16.to_le_bytes());
        bytes[40..44].copy_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&data);
        let mono = parse(&bytes).unwrap();
        assert_eq!(mono.len(), 2);
        assert!(mono[0].abs() < 1e-6);
        assert!((mono[1] - 0.5).abs() < 1e-3);
    }

    #[test]
    fn what_is_not_a_supported_wav_is_refused_with_a_reason() {
        for (bytes, reason) in [
            (b"not a wav file at all".to_vec(), "RIFF/WAVE"),
            (b"RIFF\0\0\0\0WAVE".to_vec(), "no fmt chunk"),
        ] {
            let error = parse(&bytes).unwrap_err().to_string();
            assert!(error.contains(reason), "{error}");
        }
        let empty = encode_pcm16(&[], 16_000);
        let error = parse(&empty).unwrap_err().to_string();
        assert!(error.contains("no audio"), "{error}");
    }
}
