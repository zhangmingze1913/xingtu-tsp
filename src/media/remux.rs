//! Remux decoded/encoded JT/T 1078 streams into fragmented MP4 for browser MSE
//! playback. This does not re-encode; it only repackages Annex-B/ADTS.

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use super::ffmpeg::MediaError;

pub struct FfmpegRemuxer {
    program: PathBuf,
}

impl FfmpegRemuxer {
    pub fn new(program: impl AsRef<Path>) -> Self {
        Self {
            program: program.as_ref().to_path_buf(),
        }
    }

    /// Returns a fragmented MP4 with `ftyp`/`moov`/`moof` boxes.
    pub fn to_fmp4(
        &self,
        video_annex_b: Option<&[u8]>,
        audio_adts: Option<&[u8]>,
    ) -> Result<Vec<u8>, MediaError> {
        if video_annex_b.is_none() && audio_adts.is_none() {
            return Err(MediaError::Decode("no media input".to_string()));
        }
        let dir = std::env::temp_dir().join(format!("tsp-remux-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).map_err(|error| MediaError::Spawn(error.to_string()))?;
        let video_path = dir.join("video.h264");
        let audio_path = dir.join("audio.aac");
        let mut args: Vec<String> = vec!["-hide_banner".into(), "-loglevel".into(), "error".into()];
        if let Some(video) = video_annex_b {
            std::fs::write(&video_path, video).map_err(|e| MediaError::Spawn(e.to_string()))?;
            args.extend(["-f", "h264", "-i"].map(str::to_string));
            args.push(video_path.to_string_lossy().into_owned());
        }
        if let Some(audio) = audio_adts {
            std::fs::write(&audio_path, audio).map_err(|e| MediaError::Spawn(e.to_string()))?;
            args.extend(["-f", "aac", "-i"].map(str::to_string));
            args.push(audio_path.to_string_lossy().into_owned());
        }
        args.extend(
            [
                "-c",
                "copy",
                "-bsf:a",
                "aac_adtstoasc",
                "-movflags",
                "frag_keyframe+empty_moov+default_base_moof",
                "-f",
                "mp4",
                "pipe:1",
            ]
            .map(str::to_string),
        );
        let output = Command::new(&self.program)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|error| MediaError::Spawn(error.to_string()))?;
        let _ = std::fs::remove_dir_all(&dir);
        if !output.status.success() {
            return Err(MediaError::Decode(
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ));
        }
        if output.stdout.len() < 12 || &output.stdout[4..8] != b"ftyp" {
            return Err(MediaError::Decode("missing ftyp box".to_string()));
        }
        if !output.stdout.windows(4).any(|window| window == b"moof") {
            return Err(MediaError::Decode("missing moof box".to_string()));
        }
        Ok(output.stdout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remuxes_real_streams_to_fragmented_mp4() {
        let Some(ffmpeg) = std::env::var_os("TSP_FFMPEG") else {
            return;
        };
        let Some(video) = std::env::var_os("TSP_H264_SAMPLE") else {
            return;
        };
        let Some(audio) = std::env::var_os("TSP_AAC_SAMPLE") else {
            return;
        };
        let video = std::fs::read(video).expect("video");
        let audio = std::fs::read(audio).expect("audio");
        let remuxer = FfmpegRemuxer::new(ffmpeg);
        let mp4 = remuxer.to_fmp4(Some(&video), Some(&audio)).expect("remux");
        assert_eq!(&mp4[4..8], b"ftyp");
        assert!(mp4.windows(4).any(|window| window == b"moof"));
    }
}
