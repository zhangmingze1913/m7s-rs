//! RGB frame compositing core for the Mix plugin.
pub fn blend(base: &[u8], top: &[u8], alpha: u8) -> Vec<u8> {
    base.iter().zip(top).map(|(a,b)| ((*a as u16 * (255 - alpha as u16) + *b as u16 * alpha as u16) / 255) as u8).collect()
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn full_top() { assert_eq!(blend(&[0,0,0], &[255,255,255], 255), vec![255,255,255]); }
}

fn ffmpeg() -> String {
    if let Ok(p) = std::env::var("M7S_FFMPEG") {
        return p;
    }
    let local = format!("{}/../../node_modules/ffmpeg-static/ffmpeg.exe", env!("CARGO_MANIFEST_DIR"));
    if std::path::Path::new(&local).exists() {
        local
    } else {
        "ffmpeg".into()
    }
}

/// Mix N inputs into one MP4 (hstack for 2, xstack otherwise).
pub fn mix(inputs: &[String], output: &std::path::Path, secs: u32) -> Result<(), String> {
    if inputs.is_empty() {
        return Err("no inputs".into());
    }
    if let Some(d) = output.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let mut cmd = std::process::Command::new(ffmpeg());
    cmd.args(["-hide_banner", "-loglevel", "error", "-y"]);
    for i in inputs {
        if let Some(spec) = i.strip_prefix("lavfi:") {
            cmd.args(["-f", "lavfi", "-i", spec]);
        } else {
            cmd.args(["-i", i]);
        }
    }
    let filter = if inputs.len() == 2 {
        "hstack=inputs=2".to_string()
    } else {
        let layout = (0..inputs.len())
            .map(|i| format!("{}_0", i * 64))
            .collect::<Vec<_>>()
            .join("|");
        format!("xstack=inputs={}:layout={layout}", inputs.len())
    };
    cmd.args(["-filter_complex", &filter, "-t", &secs.to_string(), "-c:v", "libx264", "-pix_fmt", "yuv420p"]);
    cmd.arg(output);
    let st = cmd.status().map_err(|e| e.to_string())?;
    if st.success() { Ok(()) } else { Err(format!("ffmpeg {st}")) }
}


/// Mix to stdout as fragmented MP4 bytes (no filesystem writes).
pub fn mix_bytes(inputs: &[String], secs: u32) -> Result<Vec<u8>, String> {
    if inputs.is_empty() {
        return Err("no inputs".into());
    }
    let mut cmd = std::process::Command::new(ffmpeg());
    cmd.args(["-hide_banner", "-loglevel", "error", "-y"]);
    for i in inputs {
        if let Some(spec) = i.strip_prefix("lavfi:") {
            cmd.args(["-f", "lavfi", "-i", spec]);
        } else {
            cmd.args(["-i", i]);
        }
    }
    let filter = if inputs.len() == 2 {
        "hstack=inputs=2".to_string()
    } else {
        format!("xstack=inputs={}", inputs.len())
    };
    cmd.args(["-filter_complex", &filter, "-t", &secs.to_string(), "-c:v", "libx264", "-pix_fmt", "yuv420p", "-movflags", "frag_keyframe+empty_moov", "-f", "mp4", "pipe:1"]);
    let out = cmd.output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("ffmpeg {:?}", out.status));
    }
    Ok(out.stdout)
}

#[cfg(test)]
mod mix_bytes_tests {
    #[test]
    fn mixes_two_sources_to_bytes() {
        let inputs = vec![
            "lavfi:color=red:s=64x64:d=1".to_string(),
            "lavfi:color=blue:s=64x64:d=1".to_string(),
        ];
        let b = super::mix_bytes(&inputs, 1).unwrap();
        assert!(b.len() > 100, "{}", b.len());
        assert_eq!(&b[4..8], b"ftyp");
    }
}

/// Blend a `top` RGB frame (w x h) into `base` at (x, y), clipping bounds.
#[allow(clippy::too_many_arguments)]
pub fn blend_region(
    base: &mut [u8],
    base_w: usize,
    base_h: usize,
    top: &[u8],
    top_w: usize,
    top_h: usize,
    x: usize,
    y: usize,
    alpha: u8,
) {
    for row in 0..top_h {
        let by = y + row;
        if by >= base_h {
            break;
        }
        for col in 0..top_w {
            let bx = x + col;
            if bx >= base_w {
                break;
            }
            let bi = (by * base_w + bx) * 3;
            let ti = (row * top_w + col) * 3;
            for c in 0..3 {
                let a = base[bi + c] as u16;
                let b = top[ti + c] as u16;
                base[bi + c] = ((a * (255 - alpha as u16) + b * alpha as u16) / 255) as u8;
            }
        }
    }
}

/// Horizontal stack of two RGB frames.
pub fn stack_horizontal(left: &[u8], right: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(w * h * 6);
    for y in 0..h {
        let row = y * w * 3;
        out.extend_from_slice(&left[row..row + w * 3]);
        out.extend_from_slice(&right[row..row + w * 3]);
    }
    out
}

/// Vertical stack of two RGB frames.
pub fn stack_vertical(top: &[u8], bottom: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(w * h * 6);
    out.extend_from_slice(&top[..w * h * 3]);
    out.extend_from_slice(&bottom[..w * h * 3]);
    out
}

#[cfg(test)]
mod native_tests {
    use super::*;

    #[test]
    fn region_and_stack() {
        let mut base = vec![0u8; 4 * 4 * 3];
        let top = vec![255u8; 2 * 2 * 3];
        blend_region(&mut base, 4, 4, &top, 2, 2, 1, 1, 255);
        let at = 5 * 3;
        assert_eq!(&base[at..at + 3], &[255, 255, 255]);
        let left = vec![1u8; 2 * 2 * 3];
        let right = vec![2u8; 2 * 2 * 3];
        let h = stack_horizontal(&left, &right, 2, 2);
        assert_eq!(h.len(), 2 * 2 * 6);
        assert_eq!(&h[..3], &[1, 1, 1]);
        assert_eq!(&h[6..9], &[2, 2, 2]);
        let v = stack_vertical(&left, &right, 2, 2);
        assert_eq!(v.len(), 24);
        assert_eq!(&v[0..3], &[1, 1, 1]);
        assert_eq!(&v[12..15], &[2, 2, 2]);
    }
}
