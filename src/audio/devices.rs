//! CPAL owns enumeration and stable host IDs. No listeners or idle discovery.
use cpal::traits::{DeviceTrait, HostTrait};
#[cfg(target_os = "linux")]
mod pulse;
#[cfg(windows)]
mod windows;

pub(super) fn id(device: &cpal::Device) -> anyhow::Result<String> {
    let id = device.id()?.to_string();
    #[cfg(target_os = "linux")]
    if let Some(id) = source_identity(&id) {
        return Ok(id);
    }
    Ok(id)
}

pub(super) fn identity(device: &cpal::Device, default: Option<&str>) -> anyhow::Result<String> {
    let id = id(device)?;
    Ok(resolve_alias(&id, default).unwrap_or(id))
}

pub(super) fn resolve_alias(id: &str, default: Option<&str>) -> Option<String> {
    #[cfg(target_os = "linux")]
    if matches!(id, "alsa:default" | "alsa:pulse" | "alsa:pipewire") {
        if let Some(default) = default {
            return Some(default.to_owned());
        }
    }
    let _ = (id, default);
    None
}

pub(super) fn default_id(device: Option<&cpal::Device>, remembered: bool) -> Option<String> {
    #[cfg(target_os = "linux")]
    if remembered {
        if let Some(snapshot) = pulse::snapshot(false) {
            return Some(pulse_id(&snapshot.default));
        }
    }
    let _ = remembered;
    device.and_then(|d| id(d).ok())
}

#[cfg(target_os = "linux")]
fn pulse_id(name: &str) -> String {
    format!("capture-server:{}", quoted(name))
}

#[cfg(any(target_os = "linux", test))]
fn quoted(name: &str) -> String {
    format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""))
}
#[cfg(any(target_os = "linux", test))]
fn source_identity(id: &str) -> Option<String> {
    id.strip_prefix("alsa:pulse:DEVICE=")
        .or_else(|| id.strip_prefix("alsa:pipewire:NODE="))
        .map(|name| format!("capture-server:{name}"))
}

pub(super) fn inputs(primary: &str) -> anyhow::Result<std::vec::IntoIter<cpal::Device>> {
    let host = cpal::default_host();
    #[cfg(target_os = "linux")]
    if primary.starts_with("capture-server:") {
        if let Some(snapshot) = pulse::snapshot(true) {
            // CPAL's ALSA backend accepts named pulse PCMs without introducing
            // a second backend, polling thread, daemon, or audio server setting.
            let mut devices = Vec::new();
            for name in snapshot.sources {
                if name.ends_with(".monitor") {
                    continue;
                }
                // pipewire-alsa may be installed without the PulseAudio ALSA
                // plugin. Try both selectors for each source; identity() makes
                // them one device, so an opened/dead source is never tried twice.
                for selector in [
                    format!("alsa:pipewire:NODE={}", quoted(&name)),
                    format!("alsa:pulse:DEVICE={}", quoted(&name)),
                ] {
                    if let Ok(id) = selector.parse::<cpal::DeviceId>() {
                        if let Some(device) = host.device_by_id(&id) {
                            devices.push(device);
                        }
                    }
                }
            }
            return Ok(devices.into_iter());
        }
    }
    let _ = primary;
    Ok(host.input_devices()?.collect::<Vec<_>>().into_iter())
}

#[cfg(windows)]
pub(super) use windows::name;

#[cfg(target_os = "linux")]
pub(super) fn name(device: &cpal::Device) -> anyhow::Result<String> {
    // 0.16 exposed friendly ALSA card names. Keep substring configuration
    // matching descriptions as well as PCM IDs after the stable-ID API update.
    let description = device.description()?;
    let mut name = description.name().to_owned();
    // ALSA card descriptions are in extended lines. Preserve existing substrings.
    for line in description.extended() {
        name.push('\n');
        name.push_str(line);
    }
    Ok(name)
}

pub(super) fn candidate(device: &cpal::Device, id: &str) -> bool {
    #[cfg(windows)]
    {
        // WASAPI's CPAL input enumeration checks eCapture, excluding eRender
        // endpoints that can only be captured through loopback.
        let _ = id;
        device.supports_input()
    }
    #[cfg(target_os = "linux")]
    {
        let name = name(device).unwrap_or_default();
        !id.contains(".monitor") && !name.contains(".monitor") && id != "alsa:null"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn linux_source_selectors_share_identity_and_quote_names() {
        let source = quoted("mic.USB\\name\"input");
        assert_eq!(
            source_identity(&format!("alsa:pulse:DEVICE={source}")),
            source_identity(&format!("alsa:pipewire:NODE={source}"))
        );
        assert_eq!(source, "\"mic.USB\\\\name\\\"input\"");
    }
}
