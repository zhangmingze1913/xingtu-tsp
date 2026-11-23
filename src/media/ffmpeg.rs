//! FFmpeg process adapter for real H.264 decoding. Kept outside the Rust link
//! graph so deployments can provide FFmpeg without compiling C/C++ codecs.

use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Debug)]
pub enum MediaError {
    Spawn(String),
    Decode(String),
    Size { expected: usize, actual: usize },
}

/// Decodes Annex-B H.264 to tightly packed RGB24 using an external FFmpeg.
pub struct FfmpegDecoder {
    program: PathBuf,
}

impl FfmpegDecoder {
    pub fn new(program: impl AsRef<Path>) -> Self {
        Self {
            program: program.as_ref().to_path_buf(),
        }
    }

    pub fn decode_rgb24(
        &self,
        annex_b: &[u8],
        width: usize,
        height: usize,
    ) -> Result<Vec<u8>, MediaError> {
        let mut child = Command::new(&self.program)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "h264",
                "-i",
                "pipe:0",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgb24",
                "pipe:1",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| MediaError::Spawn(error.to_string()))?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(annex_b)
                .map_err(|error| MediaError::Decode(error.to_string()))?;
        }
        let output = child
            .wait_with_output()
            .map_err(|error| MediaError::Decode(error.to_string()))?;
        if !output.status.success() {
            return Err(MediaError::Decode(
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ));
        }
        let frame_size = width.saturating_mul(height).saturating_mul(3);
        if frame_size == 0 || output.stdout.len() % frame_size != 0 {
            return Err(MediaError::Size {
                expected: frame_size,
                actual: output.stdout.len(),
            });
        }
        Ok(output.stdout)
    }

    /// Decodes ADTS AAC to mono 44.1 kHz signed 16-bit little-endian PCM.
    pub fn decode_audio_pcm16(&self, adts: &[u8]) -> Result<Vec<u8>, MediaError> {
        let mut child = Command::new(&self.program)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "aac",
                "-i",
                "pipe:0",
                "-f",
                "s16le",
                "-ac",
                "1",
                "-ar",
                "44100",
                "pipe:1",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| MediaError::Spawn(error.to_string()))?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(adts)
                .map_err(|error| MediaError::Decode(error.to_string()))?;
        }
        let output = child
            .wait_with_output()
            .map_err(|error| MediaError::Decode(error.to_string()))?;
        if !output.status.success() {
            return Err(MediaError::Decode(
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ));
        }
        if output.stdout.is_empty() || output.stdout.len() % 2 != 0 {
            return Err(MediaError::Size {
                expected: 2,
                actual: output.stdout.len(),
            });
        }
        Ok(output.stdout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffmpeg_decodes_a_real_h264_stream_when_configured() {
        let Some(ffmpeg) = std::env::var_os("TSP_FFMPEG") else {
            return;
        };
        let Some(sample) = std::env::var_os("TSP_H264_SAMPLE") else {
            return;
        };
        let stream = std::fs::read(sample).expect("read sample");
        let decoder = FfmpegDecoder::new(ffmpeg);
        let rgb = decoder.decode_rgb24(&stream, 64, 64).expect("decode");
        let frame_size = 64 * 64 * 3;
        assert!(rgb.len() >= frame_size);
        assert_eq!(rgb.len() % frame_size, 0);
        assert!(rgb.iter().any(|byte| *byte != 0));
    }

    #[test]
    fn ffmpeg_decodes_a_real_aac_stream_when_configured() {
        let Some(ffmpeg) = std::env::var_os("TSP_FFMPEG") else {
            return;
        };
        let Some(sample) = std::env::var_os("TSP_AAC_SAMPLE") else {
            return;
        };
        let stream = std::fs::read(sample).expect("read sample");
        let decoder = FfmpegDecoder::new(ffmpeg);
        let pcm = decoder.decode_audio_pcm16(&stream).expect("decode");
        assert!(pcm.len() >= 2);
        assert_eq!(pcm.len() % 2, 0);
        assert!(pcm.iter().any(|byte| *byte != 0));
    }
}
