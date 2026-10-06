//! Fixture sets of BENCHMARK_PLAN section 4: WAV I/O, the generated F1 set, and
//! the loader that lists counts and durations per set.

use anyhow::{Context, Result, bail};
use std::{f64::consts::PI, fs, path::Path};

/// 16-bit PCM mono is all the harness reads and writes.
pub fn write_wav(path: &Path, rate: u32, samples: &[f32]) -> Result<()> {
    let data_len = u32::try_from(samples.len() * 2).context("clip too long for a WAV file")?;
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend(b"RIFF");
    out.extend((36 + data_len).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes()); // PCM
    out.extend(1u16.to_le_bytes()); // mono
    out.extend(rate.to_le_bytes());
    out.extend((rate * 2).to_le_bytes());
    out.extend(2u16.to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend(data_len.to_le_bytes());
    for s in samples {
        out.extend(((s.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
    }
    fs::write(path, out).with_context(|| format!("writing {}", path.display()))
}

/// Duration in seconds from the header of a PCM WAV file.
pub fn wav_seconds(bytes: &[u8]) -> Result<f64> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        bail!("not a RIFF/WAVE file");
    }
    let (mut pos, mut byte_rate, mut data_len) = (12, 0u32, None);
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32::from_le_bytes([
            bytes[pos + 4],
            bytes[pos + 5],
            bytes[pos + 6],
            bytes[pos + 7],
        ]) as usize;
        let body = pos + 8;
        if id == b"fmt " && body + 12 <= bytes.len() {
            byte_rate = u32::from_le_bytes([
                bytes[body + 8],
                bytes[body + 9],
                bytes[body + 10],
                bytes[body + 11],
            ]);
        } else if id == b"data" {
            data_len = Some(len.min(bytes.len() - body));
            break;
        }
        pos = body + len + (len & 1);
    }
    match (data_len, byte_rate) {
        (Some(d), r) if r > 0 => Ok(d as f64 / f64::from(r)),
        _ => bail!("missing fmt or data chunk"),
    }
}

const SECONDS: usize = 3;

fn sweep(rate: u32) -> Vec<f32> {
    // Log sweep 100 Hz to 7 kHz, so every rate under test stays below Nyquist.
    let (f0, f1) = (100.0_f64, 7000.0_f64);
    let n = rate as usize * SECONDS;
    let t_total = SECONDS as f64;
    let k = (f1 / f0).ln() / t_total;
    (0..n)
        .map(|i| {
            let t = i as f64 / f64::from(rate);
            (0.5 * (2.0 * PI * f0 * ((k * t).exp() - 1.0) / k).sin()) as f32
        })
        .collect()
}

/// Deterministic noise so the committed files never change between runs.
fn pink_noise(rate: u32) -> Vec<f32> {
    let mut seed = 0x2545_F491_4F6C_DD1D_u64;
    let mut white = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    };
    // Paul Kellet's economy filter.
    let (mut b0, mut b1, mut b2) = (0.0, 0.0, 0.0);
    (0..rate as usize * SECONDS)
        .map(|_| {
            let w = white();
            b0 = 0.99765 * b0 + w * 0.099_046;
            b1 = 0.963 * b1 + w * 0.296_516_4;
            b2 = 0.57 * b2 + w * 1.052_691_3;
            ((b0 + b1 + b2 + w * 0.1848) * 0.11) as f32
        })
        .collect()
}

/// Writes the F1 set into `dir`.
pub fn generate_f1(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir)?;
    for rate in [16_000, 44_100, 48_000] {
        write_wav(&dir.join(format!("sweep-{rate}.wav")), rate, &sweep(rate))?;
    }
    write_wav(
        &dir.join("silence-16000.wav"),
        16_000,
        &vec![0.0; 16_000 * SECONDS],
    )?;
    write_wav(&dir.join("pink-16000.wav"), 16_000, &pink_noise(16_000))?;
    let clipped: Vec<f32> = sweep(16_000)
        .iter()
        .map(|s| (s * 4.0).clamp(-1.0, 1.0))
        .collect();
    write_wav(&dir.join("clipped-16000.wav"), 16_000, &clipped)
}

/// One line per set: WAV count and total duration, plus the count of script lines
/// when `scripts/<set>.txt` exists. Returns the lines so tests can check them.
pub fn list(root: &Path) -> Result<Vec<String>> {
    let mut sets: Vec<_> = fs::read_dir(root)
        .with_context(|| format!("reading {}", root.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir() && e.file_name() != "scripts")
        .collect();
    sets.sort_by_key(|e| e.file_name());
    let mut lines = Vec::new();
    for set in sets {
        let (mut count, mut seconds) = (0usize, 0.0);
        for wav in fs::read_dir(set.path())?.filter_map(|e| e.ok()) {
            let path = wav.path();
            if path.extension().is_some_and(|x| x == "wav") {
                seconds +=
                    wav_seconds(&fs::read(&path)?).with_context(|| path.display().to_string())?;
                count += 1;
            }
        }
        let name = set.file_name().to_string_lossy().into_owned();
        let script = fs::read_to_string(root.join("scripts").join(format!("{name}.txt")))
            .ok()
            .map(|s| {
                s.lines()
                    .filter(|l| !l.is_empty() && !l.starts_with('#'))
                    .count()
            });
        lines.push(match script {
            Some(n) => format!("{name}: {count} wav, {seconds:.1} s, script lines {n}"),
            None => format!("{name}: {count} wav, {seconds:.1} s"),
        });
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("bench-fixtures-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn wav_round_trips_its_duration() {
        let d = temp("wav");
        fs::create_dir_all(&d).unwrap();
        write_wav(&d.join("a.wav"), 16_000, &vec![0.1; 8_000]).unwrap();
        let secs = wav_seconds(&fs::read(d.join("a.wav")).unwrap()).unwrap();
        assert!((secs - 0.5).abs() < 1e-9);
        assert!(wav_seconds(b"not a wav at all").is_err());
    }

    #[test]
    fn f1_is_listed_with_count_and_duration_and_is_deterministic() {
        let root = temp("f1");
        generate_f1(&root.join("f1")).unwrap();
        assert_eq!(list(&root).unwrap(), vec!["f1: 6 wav, 18.0 s"]);
        let first = fs::read(root.join("f1/pink-16000.wav")).unwrap();
        generate_f1(&root.join("f1")).unwrap();
        assert_eq!(first, fs::read(root.join("f1/pink-16000.wav")).unwrap());
    }

    #[test]
    fn script_lines_ignore_comments() {
        let root = temp("script");
        fs::create_dir_all(root.join("f2")).unwrap();
        fs::create_dir_all(root.join("scripts")).unwrap();
        fs::write(root.join("scripts/f2.txt"), "# c\nf2-001|a\n\nf2-002|b\n").unwrap();
        assert_eq!(
            list(&root).unwrap(),
            vec!["f2: 0 wav, 0.0 s, script lines 2"]
        );
    }
}
