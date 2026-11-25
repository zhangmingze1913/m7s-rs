//! FFmpeg transcode plugin.
pub fn args(input: &str, output: &str, width: u32, bitrate_k: u32) -> Vec<String> {
    vec!["-hide_banner".into(),"-loglevel".into(),"error".into(),"-y".into(),
         "-i".into(),input.into(),"-vf".into(),format!("scale={width}:-2"),
         "-c:v".into(),"libx264".into(),"-b:v".into(),format!("{bitrate_k}k"),
         "-c:a".into(),"aac".into(),output.into()]
}
pub fn run(input: &str, output: &str, width: u32, bitrate_k: u32) -> Result<(), String> {
    run_secs(input, output, width, bitrate_k, 0)
}

pub fn run_secs(
    input: &str,
    output: &str,
    width: u32,
    bitrate_k: u32,
    secs: u32,
) -> Result<(), String> {
    let ff = std::env::var("M7S_FFMPEG").unwrap_or_else(|_| {
        if std::path::Path::new("node_modules/ffmpeg-static/ffmpeg.exe").exists() {
            "node_modules/ffmpeg-static/ffmpeg.exe".into()
        } else {
            "ffmpeg".into()
        }
    });
    let mut args = args(input, output, width, bitrate_k);
    if secs > 0 {
        let pos = args.len() - 1;
        args.splice(pos..pos, ["-t".to_string(), secs.to_string()]);
    }
    let st = std::process::Command::new(ff).args(args).status().map_err(|e| e.to_string())?;
    if st.success() { Ok(()) } else { Err(format!("ffmpeg {st}")) }
}
#[cfg(test)] mod tests { use super::*; #[test] fn has_scale() { assert!(args("a","b",640,800).iter().any(|s| s=="scale=640:-2")); } }

/// Nearest-neighbour RGB scale (native compositing building block).
pub fn scale_rgb(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u8> {
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 {
        return Vec::new();
    }
    let mut out = vec![0u8; dw * dh * 3];
    for y in 0..dh {
        let sy = y * sh / dh;
        for x in 0..dw {
            let sx = x * sw / dw;
            let si = (sy * sw + sx) * 3;
            let di = (y * dw + x) * 3;
            out[di..di + 3].copy_from_slice(&src[si..si + 3]);
        }
    }
    out
}

#[cfg(test)]
mod native_scale_tests {
    use super::*;
    #[test]
    fn scales_nearest() {
        let src = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let out = scale_rgb(&src, 2, 2, 4, 4);
        assert_eq!(out.len(), 4 * 4 * 3);
        assert_eq!(&out[0..3], &[1, 2, 3]);
        assert_eq!(&out[(3 * 4 + 3) * 3..][..3], &[10, 11, 12]);
    }
}
