//! Order 090: reading a sound file Windows itself can decode (MP3 above all - "select the in sound and an out sound with his own
//! .mp3") through Media Foundation's source reader: the file is opened by Windows' own decoder and handed over as 32-bit float
//! PCM. Nothing is installed, nothing is written; the file is only read. Called inside the decode helper ([`crate::safe`]), so
//! a decoder that falls over on a broken file can't take the app with it.

use std::path::Path;

use windows::core::HSTRING;
use windows::Win32::Media::MediaFoundation::{
    IMFSample, MFAudioFormat_Float, MFCreateMediaType, MFCreateSourceReaderFromURL, MFMediaType_Audio, MFShutdown, MFStartup, MFSTARTUP_LITE,
    MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_SOURCE_READERF_ENDOFSTREAM, MF_SOURCE_READER_ALL_STREAMS,
    MF_SOURCE_READER_FIRST_AUDIO_STREAM, MF_VERSION,
};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

/// The file at `path` as (rate, mono samples), at most `max_samples` of them (a longer file is cut).
pub fn decode_file(path: &Path, max_samples: usize) -> Result<(u32, Vec<f32>), String> {
    // SAFETY: COM + MF for this thread, balanced below.
    let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
    let mf = unsafe { MFStartup(MF_VERSION, MFSTARTUP_LITE) };
    let out = match &mf {
        Ok(()) => read(path, max_samples),
        Err(e) => Err(format!("Windows' media decoder isn't available: {}", e.message())),
    };
    // SAFETY: balanced with the calls above.
    unsafe {
        if mf.is_ok() {
            let _ = MFShutdown();
        }
        if com {
            CoUninitialize();
        }
    }
    out
}

fn read(path: &Path, max_samples: usize) -> Result<(u32, Vec<f32>), String> {
    let bad = |e: windows::core::Error| format!("Windows can't read this sound: {}", e.message().trim());
    let first = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
    // SAFETY: plain Media Foundation calls on objects this function owns.
    unsafe {
        let reader = MFCreateSourceReaderFromURL(&HSTRING::from(path.as_os_str()), None).map_err(bad)?;
        reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false).map_err(bad)?;
        reader.SetStreamSelection(first, true).map_err(|_| "there is no sound in this file".to_string())?;
        let want = MFCreateMediaType().map_err(bad)?;
        want.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio).map_err(bad)?;
        want.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_Float).map_err(bad)?;
        reader.SetCurrentMediaType(first, None, &want).map_err(bad)?;
        let got = reader.GetCurrentMediaType(first).map_err(bad)?;
        let ch = got.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS).map_err(bad)?.max(1) as usize;
        let rate = got.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND).map_err(bad)?;
        if !(8000..=192_000).contains(&rate) || ch > 16 {
            return Err(format!("unusable sound ({rate} Hz, {ch} channels)"));
        }
        let mut mono: Vec<f32> = Vec::new();
        // (a reader that keeps answering without samples must not spin for ever)
        for _ in 0..1_000_000 {
            let mut flags = 0u32;
            let mut sample: Option<IMFSample> = None;
            reader.ReadSample(first, 0, None, Some(&mut flags), None, Some(&mut sample)).map_err(bad)?;
            // MF_SOURCE_READERF_ERROR
            if flags & 1 != 0 {
                return Err("the sound file is damaged".into());
            }
            if let Some(s) = sample {
                let buf = s.ConvertToContiguousBuffer().map_err(bad)?;
                let mut p: *mut u8 = std::ptr::null_mut();
                let mut len = 0u32;
                buf.Lock(&mut p, None, Some(&mut len)).map_err(bad)?;
                if !p.is_null() && len >= 4 {
                    // the buffer is 32-bit floats, interleaved by channel (what was asked for above)
                    let n = len as usize / 4;
                    let mut floats = vec![0f32; n];
                    std::ptr::copy_nonoverlapping(p, floats.as_mut_ptr() as *mut u8, n * 4);
                    for f in floats.chunks_exact(ch) {
                        let v = f.iter().sum::<f32>() / ch as f32;
                        mono.push(if v.is_finite() { v } else { 0.0 });
                    }
                }
                let _ = buf.Unlock();
            }
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 || mono.len() >= max_samples {
                break;
            }
        }
        mono.truncate(max_samples);
        if mono.is_empty() {
            return Err("there is no sound in this file".into());
        }
        Ok((rate, mono))
    }
}

#[cfg(test)]
mod tests {
    /// Windows' own decoder reads a file into mono floats at its own rate (a WAV here: the same reader an MP3 goes through;
    /// no MP3 file is on this PC to test with).
    #[test]
    fn windows_decodes_a_file_to_mono_floats() {
        let rate = 22_050u32;
        let n = (rate / 5) as usize;
        let mut d = Vec::new();
        for i in 0..n {
            let t = i as f32 / rate as f32;
            d.extend_from_slice(&(((t * 440.0 * std::f32::consts::TAU).sin() * 20000.0) as i16).to_le_bytes());
        }
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&(36 + d.len() as u32).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        for x in [16u32.to_le_bytes().to_vec(), 1u16.to_le_bytes().to_vec(), 2u16.to_le_bytes().to_vec(), rate.to_le_bytes().to_vec(), (rate * 4).to_le_bytes().to_vec(), 4u16.to_le_bytes().to_vec(), 16u16.to_le_bytes().to_vec()] {
            v.extend_from_slice(&x);
        }
        // (stereo: each sample twice)
        let mut st = Vec::new();
        for c in d.chunks_exact(2) {
            st.extend_from_slice(c);
            st.extend_from_slice(c);
        }
        v.extend_from_slice(b"data");
        v.extend_from_slice(&(st.len() as u32).to_le_bytes());
        v.extend_from_slice(&st);
        let p = std::env::temp_dir().join(format!("bu-mf-test-{}.wav", std::process::id()));
        std::fs::write(&p, v).unwrap();
        let (r, mono) = super::decode_file(&p, 1_000_000).unwrap();
        let _ = std::fs::remove_file(&p);
        assert_eq!(r, rate);
        assert!((mono.len() as i64 - n as i64).abs() < 64, "{} of {n} samples", mono.len());
        let peak = mono.iter().fold(0f32, |m, x| m.max(x.abs()));
        assert!((peak - 20000.0 / 32768.0).abs() < 0.02, "{peak}");
        // a file that isn't a sound: a sentence, no panic
        let q = std::env::temp_dir().join(format!("bu-mf-test-{}.mp3", std::process::id()));
        std::fs::write(&q, b"not a sound at all").unwrap();
        assert!(super::decode_file(&q, 1000).is_err());
        let _ = std::fs::remove_file(&q);
    }
}
