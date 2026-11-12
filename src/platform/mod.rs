use std::process::{Child, Command};

pub const INDICATOR_WIDTH: u32 = 64;
pub const INDICATOR_HEIGHT: u32 = 24;
pub const INDICATOR_SCALE: u32 = 2;
pub const RELEASES_URL: &str = "https://github.com/dayblox/whispertype/releases/latest";
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

pub fn spawn_hidden(command: &mut Command) -> std::io::Result<Child> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command.spawn()
}

/// Small signed-distance shapes, premultiplied BGRA for both Wayland and Win32.
pub fn pixels(kind: &str, width: usize, height: usize) -> Vec<u8> {
    let mut image = vec![0; width * height * 4];
    let color = match kind {
        "transcribing" => [122.0, 162.0, 247.0],
        "enrolling" | "shortcut" => [224.0, 175.0, 104.0],
        _ => [158.0, 206.0, 106.0],
    };
    let scale = height as f32 / INDICATOR_HEIGHT as f32;
    let rounded = |x: f32, y: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32| {
        let dx = (x - cx).abs() - hw + r;
        let dy = (y - cy).abs() - hh + r;
        dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0) - r
    };
    for y in 0..height {
        for x in 0..width {
            let px = (x as f32 + 0.5) / scale;
            let py = (y as f32 + 0.5) / scale;
            let d = rounded(px, py, 32.0, 12.0, 32.0, 12.0, 12.0);
            let a = (0.5 - d * scale).clamp(0.0, 1.0) * 0.96;
            let mut rgb = [23.0, 26.0, 35.0];
            let border = (1.5 - d.abs() * scale).clamp(0.0, 1.0) * 0.4;
            for i in 0..3 {
                rgb[i] += border * (color[i] - rgb[i]);
            }
            for (i, h) in [4.0, 8.0, 14.0, 8.0, 4.0].into_iter().enumerate() {
                let bar = rounded(px, py, 20.0 + i as f32 * 6.0, 12.0, 1.3, h / 2.0, 1.3);
                let amount = (0.5 - bar * scale).clamp(0.0, 1.0);
                for c in 0..3 {
                    rgb[c] = rgb[c] * (1.0 - amount) + color[c] * amount;
                }
            }
            let at = (y * width + x) * 4;
            image[at] = (rgb[2] * a) as u8;
            image[at + 1] = (rgb[1] * a) as u8;
            image[at + 2] = (rgb[0] * a) as u8;
            image[at + 3] = (255.0 * a) as u8;
        }
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indicator_reaches_bottom_edge_without_transparent_padding() {
        let width = (INDICATOR_WIDTH * INDICATOR_SCALE) as usize;
        let height = (INDICATOR_HEIGHT * INDICATOR_SCALE) as usize;
        for kind in ["recording", "transcribing", "enrolling"] {
            let image = pixels(kind, width, height);
            assert_eq!(image.len(), width * height * 4);
            assert!(image[((height - 1) * width + width / 2) * 4 + 3] > 200);
            assert_eq!(image[3], 0, "Corners must remain rounded");
        }
    }
}
