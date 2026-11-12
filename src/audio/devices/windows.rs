use anyhow::{ensure, Result};
use windows::Win32::{
    Devices::Properties::DEVPKEY_Device_FriendlyName,
    System::{
        Com::{StructuredStorage::PropVariantClear, STGM_READ},
        Variant::VT_LPWSTR,
    },
};

pub(crate) fn name(device: &cpal::Device) -> Result<String> {
    let cpal::platform::DeviceInner::Wasapi(device) = device.as_inner();
    // Read exactly the property used by CPAL 0.16. The newer structured
    // description queries unrelated properties for every enumerated endpoint.
    unsafe {
        let store = device.immdevice().OpenPropertyStore(STGM_READ)?;
        let mut value = store.GetValue(&DEVPKEY_Device_FriendlyName as *const _ as *const _)?;
        let result = (|| {
            let variant = &value.Anonymous.Anonymous;
            ensure!(variant.vt == VT_LPWSTR, "Invalid microphone name type");
            let ptr = variant.Anonymous.pwszVal.0;
            ensure!(!ptr.is_null(), "Missing microphone name");
            let mut len = 0;
            while len < 32768 && *ptr.add(len) != 0 {
                len += 1;
            }
            ensure!(len < 32768, "Unterminated microphone name");
            Ok(String::from_utf16_lossy(std::slice::from_raw_parts(
                ptr, len,
            )))
        })();
        let _ = PropVariantClear(&mut value);
        result
    }
}
